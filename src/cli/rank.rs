//! The ranking and checking commands: `instructors`, `hot` and `backtest`.
//!
//! Each one is a thin layer over `data::query` and `data::backtest`: it reads the arguments, asks the
//! query layer, and prints a table or, with `--json`, the same result as JSON.

use super::output::{self, Table};
use super::{count_at_least_one, legacy, parse_window, CliError, Ctx};
use crate::data::backtest::{self, BacktestReport, LevelResult};
use crate::data::filter::WindowSel;
use crate::data::query::{
    hot_courses_with_min_filled, rank_instructors, DEFAULT_HOT_MIN_FILLED, round2, round4, HotCourse, InstructorQuery, InstructorSort,
    DEFAULT_MIN_COMPARED,
};
use crate::data::stats::InstructorStats;
use crate::data::{AppData, BidRow, Descriptions, Term};
use clap::Args;
use serde::Serialize;

/// The orders `instructors --sort` accepts, as the user types them.
const SORT_NAMES: [&str; 4] = ["demand", "easiest", "clearing", "sections"];

/// The line under the `instructors` table that explains the VS PEERS column.
const PREMIUM_KEY: &str = "+40% = getting into their section cost ~40% more than other sections of the same course, term and window; shrunk toward 0 when there are few comparisons.";

#[derive(Debug, Args)]
pub struct InstructorsArgs {
    /// Only instructors who taught this course. When set, --min-compared does not apply.
    #[arg(long, value_name = "CODE")]
    pub course: Option<String>,
    /// Round and window, e.g. R1W1, R1AW2 or 1:1. "R1" means every window of that round, and "all" every round and window together.
    #[arg(long, default_value = "R1W1", value_name = "W")]
    pub window: String,
    /// Only instructors whose sections were offered by this school. Peer comparisons then use only this school's sections.
    #[arg(long, value_name = "SCHOOL")]
    pub school: Option<String>,
    /// Only instructors whose name contains this text. Case-insensitive.
    #[arg(long, value_name = "NAME")]
    pub search: Option<String>,
    /// Hide instructors with fewer comparable sections than this. Does not apply with --course.
    ///
    /// Below about ten comparisons, a few unusual sections can swing the premium a long way. Lower this to
    /// see more instructors, with that noise.
    #[arg(long, default_value_t = DEFAULT_MIN_COMPARED, value_name = "N")]
    pub min_compared: usize,
    /// The order: demand (hardest to get into first), easiest (lowest premium first), clearing (highest median
    /// price first) or sections (most sections first).
    #[arg(long, default_value = "demand", value_name = "ORDER")]
    pub sort: String,
    /// Show at most this many instructors.
    #[arg(long, default_value_t = 30, value_name = "N", value_parser = count_at_least_one)]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct HotArgs {
    /// Term to rank, e.g. "2025-26 T2". Defaults to the latest regular (non-summer) term in the data, the one the
    /// Overview tab ranks.
    #[arg(long, value_name = "TERM")]
    pub term: Option<String>,
    /// Round and window, e.g. R1W1, R1AW2 or 1:1. "R1" means every window of that round, and "all" every round and window together. Prices differ between windows, so "all" is only a rough view.
    #[arg(long, default_value = "R1W1", value_name = "W")]
    pub window: String,
    /// Rank only courses with at least this many filled sections. A median from one filled section is one
    /// price, so one course can top the list on a single section; the default of 2 leaves those out. Use 1
    /// to rank every course with a filled section.
    #[arg(long, default_value_t = DEFAULT_HOT_MIN_FILLED, value_name = "N", value_parser = count_at_least_one)]
    pub min_filled: usize,
    /// Show at most this many courses.
    #[arg(long, default_value_t = 25, value_name = "N", value_parser = count_at_least_one)]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct BacktestArgs {
    /// Round and window to check, e.g. R1W1, R1AW2 or 1:1. "R1" means every window of that round, and "all" every round and window together. With neither this flag nor --all-windows, R1W1 is used.
    #[arg(long, value_name = "W", conflicts_with = "all_windows")]
    pub window: Option<String>,
    /// Check every window together, not one.
    #[arg(long)]
    pub all_windows: bool,
    /// Grid-search the model's parameters (slow) instead of checking the current ones. Scores every window
    /// together, and prints text only.
    #[arg(long, conflicts_with = "window")]
    pub tune: bool,
}

// ---------------------------------------------------------------------------------------------------
// instructors

pub fn run_instructors(ctx: &Ctx, args: &InstructorsArgs) -> Result<(), CliError> {
    let (out, shown) = instructor_report(ctx, args)?;
    ctx.warn_text(&out.warnings);
    ctx.emit(&out, || print_instructors(&shown, out.total, &out.window))
}

