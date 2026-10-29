//! Descriptive statistics for the Overview and Instructors tabs.

use crate::data::model::MIN_BID;
use crate::data::{BidRow, Term};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

/// What happened in one term for the selected course and window.
///
/// Two units are counted, and the field names say which. `sections` counts each section once, however
/// many bidding windows it was offered in. The others count section-windows, one section in one bidding
/// window: `section_windows`, `available` (seats were open) and `filled`. The seat totals add up over the
/// same section-windows. With one window selected a section-window is a section, so the counts agree.
#[derive(Debug, Clone)]
pub struct TermSummary {
    pub term: Term,
    /// Distinct sections, counted by session and name.
    pub sections: usize,
    /// Section-windows in the term, open or not.
    pub section_windows: usize,
    /// Section-windows where seats were open for bidding.
    pub available: usize,
    /// Section-windows that filled.
    pub filled: usize,
    pub seats_open: i32,
    pub seats_left: i32,
    /// Median / lowest / highest clearing price over the section-windows that filled.
    pub clearing_median: Option<f64>,
    pub clearing_low: Option<f64>,
    pub clearing_high: Option<f64>,
    /// Median of the median bids of every section-window that has one. BOSS reports a bid for a few
    /// section-windows with no seats open, and they are counted here; the other figures leave them out.
    pub median_bid: Option<f64>,
}

impl TermSummary {
    /// The share of open section-windows that filled.
    pub fn fill_rate(&self) -> f64 {
        if self.available == 0 { 0.0 } else { self.filled as f64 / self.available as f64 }
    }
}

pub fn term_summaries(rows: &[&BidRow]) -> Vec<TermSummary> {
    let mut by_term: BTreeMap<Term, Vec<&BidRow>> = BTreeMap::new();
    for r in rows {
        by_term.entry(r.term).or_default().push(r);
    }
    by_term
        .into_iter()
        .map(|(term, rs)| {
            let available: Vec<_> = rs.iter().filter(|r| r.available()).collect();
            let mut clearing: Vec<f64> = available.iter().filter(|r| r.filled()).map(|r| r.min_bid.max(MIN_BID)).collect();
            clearing.sort_by(f64::total_cmp);
            let mut medians: Vec<f64> = rs.iter().filter(|r| r.median_bid > 0.0).map(|r| r.median_bid).collect();
            medians.sort_by(f64::total_cmp);
            TermSummary {
                term,
                sections: section_count(&rs),
                section_windows: rs.len(),
                available: available.len(),
                filled: clearing.len(),
                seats_open: available.iter().map(|r| r.before_proc).sum(),
                seats_left: available.iter().map(|r| r.after_proc.max(0)).sum(),
                clearing_median: median(&clearing),
                clearing_low: clearing.first().copied(),
                clearing_high: clearing.last().copied(),
                median_bid: median(&medians),
            }
        })
        .collect()
}

/// Distinct sections among one term's rows. A section appears once per bidding window it was open
/// in, so this counts `(session, section)` pairs. Names alone would undercount: "G1" can exist in
/// two sessions of the same term.
fn section_count(rows: &[&BidRow]) -> usize {
    rows.iter().map(|r| (&*r.session, &*r.section)).collect::<HashSet<_>>().len()
}

pub fn median(sorted: &[f64]) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let m = sorted.len() / 2;
    Some(if sorted.len().is_multiple_of(2) { (sorted[m - 1] + sorted[m]) / 2.0 } else { sorted[m] })
}

/// How big one whole term was: what was on offer and how much of it bidding took up.
///
/// Built from every row the caller passes (usually every course, after the session and window
/// filters), unlike `TermSummary`, which is one course. A section is a `(course, session, section)`
/// triple and counts once however many windows it was offered in. Seat counts avoid counting a seat
/// once per window: `first_window_seats` takes each section's first open window only, and
/// `seats_unclaimed` its last window only.
///
/// The BOSS export has no class days or times, so nothing here is about the timetable.
/// Serialised values are rounded (see `query::ser`); the fields keep full precision for the UI.
#[derive(Debug, Clone, Serialize)]
pub struct TermOverview {
    #[serde(serialize_with = "crate::data::query::ser::term")]
    pub term: Term,
    /// Distinct course codes.
    pub courses: usize,
    /// Distinct sections (classes): course, session and section name.
    pub sections: usize,
    /// Distinct instructor entries. A co-taught entry ("A, B") counts as one.
    pub instructors: usize,
    /// Distinct bidding windows (round and window) that had any rows.
    pub windows: usize,
    /// Class size summed over the sections: each section's largest `Vacancy`.
    pub class_seats: i64,
    /// Seats open for bidding in each section's first window with seats open, summed.
    pub first_window_seats: i64,
    /// Seats won through bidding, summed over every window.
    pub seats_won: i64,
    /// Seats still open after each section's last window, summed.
    pub seats_unclaimed: i64,
    /// Section-windows with seats open for bidding.
    pub available: usize,
    /// Section-windows that filled.
    pub filled: usize,
    /// `filled` over `available`, or 0 when no seats were open.
    #[serde(serialize_with = "crate::data::query::ser::ratio")]
    pub fill_rate: f64,
    /// Median clearing price over the section-windows that filled.
    #[serde(serialize_with = "crate::data::query::ser::opt_money")]
    pub clearing_median: Option<f64>,
}

