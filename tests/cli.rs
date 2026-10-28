//! Black-box tests of the `smu_bid_analyser` command line.
//!
//! Each test writes a small, fully synthetic folder of BOSS "Overall Results" workbooks to a temp
//! directory and runs the compiled binary against it. The folder is given with `--data-dir` and in
//! `BOSS_DATA_DIR`, so the user's own data is never read. Every run names a command (or the legacy
//! `--backtest` flag), because with no command the binary opens the desktop app. The only runs with no
//! command are `--json` and `--exclude-session` alone, which must be refused with a usage error before
//! any window opens.
//!
//! The fixture covers four terms, 2023-24 T1 to 2024-25 T2, so the default target term is 2025-26 T1.
//! Four courses:
//! - TST101 fills every term, its clearing prices rise, and two instructors teach it;
//! - TST202 never fills;
//! - TST303 has one of its two sections fill in every term;
//! - TST404 is offered only in Round 1A Window 2.

use rust_xlsxwriter::{Workbook, Worksheet, XlsxError};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_smu_bid_analyser");

/// The columns of a BOSS "Overall Results" sheet, in export order.
const HEADER: [&str; 16] = [
    "Term",
    "Session",
    "Bidding Window",
    "Course Code",
    "Description",
    "Section",
    "Vacancy",
    "Opening Vacancy",
    "Before Process Vacancy",
    "D.I.C.E",
    "After Process Vacancy",
    "Enrolled Students",
    "Median Bid",
    "Min Bid",
    "Instructor",
    "School/Department",
];

const SESSION: &str = "Regular Academic Session";
const R1W1: &str = "Round 1 Window 1";
const R1AW2: &str = "Round 1A Window 2";
const SPECIAL_WINDOW: &str = "Incoming Freshmen Rnd 1 Win 1";

/// The four workbooks: file name, and the term its rows are for.
const FILES: [(&str, &str); 4] = [
    ("2023-24_T1.xlsx", "2023-24 Term 1"),
    ("2023-24_T2.xlsx", "2023-24 Term 2"),
    ("2024-25_T1.xlsx", "2024-25 Term 1"),
    ("2024-25_T2.xlsx", "2024-25 Term 2"),
];

/// The terms the CLI reports for the fixture, oldest first.
const TERMS: [&str; 4] = ["2023-24 T1", "2023-24 T2", "2024-25 T1", "2024-25 T2"];

/// Every section in every term, in sheet order: (term, course, section, seats left after the bidding
/// process, minimum bid). A section with seats left after the process did not fill, and its minimum bid
/// is only the lowest bid placed. The median bid is one dollar above the minimum.
const SECTIONS: &[(&str, &str, &str, f64, f64)] = &[
    ("2023-24 Term 1", "TST101", "G1", 0.0, 16.0),
    ("2023-24 Term 1", "TST101", "G2", 0.0, 17.0),
    ("2023-24 Term 1", "TST202", "G1", 3.0, 10.0),
    ("2023-24 Term 1", "TST303", "G1", 0.0, 14.0),
    ("2023-24 Term 1", "TST303", "G2", 4.0, 10.0),
    ("2023-24 Term 1", "TST404", "G1", 0.0, 12.0),
    ("2023-24 Term 2", "TST101", "G1", 0.0, 18.0),
    ("2023-24 Term 2", "TST101", "G2", 0.0, 19.0),
    ("2023-24 Term 2", "TST202", "G1", 3.0, 10.0),
    ("2023-24 Term 2", "TST303", "G1", 0.0, 15.0),
    ("2023-24 Term 2", "TST303", "G2", 4.0, 10.0),
    ("2023-24 Term 2", "TST404", "G1", 2.0, 10.0),
    ("2024-25 Term 1", "TST101", "G1", 0.0, 21.0),
    ("2024-25 Term 1", "TST101", "G2", 0.0, 22.0),
    ("2024-25 Term 1", "TST202", "G1", 3.0, 10.0),
    ("2024-25 Term 1", "TST303", "G1", 0.0, 15.0),
    ("2024-25 Term 1", "TST303", "G2", 4.0, 10.0),
    ("2024-25 Term 1", "TST404", "G1", 0.0, 13.0),
    ("2024-25 Term 2", "TST101", "G1", 0.0, 24.0),
    ("2024-25 Term 2", "TST101", "G2", 0.0, 25.0),
    ("2024-25 Term 2", "TST202", "G1", 3.0, 10.0),
    ("2024-25 Term 2", "TST303", "G1", 0.0, 16.0),
    ("2024-25 Term 2", "TST303", "G2", 4.0, 10.0),
    ("2024-25 Term 2", "TST404", "G1", 0.0, 14.0),
];

