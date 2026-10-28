//! The query layer shared by the GUI and the command line.
//!
//! Everything returned here is plain data that derives `Serialize`; nothing depends on egui.
//! Output structs are rounded when built: money to cents, probabilities and shares to four
//! decimals. `InstructorStats` is the exception: the Instructors tab reads it directly, so it keeps
//! full precision and rounds only when serialised (see `ser`).

use crate::data::filter::{course_rows, instructors_for_course, windows_for_course, GlobalFilters, WindowSel};
use crate::data::model::{forecast, Confidence, Forecast, ForecastQuery, ModelParams, Observation, MIN_BID};
use crate::data::stats::{instructor_ranking, median, term_summaries, InstructorStats, TermSummary};
use crate::data::{AppData, BidRow, CourseInfo, Term};
use serde::Serialize;
use std::collections::BTreeMap;

/// The chances the bid planner reports by default.
pub const DEFAULT_LEVELS: [f64; 5] = [0.5, 0.7, 0.8, 0.9, 0.95];

/// The fewest comparable sections an instructor needs before the instructor rankings show them. Below
/// about ten comparisons, a few unusual sections can swing the premium a long way.
pub const DEFAULT_MIN_COMPARED: usize = 10;

/// The fewest filled sections a course needs to be ranked among the most contested courses. One filled
/// section is one price, so a single section could otherwise put a course at the top on its own.
pub const DEFAULT_HOT_MIN_FILLED: usize = 2;

/// Rounds to two decimals exactly as `{:.2}` displays them, so rounded money prints as it did before.
pub fn round2(x: f64) -> f64 {
    format!("{x:.2}").parse().unwrap_or(x)
}

/// Rounds to four decimals exactly as `{:.4}` displays them.
pub fn round4(x: f64) -> f64 {
    format!("{x:.4}").parse().unwrap_or(x)
}

/// Bids go up to the next cent, so a rounded bid never has a lower chance than the exact one.
fn bid_cents(x: f64) -> f64 {
    ((x * 100.0) - 1e-9).ceil() / 100.0
}

/// Serialisers that round values written to JSON, for fields that must keep full precision in Rust.
pub mod ser {
    use super::{round2, round4};
    use crate::data::Term;
    use serde::{Serialize, Serializer};

    pub fn ratio<S: Serializer>(x: &f64, s: S) -> Result<S::Ok, S::Error> {
        round4(*x).serialize(s)
    }

    pub fn opt_money<S: Serializer>(x: &Option<f64>, s: S) -> Result<S::Ok, S::Error> {
        x.map(round2).serialize(s)
    }

    pub fn term<S: Serializer>(t: &Term, s: S) -> Result<S::Ok, S::Error> {
        t.label().serialize(s)
    }
}

/// Sort key that puts "G2" before "G10".
pub fn natural_section(s: &str) -> (String, u32) {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    let prefix: String = s.chars().filter(|c| !c.is_ascii_digit()).collect();
    (prefix, digits.parse().unwrap_or(0))
}

/// The exact-code form of user input: no whitespace, in capitals. "is 216" becomes "IS216".
fn compact_code(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_uppercase()
}

/// The form codes and titles are compared in: lower case, with whitespace, hyphens, dashes, underscores
/// and dots left out. "COR-COMM1304", "cor comm1304" and "corcomm1304" all become "corcomm1304".
fn course_key(s: &str) -> String {
    s.chars()
        .filter(|c| !(c.is_whitespace() || matches!(c, '-' | '_' | '.' | '\u{2010}'..='\u{2015}')))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Indices into `data.courses`, best match first. An empty query lists every course in the school;
/// a fuzzy query matches code and title, with exact code prefixes first, and returns at most 200.
/// The query also matches with spaces and hyphens left out of both sides, so "cor comm1304" finds
/// COR-COMM1304.
pub fn search_courses(data: &AppData, query: &str, school: Option<&str>) -> Vec<usize> {
    let in_school = |i: &usize| school.is_none_or(|s| &*data.courses[*i].school == s);
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return (0..data.courses.len()).filter(in_school).collect();
    }
    let key = course_key(&query);

    let mut matcher = nucleo::Matcher::new(nucleo::Config::DEFAULT);
    let needle = nucleo::Utf32String::from(query.as_str());
    let key_needle = nucleo::Utf32String::from(key.as_str());
    let mut scored: Vec<(u16, usize)> = (0..data.courses.len())
        .filter(in_school)
        .filter_map(|i| {
            let c = &data.courses[i];
            let text = format!("{} {}", c.code, c.description).to_lowercase();
            let as_typed = matcher.fuzzy_match(nucleo::Utf32String::from(text.as_str()).slice(..), needle.slice(..));
            let joined = if key.is_empty() {
                None
            } else {
                let hay = nucleo::Utf32String::from(course_key(&text).as_str());
                matcher.fuzzy_match(hay.slice(..), key_needle.slice(..))
            };
            let mut score = as_typed.into_iter().chain(joined).max()?;
            // Exact course-code prefix matches go first ("is1" -> IS111, IS112...), with or without a hyphen.
            let code = c.code.to_lowercase();
            if code.starts_with(&query) || (!key.is_empty() && course_key(&code).starts_with(&key)) {
                score = score.saturating_add(1000);
            }
            Some((score, i))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, i)| i).take(200).collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct CourseHit {
    pub code: String,
    pub description: String,
    pub school: String,
    pub last_term: String,
}

/// `search_courses`, cut to `limit` results and returned as plain values.
pub fn course_hits(data: &AppData, query: &str, school: Option<&str>, limit: usize) -> Vec<CourseHit> {
    search_courses(data, query, school)
        .into_iter()
        .take(limit)
        .map(|i| {
            let c = &data.courses[i];
            CourseHit {
                code: c.code.to_string(),
                description: c.description.clone(),
                school: c.school.to_string(),
                last_term: c.last_term.label(),
            }
        })
        .collect()
}

/// Finds a course by code, ignoring case, spaces and hyphens ("is 216" is IS216, "cor comm1304" is
/// COR-COMM1304). An exact code wins. On a miss, returns up to five close matches; if two codes agree
/// once hyphens are left out, both are returned as the choices.
pub fn resolve_course<'a>(data: &'a AppData, input: &str) -> Result<&'a CourseInfo, Vec<String>> {
    let key = course_key(input);
    if key.is_empty() {
        return Err(Vec::new());
    }
    if let Some(c) = data.course(&compact_code(input)) {
        return Ok(c);
    }
    let same: Vec<&'a CourseInfo> = data.courses.iter().filter(|c| course_key(&c.code) == key).collect();
    match same.as_slice() {
        [one] => Ok(*one),
        [] => Err(search_courses(data, input, None).into_iter().take(5).map(|i| data.courses[i].code.to_string()).collect()),
        many => Err(many.iter().map(|c| c.code.to_string()).collect()),
    }
}