/// The instructors `args` asks for, as the JSON view and as the full-precision rows the table is drawn from.
fn instructor_report(ctx: &Ctx, args: &InstructorsArgs) -> Result<(InstructorsOut, Vec<InstructorStats>), CliError> {
    let sel = parse_window(&args.window)?;
    let sort = parse_sort(&args.sort)?;
    let course = args.course.as_deref().map(|input| ctx.course(input)).transpose()?;
    let course_code = course.map(|c| &*c.code);
    let school = args.school.as_deref().map(|input| ctx.school(input)).transpose()?;
    let search = args.search.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let query = InstructorQuery {
        sel,
        school: school.as_deref(),
        course: course_code,
        search,
        min_compared: args.min_compared,
        sort,
    };
    let (total, shown) = page(rank_instructors(&ctx.data, &ctx.filters, &query), args.limit);
    let mut warnings = ctx.data_warnings();
    if !sel.is_specific() {
        warnings.push(
            "Prices differ a lot between windows, and this list mixes them. Use a specific window such as R1W1 for meaningful clearing prices and fill rates."
                .to_string(),
        );
    }
    let out = InstructorsOut {
        window: sel.label(),
        window_code: sel.short_label(),
        course: course_code.map(str::to_string),
        school,
        search: search.map(str::to_string),
        sort: sort_name(sort),
        // The query layer ignores --min-compared for one course, so the output reports it as null.
        min_compared: course_code.is_none().then_some(args.min_compared),
        limit: args.limit,
        total,
        shown: shown.len(),
        warnings,
        instructors: shown.iter().map(InstructorOut::from).collect(),
    };
    Ok((out, shown))
}

fn parse_sort(input: &str) -> Result<InstructorSort, CliError> {
    InstructorSort::parse(input).ok_or_else(|| {
        CliError::usage(format!(
            "invalid sort \"{}\"; use one of {}",
            input.trim(),
            SORT_NAMES.join(", ")
        ))
    })
}

fn sort_name(sort: InstructorSort) -> &'static str {
    match sort {
        InstructorSort::MostInDemand => "demand",
        InstructorSort::EasiestToGet => "easiest",
        InstructorSort::ClearingPrice => "clearing",
        InstructorSort::Sections => "sections",
    }
}

#[derive(Serialize)]
struct InstructorsOut {
    window: String,
    window_code: String,
    /// The course code as the data spells it, or null for every course.
    course: Option<String>,
    /// The school as the data spells it, or null for every school.
    school: Option<String>,
    search: Option<String>,
    sort: &'static str,
    /// Null when --course is set, because the minimum does not apply to one course.
    min_compared: Option<usize>,
    /// The most instructors to list, as asked with --limit.
    limit: usize,
    /// How many instructors match, before --limit.
    total: usize,
    /// How many are in `instructors`.
    shown: usize,
    /// Things to know before trusting the numbers, such as a window that mixes prices.
    warnings: Vec<String>,
    instructors: Vec<InstructorOut>,
}

/// One instructor as JSON. `premium` is a fraction (0.4 means their sections cost 40% more than their peers')
/// and is null when none of their sections had a peer to compare with. Fill rate and prices are rounded.
#[derive(Serialize)]
struct InstructorOut {
    name: String,
    school: String,
    courses: usize,
    sections: usize,
    fill_rate: f64,
    median_clearing: Option<f64>,
    premium: Option<f64>,
    compared: usize,
    last_term: String,
}

impl From<&InstructorStats> for InstructorOut {
    fn from(s: &InstructorStats) -> Self {
        InstructorOut {
            name: s.name.clone(),
            school: s.school.clone(),
            courses: s.courses,
            sections: s.sections,
            fill_rate: round4(s.fill_rate),
            median_clearing: s.median_clearing.map(round2),
            premium: (s.compared > 0).then(|| round4(s.premium)),
            compared: s.compared,
            last_term: s.last_term.label(),
        }
    }
}

fn print_instructors(list: &[InstructorStats], total: usize, window: &str) {
    if !list.is_empty() {
        let mut table = Table::new(&[
            "#",
            "INSTRUCTOR",
            "SCHOOL",
            "VS PEERS",
            "COMPARED",
            "SECTIONS",
            "FILLED",
            "MEDIAN CLEARING",
            "COURSES",
            "LAST TAUGHT",
        ])
        .right_align(&[0, 3, 4, 5, 6, 7, 8])
        .max_width(1, 30);
        for (i, s) in list.iter().enumerate() {
            table.push(vec![
                (i + 1).to_string(),
                s.name.clone(),
                or_dash(&s.school),
                premium_cell(s),
                s.compared.to_string(),
                s.sections.to_string(),
                output::pct(s.fill_rate),
                output::opt_money(s.median_clearing),
                s.courses.to_string(),
                s.last_term.label(),
            ]);
        }
        println!("{}\n", table.render());
    }
    println!("{} of {total} instructors in {window}", list.len());
    println!("{PREMIUM_KEY}");
    if list.iter().any(|s| s.compared == 0) {
        println!("- = none of their sections had a peer section to compare with");
    }
}

/// The VS PEERS cell: the premium as a signed percentage, or "-" when there was no peer to compare with.
fn premium_cell(s: &InstructorStats) -> String {
    if s.compared == 0 {
        "-".to_string()
    } else {
        premium_pct(s.premium)
    }
}