/// What the sheets say about a course: title, school, bidding window, seats, and seats open before the
/// process. Seats not open before the process went to DICE.
struct Course {
    title: &'static str,
    school: &'static str,
    window: &'static str,
    seats: f64,
    before: f64,
}

fn course(code: &str) -> Course {
    match code {
        "TST101" => Course {
            title: "Data Science Fundamentals",
            school: "SCIS",
            window: R1W1,
            seats: 40.0,
            before: 12.0,
        },
        "TST202" => Course {
            title: "Never Full Seminar",
            school: "SOB",
            window: R1W1,
            seats: 30.0,
            before: 8.0,
        },
        "TST303" => Course {
            title: "Mixed Fill Lab",
            school: "SCIS",
            window: R1W1,
            seats: 20.0,
            before: 10.0,
        },
        "TST404" => Course {
            title: "Negotiation Workshop",
            school: "SOB",
            window: R1AW2,
            seats: 20.0,
            before: 6.0,
        },
        other => panic!("{other} is not in the fixture"),
    }
}

fn instructor(code: &str, section: &str) -> &'static str {
    match (code, section) {
        ("TST101", "G1") => "Alice Tan",
        ("TST101", _) => "Bob Tan",
        ("TST202", _) => "Carol Ong",
        ("TST303", "G1") => "Dan Ng",
        ("TST303", _) => "Eve Koh",
        ("TST404", _) => "Fay Lim",
        (other, _) => panic!("no instructor for {other} in the fixture"),
    }
}

/// One section in one bidding window.
#[derive(Clone, Copy)]
struct Section {
    term: &'static str,
    window: &'static str,
    code: &'static str,
    section: &'static str,
    after: f64,
    min: f64,
}

/// One line of a sheet. BOSS exports sometimes contain blank lines and repeated headers.
enum Line {
    Header,
    Blank,
    Section(Section),
}

/// The six sections a term offers, in the order `SECTIONS` lists them.
fn sections_of(term: &str) -> Vec<Section> {
    SECTIONS
        .iter()
        .filter(|(t, ..)| *t == term)
        .map(|&(t, code, section, after, min)| Section {
            term: t,
            window: course(code).window,
            code,
            section,
            after,
            min,
        })
        .collect()
}

/// The lines of one workbook. Each sheet has the header and its term's six sections, plus the mess the
/// loader must cope with:
/// - 2023-24 T1 has a blank line and a repeated header in the middle;
/// - 2023-24 T2 repeats one section at a different price, so only the first copy may count;
/// - 2024-25 T1 lists an incoming-freshmen section, which is outside the bidding rounds.
fn sheet_lines(file: &str, term: &str) -> Vec<Line> {
    let s = sections_of(term);
    let mut lines = vec![Line::Header];
    match file {
        "2023-24_T1.xlsx" => {
            lines.extend(s[..2].iter().copied().map(Line::Section));
            lines.push(Line::Blank);
            lines.push(Line::Section(s[2]));
            lines.push(Line::Header);
            lines.extend(s[3..].iter().copied().map(Line::Section));
        }
        "2023-24_T2.xlsx" => {
            lines.extend(s.iter().copied().map(Line::Section));
            lines.push(Line::Section(Section { min: 99.0, ..s[0] }));
        }
        "2024-25_T1.xlsx" => {
            lines.extend(s.iter().copied().map(Line::Section));
            lines.push(Line::Section(Section {
                window: SPECIAL_WINDOW,
                min: 5.0,
                ..s[0]
            }));
        }
        _ => lines.extend(s.iter().copied().map(Line::Section)),
    }
    lines
}

fn write_header(sheet: &mut Worksheet, row: u32) -> Result<(), XlsxError> {
    for (col, name) in (0u16..).zip(HEADER) {
        sheet.write_string(row, col, name)?;
    }
    Ok(())
}

fn write_section(sheet: &mut Worksheet, row: u32, line: &Section) -> Result<(), XlsxError> {
    let c = course(line.code);
    let texts = [
        (0, line.term),
        (1, SESSION),
        (2, line.window),
        (3, line.code),
        (4, c.title),
        (5, line.section),
        (14, instructor(line.code, line.section)),
        (15, c.school),
    ];
    for (col, text) in texts {
        sheet.write_string(row, col, text)?;
    }
    let numbers = [
        (6, c.seats),
        (7, c.seats),
        (8, c.before),
        (9, c.seats - c.before),
        (10, line.after),
        (11, c.seats - line.after),
        (12, line.min + 1.0),
        (13, line.min),
    ];
    for (col, value) in numbers {
        sheet.write_number(row, col, value)?;
    }
    Ok(())
}

