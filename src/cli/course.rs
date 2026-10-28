//! `course`: the full bid history of one course.
//!
//! The numbers come from `query::course_report`. This file chooses what to show: every term (or the
//! most recent N), the sections of the latest term (or all of them), and the words around them.

use super::output::{ascii, opt_money, pct, Table};
use super::{count_at_least_one, parse_window, CliError, Ctx};
use crate::data::filter::WindowSel;
use crate::data::query::{self, CourseReport, SectionOut, TermSummaryOut};
use clap::Args;
use serde::Serialize;
use std::collections::HashSet;

/// Past this many instructors, the header lists the most recent ones and counts the rest.
const MAX_INSTRUCTORS: usize = 5;

#[derive(Debug, Args)]
pub struct CourseArgs {
    /// Course code, e.g. IS216. Case and spaces do not matter.
    pub code: String,
    /// Round and window, e.g. R1W1, R1AW2 or 1:1. "R1" means every window of that round, and "all" means every round and window together.
    #[arg(long, default_value = "R1W1", value_name = "W")]
    pub window: String,
    /// List every section of the terms shown, not only the latest term's. In JSON, `sections` is null unless this is given.
    #[arg(long)]
    pub sections: bool,
    /// Show only the N most recent terms, in both the terms table and the sections.
    #[arg(long, value_name = "N", value_parser = count_at_least_one)]
    pub terms: Option<usize>,
}

/// What `course` shows: the course report after `--terms` is applied, and the options that change the output.
#[derive(Debug)]
struct CourseView {
    report: CourseReport,
    /// Sessions left out with `--exclude-session`, in name order.
    excluded_sessions: Vec<String>,
    /// `--sections` was given.
    all_sections: bool,
    /// The `--terms` N as given, if it was.
    terms_limit: Option<usize>,
    /// Things to know before trusting the numbers. Text prints them on stderr; JSON lists them.
    warnings: Vec<String>,
}

pub fn run(ctx: &Ctx, args: &CourseArgs) -> Result<(), CliError> {
    let view = build_view(ctx, args)?;
    ctx.warn_text(&view.warnings);
    let out = CourseOut::new(&view);
    ctx.emit(&out, || println!("{}", render_text(&view)))
}

/// Looks the course up, reads the window and applies `--terms`. Fails the same way the other commands do.
fn build_view(ctx: &Ctx, args: &CourseArgs) -> Result<CourseView, CliError> {
    let info = ctx.course(&args.code)?;
    let sel = parse_window(&args.window)?;
    let mut report = query::course_report(&ctx.data, &ctx.filters, &info.code, sel)
        .ok_or_else(|| CliError::not_found(format!("no course with code \"{}\"", info.code)))?;
    // Data problems first, then what the window means for the numbers. Checked before --terms trims the report.
    let excluded_sessions: Vec<String> = ctx.filters.excluded_sessions.iter().cloned().collect();
    let mut warnings = ctx.data_warnings();
    warnings.extend(no_rows_warning(&report, &excluded_sessions));
    warnings.extend(pooled_warning(&report));
    if let Some(n) = args.terms {
        keep_recent_terms(&mut report.terms, &mut report.sections, n);
    }
    Ok(CourseView {
        report,
        excluded_sessions,
        all_sections: args.sections,
        terms_limit: args.terms,
        warnings,
    })
}

/// Keeps the `n` most recent terms. `terms` is oldest first, and a section is kept only when its term is.
fn keep_recent_terms(terms: &mut Vec<TermSummaryOut>, sections: &mut Vec<SectionOut>, n: usize) {
    let drop = terms.len().saturating_sub(n);
    terms.drain(..drop);
    let kept: HashSet<&str> = terms.iter().map(|t| t.term.as_str()).collect();
    sections.retain(|s| kept.contains(s.term.as_str()));
}

/// The sections the text view lists: all of them with `--sections`, otherwise the latest term's only.
fn sections_to_show(view: &CourseView) -> Vec<&SectionOut> {
    let latest = view.report.terms.last().map(|t| t.term.as_str());
    view.report
        .sections
        .iter()
        .filter(|s| view.all_sections || Some(s.term.as_str()) == latest)
        .collect()
}

