//! Reads the raw BOSS "Overall Results" exports (.xls / .xlsx) and cleans them.
//!
//! The exports are not uniform: older files are BIFF .xls, newer ones are .xlsx, some have the
//! header row repeated mid-sheet, pagination debris ("Page size ...") when scraped, and the
//! scraper in `Bid History/` uses the short web column names ("Sect", "Bef Proc", ...).
//! Everything is normalised here so the rest of the app only sees clean `BidRow`s.

use crate::data::{AppData, BidRow, Descriptions, FileReport, LoadReport, RoundKind, Term, TermKind};
use calamine::{open_workbook_auto, Data, Reader};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

const EXTENSIONS: &[&str] = &["xls", "xlsx", "xlsm", "xlsb", "ods"];

/// Bid rows and course descriptions read from one workbook.
type FileRows = (Vec<BidRow>, Descriptions);
/// One workbook's whole contribution: its rows, its descriptions and the report shown in the Data tab.
type FileParse = (Vec<BidRow>, Descriptions, FileReport);

pub fn load_all_data(data_dir: &Path) -> Result<AppData, String> {
    let started = Instant::now();
    if !data_dir.is_dir() {
        return Err(format!("Data folder does not exist:\n{}", data_dir.display()));
    }

    let mut paths: Vec<PathBuf> = std::fs::read_dir(data_dir)
        .map_err(|e| format!("Cannot read {}: {}", data_dir.display(), e))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip Excel/OneDrive lock files such as "~$2025-26_T1.xlsx".
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            p.is_file() && !name.starts_with("~$") && EXTENSIONS.contains(&ext.as_str())
        })
        .collect();
    paths.sort();

    if paths.is_empty() {
        return Err(format!(
            "No .xls/.xlsx files found in:\n{}\n\nDownload the BOSS Overall Results for each term and put them there.",
            data_dir.display()
        ));
    }

    // Files are independent, so parse them in parallel.
    let results: Vec<FileParse> = std::thread::scope(|s| {
        let handles: Vec<_> = paths.iter().map(|p| s.spawn(move || load_file(p))).collect();
        handles.into_iter().zip(&paths).map(|(h, path)| file_or_panic(path, h.join())).collect()
    });

    let mut rows = Vec::new();
    let mut descriptions = Descriptions::new();
    let mut files = Vec::new();
    for (file_rows, file_desc, report) in results {
        rows.extend(file_rows);
        for (code, (term, desc)) in file_desc {
            let e = descriptions.entry(code).or_insert((term, desc.clone()));
            if term >= e.0 {
                *e = (term, desc);
            }
        }
        files.push(report);
    }

    if rows.is_empty() {
        let errors: Vec<String> = files
            .iter()
            .filter_map(|f| f.error.as_ref().map(|e| format!("{}: {}", f.name, e)))
            .collect();
        return Err(format!("No usable bid rows found in {}.\n{}", data_dir.display(), errors.join("\n")));
    }

    // Names are only comparable across files, so this has to wait until every file has been read.
    normalise_instructors(&mut rows);
    let report = LoadReport { dir: data_dir.to_path_buf(), files, millis: started.elapsed().as_millis() };
    Ok(AppData::new(rows, descriptions, report))
}

#[derive(Default)]
struct Columns {
    term: Option<usize>,
    session: Option<usize>,
    window: Option<usize>,
    course: Option<usize>,
    description: Option<usize>,
    section: Option<usize>,
    vacancy: Option<usize>,
    opening: Option<usize>,
    before: Option<usize>,
    dice: Option<usize>,
    after: Option<usize>,
    enrolled: Option<usize>,
    median: Option<usize>,
    min: Option<usize>,
    instructor: Option<usize>,
    school: Option<usize>,
}

impl Columns {
    fn from_header(cells: &[Data]) -> Self {
        let mut c = Columns::default();
        for (i, cell) in cells.iter().enumerate() {
            let slot = match normalise_header(&cell_str(cell)).as_str() {
                "term" => &mut c.term,
                "session" => &mut c.session,
                "biddingwindow" | "window" => &mut c.window,
                "coursecode" | "course" => &mut c.course,
                "description" | "coursetitle" | "title" => &mut c.description,
                "section" | "sect" => &mut c.section,
                "vacancy" => &mut c.vacancy,
                "openingvacancy" | "open" => &mut c.opening,
                "beforeprocessvacancy" | "befproc" => &mut c.before,
                "dice" => &mut c.dice,
                "afterprocessvacancy" | "aftproc" => &mut c.after,
                "enrolledstudents" | "enrolled" => &mut c.enrolled,
                "medianbid" | "median" => &mut c.median,
                "minbid" | "min" => &mut c.min,
                "instructor" | "instructors" => &mut c.instructor,
                "schooldepartment" | "school" => &mut c.school,
                _ => continue,
            };
            slot.get_or_insert(i);
        }
        c
    }