/// A premium as a signed whole percentage, as the GUI shows it: 0.4 is "+40%". A value that rounds to zero
/// reads "+0%", never "-0%".
fn premium_pct(premium: f64) -> String {
    let percent = (premium * 100.0).round() + 0.0; // adding 0.0 turns a negative zero into a positive one
    format!("{percent:+.0}%")
}

// ---------------------------------------------------------------------------------------------------
// hot

pub fn run_hot(ctx: &Ctx, args: &HotArgs) -> Result<(), CliError> {
    let out = hot_report(ctx, args)?;
    ctx.warn_text(&out.warnings);
    ctx.emit(&out, || print_hot(&out))
}

fn hot_report(ctx: &Ctx, args: &HotArgs) -> Result<HotOut, CliError> {
    let sel = parse_window(&args.window)?;
    let term = match &args.term {
        Some(input) => ctx.known_term(input)?,
        None => latest_regular_term(&ctx.data)?,
    };
    // The filter comes before the ranking, so `total` and the list both count only the courses it keeps.
    let ranked = hot_courses_with_min_filled(&ctx.data, &ctx.filters, term, sel, args.min_filled);
    let (total, courses) = page(ranked, args.limit);
    let mut warnings = ctx.data_warnings();
    if !sel.is_specific() {
        warnings.push(format!(
            "Prices differ a lot between windows, so ranking across {} mixes them. Use a specific window such as R1W1.",
            sel.label()
        ));
    }
    Ok(HotOut {
        term: term.label(),
        window: sel.label(),
        window_code: sel.short_label(),
        min_filled: args.min_filled,
        limit: args.limit,
        total,
        shown: courses.len(),
        warnings,
        courses,
    })
}

/// The latest term that is not a summer term: the term the Overview tab ranks.
fn latest_regular_term(data: &AppData) -> Result<Term, CliError> {
    crate::data::query::latest_regular_term(data)
        .ok_or_else(|| CliError::not_found("the data has no regular (non-summer) term to rank"))
}

#[derive(Serialize)]
struct HotOut {
    term: String,
    window: String,
    window_code: String,
    /// The fewest filled sections a course needed to be ranked, as asked with --min-filled.
    min_filled: usize,
    /// The most courses to list, as asked with --limit.
    limit: usize,
    /// How many courses had at least `min_filled` filled sections, before --limit.
    total: usize,
    /// How many are in `courses`.
    shown: usize,
    /// Things to know before trusting the numbers, such as a window that mixes prices.
    warnings: Vec<String>,
    courses: Vec<HotCourse>,
}

/// The cut-off in words, for the empty-result line: "a filled section" or "2 or more filled sections".
fn filled_at_least(n: usize) -> String {
    if n == 1 {
        "a filled section".to_string()
    } else {
        format!("{n} or more filled sections")
    }
}

fn print_hot(out: &HotOut) {
    if out.courses.is_empty() {
        println!(
            "No course had {} in {}, {}.",
            filled_at_least(out.min_filled),
            out.term,
            out.window
        );
        return;
    }
    let mut table = Table::new(&["#", "COURSE", "TITLE", "MEDIAN CLEARING", "FILLED"])
        .right_align(&[0, 3, 4])
        .max_width(2, 50);
    for (i, c) in out.courses.iter().enumerate() {
        table.push(vec![
            (i + 1).to_string(),
            c.code.clone(),
            or_dash(&c.description),
            output::money(c.median_clearing),
            format!("{}/{}", c.sections_filled, c.sections_available),
        ]);
    }
    println!("{}\n", table.render());
    println!("{} of {} courses in {}, {}", out.shown, out.total, out.term, out.window);
}

// ---------------------------------------------------------------------------------------------------
// backtest

pub fn run_backtest(ctx: &Ctx, args: &BacktestArgs) -> Result<(), CliError> {
    if args.tune {
        return run_tune(ctx);
    }
    let (out, report) = backtest_report(ctx, args)?;
    ctx.warn_text(&out.warnings);
    ctx.emit(&out, || print_backtest(&report))
}

/// `--tune`: the legacy grid search, which prints its own progress table and scores every window together.
fn run_tune(ctx: &Ctx) -> Result<(), CliError> {
    if ctx.json {
        return Err(CliError::usage(
            "--tune prints a progress table and has no JSON form; run it without --json",
        ));
    }
    let filtered = backtest_data(ctx);
    legacy::tune_params(filtered.as_ref().unwrap_or(&ctx.data), ctx.params);
    Ok(())
}

/// The windows the backtest checks: every window with `--all-windows` (None), one with `--window`, or R1W1.
fn backtest_scope(args: &BacktestArgs) -> Result<Option<WindowSel>, CliError> {
    if args.all_windows {
        return Ok(None);
    }
    match &args.window {
        Some(input) => parse_window(input).map(Some),
        None => Ok(Some(WindowSel::ROUND1_W1)),
    }
}