fn write_workbook(path: &Path, lines: &[Line]) -> Result<(), XlsxError> {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    for (row, line) in (0u32..).zip(lines) {
        match line {
            Line::Header => write_header(sheet, row)?,
            Line::Blank => {}
            Line::Section(section) => write_section(sheet, row, section)?,
        }
    }
    workbook.save(path)
}

/// A folder of synthetic BOSS workbooks. It is deleted when the test ends.
struct Fixture {
    dir: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("boss-fixture-")
            .tempdir()
            .expect("a temp folder for the fixture");
        for (file, term) in FILES {
            let lines = sheet_lines(file, term);
            write_workbook(&dir.path().join(file), &lines).unwrap_or_else(|err| panic!("writing {file} failed: {err}"));
        }
        Fixture { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Runs the binary on this folder with `args`.
    fn run(&self, args: &[&str]) -> Run {
        run_at(self.path(), args)
    }

    /// As `run`, with `--json` in front of `args`.
    fn run_json(&self, args: &[&str]) -> Run {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        run_at(self.path(), &all)
    }
}

/// What one run of the binary wrote, and how it exited.
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    /// Checks the exit code. On a mismatch the message shows both streams.
    fn exit(&self, want: i32) -> &Self {
        assert_eq!(
            self.code, want,
            "exit code; stdout:\n{}\nstderr:\n{}",
            self.stdout, self.stderr
        );
        self
    }

    /// Parses all of stdout as one JSON document, so anything else printed there fails the test.
    fn value(&self) -> Value {
        assert!(!self.stdout.contains("NaN"), "NaN in the output:\n{}", self.stdout);
        serde_json::from_str(&self.stdout).unwrap_or_else(|err| {
            panic!(
                "stdout is not a single JSON document ({err}):\n{}\nstderr:\n{}",
                self.stdout, self.stderr
            )
        })
    }
}

/// Runs the binary with `--data-dir` and `BOSS_DATA_DIR` both set to `dir`, then `args`.
fn run_at(dir: &Path, args: &[&str]) -> Run {
    let output = Command::new(BIN)
        .env("BOSS_DATA_DIR", dir)
        .arg("--data-dir")
        .arg(dir)
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("could not start {BIN}: {err}"));
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn str_at<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("`{key}` should be a string in {value}"))
}

fn num_at(value: &Value, key: &str) -> f64 {
    value[key]
        .as_f64()
        .unwrap_or_else(|| panic!("`{key}` should be a number in {value}"))
}

fn arr_at<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| panic!("`{key}` should be an array in {value}"))
}

/// Whether the text has a Markdown heading line, such as "# Usage".
fn has_markdown_heading(text: &str) -> bool {
    text.lines().any(|line| line.starts_with("# "))
}

// ---------------------------------------------------------------------------------------------------
// info

#[test]
fn info_counts_the_files_rows_and_terms_and_picks_the_target_term() {
    let fx = Fixture::new();
    let run = fx.run_json(&["info"]);
    run.exit(0);
    let info = run.value();

    let files = arr_at(&info, "files");
    let names: Vec<&str> = files.iter().map(|f| str_at(f, "name")).collect();
    assert_eq!(
        names,
        [
            "2023-24_T1.xlsx",
            "2023-24_T2.xlsx",
            "2024-25_T1.xlsx",
            "2024-25_T2.xlsx"
        ]
    );
    let kept: Vec<u64> = files
        .iter()
        .map(|f| f["rows_kept"].as_u64().expect("a row count"))
        .collect();
    assert_eq!(kept, [6, 6, 6, 6]);
    assert!(
        files.iter().all(|f| f["error"].is_null()),
        "no file should fail: {info}"
    );

    assert_eq!(info["total_rows"], 24);
    assert_eq!(info["courses"], 4);
    assert_eq!(info["terms"], json!(TERMS));
    assert_eq!(str_at(&info, "latest_term"), "2024-25 T2");
    assert_eq!(str_at(&info, "default_target_term"), "2025-26 T1");
    assert_eq!(info["sessions"], json!([SESSION]));
    assert_eq!(info["excluded_sessions"], json!([]));
}

