//! Walk-forward evaluation of the bid model.
//!
//! For each recent regular term `T`, every section that was open for bidding in `T` is predicted
//! using only data from before `T`. For a target probability (say 80%) we take the recommended
//! bid and check whether it would have won a seat (section did not fill, or bid >= clearing
//! price). A calibrated model wins ~80% of the time at its 80% bid, and among calibrated models
//! the one with the lower average bid is better.

use crate::data::filter::{GlobalFilters, WindowSel};
use crate::data::model::{forecast_from_rows, ModelParams, MIN_BID};
use crate::data::{AppData, BidRow, RoundKind, Term};
use std::collections::{BTreeMap, HashMap};

pub const LEVELS: [f64; 4] = [0.5, 0.7, 0.8, 0.9];

#[derive(Debug, Clone, Default)]
pub struct LevelResult {
    pub level: f64,
    pub n: usize,
    pub hits: usize,
    pub total_bid: f64,
    /// Pinball (quantile) loss of the bid against the bid that was actually needed.
    pub pinball: f64,
}

impl LevelResult {
    pub fn hit_rate(&self) -> f64 {
        self.hits as f64 / self.n.max(1) as f64
    }
    pub fn mean_bid(&self) -> f64 {
        self.total_bid / self.n.max(1) as f64
    }
    pub fn mean_pinball(&self) -> f64 {
        self.pinball / self.n.max(1) as f64
    }
}

#[derive(Debug, Clone)]
pub struct MethodResult {
    pub name: &'static str,
    pub levels: Vec<LevelResult>,
}

#[derive(Debug, Clone)]
pub struct BacktestReport {
    pub scope: String,
    pub targets: Vec<Term>,
    pub sections: usize,
    pub methods: Vec<MethodResult>,
}

impl BacktestReport {
    pub fn to_text(&self) -> String {
        let mut s = format!(
            "Backtest: {} | {} sections | target terms: {}\n",
            self.scope,
            self.sections,
            self.targets.iter().map(|t| t.label()).collect::<Vec<_>>().join(", ")
        );
        s.push_str(&format!("{:<38} {:>7} {:>10} {:>10} {:>9}\n", "method", "target", "won seat", "avg bid", "loss"));
        for m in &self.methods {
            for l in &m.levels {
                // The naive baseline has no target probability, so no quantile loss either.
                let (target, loss) = if l.level.is_nan() {
                    ("-".to_string(), "-".to_string())
                } else {
                    (format!("{:.0}%", l.level * 100.0), format!("{:.3}", l.mean_pinball()))
                };
                s.push_str(&format!(
                    "{:<38} {:>7} {:>9.1}% {:>10.2} {:>9}\n",
                    m.name,
                    target,
                    l.hit_rate() * 100.0,
                    l.mean_bid(),
                    loss
                ));
            }
        }
        s
    }
}

pub fn run(data: &AppData, scope: Option<WindowSel>, params: &ModelParams) -> BacktestReport {
    let filters = GlobalFilters::default();
    let regular: Vec<Term> = data.terms.iter().copied().filter(|t| !t.kind.is_summer()).collect();
    // Need a few terms of history before a term is worth predicting.
    let targets: Vec<Term> = regular.iter().copied().skip(3).collect();

    let mut new_model = vec![LevelResult::default(); LEVELS.len()];
    let mut old_model = vec![LevelResult::default(); LEVELS.len()];
    let mut last_term = vec![LevelResult::default(); 1];
    let mut sections = 0;

    for &target in &targets {
        // Group the target term's open sections by (course, round, window).
        let mut groups: BTreeMap<(&str, RoundKind, u8), Vec<&BidRow>> = BTreeMap::new();
        for r in data.rows.iter().filter(|r| r.term == target && r.available()) {
            if scope.is_none_or(|s| s.matches(r)) {
                groups.entry((&r.course_code, r.round, r.window)).or_default().push(r);
            }
        }

        for ((course, round, window), outcomes) in groups {
            let sel = WindowSel { round: Some(round), window: Some(window) };
            let history: Vec<&BidRow> = data
                .course_rows(course)
                .filter(|r| r.term < target && sel.matches(r) && filters.allows(r))
                .collect();
            let Some(old) = old_regression(&history) else { continue };
            let Some(naive) = last_term_max(&history) else { continue };
            // The same history-only prior that `model::forecast` uses for this target.
            let prior = data.fill_prior_before(round, window, target).unwrap_or(0.5);

            let mut cache: HashMap<&str, Option<Vec<f64>>> = HashMap::new();
            for r in outcomes {
                let bids = cache
                    .entry(&r.instructor)
                    .or_insert_with(|| {
                        forecast_from_rows(&history, Some(&r.instructor), target, prior, params)
                            .map(|f| LEVELS.iter().map(|&l| f.bid_for(l)).collect())
                    })
                    .clone();
                let Some(bids) = bids else { continue };
                sections += 1;
                for (i, &bid) in bids.iter().enumerate() {
                    record(&mut new_model[i], LEVELS[i], bid, r);
                }
                for (i, &level) in LEVELS.iter().enumerate() {
                    record(&mut old_model[i], level, old.bid_for(level), r);
                }
                record(&mut last_term[0], f64::NAN, naive, r);
            }
        }
    }

    BacktestReport {
        scope: scope.map_or("all rounds & windows".to_string(), |s| s.label()),
        targets,
        sections,
        methods: vec![
            MethodResult { name: "New model (recommended bid)", levels: new_model },
            MethodResult { name: "Old linear regression (v0.1)", levels: old_model },
            MethodResult { name: "Last term's highest clearing price", levels: last_term },
        ],
    }
}