/// The JSON for `course`. Every key is always present.
#[derive(Serialize)]
struct CourseOut<'a> {
    code: &'a str,
    description: &'a str,
    school: &'a str,
    last_term: &'a str,
    /// The window in words, e.g. "Round 1 Window 1", or "All rounds".
    window: &'a str,
    /// The window in short form, e.g. "R1W1", or "all". It can be passed back to --window.
    window_code: &'a str,
    /// Every window the course has been offered in, short form, in bidding order. Not affected by --exclude-session.
    windows_offered: &'a [String],
    /// Everyone who has taught the course, most recent first.
    instructors: &'a [String],
    /// Sessions left out with --exclude-session.
    excluded_sessions: &'a [String],
    /// The N given to --terms, or null when every term is shown.
    terms_limit: Option<usize>,
    /// Oldest first. Only the N most recent with --terms. Empty when the window has no rows. `sections`
    /// counts each section once; `section_windows`, `available` (open) and `filled` count section-windows.
    terms: &'a [TermSummaryOut],
    /// Null unless --sections is given. Then one row per section-window of the terms shown, newest term first.
    sections: Option<&'a [SectionOut]>,
    /// Things to know before trusting the numbers, such as a window with no rows, a window that pools
    /// prices across windows, or a data file that could not be read. Empty when there is nothing to say.
    warnings: &'a [String],
}

impl<'a> CourseOut<'a> {
    fn new(view: &'a CourseView) -> Self {
        let r = &view.report;
        CourseOut {
            code: &r.code,
            description: &r.description,
            school: &r.school,
            last_term: &r.last_term,
            window: &r.window,
            window_code: &r.window_code,
            windows_offered: &r.windows_offered,
            instructors: &r.instructors,
            excluded_sessions: &view.excluded_sessions,
            terms_limit: view.terms_limit,
            terms: &r.terms,
            sections: view.all_sections.then_some(&r.sections[..]),
            warnings: &view.warnings,
        }
    }
}

/// The whole text view. Tables are left out when the window has no rows, so only the header prints.
fn render_text(view: &CourseView) -> String {
    let r = &view.report;
    let mut blocks = vec![header_lines(view).join("\n")];
    if !r.terms.is_empty() {
        blocks.push(terms_table(&r.terms));
        if !one_window(r) {
            blocks.push(UNITS_NOTE.to_string());
        }
        let shown = sections_to_show(view);
        blocks.push(format!(
            "{}\n{}",
            sections_heading(view),
            sections_table(&shown, spans_sessions(&shown))
        ));
        blocks.push(format!(
            "Next: smu_bid_analyser plan {} --window {}",
            r.code, r.window_code
        ));
    }
    ascii(&blocks.join("\n\n"))
}

/// Under the terms table when several windows are shown: the two units it mixes.
const UNITS_NOTE: &str = "\
SECTIONS counts each section once. OPEN WINDOWS and FILLED WINDOWS count section-windows: one section
in one bidding window, so a section open in two windows counts twice. FILL% and the seat columns use
the same section-windows.";

/// True when the report covers one round and window (e.g. "R1W1"). Then a section is a section-window.
fn one_window(report: &CourseReport) -> bool {
    WindowSel::parse(&report.window_code).is_some_and(|sel| sel.is_specific())
}

/// True when the sections come from more than one session, so a session column tells rows apart.
fn spans_sessions(sections: &[&SectionOut]) -> bool {
    sections.iter().map(|s| s.session.as_str()).collect::<HashSet<_>>().len() > 1
}

/// The header: the course, then its school, last term, window, windows offered and instructors.
fn header_lines(view: &CourseView) -> Vec<String> {
    let r = &view.report;
    let title = if r.description.trim().is_empty() {
        r.code.clone()
    } else {
        format!("{} - {}", r.code, r.description)
    };
    let mut lines = vec![
        title,
        field("School:", &r.school),
        field("Last offered:", &r.last_term),
        field("Window:", &window_text(r)),
        field("Windows offered:", &list_or_none(&r.windows_offered)),
        field("Instructors:", &instructors_text(&r.instructors)),
    ];
    if !view.excluded_sessions.is_empty() {
        lines.push(field("Excluded:", &view.excluded_sessions.join(", ")));
    }
    lines.iter().map(|line| ascii(line)).collect()
}

/// One header line: a label in a fixed column, then the value.
fn field(label: &str, value: &str) -> String {
    format!("{label:<17}{value}")
}