fn backtest_report(ctx: &Ctx, args: &BacktestArgs) -> Result<(BacktestOut, BacktestReport), CliError> {
    let scope = backtest_scope(args)?;
    let filtered = backtest_data(ctx);
    let report = backtest::run(filtered.as_ref().unwrap_or(&ctx.data), scope, &ctx.params);
    let mut warnings = ctx.data_warnings();
    if report.sections == 0 {
        warnings.push(if report.targets.is_empty() {
            "Nothing was checked: the data has fewer than four regular terms, and the first three only serve as history.".to_string()
        } else {
            "Nothing was checked: no section in this scope could be predicted from the earlier terms.".to_string()
        });
    }
    let out = BacktestOut {
        scope: report.scope.clone(),
        window_code: scope.map_or_else(|| "all".to_string(), |s| s.short_label()),
        targets: report.targets.iter().map(Term::label).collect(),
        sections: report.sections,
        warnings,
        methods: report
            .methods
            .iter()
            .map(|m| MethodOut {
                name: m.name,
                levels: m.levels.iter().map(level_out).collect(),
            })
            .collect(),
    };
    Ok((out, report))
}

/// The data with the sessions `--exclude-session` names left out, or `None` when none are excluded.
/// `backtest::run` applies no session filter of its own, so the rows are filtered here.
fn backtest_data(ctx: &Ctx) -> Option<AppData> {
    if ctx.filters.excluded_sessions.is_empty() {
        return None;
    }
    let rows: Vec<BidRow> = ctx
        .data
        .rows
        .iter()
        .filter(|r| ctx.filters.allows(r))
        .cloned()
        .collect();
    let descriptions: Descriptions = ctx
        .data
        .courses
        .iter()
        .map(|c| (c.code.clone(), (c.last_term, c.description.clone())))
        .collect();
    Some(AppData::new(rows, descriptions, ctx.data.report.clone()))
}

#[derive(Serialize)]
struct BacktestOut {
    scope: String,
    window_code: String,
    targets: Vec<String>,
    sections: usize,
    /// Things to know before trusting the numbers, such as nothing having been checked.
    warnings: Vec<String>,
    methods: Vec<MethodOut>,
}

#[derive(Serialize)]
struct MethodOut {
    name: &'static str,
    levels: Vec<LevelOut>,
}

#[derive(Serialize)]
struct LevelOut {
    /// The chance the bid aims for, e.g. 0.8. Null for the naive baseline, which has no target chance.
    level: Option<f64>,
    /// How many sections were checked at this level.
    n: usize,
    /// The share of sections where the bid would have won a seat.
    hit_rate: Option<f64>,
    /// The average bid, in e$.
    mean_bid: Option<f64>,
    /// The quantile loss, lower is better. Null for the naive baseline.
    mean_pinball: Option<f64>,
}

/// One result as JSON. The naive baseline records NaN as its level and has no quantile loss. A level that
/// checked no section was never recorded, so its figures say nothing and are null too.
fn level_out(l: &LevelResult) -> LevelOut {
    let baseline = l.level.is_nan();
    let checked = l.n > 0;
    LevelOut {
        level: if baseline || !checked {
            None
        } else {
            finite(round4(l.level))
        },
        n: l.n,
        hit_rate: if checked { finite(round4(l.hit_rate())) } else { None },
        mean_bid: if checked { finite(round2(l.mean_bid())) } else { None },
        mean_pinball: if checked && !baseline {
            finite(round4(l.mean_pinball()))
        } else {
            None
        },
    }
}

/// A finite number, or `None`. JSON has no NaN or infinity, so those become null.
fn finite(x: f64) -> Option<f64> {
    x.is_finite().then_some(x)
}

/// The text view: the report's table, or with nothing checked just its header (the reason is a warning).
fn print_backtest(report: &BacktestReport) {
    if report.sections == 0 {
        println!("Backtest: {} | 0 sections", output::ascii(&report.scope));
        return;
    }
    println!("{}", output::ascii(&report.to_text()));
}

// ---------------------------------------------------------------------------------------------------
// shared helpers

/// Splits a ranked list into how many there were and the first `limit` of them.
fn page<T>(mut all: Vec<T>, limit: usize) -> (usize, Vec<T>) {
    let total = all.len();
    all.truncate(limit);
    (total, all)
}