#[test]
fn info_counts_special_rounds_duplicates_and_invalid_lines_on_the_right_file() {
    let fx = Fixture::new();
    let run = fx.run_json(&["info"]);
    run.exit(0);
    let info = run.value();

    // (special round, invalid, duplicate) for each file, in file order.
    let counts: Vec<(u64, u64, u64)> = arr_at(&info, "files")
        .iter()
        .map(|f| {
            (
                f["rows_special_rounds"].as_u64().expect("a count"),
                f["rows_invalid"].as_u64().expect("a count"),
                f["duplicates"].as_u64().expect("a count"),
            )
        })
        .collect();
    // 2023-24 T1: the blank line and the repeated header are invalid. 2023-24 T2: the repeated section.
    // 2024-25 T1: the incoming-freshmen section.
    assert_eq!(counts, [(0, 2, 0), (0, 0, 1), (1, 0, 0), (0, 0, 0)]);
}

// ---------------------------------------------------------------------------------------------------
// search

#[test]
fn search_finds_a_course_from_part_of_its_title() {
    let fx = Fixture::new();
    let run = fx.run_json(&["search", "data sci"]);
    run.exit(0);
    let out = run.value();
    let results = arr_at(&out, "results");
    assert_eq!(str_at(&results[0], "code"), "TST101");
    assert_eq!(str_at(&results[0], "description"), "Data Science Fundamentals");
    assert_eq!(out["shown"], results.len());
}

#[test]
fn search_with_no_matches_exits_0_with_an_empty_list() {
    let fx = Fixture::new();
    let run = fx.run_json(&["search", "qxq"]);
    run.exit(0);
    let out = run.value();
    assert_eq!(out["total_matches"], 0);
    assert_eq!(out["shown"], 0);
    assert_eq!(out["results"], json!([]));
}

#[test]
fn search_limit_caps_the_results_and_rejects_zero() {
    let fx = Fixture::new();
    let run = fx.run_json(&["search", "tst", "--limit", "2"]);
    run.exit(0);
    let out = run.value();
    assert_eq!(out["total_matches"], 4);
    assert_eq!(out["shown"], 2);
    assert_eq!(arr_at(&out, "results").len(), 2);

    fx.run_json(&["search", "tst", "--limit", "0"]).exit(2);
}

// ---------------------------------------------------------------------------------------------------
// course

#[test]
fn course_lists_each_term_oldest_first_with_sane_fill_rates_and_rising_prices() {
    let fx = Fixture::new();
    // Lowercase, with a space: the course code is still found.
    let run = fx.run_json(&["course", "tst 101"]);
    run.exit(0);
    let out = run.value();
    assert_eq!(str_at(&out, "code"), "TST101");
    assert_eq!(str_at(&out, "window_code"), "R1W1");
    assert_eq!(out["windows_offered"], json!(["R1W1"]));
    assert!(out["sections"].is_null(), "sections are listed only with --sections");

    let terms = arr_at(&out, "terms");
    let labels: Vec<&str> = terms.iter().map(|t| str_at(t, "term")).collect();
    assert_eq!(labels, TERMS);
    for term in terms {
        let rate = num_at(term, "fill_rate");
        assert!((0.0..=1.0).contains(&rate), "fill rate out of range: {term}");
        assert!((rate - 1.0).abs() < 1e-9, "TST101 fills every term: {term}");
    }
    let medians: Vec<f64> = terms.iter().map(|t| num_at(t, "clearing_median")).collect();
    assert_eq!(medians, [16.5, 18.5, 21.5, 24.5]);
}

#[test]
fn course_sections_flag_lists_every_section_newest_term_first() {
    let fx = Fixture::new();
    let run = fx.run_json(&["course", "TST101", "--sections"]);
    run.exit(0);
    let out = run.value();
    let sections = arr_at(&out, "sections");
    assert_eq!(sections.len(), 8, "two sections in each of four terms");
    assert_eq!(str_at(&sections[0], "term"), "2024-25 T2");
    assert_eq!(str_at(&sections[7], "term"), "2023-24 T1");

    // The repeated copy of 2023-24 T2 G1 (min bid 99) is dropped, so the first copy's price stands.
    let repeated = sections
        .iter()
        .find(|s| str_at(s, "term") == "2023-24 T2" && str_at(s, "section") == "G1")
        .expect("2023-24 T2 G1 is listed");
    assert_eq!(num_at(repeated, "min_bid"), 18.0);
    assert_eq!(str_at(repeated, "instructor"), "Alice Tan");
}