fn record(acc: &mut LevelResult, level: f64, bid: f64, outcome: &BidRow) {
    let bid = bid.max(MIN_BID);
    acc.level = level;
    acc.n += 1;
    acc.total_bid += bid;
    let needed = if outcome.filled() { outcome.min_bid.max(MIN_BID) } else { MIN_BID };
    if bid >= needed {
        acc.hits += 1;
    }
    if !level.is_nan() {
        acc.pinball += (if needed <= bid { 1.0 } else { 0.0 } - level) * (bid - needed);
    }
}

/// The v0.1 approach: OLS of per-term median "Median Bid" against term index, +/- a fixed e$2.
struct OldRegression {
    pred: f64,
}

impl OldRegression {
    fn bid_for(&self, level: f64) -> f64 {
        let z = match level {
            l if l >= 0.9 => 1.28,
            l if l >= 0.8 => 0.84,
            l if l >= 0.7 => 0.52,
            _ => 0.0,
        };
        (self.pred + 2.0 * z).max(0.0)
    }
}

fn old_regression(history: &[&BidRow]) -> Option<OldRegression> {
    let mut by_term: BTreeMap<Term, Vec<f64>> = BTreeMap::new();
    for r in history.iter().filter(|r| r.median_bid > 0.0) {
        by_term.entry(r.term).or_default().push(r.median_bid);
    }
    if by_term.len() < 2 {
        return None;
    }
    let ys: Vec<f64> = by_term
        .into_values()
        .map(|mut v| {
            v.sort_by(f64::total_cmp);
            crate::data::stats::median(&v).unwrap()
        })
        .collect();
    let n = ys.len() as f64;
    let mx = (n - 1.0) / 2.0;
    let my = ys.iter().sum::<f64>() / n;
    let sxy: f64 = ys.iter().enumerate().map(|(i, y)| (i as f64 - mx) * (y - my)).sum();
    let sxx: f64 = (0..ys.len()).map(|i| (i as f64 - mx).powi(2)).sum();
    let slope = sxy / sxx;
    Some(OldRegression { pred: my + slope * (n - mx) })
}

fn last_term_max(history: &[&BidRow]) -> Option<f64> {
    let last = history.iter().filter(|r| r.available()).map(|r| r.term).max()?;
    Some(
        history
            .iter()
            .filter(|r| r.term == last && r.filled())
            .map(|r| r.min_bid)
            .fold(MIN_BID, f64::max),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Descriptions, LoadReport, TermKind};
    use std::sync::Arc;

    fn row(year: u16, kind: TermKind, section: &str, before: i32, after: i32, min: f64) -> BidRow {
        BidRow {
            term: Term { year, kind },
            session: Arc::from("Regular Academic Session"),
            round: RoundKind::Round1,
            window: 1,
            course_code: Arc::from("TEST101"),
            section: Arc::from(section),
            instructor: Arc::from("A"),
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

    /// Regular terms 2022 T1 to 2024 T1 (so the backtest targets are 2023 T2 and 2024 T1), then a
    /// summer 2024 T3A that is never a target and sorts after both. Only its outcomes differ.
    fn dataset(summer_filled: bool) -> AppData {
        let mut rows = Vec::new();
        for (year, kind) in [
            (2022, TermKind::T1),
            (2022, TermKind::T2),
            (2023, TermKind::T1),
            (2023, TermKind::T2),
            (2024, TermKind::T1),
        ] {
            for i in 0..4 {
                let min = 15.0 + 10.0 * i as f64 + 2.0 * f64::from(year - 2022);
                let after = if i % 2 == 0 { 0 } else { 3 };
                rows.push(row(year, kind, &format!("S{i}"), 10, after, min));
            }
        }
        for i in 0..3 {
            let after = if summer_filled { 0 } else { 3 };
            rows.push(row(2024, TermKind::T3A, &format!("U{i}"), 10, after, 30.0));
        }
        AppData::new(rows, Descriptions::new(), LoadReport::default())
    }

    #[test]
    fn backtest_ignores_outcomes_of_later_terms() {
        // Outcomes after every target must not change any backtest number: predictions for a
        // target may only use terms before it.
        let params = ModelParams::default();
        let with_filled = run(&dataset(true), None, &params);
        let with_unfilled = run(&dataset(false), None, &params);
        assert_eq!(with_filled.targets.len(), 2);
        assert!(with_filled.sections > 0);
        assert_eq!(with_filled.to_text(), with_unfilled.to_text());
    }
}