    fn missing_required(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        let required = [
            ("Bidding Window", self.window),
            ("Course Code", self.course),
            ("Section", self.section),
            ("Before Process Vacancy", self.before),
            ("After Process Vacancy", self.after),
            ("Median Bid", self.median),
            ("Min Bid", self.min),
        ];
        for (name, col) in required {
            if col.is_none() {
                missing.push(name);
            }
        }
        missing
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn load_file(path: &Path) -> FileParse {
    let mut report = FileReport { name: file_name(path), term: term_from_filename(path), ..Default::default() };
    match parse_file(path, &mut report) {
        Ok((rows, desc)) => (rows, desc, report),
        Err(e) => {
            report.error = Some(e);
            (Vec::new(), Descriptions::new(), report)
        }
    }
}

/// A parser thread that panicked (a corrupt workbook can trip a bug in the reader) fails only its
/// own file. Re-raising the panic would take the whole app down at startup.
fn file_or_panic(path: &Path, joined: std::thread::Result<FileParse>) -> FileParse {
    joined.unwrap_or_else(|_| {
        let report = FileReport {
            name: file_name(path),
            term: term_from_filename(path),
            error: Some("the file could not be read (the parser crashed)".to_string()),
            ..Default::default()
        };
        (Vec::new(), Descriptions::new(), report)
    })
}

fn parse_file(path: &Path, report: &mut FileReport) -> Result<FileRows, String> {
    let mut workbook = open_workbook_auto(path).map_err(|e| e.to_string())?;
    let sheet = workbook.sheet_names().first().cloned().ok_or("workbook has no sheets")?;
    let range = workbook.worksheet_range(&sheet).map_err(|e| e.to_string())?;

    let mut iter = range.rows();
    let mut cols = None;
    for row in iter.by_ref().take(30) {
        let c = Columns::from_header(row);
        if c.course.is_some() && c.window.is_some() {
            cols = Some(c);
            break;
        }
    }
    let cols = cols.ok_or("could not find a header row with 'Course Code' and 'Bidding Window'")?;
    let missing = cols.missing_required();
    if !missing.is_empty() {
        return Err(format!("missing columns: {}", missing.join(", ")));
    }

    let mut interner = Interner::default();
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    let mut descriptions = Descriptions::new();

    for row in iter {
        let get = |col: Option<usize>| col.and_then(|i| row.get(i)).unwrap_or(&Data::Empty);
        let text = |col: Option<usize>| cell_str(get(col));
        let num = |col: Option<usize>| cell_f64(get(col));

        let window_text = text(cols.window);
        let course_code = text(cols.course).to_ascii_uppercase();
        // Repeated header rows, pagination debris and blank lines.
        if course_code.is_empty() || window_text.is_empty() || normalise_header(&window_text) == "biddingwindow" {
            report.rows_invalid += 1;
            continue;
        }

        let (round, window) = match parse_window(&window_text) {
            WindowParse::Regular(r, w) => (r, w),
            WindowParse::Special => {
                // Incoming freshmen / exchange rounds are not open to regular bidders.
                report.rows_special_rounds += 1;
                continue;
            }
            WindowParse::Unknown => {
                report.rows_invalid += 1;
                continue;
            }
        };

        let term = match cols.term.map(|_| text(cols.term)).and_then(|t| parse_term(&t)).or(report.term) {
            Some(t) => t,
            None => {
                report.rows_invalid += 1;
                continue;
            }
        };

        let section = text(cols.section);
        let session = {
            let s = text(cols.session);
            if s.is_empty() { "Regular Academic Session".to_string() } else { s }
        };
        if !seen.insert((term, round, window, course_code.clone(), section.clone(), session.clone())) {
            report.duplicates += 1;
            continue;
        }

        let course_code = interner.get(&course_code);
        let description = collapse_whitespace(&text(cols.description));
        if !description.is_empty() {
            let e = descriptions.entry(course_code.clone()).or_insert((term, description.clone()));
            if term > e.0 {
                *e = (term, description);
            }
        }

        rows.push(BidRow {
            term,
            session: interner.get(&session),
            round,
            window,
            course_code,
            section: interner.get(&section),
            instructor: interner.get(&collapse_whitespace(&text(cols.instructor))),
            school: interner.get(&text(cols.school)),
            vacancy: num(cols.vacancy) as i32,
            opening_vacancy: num(cols.opening) as i32,
            before_proc: num(cols.before) as i32,
            dice: num(cols.dice) as i32,
            after_proc: num(cols.after) as i32,
            enrolled: num(cols.enrolled) as i32,
            median_bid: num(cols.median).max(0.0),
            min_bid: num(cols.min).max(0.0),
        });
        report.term.get_or_insert(term);
    }

    report.rows_kept = rows.len();
    Ok((rows, descriptions))
}

#[derive(Default)]
struct Interner(HashMap<String, Arc<str>>);

impl Interner {
    fn get(&mut self, s: &str) -> Arc<str> {
        if let Some(a) = self.0.get(s) {
            return a.clone();
        }
        let a: Arc<str> = Arc::from(s);
        self.0.insert(s.to_string(), a.clone());
        a
    }
}

fn cell_str(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(s) => s.trim().to_string(),
        Data::Float(f) if f.fract() == 0.0 => format!("{}", *f as i64),
        other => other.to_string().trim().to_string(),
    }
}

fn cell_f64(cell: &Data) -> f64 {
    let v = match cell {
        Data::Float(f) => *f,
        Data::Int(i) => *i as f64,
        Data::String(s) => s.trim().trim_start_matches('$').replace(',', "").parse().unwrap_or(0.0),
        _ => 0.0,
    };
    // Text such as "inf" parses as a float. An infinite bid would turn the model's log prices and
    // bandwidths into NaN, so treat it like a blank cell.
    if v.is_finite() { v } else { 0.0 }
}

fn normalise_header(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Rewrites every row's instructor to its normalised name. Rows that share a name share one `Arc`.
fn normalise_instructors(rows: &mut [BidRow]) {
    let mut counts: HashMap<Arc<str>, usize> = HashMap::new();
    for r in rows.iter() {
        *counts.entry(r.instructor.clone()).or_default() += 1;
    }
    let names = normalised_names(&counts);
    for r in rows.iter_mut() {
        if let Some(name) = names.get(&r.instructor) {
            r.instructor = name.clone();
        }
    }
}

/// The normalised name of every spelling in `counts` (how many rows use each one).
///
/// Spellings that differ only in case are unified to the most common one, so "Mok Heng Ngee" becomes
/// "MOK HENG NGEE" when the upper-case form is used more. Known names that differ only by hyphens and
/// spaces are unified as well (see `Known`). A comma list can only merge into a name that stands alone
/// somewhere in the data (see `normalise_instructor`).
fn normalised_names(counts: &HashMap<Arc<str>, usize>) -> HashMap<Arc<str>, Arc<str>> {
    let mut by_use: Vec<(&str, usize)> = counts.iter().map(|(name, &n)| (&**name, n)).collect();
    by_use.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

    // Each cell (one whole instructor entry, ignoring case) takes its most common spelling, and the rows of
    // every spelling count towards it.
    let mut cells: Vec<(&str, usize)> = Vec::new();
    let mut cell_of: HashMap<String, usize> = HashMap::new();
    for &(name, n) in &by_use {
        let i = *cell_of.entry(name.to_uppercase()).or_insert_with(|| {
            cells.push((name, 0));
            cells.len() - 1
        });
        cells[i].1 += n;
    }

    // A name stands alone when its cell holds one distinct name: "ONG SIOW HENG" or "ONG SIOW HENG, ONG SIOW HENG".
    let standalone: Vec<(String, usize)> = cells
        .iter()
        .filter_map(|&(cell, n)| match distinct_parts(cell).as_slice() {
            [only] => Some((only.clone(), n)),
            _ => None,
        })
        .collect();
    let known = Known::new(&standalone);

    let mut interned: HashMap<String, Arc<str>> = HashMap::new();
    let mut out = HashMap::with_capacity(counts.len());
    for name in counts.keys() {
        let cell = cells[cell_of[&name.to_uppercase()]].0;
        let normalised = normalise_instructor(cell, &known);
        let shared = interned.entry(normalised).or_insert_with_key(|n| Arc::from(n.as_str())).clone();
        out.insert(name.clone(), shared);
    }
    out
}

/// The distinct names in one instructor cell, in order: "A, B, A" gives ["A", "B"]. Empty parts are dropped,
/// and a repeat that differs only in case is the same name: "A, a" gives ["A"].
fn distinct_parts(cell: &str) -> Vec<String> {
    let collapsed = collapse_whitespace(cell);
    let mut parts: Vec<String> = Vec::new();
    for part in collapsed.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if !parts.iter().any(|p| p.to_uppercase() == part.to_uppercase()) {
            parts.push(part.to_string());
        }
    }
    parts
}

/// The names that stand alone somewhere in the data, and the spelling each one is stored under.
///
/// Lookups ignore case. A name that is not known exactly can still match a known name that differs from
/// it only by hyphens and spaces: "KIM JUNGBAE" finds "KIM JUNG-BAE". Names that differ only that way are
/// all stored under the spelling that more rows use (a tie goes to the alphabetically first), so
/// "ALVIN SEE WEI LIANG" and "ALVIN SEE WEI-LIANG" both become whichever is more common. Such a match needs
/// a hyphen to account for the difference: "TAN RANI" and "TANRANI" are not joined, and a plain word reorder
/// such as "RANI TAN" and "TAN RANI" is never joined.
#[derive(Default)]
struct Known {
    /// Upper-case name -> the spelling it is stored under.
    stored: HashMap<String, String>,
    /// `squash` of a name -> the stored spelling, for the groups of names that contain a hyphen.
    by_squashed: HashMap<String, String>,
}

impl Known {
    /// `names` holds each standalone name with the number of rows that use it.
    fn new(names: &[(String, usize)]) -> Self {
        // The rows of every spelling of an upper-case name add up, and its most used spelling is the one
        // stored. Sorting first makes that choice independent of hash order.
        let mut ordered: Vec<&(String, usize)> = names.iter().collect();
        ordered.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let mut uses: Vec<(String, String, usize)> = Vec::new(); // (upper-case name, spelling, rows)
        let mut index: HashMap<String, usize> = HashMap::new();
        for (name, n) in ordered {
            let upper = name.to_uppercase();
            let i = match index.get(&upper) {
                Some(&i) => i,
                None => {
                    uses.push((upper.clone(), name.clone(), 0));
                    index.insert(upper, uses.len() - 1);
                    uses.len() - 1
                }
            };
            uses[i].2 += *n;
        }

        let mut known = Known::default();
        for (upper, spelling, _) in &uses {
            known.stored.insert(upper.clone(), spelling.clone());
        }
        let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, (upper, _, _)) in uses.iter().enumerate() {
            let key = squash(upper);
            if !key.is_empty() {
                groups.entry(key).or_default().push(i);
            }
        }
        for (key, members) in groups {
            if !members.iter().any(|&i| uses[i].0.contains('-')) {
                continue;
            }
            // The group takes the spelling of its most used member; a tie goes to the alphabetically first.
            let best = *members
                .iter()
                .min_by(|&&a, &&b| uses[b].2.cmp(&uses[a].2).then_with(|| uses[a].1.cmp(&uses[b].1)))
                .expect("a group has members");
            let target = uses[best].1.clone();
            for &i in &members {
                known.stored.insert(uses[i].0.clone(), target.clone());
            }
            known.by_squashed.insert(key, target);
        }
        known
    }