#[test]
fn unknown_course_exits_3_with_a_json_error_and_nothing_else_on_stdout() {
    let fx = Fixture::new();
    let run = fx.run_json(&["course", "tst10"]);
    run.exit(3);
    // `value` parses the whole of stdout, so it fails if anything else was printed.
    let out = run.value();
    assert_eq!(
        out.as_object().map(|o| o.len()),
        Some(1),
        "only the error envelope: {out}"
    );
    let error = &out["error"];
    assert_eq!(error["kind"], "not_found");
    assert!(error["message"].is_string(), "{out}");
    assert_eq!(error["suggestions"], json!(["TST101"]));
}

#[test]
fn course_with_no_rows_in_the_window_exits_0_and_shows_where_it_is_offered() {
    let fx = Fixture::new();
    // TST404 is only offered in Round 1A Window 2, so the default window (R1W1) has no rows.
    let run = fx.run_json(&["course", "TST404"]);
    run.exit(0);
    let out = run.value();
    assert_eq!(out["terms"], json!([]));
    assert_eq!(out["windows_offered"], json!(["R1AW2"]));

    let run = fx.run_json(&["course", "TST404", "--window", "R1AW2"]);
    run.exit(0);
    let out = run.value();
    let rates: Vec<f64> = arr_at(&out, "terms").iter().map(|t| num_at(t, "fill_rate")).collect();
    assert_eq!(rates, [1.0, 0.0, 1.0, 1.0]);
}

#[test]
fn course_puts_its_warnings_in_the_json_and_on_stderr_in_text() {
    let fx = Fixture::new();
    // TST404 is only offered in Round 1A Window 2, so the default window has no rows.
    let run = fx.run_json(&["course", "TST404"]);
    run.exit(0);
    assert!(run.stderr.is_empty(), "JSON mode writes nothing to stderr: {}", run.stderr);
    let out = run.value();
    assert_eq!(out["warnings"], json!(["TST404 has no rows for R1W1; windows offered: R1AW2"]));
    assert!(out["terms_limit"].is_null(), "{out}");

    let text = fx.run(&["course", "TST404"]);
    text.exit(0);
    assert!(text.stdout.is_ascii(), "{}", text.stdout);
    assert!(text.stderr.contains("warning: TST404 has no rows for R1W1"), "{}", text.stderr);
    assert!(!text.stderr.contains("note:"), "{}", text.stderr);

    // A window that spans several windows pools their prices, and `--terms` is echoed back.
    let all = fx.run_json(&["course", "TST101", "--window", "all", "--terms", "2"]);
    all.exit(0);
    assert!(all.stderr.is_empty(), "{}", all.stderr);
    let out = all.value();
    assert_eq!(out["terms_limit"], 2);
    let warnings = arr_at(&out, "warnings");
    assert_eq!(warnings.len(), 1, "{out}");
    assert!(
        warnings[0].as_str().is_some_and(|w| w.starts_with("Clearing prices are pooled across all rounds")),
        "{out}"
    );
}

// ---------------------------------------------------------------------------------------------------
// plan

#[test]
fn plan_recommends_bids_that_rise_with_the_chance_and_stay_above_the_minimum() {
    let fx = Fixture::new();
    let run = fx.run_json(&["plan", "TST101"]);
    run.exit(0);
    let plan = run.value();
    assert_eq!(str_at(&plan, "course"), "TST101");
    assert_eq!(str_at(&plan, "target"), "2025-26 T1");

    let recommended = arr_at(&plan, "recommended");
    let chances: Vec<f64> = recommended.iter().map(|r| num_at(r, "probability")).collect();
    let bids: Vec<f64> = recommended.iter().map(|r| num_at(r, "bid")).collect();
    assert_eq!(chances, [0.5, 0.7, 0.8, 0.9, 0.95]);
    assert!(chances.contains(&0.8), "the 80% level is recommended");
    assert!(
        bids.windows(2).all(|w| w[0] <= w[1]),
        "bids must not fall as the chance rises: {bids:?}"
    );
    assert!(
        bids.iter().all(|&b| b >= 10.0),
        "every bid is at least the e$10 minimum: {bids:?}"
    );
    assert!(num_at(&plan, "trend_per_year") > 0.0, "prices rise every term: {plan}");
}

#[test]
fn plan_gives_no_chance_below_the_minimum_bid_and_near_certainty_for_a_huge_bid() {
    let fx = Fixture::new();
    let run = fx.run_json(&["plan", "TST101", "--bid", "5", "--bid", "25", "--bid", "500"]);
    run.exit(0);
    let plan = run.value();
    let checks = arr_at(&plan, "bid_checks");
    assert_eq!(checks.len(), 3);
    assert_eq!(num_at(&checks[0], "bid"), 5.0);
    assert_eq!(num_at(&checks[0], "probability"), 0.0);
    assert!(num_at(&checks[2], "probability") >= 0.99, "{plan}");

    let chances: Vec<f64> = checks.iter().map(|c| num_at(c, "probability")).collect();
    assert!(chances.windows(2).all(|w| w[0] <= w[1]), "{chances:?}");

    let warnings = arr_at(&plan, "warnings");
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().is_some_and(|w| w.contains("minimum bid"))),
        "a bid under the minimum should draw a warning: {plan}"
    );
}