/// Finds an instructor who has taught `course`. An exact name (ignoring case) wins, then a unique
/// substring. Otherwise the error lists the substring matches, or every instructor of the course if
/// none match.
pub fn resolve_instructor(data: &AppData, course: &str, input: &str) -> Result<String, Vec<String>> {
    let names = match resolve_course(data, course) {
        Ok(c) => instructors_for_course(data, &c.code),
        Err(_) => Vec::new(),
    };
    let needle = input.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    if needle.is_empty() {
        return Err(names);
    }
    if let Some(name) = names.iter().find(|n| n.to_lowercase() == needle) {
        return Ok(name.clone());
    }
    let mut matches: Vec<String> = names.iter().filter(|n| n.to_lowercase().contains(&needle)).cloned().collect();
    if matches.len() == 1 {
        return Ok(matches.swap_remove(0));
    }
    Err(if matches.is_empty() { names } else { matches })
}

#[derive(Debug, Clone, Serialize)]
pub struct HotCourse {
    pub code: String,
    pub description: String,
    /// Median clearing price of the sections that filled.
    pub median_clearing: f64,
    pub sections_filled: usize,
    pub sections_available: usize,
}

/// The latest term that is not a summer term, or `None` when the data has no regular term. `hot` ranks
/// it by default, and `plan` warns about a course that was not offered in it.
pub fn latest_regular_term(data: &AppData) -> Option<Term> {
    data.terms.iter().rev().find(|t| !t.kind.is_summer()).copied()
}

/// Courses ranked by the median clearing price of their filled sections in one term and window.
/// Courses where nothing filled are left out.
pub fn hot_courses(data: &AppData, filters: &GlobalFilters, term: Term, sel: WindowSel) -> Vec<HotCourse> {
    hot_courses_with_min_filled(data, filters, term, sel, 1)
}

/// `hot_courses`, keeping only the courses with at least `min_filled` filled sections. One filled section
/// is one price, so the `hot` command asks for two by default.
pub fn hot_courses_with_min_filled(
    data: &AppData,
    filters: &GlobalFilters,
    term: Term,
    sel: WindowSel,
    min_filled: usize,
) -> Vec<HotCourse> {
    let mut by_course: BTreeMap<&str, (Vec<f64>, usize)> = BTreeMap::new();
    for r in data.rows.iter().filter(|r| r.term == term && sel.matches(r) && r.available() && filters.allows(r)) {
        let e = by_course.entry(&r.course_code).or_default();
        e.1 += 1;
        if r.filled() {
            e.0.push(r.min_bid.max(MIN_BID));
        }
    }
    // Sort on the exact medians, then round, so the order never depends on rounding.
    let mut v: Vec<(f64, HotCourse)> = by_course
        .into_iter()
        .filter_map(|(code, (mut prices, available))| {
            if prices.len() < min_filled {
                return None;
            }
            prices.sort_by(f64::total_cmp);
            let clearing = median(&prices)?;
            let course = HotCourse {
                code: code.to_string(),
                description: data.course(code).map(|c| c.description.clone()).unwrap_or_default(),
                median_clearing: round2(clearing),
                sections_filled: prices.len(),
                sections_available: available,
            };
            Some((clearing, course))
        })
        .collect();
    v.sort_by(|a, b| b.0.total_cmp(&a.0));
    v.into_iter().map(|(_, c)| c).collect()
}

/// One term of a course. Two units appear, so the field docs say which: `sections` counts each section
/// once, the rest count section-windows (one section in one bidding window).
#[derive(Debug, Clone, Serialize)]
pub struct TermSummaryOut {
    pub term: String,
    /// Distinct sections (by session and name). A section offered in several windows counts once.
    pub sections: usize,
    /// Section-windows in the term, open or not. Equal to `sections` for one window.
    pub section_windows: usize,
    /// Section-windows where seats were open for bidding.
    pub available: usize,
    /// Section-windows that filled.
    pub filled: usize,
    /// `filled` over `available`, or 0 when no seats were open.
    pub fill_rate: f64,
    /// Seats open before bidding, summed over the open section-windows.
    pub seats_open: i32,
    /// Seats left after bidding, summed over the open section-windows.
    pub seats_left: i32,
    pub clearing_median: Option<f64>,
    pub clearing_low: Option<f64>,
    pub clearing_high: Option<f64>,
    pub median_bid: Option<f64>,
}

