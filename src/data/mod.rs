pub mod backtest;
pub mod filter;
pub mod loader;
pub mod model;
pub mod query;
pub mod stats;

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TermKind {
    T1,
    T2,
    T3A,
    T3B,
}

impl TermKind {
    pub fn is_summer(self) -> bool {
        matches!(self, TermKind::T3A | TermKind::T3B)
    }

    pub fn label(self) -> &'static str {
        match self {
            TermKind::T1 => "T1",
            TermKind::T2 => "T2",
            TermKind::T3A => "T3A",
            TermKind::T3B => "T3B",
        }
    }
}

/// An academic term, e.g. AY2025-26 Term 2 is `Term { year: 2025, kind: T2 }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Term {
    pub year: u16,
    pub kind: TermKind,
}

impl Term {
    /// Position on a continuous time axis, in academic years.
    pub fn time(&self) -> f64 {
        self.year as f64
            + match self.kind {
                TermKind::T1 => 0.0,
                TermKind::T2 => 0.5,
                TermKind::T3A => 0.75,
                TermKind::T3B => 0.875,
            }
    }

    /// The next regular (non-summer) term.
    pub fn next_regular(&self) -> Term {
        match self.kind {
            TermKind::T1 => Term { year: self.year, kind: TermKind::T2 },
            _ => Term { year: self.year + 1, kind: TermKind::T1 },
        }
    }

    pub fn label(&self) -> String {
        format!("{}-{:02} {}", self.year, (self.year + 1) % 100, self.kind.label())
    }

    /// Parses "2026-27 T2", "2026-27_T2", "2026-27 Term 2", "AY2026-27 T3A", "2026 T1" and the like.
    /// The year is four digits from 2000 to 2099, optionally followed by an academic-year span ("-27", "/27"
    /// or "-2027") that must end in the next year, so "2026-30" is rejected. The term is `T1`, `T2`, `T3A` or
    /// `T3B`, written "T2" or "Term 2". Nothing else may appear: no second year or term, and no other words
    /// apart from a leading "AY".
    pub fn parse(s: &str) -> Option<Term> {
        let norm = s.trim().to_ascii_lowercase().replace('_', " ");
        let rest = norm.strip_prefix("ay").unwrap_or(norm.as_str()).trim_start();
        let year_digits = rest.get(..4).filter(|d| d.bytes().all(|b| b.is_ascii_digit()))?;
        let year: u16 = year_digits.parse().ok()?;
        if !(2000..2100).contains(&year) {
            return None;
        }
        let mut rest = rest[4..].trim_start();
        if let Some(span) = rest.strip_prefix(['-', '/']) {
            let span = span.trim_start();
            let end_len = span.bytes().take_while(u8::is_ascii_digit).count();
            let end: u16 = span[..end_len].parse().ok()?;
            let ends_next_year = match end_len {
                2 => end == (year + 1) % 100,
                4 => end == year + 1,
                _ => false,
            };
            if !ends_next_year {
                return None;
            }
            rest = span[end_len..].trim_start();
        }
        let kind = rest.strip_prefix("term").or_else(|| rest.strip_prefix('t'))?;
        Some(Term { year, kind: loader::parse_term_kind(kind)? })
    }
}

impl std::fmt::Display for Term {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RoundKind {
    Round1,
    Round1A,
    Round1B,
    Round2,
    Round2A,
}

impl std::fmt::Display for RoundKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoundKind::Round1 => write!(f, "Round 1"),
            RoundKind::Round1A => write!(f, "Round 1A"),
            RoundKind::Round1B => write!(f, "Round 1B"),
            RoundKind::Round2 => write!(f, "Round 2"),
            RoundKind::Round2A => write!(f, "Round 2A"),
        }
    }
}

/// Course descriptions keyed by course code, each with the latest term it was described in.
pub type Descriptions = HashMap<Arc<str>, (Term, String)>;