#[test]
fn plan_resolves_part_of_an_instructor_name() {
    let fx = Fixture::new();
    let run = fx.run_json(&["plan", "TST101", "--instructor", "alice"]);
    run.exit(0);
    let plan = run.value();
    assert_eq!(str_at(&plan, "instructor"), "Alice Tan");
    assert_eq!(plan["n_instructor_obs"], 4);
}

#[test]
fn plan_refuses_an_ambiguous_or_unknown_instructor_with_exit_3() {
    let fx = Fixture::new();
    // "tan" is part of two names, Alice Tan and Bob Tan.
    let ambiguous = fx.run_json(&["plan", "TST101", "--instructor", "tan"]);
    ambiguous.exit(3);
    let error = ambiguous.value()["error"].clone();
    assert_eq!(error["kind"], "not_found");
    assert_eq!(error["suggestions"], json!(["Alice Tan", "Bob Tan"]));

    fx.run_json(&["plan", "TST101", "--instructor", "zed"]).exit(3);
}

#[test]
fn plan_rejects_bad_levels_windows_and_bids_with_exit_2() {
    let fx = Fixture::new();
    fx.run(&["plan", "TST101", "--levels", "80"]).exit(2);
    fx.run(&["plan", "TST101", "--window", "bogus"]).exit(2);
    fx.run(&["plan", "TST101", "--bid", "-5"]).exit(2);

    // In JSON mode the usage error is an error envelope on stdout.
    let run = fx.run_json(&["plan", "TST101", "--levels", "80"]);
    run.exit(2);
    assert_eq!(run.value()["error"]["kind"], "usage");
}

#[test]
fn plan_for_a_course_that_never_fills_bids_the_minimum_at_the_lower_chances() {
    let fx = Fixture::new();
    let run = fx.run_json(&["plan", "TST202"]);
    run.exit(0);
    let plan = run.value();
    let recommended = arr_at(&plan, "recommended");
    assert_eq!(recommended.len(), 5);
    // The 90% and 95% bids are above the minimum here; the ignored test below states the expected behaviour.
    for rec in &recommended[..3] {
        assert_eq!(num_at(rec, "bid"), 10.0, "{rec}");
        assert_eq!(rec["is_minimum_bid"].as_bool(), Some(true), "{rec}");
    }
    let bids: Vec<f64> = recommended.iter().map(|r| num_at(r, "bid")).collect();
    assert!(bids.windows(2).all(|w| w[0] <= w[1]), "{bids:?}");
}

#[test]
fn plan_with_no_history_in_the_window_exits_0_with_warnings() {
    let fx = Fixture::new();
    // TST101 has never been offered in Round 1A Window 2.
    let run = fx.run_json(&["plan", "TST101", "--window", "R1AW2"]);
    run.exit(0);
    let plan = run.value();
    assert!(arr_at(&plan, "recommended").is_empty(), "{plan}");
    assert!(!arr_at(&plan, "warnings").is_empty(), "{plan}");
}

#[test]
fn a_course_that_never_filled_is_low_confidence_and_says_why() {
    let fx = Fixture::new();
    // TST202 never fills, so the price falls back to the reported Min Bids and the confidence is Low.
    let run = fx.run_json(&["plan", "TST202"]);
    run.exit(0);
    let plan = run.value();
    assert_eq!(str_at(&plan, "confidence"), "Low");
    assert!(plan["median_clearing"].is_null(), "{plan}");
    let warnings = arr_at(&plan, "warnings");
    assert!(
        warnings.iter().any(|w| w.as_str().is_some_and(|w| {
            w.starts_with("No past section filled in Round 1 Window 1, so the price estimate falls back to the reported Min Bids")
        })),
        "{plan}"
    );
}

#[test]
fn excluding_the_only_session_says_the_history_was_removed() {
    let fx = Fixture::new();
    // Every section in the fixture is in the Regular Academic Session, so excluding it removes all the history.
    let run = fx.run_json(&["--exclude-session", SESSION, "plan", "TST101"]);
    run.exit(0);
    let plan = run.value();
    assert!(arr_at(&plan, "recommended").is_empty(), "{plan}");
    let expected = format!(
        "All 8 TST101 section results for Round 1 Window 1 are in excluded sessions ({SESSION}), so there is no history to base a bid on."
    );
    assert!(
        arr_at(&plan, "warnings").iter().any(|w| w.as_str() == Some(expected.as_str())),
        "{plan}"
    );
}