/// One `TermOverview` per term in `rows`, oldest first.
pub fn term_overviews(rows: &[&BidRow]) -> Vec<TermOverview> {
    let mut by_term: BTreeMap<Term, Vec<&BidRow>> = BTreeMap::new();
    for r in rows {
        by_term.entry(r.term).or_default().push(r);
    }
    by_term.into_iter().map(|(term, rs)| term_overview(term, &rs)).collect()
}

fn term_overview(term: Term, rows: &[&BidRow]) -> TermOverview {
    /// Per section: largest class size, (first open window, its seats), (last window, seats left).
    #[derive(Default)]
    struct Section {
        class_size: i32,
        first_open: Option<((crate::data::RoundKind, u8), i32)>,
        last: Option<((crate::data::RoundKind, u8), i32)>,
    }
    let mut sections: HashMap<(&str, &str, &str), Section> = HashMap::new();
    let mut courses = HashSet::new();
    let mut instructors = HashSet::new();
    let mut windows = HashSet::new();
    let mut clearing = Vec::new();
    let (mut available, mut seats_won) = (0usize, 0i64);
    for r in rows {
        courses.insert(&*r.course_code);
        if !r.instructor.is_empty() {
            instructors.insert(&*r.instructor);
        }
        let window = (r.round, r.window);
        windows.insert(window);
        let s = sections.entry((&*r.course_code, &*r.session, &*r.section)).or_default();
        s.class_size = s.class_size.max(r.vacancy);
        if s.last.is_none_or(|(w, _)| window >= w) {
            s.last = Some((window, r.after_proc.max(0)));
        }
        if r.available() {
            available += 1;
            seats_won += r.seats_taken() as i64;
            if r.filled() {
                clearing.push(r.min_bid.max(MIN_BID));
            }
            if s.first_open.is_none_or(|(w, _)| window < w) {
                s.first_open = Some((window, r.before_proc));
            }
        }
    }
    clearing.sort_by(f64::total_cmp);
    let filled = clearing.len();
    TermOverview {
        term,
        courses: courses.len(),
        sections: sections.len(),
        instructors: instructors.len(),
        windows: windows.len(),
        class_seats: sections.values().map(|s| s.class_size.max(0) as i64).sum(),
        first_window_seats: sections.values().filter_map(|s| s.first_open).map(|(_, n)| n as i64).sum(),
        seats_won,
        seats_unclaimed: sections.values().filter_map(|s| s.last).map(|(_, n)| n as i64).sum(),
        available,
        filled,
        fill_rate: if available == 0 { 0.0 } else { filled as f64 / available as f64 },
        clearing_median: median(&clearing),
    }
}

/// Serialised values are rounded (see `query::ser`); the fields keep full precision for the UI.
#[derive(Debug, Clone, Serialize)]
pub struct InstructorStats {
    pub name: String,
    pub school: String,
    pub courses: usize,
    /// Section-windows they taught with seats open. A section offered in two windows counts twice.
    pub sections: usize,
    #[serde(serialize_with = "crate::data::query::ser::ratio")]
    pub fill_rate: f64,
    /// Median clearing price of their filled sections.
    #[serde(serialize_with = "crate::data::query::ser::opt_money")]
    pub median_clearing: Option<f64>,
    /// How much more (or less) it cost to get their section than other sections of the same
    /// course in the same term and window, shrunk toward 0 when there is little evidence.
    #[serde(serialize_with = "crate::data::query::ser::ratio")]
    pub premium: f64,
    /// Sections that could be compared against a peer section.
    pub compared: usize,
    #[serde(serialize_with = "crate::data::query::ser::term")]
    pub last_term: Term,
}