impl RoundKind {
    /// Reads a round code such as "1", "1a" or "2A" (case-insensitive); `None` for anything else.
    /// Not a `FromStr` impl because it returns an `Option`, not a `Result`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "1" => Some(RoundKind::Round1),
            "1A" => Some(RoundKind::Round1A),
            "1B" => Some(RoundKind::Round1B),
            "2" => Some(RoundKind::Round2),
            "2A" => Some(RoundKind::Round2A),
            _ => None,
        }
    }
}

/// One row of the BOSS "Overall Results" report: a single section in a single bidding window.
#[derive(Debug, Clone)]
pub struct BidRow {
    pub term: Term,
    pub session: Arc<str>,
    pub round: RoundKind,
    pub window: u8,
    pub course_code: Arc<str>,
    pub section: Arc<str>,
    pub instructor: Arc<str>,
    pub school: Arc<str>,
    pub vacancy: i32,
    pub opening_vacancy: i32,
    pub before_proc: i32,
    pub dice: i32,
    pub after_proc: i32,
    pub enrolled: i32,
    pub median_bid: f64,
    pub min_bid: f64,
}

impl BidRow {
    /// Seats were open for bidding in this window.
    pub fn available(&self) -> bool {
        self.before_proc > 0
    }

    /// Every open seat was taken, so `min_bid` is a real clearing price.
    pub fn filled(&self) -> bool {
        self.before_proc > 0 && self.after_proc <= 0
    }

    pub fn seats_taken(&self) -> i32 {
        (self.before_proc - self.after_proc).max(0)
    }
}

#[derive(Debug, Clone)]
pub struct CourseInfo {
    pub code: Arc<str>,
    pub description: String,
    pub school: Arc<str>,
    pub last_term: Term,
}