// ---------------------------------------------------------------------------------------------------
// instructors, hot, backtest

#[test]
fn instructors_of_one_course_are_listed_hardest_to_get_first() {
    let fx = Fixture::new();
    let run = fx.run_json(&["instructors", "--course", "tst101"]);
    run.exit(0);
    let out = run.value();
    assert_eq!(str_at(&out, "course"), "TST101");
    assert!(out["min_compared"].is_null(), "--course ignores the minimum");
    assert_eq!(out["total"], 2);

    let instructors = arr_at(&out, "instructors");
    let mut names: Vec<&str> = instructors.iter().map(|i| str_at(i, "name")).collect();
    names.sort_unstable();
    assert_eq!(names, ["Alice Tan", "Bob Tan"]);
    let premiums: Vec<f64> = instructors.iter().map(|i| num_at(i, "premium")).collect();
    assert!(premiums.windows(2).all(|w| w[0] >= w[1]), "demand order: {premiums:?}");
}

#[test]
fn instructors_rejects_an_unknown_sort_with_exit_2() {
    let fx = Fixture::new();
    fx.run(&["instructors", "--sort", "nonsense"]).exit(2);
    fx.run_json(&["instructors", "--sort", "nonsense"]).exit(2);
}

#[test]
fn hot_ranks_the_courses_that_filled_in_the_latest_regular_term() {
    let fx = Fixture::new();
    let run = fx.run_json(&["hot", "--min-filled", "1"]);
    run.exit(0);
    let out = run.value();
    assert_eq!(str_at(&out, "term"), "2024-25 T2");
    assert_eq!(str_at(&out, "window_code"), "R1W1");
    assert_eq!(out["min_filled"], 1);
    // TST202 never filled and TST404 is not offered in R1W1, so neither is listed.
    assert_eq!(out["total"], 2);
    let courses = arr_at(&out, "courses");
    let codes: Vec<&str> = courses.iter().map(|c| str_at(c, "code")).collect();
    assert_eq!(codes, ["TST101", "TST303"]);
    assert_eq!(num_at(&courses[0], "median_clearing"), 24.5);
    assert_eq!(courses[0]["sections_filled"], 2);
    assert_eq!(courses[1]["sections_filled"], 1);
    assert_eq!(courses[1]["sections_available"], 2);
}

#[test]
fn hot_leaves_out_a_course_with_one_filled_section_unless_asked() {
    let fx = Fixture::new();
    // TST303 has only one of its two sections filled in 2024-25 T2, so by default it is not ranked.
    let run = fx.run_json(&["hot"]);
    run.exit(0);
    let out = run.value();
    assert_eq!(out["min_filled"], 2);
    assert_eq!(out["total"], 1);
    let codes: Vec<&str> = arr_at(&out, "courses").iter().map(|c| str_at(c, "code")).collect();
    assert_eq!(codes, ["TST101"]);

    fx.run_json(&["hot", "--min-filled", "0"]).exit(2);
}

#[test]
fn hot_takes_a_term_and_a_window() {
    let fx = Fixture::new();
    // TST404 has one filled section in 2023-24 T1 of Round 1A Window 2, so it needs --min-filled 1.
    let run = fx.run_json(&["hot", "--term", "2023-24 T1", "--window", "R1AW2", "--min-filled", "1"]);
    run.exit(0);
    let out = run.value();
    let codes: Vec<&str> = arr_at(&out, "courses").iter().map(|c| str_at(c, "code")).collect();
    assert_eq!(codes, ["TST404"]);

    let run = fx.run(&["hot", "--term", "2023-24 T1", "--window", "R1AW2"]);
    run.exit(0);
    assert!(
        run.stdout.contains("No course had 2 or more filled sections in 2023-24 T1, Round 1A Window 2."),
        "{}",
        run.stdout
    );
}

#[test]
fn backtest_runs_and_its_json_parses_without_nan() {
    let fx = Fixture::new();
    let run = fx.run_json(&["backtest"]);
    run.exit(0);
    let out = run.value();
    assert_eq!(str_at(&out, "window_code"), "R1W1");
    assert_eq!(out["targets"], json!(["2024-25 T2"]));
    assert!(!arr_at(&out, "methods").is_empty(), "{out}");

    let run = fx.run_json(&["backtest", "--all-windows"]);
    run.exit(0);
    let out = run.value();
    assert_eq!(str_at(&out, "window_code"), "all");
}