impl From<&TermSummary> for TermSummaryOut {
    fn from(t: &TermSummary) -> Self {
        TermSummaryOut {
            term: t.term.label(),
            sections: t.sections,
            section_windows: t.section_windows,
            available: t.available,
            filled: t.filled,
            fill_rate: round4(t.fill_rate()),
            seats_open: t.seats_open,
            seats_left: t.seats_left,
            clearing_median: t.clearing_median.map(round2),
            clearing_low: t.clearing_low.map(round2),
            clearing_high: t.clearing_high.map(round2),
            median_bid: t.median_bid.map(round2),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SectionOut {
    pub term: String,
    /// The BOSS session the section ran in, e.g. "Regular Academic Session" or "1A Session". The same
    /// section name can appear in two sessions of one term.
    pub session: String,
    pub window_code: String,
    pub section: String,
    pub instructor: String,
    pub vacancy: i32,
    pub before_proc: i32,
    pub after_proc: i32,
    pub enrolled: i32,
    pub filled: bool,
    pub available: bool,
    /// Zero when BOSS reported no minimum bid.
    pub min_bid: f64,
    /// Zero when BOSS reported no median bid.
    pub median_bid: f64,
}

fn section_out(r: &BidRow) -> SectionOut {
    SectionOut {
        term: r.term.label(),
        session: r.session.to_string(),
        window_code: WindowSel { round: Some(r.round), window: Some(r.window) }.short_label(),
        section: r.section.to_string(),
        instructor: r.instructor.to_string(),
        vacancy: r.vacancy,
        before_proc: r.before_proc,
        after_proc: r.after_proc,
        enrolled: r.enrolled,
        filled: r.filled(),
        available: r.available(),
        min_bid: round2(r.min_bid),
        median_bid: round2(r.median_bid),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CourseReport {
    pub code: String,
    pub description: String,
    pub school: String,
    pub last_term: String,
    pub window: String,
    pub window_code: String,
    /// Short codes such as "R1W1" for every window the course has been offered in.
    pub windows_offered: Vec<String>,
    /// Everyone who has taught the course, most recent first.
    pub instructors: Vec<String>,
    pub terms: Vec<TermSummaryOut>,
    /// One row per section-window. Newest term first, then round, window, section and session.
    pub sections: Vec<SectionOut>,
}

/// Everything the data says about one course in one window (or all of them). `None` if the course is unknown.
pub fn course_report(data: &AppData, filters: &GlobalFilters, code: &str, sel: WindowSel) -> Option<CourseReport> {
    let info = resolve_course(data, code).ok()?;
    let rows = course_rows(data, &info.code, sel, filters);
    let terms: Vec<TermSummaryOut> = term_summaries(&rows).iter().map(TermSummaryOut::from).collect();
    let mut sections = rows.clone();
    sections.sort_by(|a, b| {
        b.term
            .cmp(&a.term)
            .then(a.round.cmp(&b.round))
            .then(a.window.cmp(&b.window))
            .then_with(|| natural_section(&a.section).cmp(&natural_section(&b.section)))
            .then_with(|| a.section.cmp(&b.section))
            .then_with(|| a.session.cmp(&b.session))
    });
    Some(CourseReport {
        code: info.code.to_string(),
        description: info.description.clone(),
        school: info.school.to_string(),
        last_term: info.last_term.label(),
        window: sel.label(),
        window_code: sel.short_label(),
        windows_offered: windows_for_course(data, &info.code)
            .into_iter()
            .map(|(round, window)| WindowSel { round: Some(round), window: Some(window) }.short_label())
            .collect(),
        instructors: instructors_for_course(data, &info.code),
        terms,
        sections: sections.into_iter().map(section_out).collect(),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct LevelBid {
    pub probability: f64,
    pub bid: f64,
    /// The forecast needs no more than the minimum bid for this chance.
    pub is_minimum_bid: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct BidCheck {
    pub bid: f64,
    pub probability: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObservationOut {
    pub term: String,
    pub section: String,
    pub instructor: String,
    pub filled: bool,
    pub min_bid: f64,
    pub median_bid: f64,
    /// This past section's share of the total weight behind the forecast.
    pub weight_share: f64,
}

/// Percentiles of the forecast clearing price, given that the section fills (the GUI planner's band).
#[derive(Debug, Clone, Serialize)]
pub struct ClearingQuantiles {
    pub p10: f64,
    pub p25: f64,
    pub p50: f64,
    pub p75: f64,
    pub p90: f64,
}

fn clearing_quantiles(f: &Forecast) -> Option<ClearingQuantiles> {
    any_filled(f).then(|| {
        let [p10, p25, p50, p75, p90] = [0.1, 0.25, 0.5, 0.75, 0.9].map(|q| round2(f.price_quantile(q)));
        ClearingQuantiles { p10, p25, p50, p75, p90 }
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct BidPlan {
    pub course: String,
    pub description: String,
    pub window: String,
    pub window_code: String,
    pub instructor: Option<String>,
    pub target: String,
    /// One bid per requested chance, in the order requested.
    pub recommended: Vec<LevelBid>,
    /// The chance of a seat at each bid the caller asked about.
    pub bid_checks: Vec<BidCheck>,
    pub fill_probability: f64,
    pub availability: f64,
    /// Price change per year, e.g. 0.08 for +8%.
    pub trend_per_year: f64,
    pub trend_applied: bool,
    pub median_clearing: Option<f64>,
    /// The weighted clearing-price distribution when the section fills. Null when no past section filled.
    pub clearing_quantiles: Option<ClearingQuantiles>,
    /// "Low", "Medium" or "High". Always "Low" when no past section filled, because there is no clearing price.
    pub confidence: String,
    pub n_observations: usize,
    pub n_terms: usize,
    pub n_instructor_obs: usize,
    pub effective_n: f64,
    pub observations: Vec<ObservationOut>,
    pub warnings: Vec<String>,
}

fn observation_rows(f: &Forecast) -> Vec<ObservationOut> {
    let total: f64 = f.observations.iter().map(|o| o.weight).sum();
    let mut obs: Vec<&Observation> = f.observations.iter().collect();
    obs.sort_by(|a, b| b.term.cmp(&a.term).then(b.weight.total_cmp(&a.weight)));
    obs.into_iter()
        .map(|o| ObservationOut {
            term: o.term.label(),
            section: o.section.to_string(),
            instructor: o.instructor.to_string(),
            filled: o.filled,
            min_bid: round2(o.min_bid),
            median_bid: round2(o.median_bid),
            weight_share: round4(if total > 0.0 { o.weight / total } else { 0.0 }),
        })
        .collect()
}

/// The bids for a course, window and target term. `None` only if the course is unknown. With no
/// earlier history for the window there is no forecast: `recommended` and `bid_checks` are empty,
/// the numeric forecast fields are zero, and `warnings` says why. A chance or bid that is not a
/// number is left out, with a warning, because it cannot be priced and would print as null.
///
/// The warnings also cover a course not offered since the latest regular term, a window where no past
/// section filled (the price then falls back to the reported Min Bids, and the confidence is Low), and
/// history that `--exclude-session` removed.
pub fn plan_bid(
    data: &AppData,
    filters: &GlobalFilters,
    params: &ModelParams,
    q: &ForecastQuery,
    levels: &[f64],
    bids: &[f64],
) -> Option<BidPlan> {
    let info = resolve_course(data, q.course).ok()?;
    let sel = q.sel;
    // The instructor must match a stored name exactly; `resolve_instructor` finds it from user input.
    let q = ForecastQuery { course: &info.code, sel, instructor: q.instructor, target: q.target };
    let fc = forecast(data, filters, &q, params);

    let all_rows = course_rows(data, &info.code, sel, &GlobalFilters::default()).len();
    let kept_rows = course_rows(data, &info.code, sel, filters).len();
    let mut warnings = Vec::new();
    if all_rows == 0 {
        warnings.push(format!("{} has no results for {}, so there is no history to base a bid on.", info.code, sel.label()));
    }
    warnings.extend(exclusion_warnings(filters, &info.code, sel, all_rows, kept_rows));
    // No offering in the latest regular term or later, so the course may not run in the target term.
    let not_in_latest = latest_regular_term(data).is_some_and(|last| info.last_term < last);
    if not_in_latest && info.last_term < q.target {
        warnings.push(format!(
            "{} was last offered in {}; it may not be offered in {}.",
            info.code,
            info.last_term.label(),
            q.target.label()
        ));
    }
    match &fc {
        Some(f) => {
            if let Some(name) = q.instructor.filter(|_| f.n_instructor_obs == 0) {
                warnings.push(format!("{name} has no history in this window; using course-wide data."));
            }
            if !any_filled(f) {
                warnings.push(format!(
                    "No past section filled in {}, so the price estimate falls back to the reported Min Bids, which are a loose upper bound. The real risk is mostly whether the section fills.",
                    window_words(sel)
                ));
            } else if f.confidence == Confidence::Low {
                warnings.push(format!(
                    "Low confidence: only {} past section(s) across {} term(s); treat these bids as rough.",
                    f.observations.len(),
                    f.n_terms
                ));
            }
        }
        None if kept_rows > 0 => warnings.push(format!(
            "No section in {} had seats open before {}, so there is no history to base a bid on.",
            sel.label(),
            q.target.label()
        )),
        None => {}
    }
    if !sel.is_specific() {
        warnings.push("Prices differ a lot between windows; pick a specific round and window for meaningful numbers.".to_string());
    }
    // A chance or bid that is not a number cannot be priced, and would print as null, so it is left out.
    if levels.iter().any(|p| !p.is_finite()) {
        warnings.push("Some chances are not numbers, so they were left out.".to_string());
    }
    if bids.iter().any(|b| !b.is_finite()) {
        warnings.push("Some bids are not amounts, so they were left out.".to_string());
    }
    let levels: Vec<f64> = levels.iter().copied().filter(|p| p.is_finite()).collect();
    let bids: Vec<f64> = bids.iter().copied().filter(|b| b.is_finite()).collect();

    let (recommended, bid_checks, observations) = match &fc {
        Some(f) => {
            let recommended: Vec<LevelBid> = levels
                .iter()
                .map(|&p| {
                    let raw = f.bid_for(p);
                    LevelBid { probability: round4(p), bid: bid_cents(raw), is_minimum_bid: raw <= MIN_BID + 1e-9 }
                })
                .collect();
            let bid_checks: Vec<BidCheck> = bids
                .iter()
                .map(|&b| {
                    let bid = round2(b);
                    BidCheck { bid, probability: round4(f.success_prob(bid)) }
                })
                .collect();
            (recommended, bid_checks, observation_rows(f))
        }
        None => (Vec::new(), Vec::new(), Vec::new()),
    };
    let f = fc.as_ref();
    Some(BidPlan {
        course: info.code.to_string(),
        description: info.description.clone(),
        window: sel.label(),
        window_code: sel.short_label(),
        instructor: q.instructor.map(str::to_string),
        target: q.target.label(),
        recommended,
        bid_checks,
        fill_probability: f.map_or(0.0, |f| round4(f.fill_prob)),
        availability: f.map_or(0.0, |f| round4(f.availability)),
        trend_per_year: f.map_or(0.0, |f| round4(f.trend_per_year)),
        trend_applied: f.is_some_and(|f| f.trend_applied),
        median_clearing: f.and_then(|f| f.median_clearing()).map(round2),
        clearing_quantiles: f.and_then(clearing_quantiles),
        confidence: f.map_or("Low", |f| reported_confidence(f).label()).to_string(),
        n_observations: f.map_or(0, |f| f.observations.len()),
        n_terms: f.map_or(0, |f| f.n_terms),
        n_instructor_obs: f.map_or(0, |f| f.n_instructor_obs),
        effective_n: f.map_or(0.0, |f| round2(f.effective_n)),
        observations,
        warnings,
    })
}

/// True when at least one past section filled, so the forecast has a clearing price to work from.
fn any_filled(f: &Forecast) -> bool {
    f.observations.iter().any(|o| o.filled)
}

/// The confidence `plan` reports. Without a filled section there is no clearing price, so it is Low
/// whatever the forecast's own count of sections says.
fn reported_confidence(f: &Forecast) -> Confidence {
    if any_filled(f) {
        f.confidence
    } else {
        Confidence::Low
    }
}

/// The window in the middle of a sentence: "Round 1 Window 1", or "all rounds" and "round 2 (all windows)".
fn window_words(sel: WindowSel) -> String {
    if sel.is_specific() {
        sel.label()
    } else {
        sel.label().to_lowercase()
    }
}

/// Warnings for history that `--exclude-session` removed from one course and window: `in_window` rows in all,
/// `kept` of them left in. Nothing is said when no session is excluded or no row was left out.
fn exclusion_warnings(filters: &GlobalFilters, code: &str, sel: WindowSel, in_window: usize, kept: usize) -> Vec<String> {
    let left_out = in_window.saturating_sub(kept);
    if filters.excluded_sessions.is_empty() || left_out == 0 {
        return Vec::new();
    }
    let names: Vec<&str> = filters.excluded_sessions.iter().map(String::as_str).collect();
    let names = names.join(", ");
    let window = window_words(sel);
    if kept == 0 {
        vec![format!(
            "All {in_window} {code} section results for {window} are in excluded sessions ({names}), so there is no history to base a bid on."
        )]
    } else {
        vec![format!(
            "{left_out} of {in_window} {code} section results for {window} are in excluded sessions ({names}) and were left out."
        )]
    }
}

/// How to order instructors. `InstructorStats::premium` is how much more (or less) their sections cost
/// than the other sections of the same course, term and window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum InstructorSort {
    /// Highest premium first: the hardest sections to get into.
    MostInDemand,
    /// Lowest premium first.
    EasiestToGet,
    /// Highest median clearing price first.
    ClearingPrice,
    /// Most sections first.
    Sections,
}

impl InstructorSort {
    /// "demand", "easiest", "clearing" or "sections".
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "demand" => Some(InstructorSort::MostInDemand),
            "easiest" => Some(InstructorSort::EasiestToGet),
            "clearing" => Some(InstructorSort::ClearingPrice),
            "sections" => Some(InstructorSort::Sections),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstructorQuery<'a> {
    pub sel: WindowSel,
    pub school: Option<&'a str>,
    /// Restrict to one course. When set, `min_compared` is ignored.
    pub course: Option<&'a str>,
    /// Case-insensitive part of the name.
    pub search: Option<&'a str>,
    /// Hide instructors with fewer comparable sections than this.
    pub min_compared: usize,
    pub sort: InstructorSort,
}

/// Instructors matching `q`, sorted as asked. Ties are broken by name.
pub fn rank_instructors(data: &AppData, filters: &GlobalFilters, q: &InstructorQuery) -> Vec<InstructorStats> {
    let rows: Vec<&BidRow> = match q.course {
        Some(input) => match resolve_course(data, input) {
            Ok(info) => data.course_rows(&info.code).collect(),
            Err(_) => Vec::new(),
        },
        None => data.rows.iter().collect(),
    };
    let rows: Vec<&BidRow> = rows
        .into_iter()
        .filter(|r| q.sel.matches(r) && filters.allows(r) && q.school.is_none_or(|s| &*r.school == s))
        .collect();

    let needle = q.search.map(|s| s.trim().to_lowercase()).unwrap_or_default();
    let mut list: Vec<InstructorStats> = instructor_ranking(&rows)
        .into_iter()
        .filter(|s| q.course.is_some() || s.compared >= q.min_compared)
        .filter(|s| needle.is_empty() || s.name.to_lowercase().contains(&needle))
        .collect();
    list.sort_by(|a, b| {
        let by = match q.sort {
            InstructorSort::MostInDemand => b.premium.total_cmp(&a.premium),
            InstructorSort::EasiestToGet => a.premium.total_cmp(&b.premium),
            InstructorSort::ClearingPrice => {
                b.median_clearing.unwrap_or(0.0).total_cmp(&a.median_clearing.unwrap_or(0.0))
            }
            InstructorSort::Sections => b.sections.cmp(&a.sections),
        };
        by.then_with(|| a.name.cmp(&b.name))
    });
    list
}

#[derive(Debug, Clone, Serialize)]
pub struct FileSummary {
    pub name: String,
    pub term: Option<String>,
    pub rows_kept: usize,
    pub rows_special_rounds: usize,
    pub rows_invalid: usize,
    pub duplicates: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DataSummary {
    pub data_dir: String,
    pub files: Vec<FileSummary>,
    pub total_rows: usize,
    pub courses: usize,
    pub terms: Vec<String>,
    pub latest_term: Option<String>,
    pub default_target_term: String,
    pub sessions: Vec<String>,
    pub schools: Vec<String>,
    pub load_millis: u64,
}

pub fn data_summary(data: &AppData) -> DataSummary {
    DataSummary {
        data_dir: data.report.dir.display().to_string(),
        files: data
            .report
            .files
            .iter()
            .map(|f| FileSummary {
                name: f.name.clone(),
                term: f.term.map(|t| t.label()),
                rows_kept: f.rows_kept,
                rows_special_rounds: f.rows_special_rounds,
                rows_invalid: f.rows_invalid,
                duplicates: f.duplicates,
                error: f.error.clone(),
            })
            .collect(),
        total_rows: data.rows.len(),
        courses: data.courses.len(),
        terms: data.terms.iter().map(Term::label).collect(),
        latest_term: data.latest_term().map(|t| t.label()),
        default_target_term: data.default_target_term().label(),
        sessions: data.sessions.iter().map(|s| s.to_string()).collect(),
        schools: data.schools.iter().map(|s| s.to_string()).collect(),
        load_millis: data.report.millis as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{LoadReport, RoundKind, TermKind};
    use std::collections::HashMap;
    use std::sync::Arc;

    #[allow(clippy::too_many_arguments)]
    fn row(year: u16, kind: TermKind, window: u8, course: &str, section: &str, instr: &str, before: i32, after: i32, min: f64) -> BidRow {
        BidRow {
            term: Term { year, kind },
            session: Arc::from("Regular Academic Session"),
            round: RoundKind::Round1,
            window,
            course_code: Arc::from(course),
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

    /// IS111 (Business Intelligence) and IS112 (Data Mining), every term from 2022 to 2025.
    /// In window 1 of IS111: G1 (Alice Tan) and G2 (Bob Tan) fill, G3 (Carol Lee) does not.
    fn sample() -> AppData {
        let mut rows = Vec::new();
        for year in 2022..2026 {
            for kind in [TermKind::T1, TermKind::T2] {
                let base = 20.0 + (year - 2022) as f64 * 2.0;
                rows.push(row(year, kind, 1, "IS111", "G1", "Alice Tan", 10, 0, base));
                rows.push(row(year, kind, 1, "IS111", "G2", "Bob Tan", 10, 0, base * 1.5));
                rows.push(row(year, kind, 1, "IS111", "G3", "Carol Lee", 10, 4, 10.0));
                rows.push(row(year, kind, 2, "IS111", "G1", "Alice Tan", 5, 0, base * 3.0));
                rows.push(row(year, kind, 1, "IS112", "G1", "Dan Ng", 10, 0, 15.0));
            }
        }
        let descriptions: HashMap<Arc<str>, (Term, String)> = HashMap::from([
            (Arc::from("IS111"), (Term { year: 2025, kind: TermKind::T2 }, "Business Intelligence".to_string())),
            (Arc::from("IS112"), (Term { year: 2025, kind: TermKind::T2 }, "Data Mining".to_string())),
        ]);
        AppData::new(rows, descriptions, LoadReport::default())
    }

    fn target() -> Term {
        Term { year: 2026, kind: TermKind::T1 }
    }

    #[test]
    fn resolve_course_is_forgiving_about_case_and_spaces() {
        let data = sample();
        assert_eq!(resolve_course(&data, "IS111").map(|c| c.code.to_string()), Ok("IS111".to_string()));
        assert_eq!(resolve_course(&data, "is111").map(|c| c.code.to_string()), Ok("IS111".to_string()));
        assert_eq!(resolve_course(&data, " is 111 ").map(|c| c.code.to_string()), Ok("IS111".to_string()));
        assert_eq!(resolve_course(&data, "").unwrap_err(), Vec::<String>::new());
    }

    #[test]
    fn resolve_course_suggests_on_a_miss() {
        let data = sample();
        let mut suggestions = resolve_course(&data, "IS11").unwrap_err();
        suggestions.sort();
        assert_eq!(suggestions, vec!["IS111".to_string(), "IS112".to_string()]);
        assert!(resolve_course(&data, "XYZ999").unwrap_err().len() <= 5);
    }

    #[test]
    fn resolve_instructor_exact_substring_and_ambiguous() {
        let data = sample();
        assert_eq!(resolve_instructor(&data, "IS111", "Alice Tan"), Ok("Alice Tan".to_string()));
        assert_eq!(resolve_instructor(&data, "is 111", "  alice   tan "), Ok("Alice Tan".to_string()));
        assert_eq!(resolve_instructor(&data, "IS111", "carol"), Ok("Carol Lee".to_string()));
        assert_eq!(resolve_instructor(&data, "IS111", "tan"), Err(vec!["Alice Tan".to_string(), "Bob Tan".to_string()]));
        assert_eq!(
            resolve_instructor(&data, "IS111", "zed"),
            Err(vec!["Alice Tan".to_string(), "Bob Tan".to_string(), "Carol Lee".to_string()])
        );
    }

    #[test]
    fn search_and_hits() {
        let data = sample();
        assert_eq!(search_courses(&data, "business", None).first(), Some(&0));
        let all: Vec<String> = course_hits(&data, "", None, 10).into_iter().map(|h| h.code).collect();
        assert_eq!(all, vec!["IS111", "IS112"]);
        assert!(course_hits(&data, "", Some("NOPE"), 10).is_empty());
        assert_eq!(course_hits(&data, "", None, 1).len(), 1);
    }

    #[test]
    fn plan_bid_recommends_rising_bids() {
        let data = sample();
        let filters = GlobalFilters::default();
        let q = ForecastQuery { course: "IS111", sel: WindowSel::ROUND1_W1, instructor: None, target: target() };
        let plan = plan_bid(&data, &filters, &ModelParams::default(), &q, &DEFAULT_LEVELS, &[10.0, 25.0]).unwrap();

        assert_eq!(plan.recommended.len(), DEFAULT_LEVELS.len());
        assert!(plan.recommended.windows(2).all(|w| w[0].bid <= w[1].bid), "{:?}", plan.recommended);
        assert!(plan.recommended.windows(2).all(|w| w[0].probability < w[1].probability));
        assert_eq!(plan.bid_checks.len(), 2);
        assert!(plan.bid_checks[0].probability <= plan.bid_checks[1].probability);
        assert_eq!(plan.observations.len(), 24);
        assert_eq!(plan.window_code, "R1W1");
        assert_eq!(plan.target, "2026-27 T1");
        // Each share is rounded to four decimals, so 24 of them can drift by up to 0.0012.
        let share: f64 = plan.observations.iter().map(|o| o.weight_share).sum();
        assert!((share - 1.0).abs() < 0.0025, "weight shares sum to {share}");

        // Each recommended bid, asked back as a bid check, should meet its chance (up to rounding).
        let bids: Vec<f64> = plan.recommended.iter().map(|r| r.bid).collect();
        let check = plan_bid(&data, &filters, &ModelParams::default(), &q, &DEFAULT_LEVELS, &bids).unwrap();
        for (level, c) in DEFAULT_LEVELS.iter().zip(&check.bid_checks) {
            assert!(c.probability >= level - 1e-3, "bid {} gives {} for target {level}", c.bid, c.probability);
        }
    }

    #[test]
    fn plan_bid_explains_missing_history() {
        let data = sample();
        let filters = GlobalFilters::default();
        let unknown = ForecastQuery { course: "NOPE", sel: WindowSel::ROUND1_W1, instructor: None, target: target() };
        assert!(plan_bid(&data, &filters, &ModelParams::default(), &unknown, &DEFAULT_LEVELS, &[]).is_none());

        let empty_window = WindowSel { round: Some(RoundKind::Round2), window: Some(1) };
        let q = ForecastQuery { course: "IS111", sel: empty_window, instructor: None, target: target() };
        let plan = plan_bid(&data, &filters, &ModelParams::default(), &q, &DEFAULT_LEVELS, &[]).unwrap();
        assert!(plan.recommended.is_empty());
        assert_eq!(plan.n_observations, 0);
        assert!(plan.warnings.iter().any(|w| w.contains("has no results for")), "{:?}", plan.warnings);
    }

    #[test]
    fn plan_bid_warns_about_unknown_instructor_and_windows() {
        let data = sample();
        let filters = GlobalFilters::default();
        let q = ForecastQuery { course: "IS111", sel: WindowSel::default(), instructor: Some("Dan Ng"), target: target() };
        let plan = plan_bid(&data, &filters, &ModelParams::default(), &q, &DEFAULT_LEVELS, &[]).unwrap();
        assert!(plan.warnings.iter().any(|w| w.contains("Dan Ng has no history")), "{:?}", plan.warnings);
        assert!(plan.warnings.iter().any(|w| w.contains("pick a specific round and window")), "{:?}", plan.warnings);
    }

    #[test]
    fn course_report_lists_terms_and_sections() {
        let data = sample();
        let report = course_report(&data, &GlobalFilters::default(), "is 111", WindowSel::ROUND1_W1).unwrap();
        assert_eq!(report.code, "IS111");
        assert_eq!(report.windows_offered, vec!["R1W1", "R1W2"]);
        assert_eq!(report.instructors.len(), 3);
        assert_eq!(report.terms.len(), 8);
        assert_eq!(report.sections.len(), 24);
        assert_eq!(report.sections[0].term, "2025-26 T2");
        assert_eq!(report.sections[0].section, "G1");
        assert!(report.sections.iter().all(|s| s.window_code == "R1W1"));
        assert!(course_report(&data, &GlobalFilters::default(), "NOPE", WindowSel::default()).is_none());
    }

    #[test]
    fn hot_courses_rank_by_median_clearing() {
        let data = sample();
        let term = Term { year: 2025, kind: TermKind::T2 };
        let hot = hot_courses(&data, &GlobalFilters::default(), term, WindowSel::ROUND1_W1);
        let codes: Vec<&str> = hot.iter().map(|h| h.code.as_str()).collect();
        assert_eq!(codes, vec!["IS111", "IS112"]);
        assert_eq!(hot[0].median_clearing, 32.5);
        assert_eq!((hot[0].sections_filled, hot[0].sections_available), (2, 3));
        assert_eq!(hot[1].median_clearing, 15.0);
    }

    #[test]
    fn instructor_ranking_filters_and_sorts() {
        let data = sample();
        let filters = GlobalFilters::default();
        let query = |course, min_compared, sort| InstructorQuery {
            sel: WindowSel::ROUND1_W1,
            school: None,
            course,
            search: None,
            min_compared,
            sort,
        };
        let by_sections = rank_instructors(&data, &filters, &query(None, 0, InstructorSort::Sections));
        let names: Vec<&str> = by_sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Alice Tan", "Bob Tan", "Carol Lee", "Dan Ng"]);

        // A single-course view keeps instructors who have no peer section to compare with.
        let one_course = rank_instructors(&data, &filters, &query(Some("IS112"), 3, InstructorSort::MostInDemand));
        assert_eq!(one_course.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["Dan Ng"]);

        let searched = InstructorQuery { search: Some(" CAROL "), ..query(None, 0, InstructorSort::Sections) };
        assert_eq!(rank_instructors(&data, &filters, &searched).len(), 1);

        assert_eq!(InstructorSort::parse("Easiest"), Some(InstructorSort::EasiestToGet));
        assert_eq!(InstructorSort::parse("nope"), None);
    }

    #[test]
    fn rounding_matches_display() {
        for x in [11.625, 23.355, 10.0, 0.0, 1.0 / 3.0] {
            assert_eq!(format!("{:.2}", round2(x)), format!("{x:.2}"));
            assert_eq!(format!("{:.4}", round4(x)), format!("{x:.4}"));
        }
        assert_eq!(bid_cents(23.354), 23.36);
        assert_eq!(bid_cents(23.35), 23.35);
        assert_eq!(bid_cents(10.0), 10.0);
    }

    #[test]
    fn data_summary_describes_the_load() {
        let data = sample();
        let s = data_summary(&data);
        assert_eq!(s.total_rows, 40);
        assert_eq!(s.courses, 2);
        assert_eq!(s.terms.len(), 8);
        assert_eq!(s.latest_term.as_deref(), Some("2025-26 T2"));
        assert_eq!(s.default_target_term, "2026-27 T1");
        assert_eq!(s.schools, vec!["SCIS"]);
    }

    #[test]
    fn the_latest_regular_term_skips_summer_terms() {
        assert_eq!(latest_regular_term(&sample()), Some(Term { year: 2025, kind: TermKind::T2 }));
        let summer_only = AppData::new(
            vec![row(2025, TermKind::T3A, 1, "IS111", "G1", "Alice Tan", 10, 0, 20.0)],
            HashMap::new(),
            LoadReport::default(),
        );
        assert_eq!(latest_regular_term(&summer_only), None);
    }

    #[test]
    fn hot_can_leave_out_courses_with_too_few_filled_sections() {
        let data = sample();
        let filters = GlobalFilters::default();
        let term = Term { year: 2025, kind: TermKind::T2 };
        let codes = |min_filled| -> Vec<String> {
            hot_courses_with_min_filled(&data, &filters, term, WindowSel::ROUND1_W1, min_filled)
                .into_iter()
                .map(|h| h.code)
                .collect()
        };
        // In 2025 T2 IS111 has two filled sections and IS112 has one.
        assert_eq!(codes(1), vec!["IS111", "IS112"]);
        assert_eq!(codes(2), vec!["IS111"]);
        assert!(codes(3).is_empty());
    }

    /// IS111 is offered in every term from 2023 T1 to 2025 T2. OLD9 is offered only in 2023 T1 and 2023 T2.
    fn retiring_data() -> AppData {
        let terms = [
            (2023, TermKind::T1),
            (2023, TermKind::T2),
            (2024, TermKind::T1),
            (2024, TermKind::T2),
            (2025, TermKind::T1),
            (2025, TermKind::T2),
        ];
        let mut rows = Vec::new();
        for (year, kind) in terms {
            rows.push(row(year, kind, 1, "IS111", "G1", "Alice Tan", 10, 0, 20.0));
            if year == 2023 {
                rows.push(row(year, kind, 1, "OLD9", "G1", "Ola Lim", 10, 0, 18.0));
            }
        }
        AppData::new(rows, HashMap::new(), LoadReport::default())
    }

    #[test]
    fn plan_bid_says_when_a_course_was_not_offered_in_the_latest_regular_term() {
        let data = retiring_data();
        let filters = GlobalFilters::default();
        let params = ModelParams::default();
        let plan_for = |code: &str, target: Term| {
            let q = ForecastQuery { course: code, sel: WindowSel::ROUND1_W1, instructor: None, target };
            plan_bid(&data, &filters, &params, &q, &DEFAULT_LEVELS, &[]).unwrap()
        };
        // The latest regular term is 2025-26 T2, so the default target is 2026-27 T1.
        let gone = plan_for("OLD9", target());
        assert!(
            gone.warnings
                .iter()
                .any(|w| w == "OLD9 was last offered in 2023-24 T2; it may not be offered in 2026-27 T1."),
            "{:?}",
            gone.warnings
        );
        assert!(!plan_for("IS111", target()).warnings.iter().any(|w| w.contains("last offered")));
        // A target no later than the course's last offering is not at risk.
        let early = plan_for("OLD9", Term { year: 2023, kind: TermKind::T2 });
        assert!(!early.warnings.iter().any(|w| w.contains("last offered")), "{:?}", early.warnings);
    }

    #[test]
    fn a_window_where_nothing_filled_says_so_and_is_low_confidence() {
        // NOFILL: two sections a term over four terms, and every section has seats left after the process.
        let mut rows = Vec::new();
        for (year, kind) in [(2023, TermKind::T1), (2023, TermKind::T2), (2024, TermKind::T1), (2024, TermKind::T2)] {
            rows.push(row(year, kind, 1, "NOFILL", "G1", "Ng Lee", 10, 4, 10.0));
            rows.push(row(year, kind, 1, "NOFILL", "G2", "Ng Lee", 10, 4, 10.0));
        }
        let data = AppData::new(rows, HashMap::new(), LoadReport::default());
        let filters = GlobalFilters::default();
        let params = ModelParams::default();
        let q = ForecastQuery { course: "NOFILL", sel: WindowSel::ROUND1_W1, instructor: None, target: target() };

        let model = forecast(&data, &filters, &q, &params).expect("the sections were open");
        assert_eq!(model.confidence, Confidence::Medium, "the forecast alone would say Medium");
        let plan = plan_bid(&data, &filters, &params, &q, &DEFAULT_LEVELS, &[]).unwrap();
        assert_eq!(plan.median_clearing, None);
        assert_eq!(plan.confidence, "Low");
        assert!(
            plan.warnings.iter().any(|w| w.starts_with("No past section filled in Round 1 Window 1, so the price estimate falls back to the reported Min Bids")),
            "{:?}",
            plan.warnings
        );

        // A course with filled sections keeps its own confidence and gets no such warning.
        let filled = plan_bid(&sample(), &filters, &params, &ForecastQuery { course: "IS111", ..q }, &DEFAULT_LEVELS, &[]).unwrap();
        assert_ne!(filled.confidence, "Low");
        assert!(!filled.warnings.iter().any(|w| w.contains("filled, so the price")), "{:?}", filled.warnings);
    }

    #[test]
    fn excluded_sessions_that_removed_the_history_are_named() {
        // ONE is only ever in the 1A Session. TWO has one Regular section and one 1A section in each term.
        let mut rows = Vec::new();
        for (year, kind) in [(2024, TermKind::T1), (2024, TermKind::T2), (2025, TermKind::T1), (2025, TermKind::T2)] {
            let mut one = row(year, kind, 1, "ONE", "G1", "Pat Ho", 10, 0, 25.0);
            one.session = Arc::from("1A Session");
            rows.push(one);
            rows.push(row(year, kind, 1, "TWO", "G1", "Quin Yu", 10, 0, 30.0));
            let mut two = row(year, kind, 1, "TWO", "G2", "Quin Yu", 10, 0, 22.0);
            two.session = Arc::from("1A Session");
            rows.push(two);
        }
        let data = AppData::new(rows, HashMap::new(), LoadReport::default());
        let mut excluded = GlobalFilters::default();
        excluded.excluded_sessions.insert("1A Session".to_string());
        let params = ModelParams::default();
        let plan_for = |code: &str, filters: &GlobalFilters| {
            let q = ForecastQuery { course: code, sel: WindowSel::ROUND1_W1, instructor: None, target: target() };
            plan_bid(&data, filters, &params, &q, &DEFAULT_LEVELS, &[]).unwrap()
        };

        // Every ONE section is excluded: the plan says so, and does not blame a missing course.
        let gone = plan_for("ONE", &excluded);
        assert!(gone.recommended.is_empty());
        assert!(
            gone.warnings.contains(&"All 4 ONE section results for Round 1 Window 1 are in excluded sessions (1A Session), so there is no history to base a bid on.".to_string()),
            "{:?}",
            gone.warnings
        );
        assert!(!gone.warnings.iter().any(|w| w.contains("has no results")), "{:?}", gone.warnings);

        // Half of TWO's sections are excluded: the plan still runs and says how many were left out.
        let thinned = plan_for("TWO", &excluded);
        assert!(!thinned.recommended.is_empty());
        assert!(
            thinned
                .warnings
                .contains(&"4 of 8 TWO section results for Round 1 Window 1 are in excluded sessions (1A Session) and were left out.".to_string()),
            "{:?}",
            thinned.warnings
        );
        assert!(!plan_for("TWO", &GlobalFilters::default()).warnings.iter().any(|w| w.contains("excluded")));
    }

    /// COR-COMM1304 (hyphenated, as the real data has it) with a filled and an open section, in two terms.
    fn hyphen_sample() -> AppData {
        let mut rows = Vec::new();
        for year in [2024, 2025] {
            rows.push(row(year, TermKind::T1, 1, "COR-COMM1304", "G1", "Eve Ong", 10, 0, 18.0));
            rows.push(row(year, TermKind::T1, 1, "COR-COMM1304", "G2", "Fay Lim", 10, 3, 10.0));
            rows.push(row(year, TermKind::T1, 1, "IS111", "G1", "Alice Tan", 10, 0, 30.0));
        }
        let descriptions: HashMap<Arc<str>, (Term, String)> = HashMap::from([(
            Arc::from("COR-COMM1304"),
            (Term { year: 2025, kind: TermKind::T1 }, "Management Communication".to_string()),
        )]);
        AppData::new(rows, descriptions, LoadReport::default())
    }

    #[test]
    fn course_codes_match_with_or_without_hyphens_and_spaces() {
        let data = hyphen_sample();
        for input in ["COR-COMM1304", "cor-comm1304", "cor comm1304", "corcomm1304", " COR COMM 1304 ", "Cor\u{2013}Comm1304"] {
            assert_eq!(resolve_course(&data, input).map(|c| c.code.to_string()), Ok("COR-COMM1304".to_string()), "{input:?}");
        }
        let misses = resolve_course(&data, "cor comm13").unwrap_err();
        assert_eq!(misses.first().map(String::as_str), Some("COR-COMM1304"), "{misses:?}");
    }

    #[test]
    fn course_search_finds_a_hyphenated_code_with_or_without_the_hyphen_or_space() {
        let data = hyphen_sample();
        for query in ["corcomm1304", "cor comm1304", "cor-comm1304", "COR COMM 1304", "comm1304"] {
            let hits: Vec<String> = course_hits(&data, query, None, 5).into_iter().map(|h| h.code).collect();
            assert_eq!(hits.first().map(String::as_str), Some("COR-COMM1304"), "{query:?} gave {hits:?}");
        }
    }

    #[test]
    fn odd_course_inputs_are_misses_not_panics() {
        let data = sample();
        assert_eq!(resolve_course(&data, "IS111\n").map(|c| c.code.to_string()), Ok("IS111".to_string()));
        for input in ["-", " - ", "_", "é", "ＩＳ１１１", "IS111\u{0}", "IS111+"] {
            assert!(resolve_course(&data, input).is_err(), "{input:?} should not resolve");
        }
        assert!(search_courses(&data, "é", None).is_empty());
        assert!(search_courses(&data, "\u{0}\u{1F600}", None).is_empty());
    }

    #[test]
    fn blank_or_unknown_instructor_inputs_list_the_course_instead() {
        let data = sample();
        let everyone = vec!["Alice Tan".to_string(), "Bob Tan".to_string(), "Carol Lee".to_string()];
        assert_eq!(resolve_instructor(&data, "IS111", ""), Err(everyone.clone()));
        assert_eq!(resolve_instructor(&data, "IS111", " \t "), Err(everyone.clone()));
        assert_eq!(resolve_instructor(&data, "IS111", "ÁLICE TAN"), Err(everyone));
        assert_eq!(resolve_instructor(&data, "NOPE", "tan"), Err(Vec::new()));
        assert_eq!(resolve_instructor(&data, "", "alice"), Err(Vec::new()));
    }

    #[test]
    fn plan_bid_keeps_numbers_finite_when_given_nan_or_infinity() {
        let data = sample();
        let filters = GlobalFilters::default();
        let q = ForecastQuery { course: "IS111", sel: WindowSel::ROUND1_W1, instructor: None, target: target() };
        let levels = [f64::NAN, 0.8];
        let bids = [f64::NAN, f64::INFINITY, 25.0];
        let plan = plan_bid(&data, &filters, &ModelParams::default(), &q, &levels, &bids).unwrap();
        assert_eq!(plan.recommended.len(), 1, "the NaN chance is left out");
        assert_eq!(plan.bid_checks.len(), 1, "the NaN and infinite bids are left out");
        assert!(plan.warnings.iter().filter(|w| w.contains("left out")).count() == 2, "{:?}", plan.warnings);
        let value = serde_json::to_value(&plan).unwrap();
        for key in ["recommended", "bid_checks"] {
            for item in value[key].as_array().unwrap() {
                assert!(item.as_object().unwrap().values().all(|v| !v.is_null()), "{key}: {item}");
            }
        }
    }

    #[test]
    fn instructor_median_is_the_usual_median_for_an_even_number_of_prices() {
        // Ann's two filled sections clear at e$10 and e$20, so her median is e$15, as for a term or course.
        let rows = vec![
            row(2025, TermKind::T1, 1, "IS111", "G1", "Ann A", 10, 0, 10.0),
            row(2025, TermKind::T1, 1, "IS111", "G2", "Ann A", 10, 0, 20.0),
            row(2025, TermKind::T1, 1, "IS111", "G3", "Ben B", 10, 0, 30.0),
            row(2025, TermKind::T1, 1, "IS111", "G4", "Ben B", 10, 0, 40.0),
        ];
        let data = AppData::new(rows, HashMap::new(), LoadReport::default());
        let query = InstructorQuery {
            sel: WindowSel::ROUND1_W1,
            school: None,
            course: None,
            search: None,
            min_compared: 0,
            sort: InstructorSort::Sections,
        };
        let list = rank_instructors(&data, &GlobalFilters::default(), &query);
        let ann = list.iter().find(|s| s.name == "Ann A").expect("Ann taught");
        assert_eq!(ann.median_clearing, Some(15.0));
    }

    #[test]
    fn course_sections_sort_by_natural_section_then_session() {
        let mut rows = vec![
            row(2025, TermKind::T1, 1, "IS111", "G1", "Alice Tan", 10, 0, 20.0),
            row(2025, TermKind::T1, 1, "IS111", "G10", "Alice Tan", 10, 0, 20.0),
            row(2025, TermKind::T1, 1, "IS111", "G2", "Bob Tan", 10, 0, 20.0),
        ];
        // The same section name in another session. Inserted last, so only the sort can put it first.
        let mut other = row(2025, TermKind::T1, 1, "IS111", "G1", "Carol Lee", 10, 0, 20.0);
        other.session = Arc::from("1A Session");
        rows.push(other);
        let data = AppData::new(rows, HashMap::new(), LoadReport::default());
        let report = course_report(&data, &GlobalFilters::default(), "IS111", WindowSel::ROUND1_W1).unwrap();
        let order: Vec<(&str, &str)> = report.sections.iter().map(|s| (s.section.as_str(), s.session.as_str())).collect();
        assert_eq!(
            order,
            [
                ("G1", "1A Session"),
                ("G1", "Regular Academic Session"),
                ("G2", "Regular Academic Session"),
                ("G10", "Regular Academic Session"),
            ]
        );
    }

    #[test]
    fn a_course_with_no_open_seats_reports_zeros_and_no_forecast() {
        let rows = vec![
            row(2025, TermKind::T1, 1, "IS111", "G1", "Alice Tan", 0, 0, 0.0),
            row(2025, TermKind::T1, 1, "IS111", "G2", "Alice Tan", 0, 0, 0.0),
        ];
        let data = AppData::new(rows, HashMap::new(), LoadReport::default());
        let filters = GlobalFilters::default();
        let report = course_report(&data, &filters, "IS111", WindowSel::ROUND1_W1).unwrap();
        let t = &report.terms[0];
        assert_eq!((t.sections, t.available, t.filled), (2, 0, 0));
        assert_eq!(t.fill_rate, 0.0);
        assert!(t.clearing_median.is_none() && t.clearing_low.is_none() && t.median_bid.is_none());

        let q = ForecastQuery { course: "IS111", sel: WindowSel::ROUND1_W1, instructor: None, target: target() };
        let plan = plan_bid(&data, &filters, &ModelParams::default(), &q, &DEFAULT_LEVELS, &[10.0]).unwrap();
        assert!(plan.recommended.is_empty() && plan.bid_checks.is_empty());
        assert_eq!((plan.fill_probability, plan.median_clearing), (0.0, None));
        assert!(plan.warnings.iter().any(|w| w.contains("had seats open")), "{:?}", plan.warnings);
    }

    #[test]
    fn zero_total_weight_gives_zero_shares_and_finite_bids() {
        // With no summer weight, a summer target sees no history at all: every weight is zero.
        let data = sample();
        let params = ModelParams { summer_weight: 0.0, ..ModelParams::default() };
        let target = Term { year: 2026, kind: TermKind::T3A };
        let q = ForecastQuery { course: "IS111", sel: WindowSel::ROUND1_W1, instructor: None, target };
        let plan = plan_bid(&data, &GlobalFilters::default(), &params, &q, &DEFAULT_LEVELS, &[20.0]).unwrap();
        assert!(!plan.observations.is_empty());
        assert!(plan.observations.iter().all(|o| o.weight_share == 0.0));
        let value = serde_json::to_value(&plan).unwrap();
        for key in ["recommended", "bid_checks"] {
            for item in value[key].as_array().unwrap() {
                assert!(item.as_object().unwrap().values().all(|v| !v.is_null()), "{key}: {item}");
            }
        }
    }
}