/// The window in words, with its short form when it names one window: "Round 1 Window 1 (R1W1)".
/// Otherwise the words alone: "Round 2 (all windows)" or "All rounds".
fn window_text(report: &CourseReport) -> String {
    if one_window(report) {
        format!("{} ({})", report.window, report.window_code)
    } else {
        report.window.clone()
    }
}

/// The instructors, most recent first. A long list keeps its first `MAX_INSTRUCTORS` names.
fn instructors_text(names: &[String]) -> String {
    if names.is_empty() {
        return "none".to_string();
    }
    let listed = names
        .iter()
        .take(MAX_INSTRUCTORS)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    match names.len().saturating_sub(MAX_INSTRUCTORS) {
        0 => format!("{listed} (most recent first)"),
        rest => format!("{listed} ... and {rest} more (most recent first)"),
    }
}

fn list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(", ")
    }
}

/// One row per term, oldest first, so the trend reads down the table.
fn terms_table(terms: &[TermSummaryOut]) -> String {
    let mut table = Table::new(&[
        "TERM",
        "SECTIONS",
        "OPEN WINDOWS",
        "FILLED WINDOWS",
        "FILL%",
        "SEATS OPEN",
        "SEATS LEFT",
        "CLEARING MEDIAN",
        "LOW",
        "HIGH",
        "MEDIAN BID",
    ])
    .right_align(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    for t in terms {
        table.push(vec![
            t.term.clone(),
            t.sections.to_string(),
            t.available.to_string(),
            t.filled.to_string(),
            fill_cell(t.available, t.fill_rate),
            t.seats_open.to_string(),
            t.seats_left.to_string(),
            opt_money(t.clearing_median),
            opt_money(t.clearing_low),
            opt_money(t.clearing_high),
            opt_money(t.median_bid),
        ]);
    }
    table.render()
}

fn sections_heading(view: &CourseView) -> String {
    if view.all_sections {
        "Sections, newest term first".to_string()
    } else {
        let latest = view.report.terms.last().map_or("", |t| t.term.as_str());
        format!("Sections, {latest} (latest term only; --sections lists every term)")
    }
}

/// One row per section-window, in the order `course_report` gives: newest term first. `with_session`
/// adds a SESSION column after TERM, so sections that share a name in two sessions can be told apart.
fn sections_table(sections: &[&SectionOut], with_session: bool) -> String {
    let mut headers = vec!["TERM"];
    if with_session {
        headers.push("SESSION");
    }
    headers.extend([
        "WINDOW",
        "SECTION",
        "INSTRUCTOR",
        "VAC",
        "BEFORE",
        "AFTER",
        "ENROLLED",
        "OUTCOME",
        "MIN BID",
        "MEDIAN BID",
    ]);
    // Positions of the columns after TERM, moved one to the right when the session column is there.
    let at = |column: usize| column + usize::from(with_session);
    let mut table = Table::new(&headers)
        .right_align(&[at(4), at(5), at(6), at(7), at(9), at(10)])
        .max_width(at(3), 24);
    if with_session {
        table = table.max_width(1, 24);
    }
    for s in sections {
        let mut cells = vec![s.term.clone()];
        if with_session {
            cells.push(s.session.clone());
        }
        cells.extend([
            s.window_code.clone(),
            s.section.clone(),
            or_dash(&s.instructor),
            s.vacancy.to_string(),
            s.before_proc.to_string(),
            s.after_proc.to_string(),
            s.enrolled.to_string(),
            outcome_label(s.filled, s.available).to_string(),
            bid_cell(s.min_bid),
            bid_cell(s.median_bid),
        ]);
        table.push(cells);
    }
    table.render()
}

/// How a section ended in its window: every seat taken, seats still open, or no seats offered.
fn outcome_label(filled: bool, available: bool) -> &'static str {
    if filled {
        "filled"
    } else if available {
        "seats left"
    } else {
        "no seats"
    }
}

/// A bid for the table: money, or "-" when BOSS reported none (it is stored as zero).
fn bid_cell(x: f64) -> String {
    opt_money((x > 0.0).then_some(x))
}

/// Text for a table cell, or "-" when it is blank.
fn or_dash(text: &str) -> String {
    if text.trim().is_empty() {
        "-".to_string()
    } else {
        text.to_string()
    }
}