    /// The stored spelling of `name`, or `None` when no known name matches it.
    fn find(&self, name: &str) -> Option<&str> {
        let upper = name.to_uppercase();
        match self.stored.get(&upper) {
            Some(spelling) => Some(spelling.as_str()),
            None => self.by_squashed.get(&squash(&upper)).map(String::as_str),
        }
    }
}

/// The key that hyphen-insensitive lookups compare: the upper-case name without hyphens or spaces.
fn squash(upper: &str) -> String {
    upper.chars().filter(|&c| c != '-' && !c.is_whitespace()).collect()
}

/// The stored form of one instructor cell. `known` holds the names that stand alone in the data.
///
/// The rules are narrow, because a wrong merge joins two people:
/// * empty and repeated parts are dropped: "A, A" becomes "A" and "A, B, A" becomes "A, B";
/// * a part that matches a known name takes that name's stored spelling (see `Known`);
/// * a pair "A, B" merges into one name only when the parts are not both known names in their own right, so
///   co-teaching lists such as "SAY GUI DENG, ARNOLD WEE" are kept. It becomes "A B" when that is known. If
///   not, it becomes "B A" when that is known, so "SURNAME, GIVEN" becomes "GIVEN SURNAME". When both joins
///   are known, "A B" wins, as the original rule did;
/// * any other list stays a co-teaching list, joined with ", ".
fn normalise_instructor(raw: &str, known: &Known) -> String {
    let mut parts: Vec<String> = Vec::new();
    for part in distinct_parts(raw) {
        let name = known.find(&part).map_or(part, str::to_string);
        if !parts.contains(&name) {
            parts.push(name);
        }
    }
    if let [a, b] = parts.as_slice() {
        let both_known = known.find(a).is_some() && known.find(b).is_some();
        if !both_known {
            let joined = known.find(&format!("{a} {b}"));
            let reversed = known.find(&format!("{b} {a}"));
            if let Some(name) = joined.or(reversed) {
                return name.to_string();
            }
        }
    }
    parts.join(", ")
}