#[derive(Debug, Clone, Default)]
pub struct FileReport {
    pub name: String,
    pub term: Option<Term>,
    pub rows_kept: usize,
    pub rows_special_rounds: usize,
    pub rows_invalid: usize,
    pub duplicates: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct LoadReport {
    pub dir: PathBuf,
    pub files: Vec<FileReport>,
    pub millis: u128,
}

impl LoadReport {
    pub fn errors(&self) -> impl Iterator<Item = &FileReport> {
        self.files.iter().filter(|f| f.error.is_some())
    }
}

/// Running totals for one (round, window): `(term, filled so far, available so far)` for every term
/// with available sections, in ascending term order.
type FillHistory = Vec<(Term, usize, usize)>;

pub struct AppData {
    pub rows: Vec<BidRow>,
    pub courses: Vec<CourseInfo>,
    course_index: HashMap<Arc<str>, usize>,
    course_rows: HashMap<Arc<str>, Vec<usize>>,
    pub sessions: Vec<Arc<str>>,
    pub schools: Vec<Arc<str>>,
    pub terms: Vec<Term>,
    /// Share of available sections that filled up, per (round, window), across every course and every
    /// term. It includes outcomes from the term being predicted and from later terms, so it must not be
    /// used as a forecast prior for a past term: use `fill_prior_before` instead. `filter::all_windows`
    /// reads its keys.
    pub fill_prior: HashMap<(RoundKind, u8), f64>,
    /// Running (filled, available) totals per (round, window), one entry per term in ascending order.
    /// Lets `fill_prior_before` find the share filled before any term with a binary search.
    fill_history: HashMap<(RoundKind, u8), FillHistory>,
    pub report: LoadReport,
}

impl AppData {
    pub fn new(mut rows: Vec<BidRow>, descriptions: Descriptions, report: LoadReport) -> Self {
        rows.sort_by(|a, b| {
            a.term
                .cmp(&b.term)
                .then_with(|| a.course_code.cmp(&b.course_code))
                .then_with(|| a.round.cmp(&b.round))
                .then_with(|| a.window.cmp(&b.window))
                .then_with(|| a.section.cmp(&b.section))
        });

        let mut course_rows: HashMap<Arc<str>, Vec<usize>> = HashMap::new();
        let mut sessions = BTreeSet::new();
        let mut schools = BTreeSet::new();
        let mut terms = BTreeSet::new();
        let mut latest: HashMap<Arc<str>, (Term, Arc<str>)> = HashMap::new();
        // Available and filled sections per (round, window) and term, for the fill priors.
        let mut fill_by_term: HashMap<(RoundKind, u8), BTreeMap<Term, (usize, usize)>> = HashMap::new();

        for (i, r) in rows.iter().enumerate() {
            course_rows.entry(r.course_code.clone()).or_default().push(i);
            sessions.insert(r.session.clone());
            if !r.school.is_empty() {
                schools.insert(r.school.clone());
            }
            terms.insert(r.term);
            let e = latest.entry(r.course_code.clone()).or_insert((r.term, r.school.clone()));
            if r.term >= e.0 {
                *e = (r.term, r.school.clone());
            }
            if r.available() {
                let c = fill_by_term.entry((r.round, r.window)).or_default().entry(r.term).or_default();
                c.0 += r.filled() as usize;
                c.1 += 1;
            }
        }

        let mut courses: Vec<CourseInfo> = latest
            .into_iter()
            .map(|(code, (last_term, school))| CourseInfo {
                description: descriptions.get(&code).map(|d| d.1.clone()).unwrap_or_default(),
                code,
                school,
                last_term,
            })
            .collect();
        courses.sort_by(|a, b| a.code.cmp(&b.code));
        let course_index = courses.iter().enumerate().map(|(i, c)| (c.code.clone(), i)).collect();

        let mut fill_prior = HashMap::new();
        let mut fill_history = HashMap::new();
        for (key, by_term) in fill_by_term {
            let mut running = (0usize, 0usize);
            let history: FillHistory = by_term
                .into_iter()
                .map(|(term, (filled, total))| {
                    running = (running.0 + filled, running.1 + total);
                    (term, running.0, running.1)
                })
                .collect();
            if let Some(&(_, filled, total)) = history.last() {
                fill_prior.insert(key, filled as f64 / total.max(1) as f64);
            }
            fill_history.insert(key, history);
        }

        Self {
            rows,
            courses,
            course_index,
            course_rows,
            sessions: sessions.into_iter().collect(),
            schools: schools.into_iter().collect(),
            terms: terms.into_iter().collect(),
            fill_prior,
            fill_history,
            report,
        }
    }

    pub fn course(&self, code: &str) -> Option<&CourseInfo> {
        self.course_index.get(code).map(|&i| &self.courses[i])
    }

    /// All rows of a course, in term order.
    pub fn course_rows<'a>(&'a self, code: &str) -> impl Iterator<Item = &'a BidRow> + 'a {
        self.course_rows
            .get(code)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
            .iter()
            .map(move |&i| &self.rows[i])
    }

    pub fn latest_term(&self) -> Option<Term> {
        self.terms.last().copied()
    }

    /// Share of available sections that filled in this round/window, counting only terms strictly
    /// before `target`. This is the prior a forecast for `target` may use: it never sees the target's
    /// own outcomes or anything later. `None` when there is no such history.
    pub fn fill_prior_before(&self, round: RoundKind, window: u8, target: Term) -> Option<f64> {
        let history = self.fill_history.get(&(round, window))?;
        // Sorted by term, so the terms before `target` are a prefix. Its last entry holds their totals.
        let n_before = history.partition_point(|&(term, _, _)| term < target);
        let (_, filled, total) = *history.get(n_before.checked_sub(1)?)?;
        (total > 0).then(|| filled as f64 / total as f64)
    }

