//! Bid forecasting.
//!
//! The question a student actually has is "if I bid e$X for this section, how likely am I to get
//! a seat?". Each row in the BOSS results answers that for one past section:
//!
//! * if the section **filled** (after-process vacancy hit 0), `Min Bid` is the clearing price:
//!   bids at or above it got in, bids below it did not;
//! * if it **did not fill**, every bid at or above the minimum allowed bid got in, whatever the
//!   reported `Min Bid` says (that is just the lowest bid someone happened to place).
//!
//! So the chance of success with a bid `b` is
//!
//! ```text
//! P(success | b) = P(not filled) + P(filled) * P(clearing price <= b)
//! ```
//!
//! and the model estimates both parts from the course's history:
//!
//! 1. **Weighting.** Every past section in the same round/window gets a weight that decays with
//!    age (half-life `half_life_years`), is lower for a different term type (T1 vs T2 vs summer),
//!    and, when an instructor is chosen, lower for other instructors' sections (partial pooling:
//!    an instructor with little history borrows strength from the course as a whole). Sections
//!    from the same term share a demand shock, so a term's weight is spread over its sections by
//!    `1/sqrt(n)` instead of counting each section as independent evidence.
//! 2. **Fill probability.** Weighted share of filled sections, shrunk toward the fill rate of this
//!    round/window across all courses in the terms before the target (a Beta prior, see
//!    `AppData::fill_prior_before`), so a course with two data points does not claim 0% or 100%.
//! 3. **Trend.** A robust Theil–Sen slope of the per-term median clearing price (in log space,
//!    so it is a % per year). It is shown to the user; shifting past prices by it is supported
//!    (`trend_damping`) but off by default because it did not improve backtest accuracy.
//! 4. **Price distribution.** A weighted Gaussian kernel density over the trend-adjusted log
//!    prices. The bandwidth follows Silverman's rule on the *effective* sample size, so thin
//!    histories give wide, honest uncertainty instead of a fake-precise line.
//!
//! The recommended bid for a target probability is found by inverting `P(success | b)`.
//! `backtest.rs` replays this against held-out terms to check that "80%" really means ~80%.

use crate::data::filter::{GlobalFilters, WindowSel};
use crate::data::{AppData, BidRow, Term, TermKind};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Lowest bid BOSS accepts. Unfilled sections are treated as "clears at the floor".
pub const MIN_BID: f64 = 10.0;

#[derive(Debug, Clone, Copy)]
pub struct ModelParams {
    pub half_life_years: f64,
    pub other_kind_weight: f64,
    pub summer_weight: f64,
    pub other_instructor_weight: f64,
    pub trend_damping: f64,
    pub max_trend: f64,
    pub fill_prior_strength: f64,
    pub min_bandwidth: f64,
}