/// Rank instructors by *relative* demand.
///
/// Comparing raw bids across courses mostly measures which courses are popular, not which
/// instructors are. Instead each section is compared with the other sections of the same course,
/// term and window: `log(effective price) - mean(log effective price of that group)`, where an
/// unfilled section's effective price is the minimum bid. An instructor's premium is the mean of
/// these residuals, shrunk toward zero (`sum / (n + k)`) so one lucky section does not top the list.
///
/// The result is sorted by name. Callers sort it again by what they show.
pub fn instructor_ranking(rows: &[&BidRow]) -> Vec<InstructorStats> {
    const SHRINK: f64 = 2.0;

    // Ordered, so the sums below always add up in the same order and the results never differ by a last digit.
    let mut groups: BTreeMap<(&str, Term, crate::data::RoundKind, u8), Vec<&BidRow>> = BTreeMap::new();
    for r in rows.iter().filter(|r| r.available() && !r.instructor.is_empty()) {
        groups.entry((&r.course_code, r.term, r.round, r.window)).or_default().push(r);
    }

    struct Acc<'a> {
        school: &'a str,
        courses: HashSet<&'a str>,
        sections: usize,
        filled: usize,
        clearing: Vec<f64>,
        resid_sum: f64,
        compared: usize,
        last_term: Term,
    }

    let mut acc: HashMap<&str, Acc> = HashMap::new();
    for group in groups.values() {
        let logs: Vec<f64> = group.iter().map(|r| effective_price(r).ln()).collect();
        let mean = logs.iter().sum::<f64>() / logs.len() as f64;
        for (&r, lp) in group.iter().zip(&logs) {
            let a = acc.entry(&r.instructor).or_insert_with(|| Acc {
                school: &r.school,
                courses: HashSet::new(),
                sections: 0,
                filled: 0,
                clearing: Vec::new(),
                resid_sum: 0.0,
                compared: 0,
                last_term: r.term,
            });
            a.courses.insert(&r.course_code);
            a.sections += 1;
            a.last_term = a.last_term.max(r.term);
            if r.filled() {
                a.filled += 1;
                a.clearing.push(r.min_bid.max(MIN_BID));
            }
            if group.len() >= 2 {
                a.resid_sum += lp - mean;
                a.compared += 1;
            }
        }
    }

    let mut out: Vec<InstructorStats> = acc
        .into_iter()
        .map(|(name, mut a)| {
            a.clearing.sort_by(f64::total_cmp);
            InstructorStats {
                name: name.to_string(),
                school: a.school.to_string(),
                courses: a.courses.len(),
                sections: a.sections,
                fill_rate: a.filled as f64 / a.sections.max(1) as f64,
                median_clearing: median(&a.clearing),
                premium: (a.resid_sum / (a.compared as f64 + SHRINK)).exp() - 1.0,
                compared: a.compared,
                last_term: a.last_term,
            }
        })
        .collect();
    // A HashMap has no order of its own. Sorting by name makes ties in any later sort come out the same every run.
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn effective_price(r: &BidRow) -> f64 {
    if r.filled() { r.min_bid.max(MIN_BID) } else { MIN_BID }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{RoundKind, TermKind};
    use std::sync::Arc;

    fn row(year: u16, session: &str, section: &str, window: u8) -> BidRow {
        BidRow {
            term: Term { year, kind: TermKind::T1 },
            session: Arc::from(session),
            round: RoundKind::Round1,
            window,
            course_code: Arc::from("IS111"),
            section: Arc::from(section),
            instructor: Arc::from("PROF A"),
            school: Arc::from("SCIS"),
            vacancy: 40,
            opening_vacancy: 40,
            before_proc: 10,
            dice: 0,
            after_proc: 0,
            enrolled: 10,
            median_bid: 20.0,
            min_bid: 15.0,
        }
    }

    #[test]
    fn same_section_name_in_two_sessions_counts_twice() {
        let rows = [
            row(2025, "Regular Academic Session", "G1", 1),
            row(2025, "Regular Academic Session", "G1", 2), // same section, another window
            row(2025, "Regular Academic Session", "G2", 1),
            row(2025, "1A Session", "G1", 1),
            row(2026, "Regular Academic Session", "G1", 1), // same name, another term
        ];
        let refs: Vec<&BidRow> = rows.iter().collect();
        let summaries = term_summaries(&refs);
        let counts: Vec<(u16, usize)> = summaries.iter().map(|s| (s.term.year, s.sections)).collect();
        assert_eq!(counts, vec![(2025, 3), (2026, 1)]);
    }

    #[test]
    fn term_overview_counts_each_section_and_seat_once() {
        // G1 opens 10 seats in window 1, 6 are won, 4 roll into window 2 where 3 are won and 1 is left.
        let mut g1w1 = row(2025, "Regular Academic Session", "G1", 1);
        g1w1.after_proc = 4;
        let mut g1w2 = row(2025, "Regular Academic Session", "G1", 2);
        g1w2.before_proc = 4;
        g1w2.after_proc = 1;
        // G2 fills its 10 seats in window 1, and is listed again in window 2 with nothing open.
        let g2w1 = row(2025, "Regular Academic Session", "G2", 1);
        let g2w2 = closed("G2", 2, 0.0);
        // Another course with a 30-seat class taught by someone else, in another term.
        let mut other = row(2026, "Regular Academic Session", "G1", 1);
        other.course_code = Arc::from("IS112");
        other.vacancy = 30;
        other.instructor = Arc::from("PROF B");
        let rows = [g1w2.clone(), g1w1, g2w1, g2w2, other];
        let refs: Vec<&BidRow> = rows.iter().collect();
        let overviews = term_overviews(&refs);
        assert_eq!(overviews.len(), 2);
        let t = &overviews[0];
        assert_eq!(t.term.year, 2025);
        assert_eq!((t.courses, t.sections, t.instructors, t.windows), (1, 2, 1, 2));
        assert_eq!(t.class_seats, 80);
        assert_eq!(t.first_window_seats, 20, "window 2's 4 seats rolled over from window 1");
        assert_eq!(t.seats_won, 6 + 3 + 10);
        assert_eq!(t.seats_unclaimed, 1);
        assert_eq!((t.available, t.filled), (3, 1));
        assert_eq!(t.clearing_median, Some(15.0));
        let next = &overviews[1];
        assert_eq!((next.courses, next.sections, next.class_seats, next.seats_won), (1, 1, 30, 10));
    }

    #[test]
    fn median_handles_odd_and_even_lengths() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[4.0]), Some(4.0));
        assert_eq!(median(&[1.0, 3.0]), Some(2.0));
        assert_eq!(median(&[1.0, 2.0, 9.0]), Some(2.0));
    }

    /// A section-window with no seats open: nothing to bid for, so no seats and no clearing price.
    fn closed(section: &str, window: u8, median: f64) -> BidRow {
        let mut r = row(2025, "Regular Academic Session", section, window);
        r.before_proc = 0;
        r.after_proc = 0;
        r.min_bid = 0.0;
        r.median_bid = median;
        r
    }

    #[test]
    fn sections_count_once_and_section_windows_count_every_window() {
        let mut rows = vec![
            row(2025, "Regular Academic Session", "G1", 1), // filled
            row(2025, "Regular Academic Session", "G1", 2), // filled
            row(2025, "Regular Academic Session", "G2", 1), // filled
            closed("G2", 2, 0.0),
        ];
        let mut open = row(2025, "Regular Academic Session", "G3", 1);
        open.after_proc = 4; // open, not filled
        rows.push(open);
        let refs: Vec<&BidRow> = rows.iter().collect();
        let s = &term_summaries(&refs)[0];
        assert_eq!((s.sections, s.section_windows), (3, 5));
        assert_eq!((s.available, s.filled), (4, 3));
        assert_eq!((s.seats_open, s.seats_left), (40, 4));
        assert!((s.fill_rate() - 0.75).abs() < 1e-12);
    }

    #[test]
    fn with_one_window_sections_and_section_windows_agree() {
        let rows = [
            row(2025, "Regular Academic Session", "G1", 1),
            row(2025, "Regular Academic Session", "G2", 1),
            row(2025, "1A Session", "G1", 1),
        ];
        let refs: Vec<&BidRow> = rows.iter().collect();
        let s = &term_summaries(&refs)[0];
        assert_eq!((s.sections, s.section_windows, s.available), (3, 3, 3));
    }

    #[test]
    fn median_bid_counts_a_bid_with_no_seats_but_clearing_does_not() {
        let rows = [row(2025, "Regular Academic Session", "G1", 1), closed("G9", 1, 50.0)];
        let refs: Vec<&BidRow> = rows.iter().collect();
        let s = &term_summaries(&refs)[0];
        assert_eq!(s.median_bid, Some(35.0), "the median of 20 and 50");
        assert_eq!((s.available, s.filled), (1, 1));
        assert_eq!(s.clearing_median, Some(15.0));
    }

    #[test]
    fn instructor_ranking_comes_back_sorted_by_name() {
        let names = ["Zed Lo", "Amy Ng", "Mia Tan", "Bo Lim", "Kai Wu", "Eve Ong"];
        let rows: Vec<BidRow> = names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let mut r = row(2025, "Regular Academic Session", &format!("G{}", i + 1), 1);
                r.instructor = Arc::from(*name);
                r
            })
            .collect();
        let refs: Vec<&BidRow> = rows.iter().collect();
        let got: Vec<String> = instructor_ranking(&refs).into_iter().map(|s| s.name).collect();
        let mut sorted = got.clone();
        sorted.sort();
        assert_eq!(got, sorted);
        assert_eq!(got.len(), names.len());
    }
}