#[derive(Debug, PartialEq)]
enum WindowParse {
    Regular(RoundKind, u8),
    Special,
    Unknown,
}

/// "Round 1A Window 2" -> (Round1A, 2). Also accepts "Rnd 1A Win 2" and "Round1A Window2".
fn parse_window(s: &str) -> WindowParse {
    let lower = s.to_ascii_lowercase();
    if lower.contains("freshmen") || lower.contains("exchange") || lower.contains("incoming") {
        return WindowParse::Special;
    }
    let tokens: Vec<&str> = lower.split_whitespace().collect();
    let mut round = None;
    let mut window = None;
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i];
        for prefix in ["round", "rnd"] {
            if let Some(rest) = t.strip_prefix(prefix) {
                let val = if rest.is_empty() { tokens.get(i + 1).copied().unwrap_or("") } else { rest };
                round = round.or(RoundKind::parse(val));
            }
        }
        for prefix in ["window", "win"] {
            if let Some(rest) = t.strip_prefix(prefix) {
                let val = if rest.is_empty() { tokens.get(i + 1).copied().unwrap_or("") } else { rest };
                window = window.or(val.parse::<u8>().ok());
            }
        }
        i += 1;
    }
    match (round, window) {
        (Some(r), Some(w)) => WindowParse::Regular(r, w),
        _ => WindowParse::Unknown,
    }
}