    /// The term students are most likely bidding for next.
    pub fn default_target_term(&self) -> Term {
        self.terms
            .iter()
            .rev()
            .find(|t| !t.kind.is_summer())
            .map(|t| t.next_regular())
            .unwrap_or(Term { year: 2026, kind: TermKind::T1 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(year: u16, kind: TermKind) -> Option<Term> {
        Some(Term { year, kind })
    }

    #[test]
    fn term_parse_accepts_common_spellings() {
        assert_eq!(Term::parse("2026-27 T2"), term(2026, TermKind::T2));
        assert_eq!(Term::parse("2026-27_T2"), term(2026, TermKind::T2));
        assert_eq!(Term::parse("2026-27 Term 2"), term(2026, TermKind::T2));
        assert_eq!(Term::parse("AY2026-27 T2"), term(2026, TermKind::T2));
        assert_eq!(Term::parse("2026-27T1"), term(2026, TermKind::T1));
        assert_eq!(Term::parse("2026 T1"), term(2026, TermKind::T1));
        assert_eq!(Term::parse("2024-25 T3A"), term(2024, TermKind::T3A));
        assert_eq!(Term::parse("2024-25 Term 3B"), term(2024, TermKind::T3B));
        assert_eq!(Term::parse("2024-25 t3b"), term(2024, TermKind::T3B));
        assert_eq!(Term::parse("  2024-25   term 3a  "), term(2024, TermKind::T3A));
        assert_eq!(Term::parse("2026/27 T2"), term(2026, TermKind::T2));
        assert_eq!(Term::parse("2026-2027 T2"), term(2026, TermKind::T2));
        assert_eq!(Term::parse("AY 2026-27 T2"), term(2026, TermKind::T2));
        assert_eq!(Term::parse("2025-26 Term 1"), term(2025, TermKind::T1));
        assert_eq!(Term::parse("2099-00 T1"), term(2099, TermKind::T1));
    }

    #[test]
    fn term_parse_rejects_a_wrong_academic_year_span() {
        for s in [
            "2026-30 T1",
            "2026-28 T1",
            "2026/30 T1",
            "2026-2028 T1",
            "2026-2026 T1",
            "2026-3 T1",
            "2026-270 T1",
            "2026- T1",
        ] {
            assert_eq!(Term::parse(s), None, "{s:?} should not parse");
        }
    }

    #[test]
    fn term_parse_rejects_extra_years_terms_and_words() {
        for s in [
            "2025-26 2026-27 T1",
            "2026 2027 T1",
            "2026 12 T1",
            "2026-27 T2 t1",
            "2026-27 T2 3",
            "2026-27 Term 2 T1",
            "Spring 2026 T1",
            "2026-27 Spring T2",
            "Term 2026-27 T2",
        ] {
            assert_eq!(Term::parse(s), None, "{s:?} should not parse");
        }
    }

    #[test]
    fn term_parse_rejects_garbage() {
        for s in ["", "2026-27", "T2", "hello", "2026 2", "2026-27 T3", "2026-27 T4", "2026-27 T12", "2026-27 Term 9", "1999 T1", "2026-27 T2 extra", "2026-27 Term"] {
            assert_eq!(Term::parse(s), None, "{s:?} should not parse");
        }
    }

    #[test]
    fn term_parse_round_trips_label() {
        for year in 2000..2100 {
            for kind in [TermKind::T1, TermKind::T2, TermKind::T3A, TermKind::T3B] {
                let t = Term { year, kind };
                assert_eq!(Term::parse(&t.label()), Some(t), "{}", t.label());
            }
        }
    }

    fn row(year: u16, kind: TermKind, round: RoundKind, window: u8, before: i32, after: i32) -> BidRow {
        BidRow {
            term: Term { year, kind },
            session: Arc::from("Regular Academic Session"),
            round,
            window,
            course_code: Arc::from("TEST101"),
            section: Arc::from("G1"),
            instructor: Arc::from("A"),
            school: Arc::from("SCIS"),
            vacancy: 40,
            opening_vacancy: 10,
            before_proc: before,
            dice: 0,
            after_proc: after,
            enrolled: before - after,
            median_bid: 20.0,
            min_bid: 20.0,
        }
    }

    /// Round 1 window 1, filled / available per term: 2022 T1 1/4, 2022 T2 2/2, 2022 T3A 0/1,
    /// 2023 T1 1/6, 2023 T2 3/3, 2023 T3A 0/2, 2024 T1 0/2. Round 1 window 2: 2023 T1 1/1, 2024 T1 0/3.
    /// Every term also has one section that was not open for bidding, which must not count.
    fn fill_test_data() -> AppData {
        let plan = [
            (2022, TermKind::T1, 1, 3),
            (2022, TermKind::T2, 2, 0),
            (2022, TermKind::T3A, 0, 1),
            (2023, TermKind::T1, 1, 5),
            (2023, TermKind::T2, 3, 0),
            (2023, TermKind::T3A, 0, 2),
            (2024, TermKind::T1, 0, 2),
        ];
        let mut rows = Vec::new();
        for (year, kind, filled, unfilled) in plan {
            for _ in 0..filled {
                rows.push(row(year, kind, RoundKind::Round1, 1, 10, 0));
            }
            for _ in 0..unfilled {
                rows.push(row(year, kind, RoundKind::Round1, 1, 10, 4));
            }
            rows.push(row(year, kind, RoundKind::Round1, 1, 0, 0));
        }
        rows.push(row(2023, TermKind::T1, RoundKind::Round1, 2, 10, 0));
        for _ in 0..3 {
            rows.push(row(2024, TermKind::T1, RoundKind::Round1, 2, 10, 4));
        }
        AppData::new(rows, Descriptions::new(), LoadReport::default())
    }

    #[test]
    fn fill_prior_before_counts_only_earlier_terms() {
        let data = fill_test_data();
        let r1w1 = |year, kind| data.fill_prior_before(RoundKind::Round1, 1, Term { year, kind });
        assert_eq!(r1w1(2022, TermKind::T1), None, "nothing before the first term");
        // The target's own term is excluded.
        assert_eq!(r1w1(2022, TermKind::T2), Some(1.0 / 4.0));
        assert_eq!(r1w1(2022, TermKind::T3A), Some(3.0 / 6.0));
        assert_eq!(r1w1(2023, TermKind::T1), Some(3.0 / 7.0));
        assert_eq!(r1w1(2023, TermKind::T2), Some(4.0 / 13.0));
        // T3A comes after T2 within a year: excluded for a T2 target, counted for a T3A target.
        assert_eq!(r1w1(2023, TermKind::T3A), Some(7.0 / 16.0));
        assert_eq!(r1w1(2024, TermKind::T1), Some(7.0 / 18.0));
        // After the last term it is the whole-dataset prior.
        assert_eq!(r1w1(2025, TermKind::T1), Some(7.0 / 20.0));
        assert_eq!(data.fill_prior.get(&(RoundKind::Round1, 1)).copied(), Some(7.0 / 20.0));

        let r1w2 = |year, kind| data.fill_prior_before(RoundKind::Round1, 2, Term { year, kind });
        assert_eq!(r1w2(2023, TermKind::T1), None);
        assert_eq!(r1w2(2023, TermKind::T2), Some(1.0));
        assert_eq!(r1w2(2024, TermKind::T1), Some(1.0), "2024 T1's own 0/3 is excluded");
        assert_eq!(r1w2(2025, TermKind::T1), Some(1.0 / 4.0));

        // Windows and rounds that are never offered have no prior at any target.
        let late = Term { year: 2025, kind: TermKind::T1 };
        assert_eq!(data.fill_prior_before(RoundKind::Round1, 3, late), None);
        assert_eq!(data.fill_prior_before(RoundKind::Round2, 1, late), None);
    }

    #[test]
    fn default_target_prior_equals_whole_dataset_prior() {
        // The next term is after all data, so its prior must be exactly the whole-dataset prior.
        let data = fill_test_data();
        let next = data.default_target_term();
        assert_eq!(next, Term { year: 2024, kind: TermKind::T2 });
        for &(round, window) in data.fill_prior.keys() {
            assert_eq!(data.fill_prior_before(round, window, next), data.fill_prior.get(&(round, window)).copied());
        }
    }
}