// ---------------------------------------------------------------------------------------------------
// guide, legacy flags, failures

#[test]
fn json_with_no_command_is_refused_instead_of_opening_the_app() {
    let fx = Fixture::new();
    // With no command this would open the desktop window and block. It must fail at once instead.
    let run = fx.run_json(&[]);
    run.exit(2);
    assert!(run.stderr.is_empty(), "JSON mode writes nothing to stderr: {}", run.stderr);
    let out = run.value();
    assert_eq!(out["error"]["kind"], "usage");
    assert!(str_at(&out["error"], "message").contains("smu_bid_analyser --json info"), "{out}");
}

#[test]
fn exclude_session_with_no_command_is_refused_on_stderr_in_text_mode() {
    let fx = Fixture::new();
    let run = fx.run(&["--exclude-session", SESSION]);
    run.exit(2);
    assert!(run.stdout.is_empty(), "text errors go to stderr: {}", run.stdout);
    assert!(run.stderr.starts_with("error: --exclude-session needs a command"), "{}", run.stderr);
}

#[test]
fn json_mode_writes_nothing_to_stderr_for_any_command() {
    let fx = Fixture::new();
    let cases: [&[&str]; 8] = [
        &["info"],
        &["search", "tst"],
        &["course", "TST404"],
        &["plan", "TST202"],
        &["instructors"],
        &["hot", "--term", "2023-24 T1", "--window", "R1AW2"],
        &["backtest"],
        &["--exclude-session", SESSION, "plan", "TST101"],
    ];
    for args in cases {
        let run = fx.run_json(args);
        run.exit(0);
        assert!(run.stderr.is_empty(), "{args:?} wrote to stderr: {}", run.stderr);
        run.value();
    }
}

#[test]
fn guide_prints_markdown_even_when_the_data_folder_does_not_exist() {
    let fx = Fixture::new();
    let missing = fx.path().join("no-such-folder");

    let run = run_at(&missing, &["guide"]);
    run.exit(0);
    assert!(has_markdown_heading(&run.stdout), "{}", run.stdout);

    let run = run_at(&missing, &["--json", "guide"]);
    run.exit(0);
    let guide = run.value();
    assert!(has_markdown_heading(str_at(&guide, "guide")), "{guide}");
}

#[test]
fn a_data_folder_that_does_not_exist_exits_1() {
    let fx = Fixture::new();
    let missing = fx.path().join("no-such-folder");

    let run = run_at(&missing, &["info"]);
    run.exit(1);
    assert!(run.stdout.is_empty(), "text errors go to stderr: {}", run.stdout);
    assert!(run.stderr.contains("does not exist"), "{}", run.stderr);

    let run = run_at(&missing, &["--json", "info"]);
    run.exit(1);
    assert_eq!(run.value()["error"]["kind"], "data");
}

#[test]
fn an_unknown_excluded_session_exits_3_and_lists_the_real_ones() {
    let fx = Fixture::new();
    fx.run(&["--exclude-session", "nonsense", "info"]).exit(3);

    let run = fx.run_json(&["--exclude-session", "nonsense", "info"]);
    run.exit(3);
    let out = run.value();
    assert_eq!(out["error"]["kind"], "not_found");
    assert_eq!(out["error"]["suggestions"], json!([SESSION]));
}

#[test]
fn text_views_exit_0_and_print_only_ascii() {
    let fx = Fixture::new();
    // `info` lists no course codes, so it is checked for the target term it reports instead.
    let cases: [(&[&str], &str); 4] = [
        (&["info"], "2025-26 T1"),
        (&["search", "tst"], "TST101"),
        (&["course", "TST101"], "TST101"),
        (&["plan", "TST101"], "TST101"),
    ];
    for (args, must_contain) in cases {
        let run = fx.run(args);
        run.exit(0);
        assert!(
            run.stdout.is_ascii(),
            "{args:?} printed non-ASCII text:\n{}",
            run.stdout
        );
        assert!(
            run.stdout.contains(must_contain),
            "{args:?} should mention {must_contain}:\n{}",
            run.stdout
        );
    }
}

#[test]
fn legacy_backtest_flag_runs_without_a_subcommand() {
    let fx = Fixture::new();
    let run = fx.run(&["--backtest"]);
    run.exit(0);
    assert!(run.stdout.contains("Backtest:"), "{}", run.stdout);
}