/// "2025-26 Term 3A" -> Term { 2025, T3A }.
fn parse_term(s: &str) -> Option<Term> {
    let year = find_year(s)?;
    let lower = s.to_ascii_lowercase();
    let after = lower.split("term").nth(1)?.trim();
    let kind = parse_term_kind(after.split_whitespace().next()?)?;
    Some(Term { year, kind })
}

/// "2025-26_T2.xlsx" -> Term { 2025, T2 }.
fn term_from_filename(path: &Path) -> Option<Term> {
    let stem = path.file_stem()?.to_str()?.to_ascii_lowercase();
    let year = find_year(&stem)?;
    let pos = stem.rfind(['t', 'T'])?;
    Some(Term { year, kind: parse_term_kind(&stem[pos + 1..])? })
}

pub(crate) fn parse_term_kind(s: &str) -> Option<TermKind> {
    match s.trim().to_ascii_uppercase().as_str() {
        "1" => Some(TermKind::T1),
        "2" => Some(TermKind::T2),
        "3A" => Some(TermKind::T3A),
        "3B" => Some(TermKind::T3B),
        _ => None,
    }
}

/// The first run of exactly four digits, if it is a plausible year (2000-2099).
pub(crate) fn find_year(s: &str) -> Option<u16> {
    let b = s.as_bytes();
    (0..b.len().saturating_sub(3))
        .find(|&i| {
            b[i..i + 4].iter().all(u8::is_ascii_digit)
                && (i == 0 || !b[i - 1].is_ascii_digit())
                && b.get(i + 4).is_none_or(|c| !c.is_ascii_digit())
        })
        .and_then(|i| s[i..i + 4].parse().ok())
        .filter(|y| (2000..2100).contains(y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_windows() {
        assert_eq!(parse_window("Round 1A Window 2"), WindowParse::Regular(RoundKind::Round1A, 2));
        assert_eq!(parse_window("Round 1 Window 1"), WindowParse::Regular(RoundKind::Round1, 1));
        assert_eq!(parse_window("Rnd 2A Win 3"), WindowParse::Regular(RoundKind::Round2A, 3));
        assert_eq!(parse_window("Round2 Window3"), WindowParse::Regular(RoundKind::Round2, 3));
        assert_eq!(parse_window("Incoming Freshmen Rnd 1 Win 4"), WindowParse::Special);
        assert_eq!(parse_window("Incoming Exchange Rnd 1C Win 1"), WindowParse::Special);
        assert_eq!(parse_window("Page size: 10"), WindowParse::Unknown);
    }

    #[test]
    fn parses_terms() {
        assert_eq!(parse_term("2025-26 Term 1"), Some(Term { year: 2025, kind: TermKind::T1 }));
        assert_eq!(parse_term("2024-25 Term 3B"), Some(Term { year: 2024, kind: TermKind::T3B }));
        assert_eq!(parse_term("Term"), None);
        assert_eq!(
            term_from_filename(Path::new("2022-23_T3A.xls")),
            Some(Term { year: 2022, kind: TermKind::T3A })
        );
        assert_eq!(
            term_from_filename(Path::new("C:/x/2026-27_T1.xlsx")),
            Some(Term { year: 2026, kind: TermKind::T1 })
        );
    }

    #[test]
    fn maps_scraper_headers() {
        let header: Vec<Data> = ["Term", "Session", "Bidding Window", "Course", "Description", "Sect", "Vacancy", "Open",
            "Bef Proc", "DICE", "Aft Proc", "Enrolled", "Median", "Min", "Instructor", "School"]
            .iter()
            .map(|s| Data::String(s.to_string()))
            .collect();
        let c = Columns::from_header(&header);
        assert!(c.missing_required().is_empty());
        assert_eq!(c.before, Some(8));
        assert_eq!(c.school, Some(15));
    }

    #[test]
    fn year_is_a_run_of_exactly_four_digits() {
        assert_eq!(find_year("2026-27 T1"), Some(2026));
        assert_eq!(find_year("AY2026-27"), Some(2026));
        assert_eq!(find_year("x2026"), Some(2026));
        assert_eq!(find_year("2026"), Some(2026));
        assert_eq!(find_year("20261 T1"), None, "five digits are not a year");
        assert_eq!(find_year("12026-27"), None);
        assert_eq!(find_year("1999-00"), None);
    }

    #[test]
    fn non_finite_cells_read_as_zero() {
        assert_eq!(cell_f64(&Data::String("inf".into())), 0.0);
        assert_eq!(cell_f64(&Data::String("-Infinity".into())), 0.0);
        assert_eq!(cell_f64(&Data::String("NaN".into())), 0.0);
        assert_eq!(cell_f64(&Data::Float(f64::INFINITY)), 0.0);
        assert_eq!(cell_f64(&Data::String(" $1,234.50 ".into())), 1234.5);
        assert_eq!(cell_f64(&Data::Float(12.5)), 12.5);
    }

    #[test]
    fn panicked_parser_fails_only_its_own_file() {
        let payload: Box<dyn std::any::Any + Send> = Box::new("parser exploded");
        let (rows, descriptions, report) = file_or_panic(Path::new("C:/data/2025-26_T1.xlsx"), Err(payload));
        assert!(rows.is_empty() && descriptions.is_empty());
        assert_eq!(report.name, "2025-26_T1.xlsx");
        assert_eq!(report.term, Some(Term { year: 2025, kind: TermKind::T1 }));
        assert!(report.error.is_some());
    }

    /// A `Known` holding `names`, each used by one row.
    fn known_names(names: &[&str]) -> Known {
        let rows: Vec<(String, usize)> = names.iter().map(|s| (s.to_string(), 1)).collect();
        Known::new(&rows)
    }

    #[test]
    fn repeated_parts_collapse() {
        let none = known_names(&[]);
        assert_eq!(normalise_instructor("ALTHEA TOH, ALTHEA TOH", &none), "ALTHEA TOH");
        assert_eq!(normalise_instructor("BEI HONG, BEI HONG, BEI HONG", &none), "BEI HONG");
        assert_eq!(normalise_instructor("AIDAN WONG, YUANTO KUSNADI, AIDAN WONG", &none), "AIDAN WONG, YUANTO KUSNADI");
        assert_eq!(distinct_parts(" A ,, B , A "), vec!["A", "B"]);
        assert_eq!(distinct_parts("A, a, B"), vec!["A", "B"], "a repeat that differs only in case");
    }

    #[test]
    fn a_name_repeated_in_the_data_counts_as_known() {
        let counts: HashMap<Arc<str>, usize> =
            [("LIU KOOI LIN EVELYN, LIU KOOI LIN EVELYN", 9), ("LIU KOOI LIN, EVELYN", 2)]
                .into_iter()
                .map(|(name, n)| (Arc::from(name), n))
                .collect();
        let names = normalised_names(&counts);
        assert_eq!(&*names["LIU KOOI LIN, EVELYN"], "LIU KOOI LIN EVELYN");
        assert_eq!(&*names["LIU KOOI LIN EVELYN, LIU KOOI LIN EVELYN"], "LIU KOOI LIN EVELYN");
    }

    #[test]
    fn surname_given_merges_only_into_a_known_name() {
        let known = known_names(&["MOK HENG NGEE"]);
        assert_eq!(normalise_instructor("MOK, HENG NGEE", &known), "MOK HENG NGEE");
        assert_eq!(normalise_instructor("LOU, YUN", &known), "LOU, YUN", "neither order is in the data");
    }

    #[test]
    fn reversed_surname_given_merges_into_the_known_name() {
        let known = known_names(&["ANDREW KOH", "HYE SUN CHANG", "DEVIN JOSHI", "TONY TANG", "YUN LEE", "NONA PEPITO"]);
        assert_eq!(normalise_instructor("KOH, ANDREW", &known), "ANDREW KOH");
        assert_eq!(normalise_instructor("CHANG, HYE SUN", &known), "HYE SUN CHANG");
        assert_eq!(normalise_instructor("JOSHI, DEVIN", &known), "DEVIN JOSHI");
        assert_eq!(normalise_instructor("TANG, TONY", &known), "TONY TANG");
        assert_eq!(normalise_instructor("LEE, YUN", &known), "YUN LEE");
        assert_eq!(normalise_instructor("PEPITO, NONA", &known), "NONA PEPITO");
        assert_eq!(normalise_instructor("Chang, Hye Sun", &known), "HYE SUN CHANG", "case does not matter");
        assert_eq!(normalise_instructor("KOH, BOB", &known), "KOH, BOB", "neither join is in the data");
    }

    #[test]
    fn reversed_merge_follows_the_order_that_is_known() {
        assert_eq!(normalise_instructor("TAN, RANI", &known_names(&["RANI TAN"])), "RANI TAN");
        assert_eq!(normalise_instructor("TAN, RANI", &known_names(&["TAN RANI"])), "TAN RANI");
        // Both orders are known: "A B" wins, as the original rule did.
        assert_eq!(normalise_instructor("TAN, RANI", &known_names(&["TAN RANI", "RANI TAN"])), "TAN RANI");
        assert_eq!(normalise_instructor("RANI, TAN", &known_names(&["TAN RANI", "RANI TAN"])), "RANI TAN");
        assert_eq!(normalise_instructor("TAN, RANI", &known_names(&["MOK HENG NGEE"])), "TAN, RANI");
    }

    #[test]
    fn co_teaching_lists_are_kept() {
        let known = known_names(&["SAY GUI DENG", "ARNOLD WEE"]);
        assert_eq!(normalise_instructor("SAY GUI DENG, ARNOLD WEE", &known), "SAY GUI DENG, ARNOLD WEE");
        // Both parts are names in their own right, so neither joined form wins even when it is known too.
        let both = known_names(&["SAY GUI DENG", "ARNOLD WEE", "SAY GUI DENG ARNOLD WEE", "ARNOLD WEE SAY GUI DENG"]);
        assert_eq!(normalise_instructor("SAY GUI DENG, ARNOLD WEE", &both), "SAY GUI DENG, ARNOLD WEE");
        assert_eq!(normalise_instructor("ARNOLD WEE, SAY GUI DENG", &both), "ARNOLD WEE, SAY GUI DENG");
    }

    #[test]
    fn ambiguous_lists_are_left_alone() {
        // One part is a known name, but neither joined form is.
        let one = known_names(&["CHEN SIYUAN"]);
        assert_eq!(normalise_instructor("CHEN SIYUAN, MELISSA NG", &one), "CHEN SIYUAN, MELISSA NG");
        // Three names: the nested "YU, SHIHAO" is not merged, even though "YU SHIHAO" is known.
        let three = known_names(&["SUN JIAN", "YU SHIHAO"]);
        assert_eq!(normalise_instructor("SUN JIAN, YU, SHIHAO", &three), "SUN JIAN, YU, SHIHAO");
    }

    #[test]
    fn whitespace_and_empty_parts_are_tidied() {
        let known = known_names(&["MOK HENG NGEE"]);
        assert_eq!(normalise_instructor("  MOK ,  HENG   NGEE  ", &known), "MOK HENG NGEE");
        assert_eq!(normalise_instructor("ALTHEA  TOH,, ,ALTHEA TOH,", &known), "ALTHEA TOH");
        assert_eq!(normalise_instructor("TONY\u{a0}TANG", &known), "TONY TANG");
        assert_eq!(normalise_instructor("BEN LEE,CHUA WAN RONG", &known), "BEN LEE, CHUA WAN RONG");
        assert_eq!(normalise_instructor(" , ", &known), "");
        assert_eq!(normalise_instructor("", &known), "");
    }

    #[test]
    fn spellings_unify_to_the_most_common_and_share_an_arc() {
        let counts: HashMap<Arc<str>, usize> = [
            ("MOK HENG NGEE", 8),
            ("MOK, HENG NGEE", 5),
            ("Mok Heng Ngee", 1),
            ("ALTHEA TOH, ALTHEA TOH", 4),
            ("ALTHEA TOH", 2),
            ("", 3),
        ]
        .into_iter()
        .map(|(name, n)| (Arc::from(name), n))
        .collect();
        let names = normalised_names(&counts);
        assert_eq!(&*names["MOK, HENG NGEE"], "MOK HENG NGEE");
        assert_eq!(&*names["Mok Heng Ngee"], "MOK HENG NGEE");
        assert_eq!(&*names["ALTHEA TOH, ALTHEA TOH"], "ALTHEA TOH");
        assert_eq!(&*names[""], "");
        assert!(Arc::ptr_eq(&names["MOK, HENG NGEE"], &names["MOK HENG NGEE"]));
        assert!(Arc::ptr_eq(&names["Mok Heng Ngee"], &names["MOK HENG NGEE"]));
    }

    #[test]
    fn hyphen_and_space_variants_match_during_a_merge() {
        let known = known_names(&["KIM JUNG-BAE", "ALVIN SEE WEI-LIANG"]);
        assert_eq!(normalise_instructor("KIM, JUNGBAE", &known), "KIM JUNG-BAE", "a hyphen against nothing");
        assert_eq!(normalise_instructor("KIM, JUNG BAE", &known), "KIM JUNG-BAE", "a hyphen against a space");
        assert_eq!(normalise_instructor("ALVIN SEE, WEI LIANG", &known), "ALVIN SEE WEI-LIANG");
        assert_eq!(normalise_instructor("KIM JUNGBAE", &known), "KIM JUNG-BAE", "a standalone name is unified too");
        // Without a hyphen to account for the difference, the names do not match.
        let plain = known_names(&["TANRANI"]);
        assert_eq!(normalise_instructor("TAN, RANI", &plain), "TAN, RANI");
        assert_eq!(normalise_instructor("TAN RANI", &plain), "TAN RANI");
    }

    #[test]
    fn hyphen_variants_unify_to_the_more_common_spelling() {
        let counts: HashMap<Arc<str>, usize> = [
            ("KIM JUNG-BAE", 30),
            ("KIM JUNGBAE", 3),
            ("KIM, JUNGBAE", 5),
            ("ALVIN SEE WEI LIANG", 2),
            ("ALVIN SEE WEI-LIANG", 9),
            ("ALVIN SEE WEI LIANG, ALVIN SEE WEI LIANG", 1),
        ]
        .into_iter()
        .map(|(name, n)| (Arc::from(name), n))
        .collect();
        let names = normalised_names(&counts);
        assert_eq!(&*names["KIM JUNG-BAE"], "KIM JUNG-BAE");
        assert_eq!(&*names["KIM JUNGBAE"], "KIM JUNG-BAE");
        assert_eq!(&*names["KIM, JUNGBAE"], "KIM JUNG-BAE");
        assert_eq!(&*names["ALVIN SEE WEI LIANG"], "ALVIN SEE WEI-LIANG");
        assert_eq!(&*names["ALVIN SEE WEI LIANG, ALVIN SEE WEI LIANG"], "ALVIN SEE WEI-LIANG");
        assert!(Arc::ptr_eq(&names["KIM, JUNGBAE"], &names["KIM JUNG-BAE"]));
    }

    #[test]
    fn equally_common_hyphen_variants_unify_to_the_alphabetically_first() {
        let counts: HashMap<Arc<str>, usize> = [("ALVIN SEE WEI-LIANG", 4), ("ALVIN SEE WEI LIANG", 4)]
            .into_iter()
            .map(|(name, n)| (Arc::from(name), n))
            .collect();
        let names = normalised_names(&counts);
        assert_eq!(&*names["ALVIN SEE WEI-LIANG"], "ALVIN SEE WEI LIANG");
        assert_eq!(&*names["ALVIN SEE WEI LIANG"], "ALVIN SEE WEI LIANG");
    }

    #[test]
    fn plain_reorders_without_a_comma_stay_separate() {
        let counts: HashMap<Arc<str>, usize> = [
            ("RANI TAN", 4),
            ("TAN RANI", 2),
            ("TANRANI", 1),
            ("KIM SEONGHOON", 24),
            ("SEONGHOON KIM", 24),
            ("TAN, RANI", 5),
        ]
        .into_iter()
        .map(|(name, n)| (Arc::from(name), n))
        .collect();
        let names = normalised_names(&counts);
        assert_eq!(&*names["RANI TAN"], "RANI TAN");
        assert_eq!(&*names["TAN RANI"], "TAN RANI");
        assert_eq!(&*names["TANRANI"], "TANRANI");
        assert_eq!(&*names["KIM SEONGHOON"], "KIM SEONGHOON");
        assert_eq!(&*names["SEONGHOON KIM"], "SEONGHOON KIM");
        // Both joins are known, so the comma list takes "A B".
        assert_eq!(&*names["TAN, RANI"], "TAN RANI");
    }

    #[test]
    fn reversed_merge_is_applied_to_the_data() {
        let counts: HashMap<Arc<str>, usize> =
            [("Chang, Hye Sun", 1), ("HYE SUN CHANG", 3)].into_iter().map(|(name, n)| (Arc::from(name), n)).collect();
        let names = normalised_names(&counts);
        assert_eq!(&*names["Chang, Hye Sun"], "HYE SUN CHANG");
        assert!(Arc::ptr_eq(&names["Chang, Hye Sun"], &names["HYE SUN CHANG"]));
    }
}