impl Default for ModelParams {
    fn default() -> Self {
        Self {
            half_life_years: 1.5,
            other_kind_weight: 0.7,
            summer_weight: 0.25,
            other_instructor_weight: 0.3,
            // Walk-forward backtests (`--tune`) found that extrapolating the trend made
            // predictions slightly worse, so it is reported but not applied by default.
            trend_damping: 0.0,
            max_trend: 0.3,
            fill_prior_strength: 2.0,
            min_bandwidth: 0.12,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    Low,
    Medium,
    High,
}

impl Confidence {
    pub fn label(self) -> &'static str {
        match self {
            Confidence::Low => "Low",
            Confidence::Medium => "Medium",
            Confidence::High => "High",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Observation {
    pub term: Term,
    pub section: Arc<str>,
    pub instructor: Arc<str>,
    pub filled: bool,
    pub min_bid: f64,
    pub median_bid: f64,
    pub weight: f64,
    /// Clearing price moved to the target term by the trend (only meaningful when `filled`).
    pub adjusted_price: f64,
}

#[derive(Debug, Clone)]
pub struct Forecast {
    pub target: Term,
    pub observations: Vec<Observation>,
    pub n_terms: usize,
    /// Past sections taught by the chosen instructor (equals `observations.len()` when none chosen).
    pub n_instructor_obs: usize,
    /// Share of past sections that had any seats left to bid for in this window.
    pub availability: f64,
    pub fill_prob: f64,
    /// Estimated change in clearing price per academic year, e.g. 0.08 = +8%/year.
    pub trend_per_year: f64,
    /// Whether (a damped version of) the trend was used to shift past prices.
    pub trend_applied: bool,
    pub effective_n: f64,
    pub confidence: Confidence,
    kernel: Vec<(f64, f64)>,
    bandwidth: f64,
}

pub struct ForecastQuery<'a> {
    pub course: &'a str,
    pub sel: WindowSel,
    pub instructor: Option<&'a str>,
    pub target: Term,
}

pub fn forecast(data: &AppData, filters: &GlobalFilters, q: &ForecastQuery, p: &ModelParams) -> Option<Forecast> {
    let rows: Vec<&BidRow> = data
        .course_rows(q.course)
        .filter(|r| r.term < q.target && q.sel.matches(r) && filters.allows(r))
        .collect();
    // Only terms before the target may inform the prior, so a past target never sees its own outcomes.
    let prior = match (q.sel.round, q.sel.window) {
        (Some(r), Some(w)) => data.fill_prior_before(r, w, q.target),
        _ => None,
    }
    .unwrap_or(0.5);
    forecast_from_rows(&rows, q.instructor, q.target, prior, p)
}

pub fn forecast_from_rows(
    rows: &[&BidRow],
    instructor: Option<&str>,
    target: Term,
    fill_prior: f64,
    p: &ModelParams,
) -> Option<Forecast> {
    let total = rows.len();
    let available: Vec<&BidRow> = rows.iter().copied().filter(|r| r.available()).collect();
    if available.is_empty() {
        return None;
    }

    let mut per_term: BTreeMap<Term, usize> = BTreeMap::new();
    for r in &available {
        *per_term.entry(r.term).or_default() += 1;
    }

    let mut observations: Vec<Observation> = available
        .iter()
        .map(|r| {
            let age = (target.time() - r.term.time()).max(0.0);
            let mut w = 0.5f64.powf(age / p.half_life_years);
            w *= term_kind_weight(r.term.kind, target.kind, p);
            if let Some(name) = instructor {
                if &*r.instructor != name {
                    w *= p.other_instructor_weight;
                }
            }
            w /= (per_term[&r.term] as f64).sqrt();
            Observation {
                term: r.term,
                section: r.section.clone(),
                instructor: r.instructor.clone(),
                filled: r.filled(),
                min_bid: r.min_bid,
                median_bid: r.median_bid,
                weight: w,
                adjusted_price: r.min_bid.max(MIN_BID),
            }
        })
        .collect();

    let w_total: f64 = observations.iter().map(|o| o.weight).sum();
    let w_filled: f64 = observations.iter().filter(|o| o.filled).map(|o| o.weight).sum();
    let fill_prob = (w_filled + p.fill_prior_strength * fill_prior) / (w_total + p.fill_prior_strength);

    // Trend from per-term medians of the clearing price of filled sections.
    let mut term_prices: BTreeMap<Term, Vec<(f64, f64)>> = BTreeMap::new();
    for o in observations.iter().filter(|o| o.filled) {
        term_prices.entry(o.term).or_default().push((o.adjusted_price.ln(), o.weight));
    }
    let term_medians: Vec<(f64, f64)> =
        term_prices.iter_mut().map(|(t, v)| (t.time(), weighted_median(v))).collect();
    let raw_slope = theil_sen(&term_medians, 0.9).unwrap_or(0.0).clamp(-p.max_trend, p.max_trend);
    let slope = raw_slope * p.trend_damping;

    for o in &mut observations {
        o.adjusted_price = (o.adjusted_price.ln() + slope * (target.time() - o.term.time())).exp();
    }

    // Kernel over trend-adjusted log prices. Without any filled section we fall back to the
    // reported minimum bids (a loose upper bound) with a deliberately wide kernel.
    let mut kernel: Vec<(f64, f64)> = observations
        .iter()
        .filter(|o| o.filled)
        .map(|o| (o.adjusted_price.ln(), o.weight))
        .collect();
    let mut min_bw = p.min_bandwidth;
    if kernel.is_empty() {
        kernel = observations
            .iter()
            .filter(|o| o.min_bid > 0.0)
            .map(|o| (o.adjusted_price.ln(), o.weight))
            .collect();
        min_bw = 0.5;
    }
    if kernel.is_empty() {
        kernel.push(((MIN_BID * 1.5).ln(), 1.0));
        min_bw = 0.6;
    }

    let effective_n = effective_n(&kernel);
    let (_, sd) = weighted_mean_sd(&kernel);
    let mut bandwidth = (1.06 * sd * effective_n.powf(-0.2)).max(min_bw);
    if effective_n < 3.0 {
        bandwidth = bandwidth.max(0.25);
    }

    let n_terms = per_term.len();
    let n_instructor_obs = match instructor {
        Some(name) => observations.iter().filter(|o| &*o.instructor == name).count(),
        None => observations.len(),
    };
    let filled_terms = term_prices.len();
    let confidence = if filled_terms >= 3 && effective_n >= 4.0 && n_instructor_obs >= 2 {
        Confidence::High
    } else if n_terms >= 2 && effective_n >= 2.0 {
        Confidence::Medium
    } else {
        Confidence::Low
    };

    Some(Forecast {
        target,
        n_terms,
        n_instructor_obs,
        availability: available.len() as f64 / total.max(1) as f64,
        fill_prob,
        trend_per_year: raw_slope.exp() - 1.0,
        trend_applied: p.trend_damping > 0.0,
        effective_n,
        confidence,
        observations,
        kernel,
        bandwidth,
    })
}

impl Forecast {
    /// P(clearing price <= bid | section fills).
    pub fn price_cdf(&self, bid: f64) -> f64 {
        if bid <= 0.0 {
            return 0.0;
        }
        let x = bid.ln();
        let (num, den) = self
            .kernel
            .iter()
            .fold((0.0, 0.0), |(n, d), &(mu, w)| (n + w * normal_cdf((x - mu) / self.bandwidth), d + w));
        if den > 0.0 { num / den } else { 0.0 }
    }

    /// Probability that a bid of `bid` wins a seat (given seats are open in this window).
    pub fn success_prob(&self, bid: f64) -> f64 {
        if bid < MIN_BID {
            return 0.0;
        }
        (1.0 - self.fill_prob) + self.fill_prob * self.price_cdf(bid)
    }

    /// Smallest bid with at least `prob` chance of success.
    pub fn bid_for(&self, prob: f64) -> f64 {
        let prob = prob.clamp(0.01, 0.995);
        if self.success_prob(MIN_BID) >= prob {
            return MIN_BID;
        }
        let needed = (prob - (1.0 - self.fill_prob)) / self.fill_prob.max(1e-9);
        self.price_quantile(needed).max(MIN_BID)
    }

    /// Quantile of the clearing price, given that the section fills.
    pub fn price_quantile(&self, q: f64) -> f64 {
        let q = q.clamp(0.001, 0.999);
        let max_mu = self.kernel.iter().map(|k| k.0).fold(f64::MIN, f64::max);
        let min_mu = self.kernel.iter().map(|k| k.0).fold(f64::MAX, f64::min);
        let (mut lo, mut hi) = (min_mu - 6.0 * self.bandwidth, max_mu + 6.0 * self.bandwidth);
        for _ in 0..60 {
            let mid = 0.5 * (lo + hi);
            if self.price_cdf(mid.exp()) < q { lo = mid } else { hi = mid }
        }
        (0.5 * (lo + hi)).exp()
    }

    /// Median clearing price when the section fills, if any past section filled.
    pub fn median_clearing(&self) -> Option<f64> {
        self.observations.iter().any(|o| o.filled).then(|| self.price_quantile(0.5))
    }

    /// Upper end for charts: comfortably past the 99% bid.
    pub fn chart_max_bid(&self) -> f64 {
        (self.bid_for(0.99) * 1.15).max(MIN_BID * 2.0)
    }
}

fn term_kind_weight(obs: TermKind, target: TermKind, p: &ModelParams) -> f64 {
    if obs == target {
        1.0
    } else if obs.is_summer() != target.is_summer() {
        p.summer_weight
    } else {
        p.other_kind_weight
    }
}

pub fn normal_cdf(z: f64) -> f64 {
    0.5 * erfc(-z / std::f64::consts::SQRT_2)
}

/// Complementary error function (Numerical Recipes `erfcc`, |error| < 1.2e-7).
fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let r = t * (-z * z - 1.26551223
        + t * (1.00002368
            + t * (0.37409196
                + t * (0.09678418
                    + t * (-0.18628806
                        + t * (0.27886807 + t * (-1.13520398 + t * (1.48851587 + t * (-0.82215223 + t * 0.17087277)))))))))
        .exp();
    if x >= 0.0 { r } else { 2.0 - r }
}

pub fn weighted_median(v: &mut [(f64, f64)]) -> f64 {
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: f64 = v.iter().map(|x| x.1).sum();
    let mut acc = 0.0;
    for &(x, w) in v.iter() {
        acc += w;
        if acc >= total / 2.0 {
            return x;
        }
    }
    v.last().map(|x| x.0).unwrap_or(0.0)
}

fn weighted_mean_sd(v: &[(f64, f64)]) -> (f64, f64) {
    let w: f64 = v.iter().map(|x| x.1).sum();
    if w <= 0.0 {
        return (0.0, 0.0);
    }
    let mean = v.iter().map(|x| x.0 * x.1).sum::<f64>() / w;
    let var = v.iter().map(|x| x.1 * (x.0 - mean).powi(2)).sum::<f64>() / w;
    (mean, var.sqrt())
}

fn effective_n(v: &[(f64, f64)]) -> f64 {
    let s: f64 = v.iter().map(|x| x.1).sum();
    let s2: f64 = v.iter().map(|x| x.1 * x.1).sum();
    if s2 > 0.0 { s * s / s2 } else { 0.0 }
}

/// Median of pairwise slopes between points at least `min_dx` apart. Needs 3+ points.
fn theil_sen(points: &[(f64, f64)], min_dx: f64) -> Option<f64> {
    if points.len() < 3 {
        return None;
    }
    let mut slopes = Vec::new();
    for i in 0..points.len() {
        for j in i + 1..points.len() {
            let dx = points[j].0 - points[i].0;
            if dx.abs() >= min_dx {
                slopes.push((points[j].1 - points[i].1) / dx);
            }
        }
    }
    if slopes.len() < 2 {
        return None;
    }
    slopes.sort_by(f64::total_cmp);
    let m = slopes.len() / 2;
    Some(if slopes.len().is_multiple_of(2) { (slopes[m - 1] + slopes[m]) / 2.0 } else { slopes[m] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Descriptions, LoadReport, RoundKind};

    fn row(year: u16, kind: TermKind, section: &str, instr: &str, before: i32, after: i32, min: f64) -> BidRow {
        BidRow {
            term: Term { year, kind },
            session: Arc::from("Regular Academic Session"),
            round: RoundKind::Round1,
            window: 1,
            course_code: Arc::from("TEST101"),
            section: Arc::from(section),
            instructor: Arc::from(instr),
            school: Arc::from("SCIS"),
            vacancy: 40,
            opening_vacancy: 10,
            before_proc: before,
            dice: 0,
            after_proc: after,
            enrolled: before - after,
            median_bid: min * 1.2,
            min_bid: min,
        }
    }

    #[test]
    fn erfc_is_accurate() {
        assert!((normal_cdf(0.0) - 0.5).abs() < 1e-7);
        assert!((normal_cdf(1.0) - 0.841_344_746).abs() < 1e-6);
        assert!((normal_cdf(-1.96) - 0.024_997_895).abs() < 1e-6);
    }

    #[test]
    fn success_curve_is_monotone_and_inverts() {
        let rows: Vec<BidRow> = (2022..2026)
            .flat_map(|y| {
                [TermKind::T1, TermKind::T2].into_iter().flat_map(move |k| {
                    let base = 30.0 + (y - 2022) as f64 * 3.0;
                    vec![row(y, k, "G1", "A", 10, 0, base), row(y, k, "G2", "B", 10, 0, base * 1.3), row(y, k, "G3", "C", 10, 4, 12.0)]
                })
            })
            .collect();
        let refs: Vec<&BidRow> = rows.iter().collect();
        let f = forecast_from_rows(&refs, None, Term { year: 2026, kind: TermKind::T1 }, 0.5, &ModelParams::default()).unwrap();
        let mut prev = 0.0;
        for b in (10..200).map(|b| b as f64) {
            let p = f.success_prob(b);
            assert!(p >= prev - 1e-12);
            prev = p;
        }
        for target in [0.5, 0.8, 0.9] {
            let b = f.bid_for(target);
            assert!((f.success_prob(b) - target).abs() < 0.01, "target {target} bid {b} got {}", f.success_prob(b));
        }
        assert!(f.trend_per_year > 0.0, "rising prices should give a positive trend");
        assert!(f.fill_prob > 0.5 && f.fill_prob < 0.8);
    }

    #[test]
    fn instructor_filter_shifts_prediction() {
        let rows: Vec<BidRow> = (2022..2026)
            .flat_map(|y| vec![row(y, TermKind::T1, "G1", "CHEAP", 10, 0, 15.0), row(y, TermKind::T1, "G2", "POPULAR", 10, 0, 60.0)])
            .collect();
        let refs: Vec<&BidRow> = rows.iter().collect();
        let t = Term { year: 2026, kind: TermKind::T1 };
        let p = ModelParams::default();
        let cheap = forecast_from_rows(&refs, Some("CHEAP"), t, 0.5, &p).unwrap();
        let popular = forecast_from_rows(&refs, Some("POPULAR"), t, 0.5, &p).unwrap();
        assert!(cheap.bid_for(0.8) < popular.bid_for(0.8));
    }

    /// TEST101 and OTHER1 in Round 1 window 1, 12 sections per term: 2022 T1 all unfilled,
    /// 2023 T1 all filled, 2023 T2 all unfilled. Whole-dataset prior 12/36; before 2023 T1 it is 0/12.
    fn prior_test_rows() -> Vec<BidRow> {
        let mut rows = Vec::new();
        for (year, kind, after) in [(2022, TermKind::T1, 4), (2023, TermKind::T1, 0), (2023, TermKind::T2, 4)] {
            for i in 0..12 {
                let mut r = row(year, kind, &format!("G{i}"), "A", 10, after, 20.0);
                if i >= 3 {
                    r.course_code = Arc::from("OTHER1");
                }
                rows.push(r);
            }
        }
        rows
    }

    #[test]
    fn default_target_forecast_uses_whole_dataset_prior() {
        // Predictions for the next term must be exactly what they were with the whole-dataset prior.
        let data = AppData::new(prior_test_rows(), Descriptions::new(), LoadReport::default());
        let p = ModelParams::default();
        let target = data.default_target_term();
        let sel = WindowSel::ROUND1_W1;
        let q = ForecastQuery { course: "TEST101", sel, instructor: None, target };
        let got = forecast(&data, &GlobalFilters::default(), &q, &p).unwrap();
        let history: Vec<&BidRow> = data.course_rows("TEST101").filter(|r| sel.matches(r)).collect();
        let full_prior = data.fill_prior[&(RoundKind::Round1, 1)];
        let expect = forecast_from_rows(&history, None, target, full_prior, &p).unwrap();
        assert_eq!(got.fill_prob, expect.fill_prob);
        for level in [0.5, 0.8, 0.9] {
            assert_eq!(got.bid_for(level), expect.bid_for(level), "level {level}");
        }
    }

    #[test]
    fn past_target_forecast_uses_only_earlier_prior() {
        let data = AppData::new(prior_test_rows(), Descriptions::new(), LoadReport::default());
        let p = ModelParams::default();
        let past = Term { year: 2023, kind: TermKind::T1 };
        let q = ForecastQuery { course: "TEST101", sel: WindowSel::ROUND1_W1, instructor: None, target: past };
        let got = forecast(&data, &GlobalFilters::default(), &q, &p).unwrap();
        // Before 2023 T1 nothing filled, so the prior is 0 and TEST101's own unfilled history gives 0.
        assert_eq!(got.fill_prob, 0.0);
        // The whole-dataset prior (which includes 2023 T1's own outcomes) would have given more.
        let history: Vec<&BidRow> = data.course_rows("TEST101").filter(|r| r.term < past).collect();
        let full_prior = data.fill_prior[&(RoundKind::Round1, 1)];
        let with_full_prior = forecast_from_rows(&history, None, past, full_prior, &p).unwrap();
        assert!(with_full_prior.fill_prob > got.fill_prob);
    }
}