/// The share of open section-windows that filled, or "-" when none were open.
fn fill_cell(available: usize, fill_rate: f64) -> String {
    if available == 0 {
        "-".to_string()
    } else {
        pct(fill_rate)
    }
}

/// A warning when the window has no rows for the course. That is not an error: the report is still
/// printed, and the terms are empty.
fn no_rows_warning(report: &CourseReport, excluded_sessions: &[String]) -> Option<String> {
    if !report.terms.is_empty() {
        return None;
    }
    let window = if report.window_code == "all" {
        "any window"
    } else {
        report.window_code.as_str()
    };
    let excluded = if excluded_sessions.is_empty() {
        ""
    } else {
        " after --exclude-session"
    };
    Some(format!(
        "{} has no rows for {window}{excluded}; windows offered: {}",
        report.code,
        list_or_none(&report.windows_offered)
    ))
}

/// A warning when the window covers more than one round or window: the clearing prices of the different
/// windows are pooled into one median, and they differ. Nothing is said for one round and window, or when
/// there are no rows to pool.
fn pooled_warning(report: &CourseReport) -> Option<String> {
    if report.terms.is_empty() || one_window(report) {
        return None;
    }
    Some(format!(
        "Clearing prices are pooled across {}, and prices differ between windows. Use a specific window such as R1W1 for meaningful prices.",
        report.window.to_lowercase()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Command, ErrorKind};
    use crate::data::filter::GlobalFilters;
    use crate::data::model::ModelParams;
    use crate::data::{AppData, Descriptions, LoadReport, Term, TermKind};
    use crate::ui::test_data::sample_data;
    use clap::Parser;
    use std::collections::HashMap;
    use std::sync::Arc;

    /// TEST101 over seven terms (2022-23 T1 to 2025-26 T1). Window 1 has four sections a term (G4 is a
    /// Freshman Session section) and window 2 has one.
    fn ctx() -> Ctx {
        Ctx {
            data: sample_data(),
            filters: GlobalFilters::default(),
            params: ModelParams::default(),
            json: false,
        }
    }

    fn args(window: &str) -> CourseArgs {
        CourseArgs {
            code: "test101".to_string(),
            window: window.to_string(),
            sections: false,
            terms: None,
        }
    }

    fn term_out(label: &str) -> TermSummaryOut {
        TermSummaryOut {
            term: label.to_string(),
            sections: 2,
            section_windows: 2,
            available: 2,
            filled: 1,
            fill_rate: 0.5,
            seats_open: 20,
            seats_left: 4,
            clearing_median: Some(21.5),
            clearing_low: Some(15.0),
            clearing_high: Some(30.0),
            median_bid: Some(18.0),
        }
    }

    fn section_out(term: &str, section: &str) -> SectionOut {
        SectionOut {
            term: term.to_string(),
            session: "Regular Academic Session".to_string(),
            window_code: "R1W1".to_string(),
            section: section.to_string(),
            instructor: "Alice Tan".to_string(),
            vacancy: 40,
            before_proc: 10,
            after_proc: 0,
            enrolled: 10,
            filled: true,
            available: true,
            min_bid: 20.0,
            median_bid: 24.0,
        }
    }

    #[test]
    fn terms_flag_keeps_the_most_recent_terms_and_their_sections() {
        let ctx = ctx();
        let view = build_view(
            &ctx,
            &CourseArgs {
                terms: Some(2),
                ..args("R1W1")
            },
        )
        .expect("found");
        let labels: Vec<&str> = view.report.terms.iter().map(|t| t.term.as_str()).collect();
        assert_eq!(labels, ["2024-25 T2", "2025-26 T1"]);
        assert_eq!(view.terms_limit, Some(2));
        assert_eq!(
            view.report.sections.len(),
            8,
            "four sections in window 1 of each kept term"
        );
        assert!(view
            .report
            .sections
            .iter()
            .all(|s| labels.contains(&s.term.as_str())));
    }

    #[test]
    fn keep_recent_terms_handles_short_long_and_empty_limits() {
        let mut terms = vec![
            term_out("2023-24 T1"),
            term_out("2023-24 T2"),
            term_out("2024-25 T1"),
        ];
        // Newest first, as course_report gives them.
        let mut sections = vec![
            section_out("2024-25 T1", "G1"),
            section_out("2023-24 T2", "G2"),
            section_out("2023-24 T1", "G1"),
        ];
        keep_recent_terms(&mut terms, &mut sections, 2);
        let labels: Vec<&str> = terms.iter().map(|t| t.term.as_str()).collect();
        assert_eq!(labels, ["2023-24 T2", "2024-25 T1"]);
        let kept: Vec<String> = sections
            .iter()
            .map(|s| format!("{} {}", s.term, s.section))
            .collect();
        assert_eq!(kept, ["2024-25 T1 G1", "2023-24 T2 G2"]);

        keep_recent_terms(&mut terms, &mut sections, 10);
        assert_eq!(terms.len(), 2, "a limit above the count keeps everything");

        keep_recent_terms(&mut terms, &mut sections, 0);
        assert!(terms.is_empty() && sections.is_empty());
    }

    #[test]
    fn text_lists_the_latest_term_unless_every_section_is_asked_for() {
        let ctx = ctx();
        let latest = build_view(&ctx, &args("R1W1")).expect("found");
        let shown = sections_to_show(&latest);
        assert_eq!(shown.len(), 4);
        assert!(shown.iter().all(|s| s.term == "2025-26 T1"));

        let every = build_view(
            &ctx,
            &CourseArgs {
                sections: true,
                ..args("R1W1")
            },
        )
        .expect("found");
        assert_eq!(sections_to_show(&every).len(), 28);
    }

    #[test]
    fn json_has_a_stable_shape_and_sections_only_when_asked() {
        let ctx = ctx();
        let plain = build_view(&ctx, &args("R1W1")).expect("found");
        let value = serde_json::to_value(CourseOut::new(&plain)).expect("serialises");
        assert!(value["sections"].is_null(), "{value}");
        assert_eq!(value["code"], "TEST101");
        assert_eq!(value["window_code"], "R1W1");
        assert_eq!(
            value["windows_offered"],
            serde_json::json!(["R1W1", "R1W2"])
        );
        assert_eq!(value["excluded_sessions"], serde_json::json!([]));
        assert!(value["terms_limit"].is_null(), "{value}");
        assert_eq!(value["warnings"], serde_json::json!([]), "{value}");
        assert_eq!(value["terms"].as_array().map(Vec::len), Some(7));
        assert_eq!(value["terms"][0]["term"], "2022-23 T1");

        let every = build_view(
            &ctx,
            &CourseArgs {
                sections: true,
                ..args("R1W1")
            },
        )
        .expect("found");
        let value = serde_json::to_value(CourseOut::new(&every)).expect("serialises");
        assert_eq!(value["sections"].as_array().map(Vec::len), Some(28));
    }

    #[test]
    fn a_window_without_rows_is_not_an_error_and_says_so_in_a_warning() {
        let ctx = ctx();
        let view = build_view(&ctx, &args("R2")).expect("not an error");
        assert!(view.report.terms.is_empty() && view.report.sections.is_empty());
        assert_eq!(
            view.warnings,
            vec!["TEST101 has no rows for R2; windows offered: R1W1, R1W2".to_string()]
        );
        let text = render_text(&view);
        assert!(!text.contains("TERM") && !text.contains("Next:"), "{text}");
        let value = serde_json::to_value(CourseOut::new(&view)).expect("serialises");
        assert_eq!(value["terms"], serde_json::json!([]));
        assert_eq!(
            value["warnings"],
            serde_json::json!(["TEST101 has no rows for R2; windows offered: R1W1, R1W2"])
        );

        let missing = build_view(&ctx, &args("R1W3")).expect("not an error");
        assert!(missing.warnings.iter().any(|w| w.contains("has no rows for R1W3")), "{:?}", missing.warnings);
        let all = build_view(&ctx, &args("all")).expect("found");
        assert!(all.warnings.iter().all(|w| !w.contains("has no rows")), "{:?}", all.warnings);
    }

    #[test]
    fn excluded_sessions_are_left_out_and_listed() {
        let mut ctx = ctx();
        ctx.filters
            .excluded_sessions
            .insert("Freshman Session".to_string());
        let view = build_view(&ctx, &args("R1W1")).expect("found");
        assert_eq!(
            view.report.sections.len(),
            21,
            "three sections a term remain"
        );
        assert_eq!(view.excluded_sessions, vec!["Freshman Session".to_string()]);
        let lines = header_lines(&view);
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("Excluded:") && l.ends_with("Freshman Session")),
            "{lines:?}"
        );
        assert!(view.warnings.is_empty(), "{:?}", view.warnings);
    }

    #[test]
    fn a_window_spanning_several_windows_warns_that_prices_are_pooled() {
        let ctx = ctx();
        let all = build_view(&ctx, &args("all")).expect("found");
        assert_eq!(all.warnings.len(), 1, "{:?}", all.warnings);
        assert!(
            all.warnings[0].starts_with("Clearing prices are pooled across all rounds, and prices differ between windows."),
            "{:?}",
            all.warnings
        );
        let round = build_view(&ctx, &args("R1")).expect("found");
        assert!(
            round.warnings[0].starts_with("Clearing prices are pooled across round 1 (all windows),"),
            "{:?}",
            round.warnings
        );
        let one = build_view(&ctx, &args("R1W1")).expect("found");
        assert!(one.warnings.is_empty(), "{:?}", one.warnings);
    }

    #[test]
    fn outcome_says_how_the_section_ended() {
        assert_eq!(outcome_label(true, true), "filled");
        assert_eq!(outcome_label(false, true), "seats left");
        assert_eq!(outcome_label(false, false), "no seats");
    }

    #[test]
    fn zero_bids_blank_names_and_unknown_fill_rates_print_a_dash() {
        assert_eq!(bid_cell(0.0), "-");
        assert_eq!(bid_cell(23.5), "e$23.50");
        assert_eq!(or_dash("  "), "-");
        assert_eq!(or_dash("Alice Tan"), "Alice Tan");
        assert_eq!(fill_cell(0, 0.0), "-");
        assert_eq!(fill_cell(4, 0.786), "78.6%");
    }

    #[test]
    fn long_instructor_lists_are_cut_after_the_most_recent() {
        assert_eq!(instructors_text(&[]), "none");
        let three: Vec<String> = ["Alice Tan", "Bob Lee", "Carol Ng"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            instructors_text(&three),
            "Alice Tan, Bob Lee, Carol Ng (most recent first)"
        );
        let seven: Vec<String> = (1..=7).map(|i| format!("Teacher {i}")).collect();
        assert_eq!(
            instructors_text(&seven),
            "Teacher 1, Teacher 2, Teacher 3, Teacher 4, Teacher 5 ... and 2 more (most recent first)"
        );
    }

    #[test]
    fn window_is_named_in_words_and_code() {
        let ctx = ctx();
        let one = build_view(&ctx, &args("1:1")).expect("found");
        assert_eq!(window_text(&one.report), "Round 1 Window 1 (R1W1)");
        let every = build_view(&ctx, &args("all")).expect("found");
        assert_eq!(window_text(&every.report), "All rounds");
        let round = build_view(&ctx, &args("R2")).expect("found");
        assert_eq!(window_text(&round.report), "Round 2 (all windows)");
    }

    #[test]
    fn header_gives_the_title_in_ascii() {
        let descriptions: Descriptions = HashMap::from([(
            Arc::from("TEST101"),
            (
                Term {
                    year: 2025,
                    kind: TermKind::T1,
                },
                "Caf\u{e9} \u{201C}Studies\u{201D}".to_string(),
            ),
        )]);
        let ctx = Ctx {
            data: AppData::new(sample_data().rows, descriptions, LoadReport::default()),
            filters: GlobalFilters::default(),
            params: ModelParams::default(),
            json: false,
        };
        let view = build_view(&ctx, &args("R1W1")).expect("found");
        let lines = header_lines(&view);
        assert_eq!(lines[0], "TEST101 - Caf? \"Studies\"");
        assert!(
            lines.iter().any(|l| l.starts_with("Instructors:")
                && l.ends_with("ALPHA, BETA, GAMMA (most recent first)")),
            "{lines:?}"
        );
    }

    #[test]
    fn text_is_ascii_and_ends_with_the_next_step() {
        let ctx = ctx();
        let text = render_text(&build_view(&ctx, &args("R1W1")).expect("found"));
        assert!(text.is_ascii(), "{text}");
        assert_eq!(
            text.lines().last(),
            Some("Next: smu_bid_analyser plan TEST101 --window R1W1")
        );
        assert!(text.contains("TERM") && text.contains("OUTCOME"), "{text}");
        assert!(text.contains("seats left"), "{text}");
        assert!(text.contains("e$"), "{text}");
    }

    #[test]
    fn terms_flag_must_be_at_least_one() {
        let parsed = Cli::try_parse_from(["smu_bid_analyser", "course", "is216", "--terms", "3"])
            .expect("valid");
        let Some(Command::Course(course)) = parsed.command else {
            panic!("expected course")
        };
        assert_eq!(
            (course.code.as_str(), course.window.as_str(), course.terms),
            ("is216", "R1W1", Some(3))
        );
        assert!(!course.sections);
        assert!(
            Cli::try_parse_from(["smu_bid_analyser", "course", "IS216", "--terms", "0"]).is_err()
        );
    }

    #[test]
    fn unknown_courses_and_bad_windows_fail_with_the_right_kind() {
        let ctx = ctx();
        let unknown = CourseArgs {
            code: "nosuch".to_string(),
            ..args("R1W1")
        };
        assert_eq!(
            build_view(&ctx, &unknown).expect_err("unknown").kind,
            ErrorKind::NotFound
        );
        assert_eq!(
            build_view(&ctx, &args("bogus"))
                .expect_err("bad window")
                .kind,
            ErrorKind::Usage
        );
        let blank = CourseArgs {
            code: "  ".to_string(),
            ..args("R1W1")
        };
        assert_eq!(
            build_view(&ctx, &blank).expect_err("blank code").kind,
            ErrorKind::Usage
        );
    }

    /// The header line of the sections table: it starts with TERM and names INSTRUCTOR.
    fn sections_header(text: &str) -> &str {
        text.lines().find(|l| l.starts_with("TERM") && l.contains("INSTRUCTOR")).expect("sections header")
    }

    #[test]
    fn session_column_appears_only_when_the_rows_span_sessions() {
        let ctx = ctx();
        // The latest term's window 1 has G4 in the Freshman Session as well as the Regular one.
        let mixed = render_text(&build_view(&ctx, &args("R1W1")).expect("found"));
        assert!(sections_header(&mixed).contains("SESSION"), "{mixed}");
        assert!(mixed.contains("Freshman Session"), "{mixed}");

        let mut regular_only = ctx;
        regular_only.filters.excluded_sessions.insert("Freshman Session".to_string());
        let text = render_text(&build_view(&regular_only, &args("R1W1")).expect("found"));
        assert!(!sections_header(&text).contains("SESSION"), "{text}");
    }

    #[test]
    fn units_are_explained_only_when_windows_are_mixed() {
        let ctx = ctx();
        let one = render_text(&build_view(&ctx, &args("R1W1")).expect("found"));
        assert!(!one.contains("counts each section once"), "{one}");
        assert!(one.contains("OPEN WINDOWS") && one.contains("FILLED WINDOWS"), "{one}");

        let every = render_text(&build_view(&ctx, &args("all")).expect("found"));
        let note = every.find("SECTIONS counts each section once").expect("the units note");
        assert!(note > every.find("OPEN WINDOWS").expect("header"), "{every}");
        assert!(note < every.find("Sections,").expect("sections heading"), "{every}");
    }

    #[test]
    fn json_counts_sections_and_section_windows_and_names_each_session() {
        let ctx = ctx();
        let every = CourseArgs {
            sections: true,
            ..args("all")
        };
        let view = build_view(&ctx, &every).expect("found");
        let value = serde_json::to_value(CourseOut::new(&view)).expect("serialises");

        // 2025-26 T1 in every window: four sections, five section-windows, all open, four filled.
        let term = &value["terms"][6];
        assert_eq!(term["term"], "2025-26 T1");
        assert_eq!(
            (term["sections"].as_u64(), term["section_windows"].as_u64()),
            (Some(4), Some(5)),
            "{term}"
        );
        assert_eq!((term["available"].as_u64(), term["filled"].as_u64()), (Some(5), Some(4)), "{term}");

        // Newest term first; within it round 1 window 1 in section order, G4 last of those.
        let sections = value["sections"].as_array().expect("sections");
        assert!(sections.iter().all(|s| s["session"].is_string()), "{value}");
        assert_eq!(sections[3]["section"], "G4");
        assert_eq!(sections[3]["session"], "Freshman Session");
        assert_eq!(sections[0]["session"], "Regular Academic Session");
    }
}