fn or_dash(text: &str) -> String {
    if text.trim().is_empty() {
        "-".to_string()
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Command, ErrorKind};
    use crate::data::filter::GlobalFilters;
    use crate::data::model::ModelParams;
    use crate::data::{LoadReport, RoundKind, TermKind};
    use clap::Parser;
    use std::collections::{BTreeSet, HashMap};
    use std::sync::Arc;

    fn term(year: u16, kind: TermKind) -> Term {
        Term { year, kind }
    }

    /// One round 1 window 1 section. A filled section cleared at `min_bid`; an unfilled one has no clearing price.
    fn bid(term: Term, course: &str, section: &str, instructor: &str, min_bid: f64, filled: bool) -> BidRow {
        BidRow {
            term,
            session: Arc::from("Regular Academic Session"),
            round: RoundKind::Round1,
            window: 1,
            course_code: Arc::from(course),
            section: Arc::from(section),
            instructor: Arc::from(instructor),
            school: Arc::from("SCIS"),
            vacancy: 40,
            opening_vacancy: 10,
            before_proc: 10,
            dice: 0,
            after_proc: if filled { 0 } else { 4 },
            enrolled: 10,
            median_bid: min_bid * 1.2,
            min_bid,
        }
    }

    /// IS111 (SCIS) has three sections in each regular term, one of them unfilled. IS112 (SOSS) has one section
    /// a term, so nothing to compare it with. IS113 has one section in the 1A session. A summer term sits after
    /// the last regular term, which the default `hot` term must skip.
    fn sample_data() -> AppData {
        let regular = [
            term(2024, TermKind::T1),
            term(2024, TermKind::T2),
            term(2025, TermKind::T1),
            term(2025, TermKind::T2),
        ];
        let mut rows = Vec::new();
        for (i, t) in regular.into_iter().enumerate() {
            let base = 20.0 + i as f64;
            rows.push(bid(t, "IS111", "G1", "Alice Tan", base + 20.0, true));
            rows.push(bid(t, "IS111", "G2", "Bob Tan", base + 5.0, true));
            rows.push(bid(t, "IS111", "G3", "Carol Lee", 10.0, false));
            rows.push(BidRow {
                school: Arc::from("SOSS"),
                ..bid(t, "IS112", "G1", "Dan Ng", 15.0, true)
            });
        }
        rows.push(bid(term(2025, TermKind::T3A), "IS111", "G1", "Alice Tan", 50.0, true));
        rows.push(BidRow {
            session: Arc::from("1A Session"),
            ..bid(term(2025, TermKind::T2), "IS113", "G1", "Eve Ong", 30.0, true)
        });
        let descriptions: Descriptions = [
            ("IS111", "Business Intelligence"),
            ("IS112", "Data Mining"),
            ("IS113", "Robotics"),
        ]
        .into_iter()
        .map(|(code, title)| (Arc::from(code), (term(2025, TermKind::T2), title.to_string())))
        .collect();
        AppData::new(rows, descriptions, LoadReport::default())
    }

    fn ctx(filters: GlobalFilters, json: bool) -> Ctx {
        Ctx {
            data: sample_data(),
            filters,
            params: ModelParams::default(),
            json,
        }
    }

    /// The CLI default is 10 comparisons, which would hide every instructor in this small sample, so the
    /// tests set a minimum of 3 that keeps them.
    fn instructor_args() -> InstructorsArgs {
        InstructorsArgs {
            course: None,
            window: "R1W1".into(),
            school: None,
            search: None,
            min_compared: 3,
            sort: "demand".into(),
            limit: 30,
        }
    }

    fn hot_args() -> HotArgs {
        HotArgs {
            term: None,
            window: "R1W1".into(),
            min_filled: 2,
            limit: 25,
        }
    }

    fn backtest_args() -> BacktestArgs {
        BacktestArgs {
            window: None,
            all_windows: false,
            tune: false,
        }
    }

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("smu_bid_analyser").chain(args.iter().copied()))
            .unwrap_or_else(|err| panic!("{err}"))
    }

    #[test]
    fn sort_names_parse_and_a_bad_one_lists_the_choices() {
        for name in SORT_NAMES {
            assert_eq!(sort_name(parse_sort(name).expect("valid")), name);
        }
        assert_eq!(
            sort_name(parse_sort(" Easiest ").expect("case and spaces are ignored")),
            "easiest"
        );
        let err = parse_sort("nonsense").expect_err("not a sort");
        assert_eq!(err.kind, ErrorKind::Usage);
        assert_eq!(err.exit_code(), 2);
        assert!(
            err.message.contains("demand, easiest, clearing, sections"),
            "{}",
            err.message
        );
    }

    #[test]
    fn instructors_hide_thin_comparisons_and_rank_by_demand() {
        let ctx = ctx(GlobalFilters::default(), true);
        let (out, shown) = instructor_report(&ctx, &instructor_args()).expect("runs");
        // Dan Ng and Eve Ong have no peer sections, so they are below the minimum of 3 comparisons.
        assert_eq!(out.total, 3);
        let names: Vec<&str> = shown.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Alice Tan", "Bob Tan", "Carol Lee"]);
        assert_eq!(
            (out.window_code.as_str(), out.sort, out.min_compared),
            ("R1W1", "demand", Some(3))
        );

        let carol = out.instructors.iter().find(|i| i.name == "Carol Lee").expect("listed");
        assert_eq!((carol.fill_rate, carol.median_clearing), (0.0, None));
        let alice = out.instructors.iter().find(|i| i.name == "Alice Tan").expect("listed");
        assert_eq!(alice.compared, 4);
        assert!(alice.premium.expect("Alice has peers") > 0.0);
    }

    #[test]
    fn one_course_keeps_instructors_without_peers_and_ignores_min_compared() {
        let ctx = ctx(GlobalFilters::default(), true);
        let args = InstructorsArgs {
            course: Some("is 112".into()),
            ..instructor_args()
        };
        let (out, _) = instructor_report(&ctx, &args).expect("runs");
        assert_eq!(out.course.as_deref(), Some("IS112"));
        assert_eq!(out.min_compared, None);
        assert_eq!(out.instructors.len(), 1);
        let dan = &out.instructors[0];
        assert_eq!(dan.name, "Dan Ng");
        assert_eq!(
            (dan.compared, dan.premium),
            (0, None),
            "no peer means no premium, not a fake zero"
        );
    }

    #[test]
    fn school_and_search_filters_narrow_the_list() {
        let ctx = ctx(GlobalFilters::default(), true);
        let school = InstructorsArgs {
            school: Some("soss".into()),
            min_compared: 0,
            ..instructor_args()
        };
        let (out, _) = instructor_report(&ctx, &school).expect("runs");
        assert_eq!(out.school.as_deref(), Some("SOSS"));
        let names: Vec<&str> = out.instructors.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["Dan Ng"]);

        let search = InstructorsArgs {
            search: Some(" CAROL ".into()),
            min_compared: 0,
            ..instructor_args()
        };
        let (out, _) = instructor_report(&ctx, &search).expect("runs");
        assert_eq!((out.search.as_deref(), out.total), (Some("CAROL"), 1));

        let unknown = InstructorsArgs {
            school: Some("NOPE".into()),
            ..instructor_args()
        };
        assert_eq!(
            instructor_report(&ctx, &unknown).err().map(|e| e.kind),
            Some(ErrorKind::NotFound)
        );
    }

    #[test]
    fn an_unknown_sort_is_a_usage_error() {
        let ctx = ctx(GlobalFilters::default(), false);
        let args = InstructorsArgs {
            sort: "nonsense".into(),
            ..instructor_args()
        };
        let err = instructor_report(&ctx, &args).err().expect("rejected");
        assert_eq!(err.kind, ErrorKind::Usage);
        assert!(
            err.message.contains("demand, easiest, clearing, sections"),
            "{}",
            err.message
        );
    }

    #[test]
    fn limit_shows_the_top_rows_and_keeps_the_total() {
        let ctx = ctx(GlobalFilters::default(), true);
        let args = InstructorsArgs {
            limit: 2,
            ..instructor_args()
        };
        let (out, shown) = instructor_report(&ctx, &args).expect("runs");
        assert_eq!((out.total, out.shown, shown.len()), (3, 2, 2));
        assert_eq!(out.instructors[0].name, "Alice Tan");
    }

    #[test]
    fn instructors_echo_the_limit_and_warn_when_the_windows_are_mixed() {
        let ctx = ctx(GlobalFilters::default(), true);
        let (out, _) = instructor_report(&ctx, &instructor_args()).expect("runs");
        assert_eq!(out.limit, 30);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);

        let all = InstructorsArgs {
            window: "all".into(),
            min_compared: 0,
            ..instructor_args()
        };
        let (out, _) = instructor_report(&ctx, &all).expect("runs");
        assert!(
            out.warnings.iter().any(|w| w.contains("mixes them")),
            "{:?}",
            out.warnings
        );
        let value = serde_json::to_value(&out).expect("serialises");
        assert_eq!(value["limit"], serde_json::json!(30));
        assert!(value["warnings"].as_array().is_some_and(|w| !w.is_empty()), "{value}");
    }

    #[test]
    fn premium_reads_as_a_signed_whole_percentage() {
        assert_eq!(premium_pct(0.4), "+40%");
        assert_eq!(premium_pct(-0.123), "-12%");
        assert_eq!(premium_pct(1.5), "+150%");
        assert_eq!(premium_pct(0.0), "+0%");
        assert_eq!(premium_pct(-0.0004), "+0%");
    }

    #[test]
    fn a_premium_without_peers_is_a_dash() {
        let stats = |compared: usize, premium: f64| InstructorStats {
            name: "X".into(),
            school: "SCIS".into(),
            courses: 1,
            sections: 2,
            fill_rate: 0.5,
            median_clearing: None,
            premium,
            compared,
            last_term: term(2025, TermKind::T2),
        };
        assert_eq!(premium_cell(&stats(0, 0.0)), "-");
        assert_eq!(premium_cell(&stats(2, 0.4)), "+40%");
    }

    #[test]
    fn instructor_json_rounds_its_fractions() {
        let stats = InstructorStats {
            name: "X".into(),
            school: "SCIS".into(),
            courses: 1,
            sections: 3,
            fill_rate: 2.0 / 3.0,
            median_clearing: Some(23.355),
            premium: 0.123456,
            compared: 3,
            last_term: term(2025, TermKind::T2),
        };
        let out = InstructorOut::from(&stats);
        assert_eq!(out.fill_rate, 0.6667);
        assert_eq!(out.premium, Some(0.1235));
        assert_eq!(out.last_term, "2025-26 T2");
    }

    #[test]
    fn hot_defaults_to_the_latest_regular_term_and_two_filled_sections() {
        let ctx = ctx(GlobalFilters::default(), true);
        let out = hot_report(&ctx, &hot_args()).expect("runs");
        // The summer term is newer, but the default skips summer terms, as the Overview tab does.
        assert_eq!((out.term.as_str(), out.window_code.as_str()), ("2025-26 T2", "R1W1"));
        // IS112 and IS113 each have one filled section in this term, so the default leaves them out.
        assert_eq!((out.total, out.shown), (1, 1));
        assert_eq!(out.courses[0].code, "IS111");
        assert_eq!(out.courses[0].median_clearing, 35.5);
        assert!(out.warnings.is_empty());
        assert_eq!((out.min_filled, out.limit), (2, 25), "the minimum and the limit are echoed back");
        let value = serde_json::to_value(&out).expect("serialises");
        assert_eq!(
            (value["min_filled"].clone(), value["limit"].clone(), value["warnings"].clone()),
            (serde_json::json!(2), serde_json::json!(25), serde_json::json!([]))
        );
    }

    #[test]
    fn min_filled_of_one_ranks_every_course_with_a_filled_section() {
        let ctx = ctx(GlobalFilters::default(), true);
        let out = hot_report(&ctx, &HotArgs { min_filled: 1, ..hot_args() }).expect("runs");
        assert_eq!((out.total, out.shown), (3, 3));
        let codes: Vec<&str> = out.courses.iter().map(|c| c.code.as_str()).collect();
        assert_eq!(codes, vec!["IS111", "IS113", "IS112"]);
        assert_eq!(out.courses[0].median_clearing, 35.5);
        // The default minimum of two leaves out IS112 and IS113, which each have one filled section.
        let default = hot_report(&ctx, &hot_args()).expect("runs");
        assert_eq!((default.total, default.courses.len()), (1, 1));
    }

    #[test]
    fn hot_respects_session_exclusions_and_limit() {
        let excluded = GlobalFilters {
            excluded_sessions: BTreeSet::from(["1A Session".to_string()]),
        };
        let one = HotArgs { min_filled: 1, ..hot_args() };
        let out = hot_report(&ctx(excluded, true), &one).expect("runs");
        assert_eq!(out.total, 2);

        let limited = HotArgs { limit: 1, min_filled: 1, ..hot_args() };
        let out = hot_report(&ctx(GlobalFilters::default(), true), &limited).expect("runs");
        assert_eq!((out.total, out.shown, out.courses.len()), (3, 1, 1));
    }

    #[test]
    fn the_empty_result_names_the_cut_off() {
        assert_eq!(filled_at_least(1), "a filled section");
        assert_eq!(filled_at_least(2), "2 or more filled sections");
    }

    #[test]
    fn a_window_that_is_not_specific_gets_a_warning() {
        let ctx = ctx(GlobalFilters::default(), true);
        let all = HotArgs {
            window: "all".into(),
            ..hot_args()
        };
        let out = hot_report(&ctx, &all).expect("runs");
        assert_eq!(out.window_code, "all");
        assert!(
            out.warnings.iter().any(|w| w.contains("mixes them")),
            "{:?}",
            out.warnings
        );
    }

    #[test]
    fn hot_checks_its_term_and_window() {
        let ctx = ctx(GlobalFilters::default(), false);
        let future = HotArgs {
            term: Some("2030-31 T1".into()),
            ..hot_args()
        };
        assert_eq!(
            hot_report(&ctx, &future).err().map(|e| e.kind),
            Some(ErrorKind::NotFound)
        );
        let junk = HotArgs {
            term: Some("next year".into()),
            ..hot_args()
        };
        assert_eq!(hot_report(&ctx, &junk).err().map(|e| e.kind), Some(ErrorKind::Usage));
        let window = HotArgs {
            window: "R9W9".into(),
            ..hot_args()
        };
        assert_eq!(hot_report(&ctx, &window).err().map(|e| e.kind), Some(ErrorKind::Usage));
    }

    #[test]
    fn the_default_hot_term_needs_a_regular_term() {
        let empty = AppData::new(Vec::new(), HashMap::new(), LoadReport::default());
        assert_eq!(
            latest_regular_term(&empty).err().map(|e| e.kind),
            Some(ErrorKind::NotFound)
        );
    }

    #[test]
    fn backtest_scope_defaults_to_round1_window1() {
        assert_eq!(
            backtest_scope(&backtest_args()).expect("valid"),
            Some(WindowSel::ROUND1_W1)
        );
        let all = BacktestArgs {
            all_windows: true,
            ..backtest_args()
        };
        assert_eq!(backtest_scope(&all).expect("valid"), None);
        let one = BacktestArgs {
            window: Some("R1AW2".into()),
            ..backtest_args()
        };
        assert_eq!(
            backtest_scope(&one).expect("valid").map(|s| s.short_label()),
            Some("R1AW2".to_string())
        );
        let bad = BacktestArgs {
            window: Some("R9W9".into()),
            ..backtest_args()
        };
        assert_eq!(backtest_scope(&bad).err().map(|e| e.kind), Some(ErrorKind::Usage));
    }

    #[test]
    fn backtest_levels_turn_nan_and_unchecked_results_into_nulls() {
        // The naive baseline: NaN level, so no target and no quantile loss.
        let baseline = LevelResult {
            level: f64::NAN,
            n: 3,
            hits: 2,
            total_bid: 30.0,
            pinball: 0.0,
        };
        let out = level_out(&baseline);
        assert_eq!((out.level, out.mean_pinball), (None, None));
        assert_eq!((out.n, out.hit_rate, out.mean_bid), (3, Some(0.6667), Some(10.0)));

        let model = LevelResult {
            level: 0.8,
            n: 4,
            hits: 3,
            total_bid: 40.0,
            pinball: 1.0,
        };
        let out = level_out(&model);
        assert_eq!(
            (out.level, out.hit_rate, out.mean_bid, out.mean_pinball),
            (Some(0.8), Some(0.75), Some(10.0), Some(0.25))
        );

        // A level that checked nothing: its default level and its rates say nothing.
        let unchecked = level_out(&LevelResult::default());
        assert_eq!(unchecked.n, 0);
        assert_eq!(
            (
                unchecked.level,
                unchecked.hit_rate,
                unchecked.mean_bid,
                unchecked.mean_pinball
            ),
            (None, None, None, None)
        );
    }

    #[test]
    fn backtest_json_has_no_nan_and_marks_the_baseline_null() {
        let ctx = ctx(GlobalFilters::default(), true);
        let (out, report) = backtest_report(&ctx, &backtest_args()).expect("runs");
        assert_eq!(out.window_code, "R1W1");
        assert_eq!(out.targets, vec!["2025-26 T2".to_string()]);
        assert_eq!(out.methods.len(), report.methods.len());
        let text = serde_json::to_string(&out).expect("serialises");
        assert!(!text.contains("NaN"), "{text}");

        let baseline = out.methods.last().expect("the baseline is the last method");
        assert!(baseline
            .levels
            .iter()
            .all(|l| l.level.is_none() && l.mean_pinball.is_none()));
        if out.sections > 0 {
            let model = &out.methods[0];
            assert!(model
                .levels
                .iter()
                .all(|l| l.level.is_some() && l.mean_pinball.is_some()));
        }
    }

    #[test]
    fn backtest_says_when_nothing_was_checked() {
        let ctx = ctx(GlobalFilters::default(), true);
        // IS111 has no rows in Round 2, so the scope has no section to check.
        let empty = BacktestArgs {
            window: Some("R2".into()),
            ..backtest_args()
        };
        let (out, _) = backtest_report(&ctx, &empty).expect("runs");
        assert_eq!(out.sections, 0);
        assert!(
            out.warnings.iter().any(|w| w.starts_with("Nothing was checked")),
            "{:?}",
            out.warnings
        );
        let (checked, _) = backtest_report(&ctx, &backtest_args()).expect("runs");
        assert!(checked.sections > 0 && checked.warnings.is_empty(), "{:?}", checked.warnings);
    }

    #[test]
    fn excluded_sessions_are_left_out_of_the_backtest_data() {
        let plain = ctx(GlobalFilters::default(), false);
        assert!(backtest_data(&plain).is_none());

        let excluded = ctx(
            GlobalFilters {
                excluded_sessions: BTreeSet::from(["1A Session".to_string()]),
            },
            false,
        );
        let data = backtest_data(&excluded).expect("a session is excluded");
        assert_eq!(data.rows.len(), excluded.data.rows.len() - 1);
        assert!(data.course("IS113").is_none());
        assert!(data.course("IS111").is_some());
    }

    #[test]
    fn tune_has_no_json_form() {
        let ctx = ctx(GlobalFilters::default(), true);
        let err = run_tune(&ctx).expect_err("tune is text only");
        assert_eq!(err.kind, ErrorKind::Usage);
        assert!(err.message.contains("--tune"), "{}", err.message);
    }

    #[test]
    fn page_splits_the_list_and_counts_all_of_it() {
        assert_eq!(page(vec![1, 2, 3], 2), (3, vec![1, 2]));
        assert_eq!(page(vec![1], 5), (1, vec![1]));
        assert_eq!(page(Vec::<u8>::new(), 3), (0, Vec::new()));
    }

    #[test]
    fn the_flags_parse_with_their_defaults_and_limits() {
        let Some(Command::Instructors(args)) = parse(&["instructors"]).command else {
            panic!("expected instructors")
        };
        assert_eq!(
            (args.sort.as_str(), args.window.as_str(), args.limit, args.min_compared),
            ("demand", "R1W1", 30, 10)
        );

        let Some(Command::Hot(args)) = parse(&["hot", "--limit", "5"]).command else {
            panic!("expected hot")
        };
        assert_eq!((args.term, args.window.as_str(), args.limit, args.min_filled), (None, "R1W1", 5, 2));
        let Some(Command::Hot(args)) = parse(&["hot", "--min-filled", "1"]).command else {
            panic!("expected hot")
        };
        assert_eq!(args.min_filled, 1);

        let Some(Command::Backtest(args)) = parse(&["backtest", "--tune"]).command else {
            panic!("expected backtest")
        };
        assert!(args.tune && args.window.is_none());

        for bad in [
            &["instructors", "--limit", "0"][..],
            &["hot", "--limit", "0"][..],
            &["hot", "--min-filled", "0"][..],
            &["backtest", "--window", "R1W1", "--all-windows"][..],
            &["backtest", "--tune", "--window", "R1W1"][..],
        ] {
            let args = std::iter::once("smu_bid_analyser").chain(bad.iter().copied());
            assert!(Cli::try_parse_from(args).is_err(), "{bad:?} should be rejected");
        }
    }
}
