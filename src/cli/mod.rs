//! The command-line interface.
//!
//! Every command is a thin layer over `data::query`: parse the arguments, call the query layer, and
//! print the result as a table or, with `--json`, as JSON. The bid logic lives in `data`, not here.
//!
//! Output contract. stdout carries only the result: the table, or one JSON document with `--json`.
//! In text mode, progress ("Loading ...") and warnings ("warning: ...") go to stderr. With `--json`
//! nothing reaches stderr: warnings are the document's `warnings` array, present (empty when there is
//! nothing to say) in every command that can warn. Errors are an `{"error": ...}` object on stdout
//! with `--json`, and an "error: ..." line on stderr without it. Exit codes: 0 ok, 1 the data folder
//! could not be loaded, 2 invalid usage, 3 not found or ambiguous.

pub mod course;
pub mod info;
pub mod legacy;
pub mod output;
pub mod plan;
pub mod rank;
pub mod search;

use crate::config::{self, Config};
use crate::data::filter::{GlobalFilters, WindowSel};
use crate::data::model::ModelParams;
use crate::data::query::{resolve_course, resolve_instructor};
use crate::data::{self, AppData, CourseInfo, Term};
use crate::ui;
use clap::builder::PossibleValuesParser;
use clap::{Args, CommandFactory, Parser, Subcommand};
use serde::Serialize;
use std::path::PathBuf;
use std::process::ExitCode;

const AFTER_HELP: &str = "\
Examples:
  smu_bid_analyser info
  smu_bid_analyser search \"data science\" --limit 10
  smu_bid_analyser --json plan IS216 --window R1W1 --target \"2026-27 T1\"
  smu_bid_analyser instructors --course IS216 --sort easiest
  smu_bid_analyser --exclude-session \"FT Workshop Session\" hot --term \"2025-26 T2\"

Run `smu_bid_analyser guide` for a usage guide written for LLMs and scripts.";

/// Analyse SMU BOSS course-bidding results and work out how much to bid.
///
/// With no command the desktop app opens, unless --json or --exclude-session is given: those need a
/// command, so they are refused instead. Every other command reads the BOSS "Overall Results"
/// folder, prints a table (or JSON with --json) and exits. The folder is the first of: --data-dir,
/// the BOSS_DATA_DIR environment variable, the folder saved in the app, the default OneDrive
/// location, then ./data.
///
/// Windows (--window) are written R1W1 (Round 1, window 1), R1AW2, 1:1 or "all". Terms are written
/// like "2026-27 T2". Course codes ignore case and spaces, so "is 216" finds IS216.
///
/// Exit codes: 0 ok; 1 the data folder could not be loaded; 2 invalid usage; 3 not found or
/// ambiguous (course, instructor, session, school or term). With --json, an error is printed on
/// stdout as {"error": {"kind", "message", "suggestions"}}.
#[derive(Debug, Parser)]
#[command(name = "smu_bid_analyser", version, after_help = AFTER_HELP)]
pub struct Cli {
    /// Folder of BOSS results to open the app on. Same as --data-dir; kept for older scripts.
    #[arg(value_name = "PATH")]
    pub folder: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Command>,

    /// Folder with the BOSS "Overall Results" exports (.xls or .xlsx). Beats BOSS_DATA_DIR and the saved folder.
    #[arg(long, global = true, value_name = "PATH")]
    pub data_dir: Option<PathBuf>,

    /// Print JSON on stdout instead of tables. Errors become {"error": {...}} on stdout too.
    #[arg(long, global = true)]
    pub json: bool,

    /// Leave a session out of every result, e.g. "1A Session". Repeat for several. Case-insensitive.
    #[arg(long = "exclude-session", global = true, value_name = "NAME")]
    pub exclude_session: Vec<String>,

    /// Legacy: print the model's accuracy on past terms, then exit. Use `backtest` instead.
    #[arg(long, hide = true)]
    pub backtest: bool,

    /// Legacy: grid-search the model's parameters, then exit. Use `backtest --tune` instead.
    #[arg(long, hide = true)]
    pub tune: bool,

    /// Legacy: course to select when the app opens. Use `gui --course` instead.
    #[arg(long, value_name = "CODE", hide = true)]
    pub course: Option<String>,

    /// Legacy: tab to open first. Use `gui --tab` instead.
    #[arg(long, value_name = "TAB", hide = true)]
    pub tab: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Open the desktop app. This is what runs when no command is given.
    Gui(GuiArgs),
    /// Summarise the data: files and rows loaded, courses, terms, the term you are bidding for, sessions and schools.
    Info,
    /// Find courses by code or title. Use this to get the exact code the other commands take.
    Search(search::SearchArgs),
    /// The full bid history of one course: each term's clearing prices and fill rate. With --sections, every section.
    Course(course::CourseArgs),
    /// Recommend bids for one course: the bid for each chance of a seat, and the chance of any bids you name.
    Plan(plan::PlanArgs),
    /// Rank instructors by how much harder their sections were to get than the other sections of the same course.
    Instructors(rank::InstructorsArgs),
    /// Courses with the highest median clearing price in one term. Courses need two filled sections by default (--min-filled).
    Hot(rank::HotArgs),
    /// Check how well the bid model's predictions held up on past terms.
    Backtest(rank::BacktestArgs),
    /// Print the usage guide for LLMs and scripts (Markdown).
    Guide,
}

#[derive(Debug, Args)]
pub struct GuiArgs {
    /// Course to select when the app opens, e.g. IS216.
    #[arg(long, value_name = "CODE")]
    pub course: Option<String>,
    /// Tab to open first: overview (the default), planner, instructors or saved.
    #[arg(long, value_name = "TAB", value_parser = PossibleValuesParser::new(["overview", "planner", "instructors", "saved"]))]
    pub tab: Option<String>,
}

impl Cli {
    /// Parses the process arguments. On `Err`, the exit code to return instead: 0 for `--help` and
    /// `--version`, 2 for invalid arguments. With `--json`, invalid arguments are reported as JSON on
    /// stdout, like every other error.
    pub fn parse_args() -> Result<Cli, ExitCode> {
        match Cli::try_parse() {
            Ok(cli) => Ok(cli),
            Err(err) => {
                let json = std::env::args_os().any(|arg| arg.to_str() == Some("--json"));
                if err.use_stderr() && json {
                    report_error(&CliError::usage(clap_message(&err.to_string())), true);
                    return Err(ExitCode::from(2));
                }
                let _ = err.print();
                Err(ExitCode::from(err.exit_code() as u8))
            }
        }
    }

    /// What this command line means for the app. `Ok(Some)` opens the app: no command (the legacy forms
    /// of it), or `gui`. `Ok(None)` runs a command or a legacy statistics flag. With no command, `--json`
    /// and `--exclude-session` only make sense to a command, so they are an `Err` rather than a window
    /// that opens and ignores them.
    pub fn gui_launch(&self) -> Result<Option<ui::Launch>, CliError> {
        let (course, tab) = match &self.command {
            Some(Command::Gui(gui)) => (
                gui.course.clone().or_else(|| self.course.clone()),
                gui.tab.clone().or_else(|| self.tab.clone()),
            ),
            Some(_) => return Ok(None),
            None if self.backtest || self.tune => return Ok(None),
            None if self.json || !self.exclude_session.is_empty() => return Err(self.needs_a_command()),
            None => (self.course.clone(), self.tab.clone()),
        };
        Ok(Some(ui::Launch {
            course: course.map(|c| c.to_ascii_uppercase()),
            tab,
        }))
    }

    /// The usage error for --json or --exclude-session given with no command. It names the commands that
    /// print a result, so the caller can add one.
    fn needs_a_command(&self) -> CliError {
        let mut given = Vec::new();
        if self.json {
            given.push("--json");
        }
        if !self.exclude_session.is_empty() {
            given.push("--exclude-session");
        }
        let verb = if given.len() == 1 { "needs" } else { "need" };
        CliError::usage(format!(
            "{} {verb} a command, because with no command the desktop app opens. Add one, e.g. `smu_bid_analyser --json info`. Commands: {}.",
            given.join(" and "),
            command_names().join(", ")
        ))
    }

    /// The data folder named on the command line: `--data-dir`, or the older positional PATH.
    pub fn data_dir_arg(&self) -> Option<PathBuf> {
        self.data_dir.clone().or_else(|| self.folder.clone())
    }
}

/// What went wrong. It decides the exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// A bad or missing argument (exit 2).
    Usage,
    /// A course, instructor, session, school or term that the data does not have (exit 3).
    NotFound,
    /// The data folder could not be read (exit 1).
    Data,
}

/// A failed command. `suggestions` are close alternatives ("did you mean").
#[derive(Debug, Clone, Serialize)]
pub struct CliError {
    pub kind: ErrorKind,
    pub message: String,
    pub suggestions: Vec<String>,
}

impl CliError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Usage,
            message: message.into(),
            suggestions: Vec::new(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::NotFound,
            message: message.into(),
            suggestions: Vec::new(),
        }
    }

    pub fn data(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Data,
            message: message.into(),
            suggestions: Vec::new(),
        }
    }

    pub fn with_suggestions(mut self, suggestions: Vec<String>) -> Self {
        self.suggestions = suggestions;
        self
    }

    pub fn exit_code(&self) -> u8 {
        match self.kind {
            ErrorKind::Data => 1,
            ErrorKind::Usage => 2,
            ErrorKind::NotFound => 3,
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

/// Everything a command needs: the loaded data, the global filters, the model settings and the output mode.
pub struct Ctx {
    pub data: AppData,
    pub filters: GlobalFilters,
    /// The model settings the forecasts use.
    pub params: ModelParams,
    pub json: bool,
}

impl Ctx {
    /// A course by code, ignoring case and spaces ("is 216" is IS216). A miss lists close matches.
    pub fn course(&self, input: &str) -> Result<&CourseInfo, CliError> {
        let wanted = input.trim();
        if wanted.is_empty() {
            return Err(CliError::usage("a course code is required, e.g. IS216"));
        }
        resolve_course(&self.data, wanted).map_err(|suggestions| {
            CliError::not_found(format!("no course with code \"{wanted}\"")).with_suggestions(suggestions)
        })
    }

    /// An instructor who taught `course`: an exact name, or a part of the name that matches only one
    /// instructor. Returns the name as the data spells it.
    pub fn instructor(&self, course: &str, input: &str) -> Result<String, CliError> {
        let code = self.course(course)?.code.to_string();
        let wanted = input.split_whitespace().collect::<Vec<_>>().join(" ");
        if wanted.is_empty() {
            return Err(CliError::usage("an instructor name is required"));
        }
        resolve_instructor(&self.data, &code, &wanted).map_err(|names| {
            let needle = wanted.to_lowercase();
            if names.iter().any(|name| name.to_lowercase().contains(&needle)) {
                CliError::not_found(format!(
                    "\"{wanted}\" matches more than one instructor of {code}; use more of the name"
                ))
                .with_suggestions(names)
            } else {
                CliError::not_found(format!("no instructor named \"{wanted}\" has taught {code}"))
                    .with_suggestions(names)
            }
        })
    }

    /// A school as the data spells it, matched without regard to case.
    pub fn school(&self, input: &str) -> Result<String, CliError> {
        let wanted = input.trim();
        match self
            .data
            .schools
            .iter()
            .find(|s| s.to_lowercase() == wanted.to_lowercase())
        {
            Some(school) => Ok(school.to_string()),
            None => {
                let all = self.data.schools.iter().map(|s| s.to_string()).collect();
                Err(CliError::not_found(format!("no school named \"{wanted}\"")).with_suggestions(all))
            }
        }
    }

    /// A term that has results in the data. Use `parse_term` for terms that may lie in the future.
    pub fn known_term(&self, input: &str) -> Result<Term, CliError> {
        let term = parse_term(input)?;
        if self.data.terms.contains(&term) {
            return Ok(term);
        }
        let known = self.data.terms.iter().map(Term::label).collect();
        Err(CliError::not_found(format!("no results for {} in the data", term.label())).with_suggestions(known))
    }

    /// Prints `value` as JSON when `--json` is set. Otherwise runs `text`, which prints the human view.
    pub fn emit<T: Serialize>(&self, value: &T, text: impl FnOnce()) -> Result<(), CliError> {
        if self.json {
            output::print_json(value)
        } else {
            text();
            Ok(())
        }
    }

    /// Prints the warnings of a text view on stderr, one "warning:" line each. JSON mode prints none,
    /// because the document carries them in its `warnings` array.
    pub fn warn_text(&self, warnings: &[String]) {
        if self.json {
            return;
        }
        for warning in warnings {
            eprintln!("warning: {}", output::ascii(warning));
        }
    }

    /// A warning for each data file that could not be read. The other files are still used, so every
    /// data command reports these.
    pub fn data_warnings(&self) -> Vec<String> {
        self.data
            .report
            .files
            .iter()
            .filter_map(|file| {
                let error = file.error.as_ref()?;
                let reason = error.split_whitespace().collect::<Vec<_>>().join(" ");
                Some(format!("{} could not be read ({reason}), so its rows are left out.", file.name))
            })
            .collect()
    }
}

/// A round and window: "R1W1", "R1AW2", "1:1", "Round 1 Window 1", "R2" (every window of round 2) or "all".
pub fn parse_window(input: &str) -> Result<WindowSel, CliError> {
    WindowSel::parse(input).ok_or_else(|| {
        CliError::usage(format!(
            "invalid window \"{}\"; use one of R1W1, R1AW2, 1:1, \"Round 1 Window 1\", R2 (all windows), all",
            input.trim()
        ))
    })
}

/// Parses a count of at least one, such as `--limit`.
pub fn count_at_least_one(text: &str) -> Result<usize, String> {
    match text.trim().parse::<usize>() {
        Ok(n) if n >= 1 => Ok(n),
        _ => Err(format!("\"{text}\" must be a whole number of at least 1")),
    }
}

/// A term such as "2026-27 T2". Any term is accepted, whether or not the data has it.
pub fn parse_term(input: &str) -> Result<Term, CliError> {
    Term::parse(input).ok_or_else(|| {
        CliError::usage(format!(
            "invalid term \"{}\"; use a form such as 2026-27 T2 (T1, T2, T3A or T3B)",
            input.trim()
        ))
    })
}

/// The first paragraph of a clap error, without its "error: " prefix, on one line. clap puts the
/// argument a message is about on the lines under its first line, so those lines stay in.
fn clap_message(text: &str) -> String {
    let paragraph: Vec<&str> = text
        .lines()
        .take_while(|line| !line.trim().is_empty())
        .map(str::trim)
        .collect();
    paragraph.join(" ").trim_start_matches("error: ").to_string()
}

/// Runs a command line that does not open the app. `main` opens the app first when it should.
pub fn run(cli: Cli) -> ExitCode {
    match execute(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => fail(&err, cli.json),
    }
}

/// Reports an error the way `--json` asks for, and returns the exit code for it. Used for errors found
/// before any command runs, as well as by `run`.
pub fn fail(err: &CliError, json: bool) -> ExitCode {
    report_error(err, json);
    ExitCode::from(err.exit_code())
}

fn execute(cli: &Cli) -> Result<(), CliError> {
    let command = match &cli.command {
        Some(command) => command,
        // Only the legacy flags reach here without a command; anything else opens the app.
        None => return legacy_run(cli),
    };
    if cli.backtest || cli.tune {
        return Err(CliError::usage(
            "--backtest and --tune cannot be combined with a command",
        ));
    }
    match command {
        Command::Gui(_) => Err(CliError::usage("the app is opened by main, not by the command runner")),
        Command::Guide => print_guide(cli.json),
        _ => {
            let ctx = load_ctx(cli)?;
            run_command(&ctx, command)
        }
    }
}

/// The legacy flags print text only, so with --json they are refused instead of printing text on stdout.
fn legacy_run(cli: &Cli) -> Result<(), CliError> {
    if cli.json {
        let message = if cli.tune {
            "--tune prints a progress table and has no JSON form; run it without --json"
        } else {
            "--backtest prints a text report; use `backtest --json` for JSON"
        };
        return Err(CliError::usage(message));
    }
    legacy::run(cli)
}

fn run_command(ctx: &Ctx, command: &Command) -> Result<(), CliError> {
    match command {
        Command::Info => info::run(ctx),
        Command::Search(args) => search::run(ctx, args),
        Command::Course(args) => course::run(ctx, args),
        Command::Plan(args) => plan::run(ctx, args),
        Command::Instructors(args) => rank::run_instructors(ctx, args),
        Command::Hot(args) => rank::run_hot(ctx, args),
        Command::Backtest(args) => rank::run_backtest(ctx, args),
        Command::Gui(_) | Command::Guide => Err(CliError::usage("internal error: this command does not load data")),
    }
}

/// Loads the data folder and builds the context every data command shares.
fn load_ctx(cli: &Cli) -> Result<Ctx, CliError> {
    let config = Config::load();
    let folder = cli.data_dir_arg();
    let dir = config::resolve_data_dir(folder.as_deref(), &config);
    if !cli.json {
        eprintln!("Loading {} ...", output::ascii(&dir.display().to_string()));
    }
    let data = data::loader::load_all_data(&dir).map_err(CliError::data)?;
    let filters = session_filters(&data, &cli.exclude_session)?;
    Ok(Ctx {
        data,
        filters,
        params: ModelParams::default(),
        json: cli.json,
    })
}

/// Turns --exclude-session names into the canonical session names the data uses.
fn session_filters(data: &AppData, names: &[String]) -> Result<GlobalFilters, CliError> {
    let mut filters = GlobalFilters::default();
    for name in names {
        let wanted = name.trim();
        if wanted.is_empty() {
            return Err(CliError::usage("--exclude-session needs a session name"));
        }
        match data.sessions.iter().find(|s| s.to_lowercase() == wanted.to_lowercase()) {
            Some(canonical) => {
                filters.excluded_sessions.insert(canonical.to_string());
            }
            None => {
                let valid = data.sessions.iter().map(|s| s.to_string()).collect();
                return Err(CliError::not_found(format!("no session named \"{wanted}\"")).with_suggestions(valid));
            }
        }
    }
    Ok(filters)
}

const GUIDE: &str = include_str!("../../LLM_GUIDE.md");

#[derive(Serialize)]
struct GuideOut {
    guide: &'static str,
}

fn print_guide(json: bool) -> Result<(), CliError> {
    if json {
        return output::print_json(&GuideOut { guide: GUIDE });
    }
    print!("{GUIDE}");
    Ok(())
}

#[derive(Serialize)]
struct ErrorEnvelope<'a> {
    error: &'a CliError,
}

/// The commands that print a result, in the order `--help` lists them. `gui` is left out because it opens
/// the app, and `help` is clap's own.
fn command_names() -> Vec<String> {
    Cli::command()
        .get_subcommands()
        .map(|command| command.get_name().to_string())
        .filter(|name| name != "gui" && name != "help")
        .collect()
}

/// Prints an error the way the mode asks for: a JSON object on stdout, or text on stderr.
fn report_error(err: &CliError, json: bool) {
    if json {
        // Nothing else reaches stdout at this point, so a failed write has nowhere to be reported.
        let _ = output::print_json(&ErrorEnvelope { error: err });
        return;
    }
    eprintln!("error: {}", output::ascii(&err.message));
    if !err.suggestions.is_empty() {
        let list: Vec<String> = err.suggestions.iter().map(|s| output::ascii(s)).collect();
        eprintln!("did you mean: {}", list.join(", "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{BidRow, FileReport, LoadReport, RoundKind, TermKind};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("smu_bid_analyser").chain(args.iter().copied()))
            .unwrap_or_else(|err| panic!("{err}"))
    }

    fn parse_error(args: &[&str]) -> clap::Error {
        Cli::try_parse_from(std::iter::once("smu_bid_analyser").chain(args.iter().copied()))
            .expect_err("the arguments should be rejected")
    }

    #[test]
    fn command_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn no_arguments_open_the_app() {
        let cli = parse(&[]);
        assert!(cli.command.is_none());
        let launch = cli.gui_launch().expect("no error").expect("the app should open");
        assert!(launch.course.is_none() && launch.tab.is_none());
        assert_eq!(cli.data_dir_arg(), None);
    }

    #[test]
    fn a_folder_path_opens_the_app_on_that_folder() {
        let cli = parse(&["./some/folder"]);
        assert!(cli.command.is_none());
        assert!(matches!(cli.gui_launch(), Ok(Some(_))));
        assert_eq!(cli.data_dir_arg(), Some(PathBuf::from("./some/folder")));
    }

    #[test]
    fn subcommand_names_are_commands_not_folders() {
        let cli = parse(&["search", "is2"]);
        assert!(matches!(cli.command, Some(Command::Search(_))));
        assert!(matches!(cli.gui_launch(), Ok(None)));
        assert_eq!(cli.data_dir_arg(), None);
    }

    #[test]
    fn search_joins_its_words_into_one_query() {
        let cli = parse(&["search", "data", "science", "--limit", "5"]);
        let Some(Command::Search(args)) = cli.command else {
            panic!("expected search")
        };
        assert_eq!(args.query.join(" "), "data science");
        assert_eq!(args.limit, 5);
    }

    #[test]
    fn json_flag_works_before_and_after_the_command() {
        assert!(parse(&["--json", "search", "x"]).json);
        assert!(parse(&["search", "x", "--json"]).json);
        assert!(!parse(&["search", "x"]).json);
    }

    #[test]
    fn plan_takes_repeated_bids_and_comma_separated_levels() {
        let cli = parse(&["plan", "is216", "--bid", "20", "--bid", "30", "--levels", "0.5,0.9"]);
        let Some(Command::Plan(args)) = cli.command else {
            panic!("expected plan")
        };
        assert_eq!(args.code, "is216");
        assert_eq!(args.bid, vec![20.0, 30.0]);
        assert_eq!(args.levels, vec![0.5, 0.9]);
    }

    #[test]
    fn plan_rejects_levels_that_are_not_chances() {
        assert!(Cli::try_parse_from(["smu_bid_analyser", "plan", "is216", "--levels", "80"]).is_err());
    }

    #[test]
    fn exclude_session_can_repeat() {
        let cli = parse(&["--exclude-session", "X", "--exclude-session", "Y", "info"]);
        assert_eq!(cli.exclude_session, vec!["X", "Y"]);
        assert!(matches!(cli.command, Some(Command::Info)));
    }

    #[test]
    fn legacy_backtest_and_tune_flags_run_without_a_command() {
        let cli = parse(&["--backtest"]);
        assert!(cli.backtest && cli.command.is_none());
        assert!(matches!(cli.gui_launch(), Ok(None)));
        assert!(parse(&["--tune"]).tune);
    }

    #[test]
    fn legacy_course_and_tab_still_open_the_app() {
        let launch = parse(&["--course", "is216", "--tab", "planner"])
            .gui_launch()
            .expect("no error")
            .expect("the app should open");
        assert_eq!(launch.course.as_deref(), Some("IS216"));
        assert_eq!(launch.tab.as_deref(), Some("planner"));
    }

    #[test]
    fn gui_subcommand_takes_course_and_tab() {
        let launch = parse(&["gui", "--course", "is216", "--tab", "instructors"])
            .gui_launch()
            .expect("no error")
            .expect("the app should open");
        assert_eq!(launch.course.as_deref(), Some("IS216"));
        assert_eq!(launch.tab.as_deref(), Some("instructors"));
        let err = parse_error(&["gui", "--tab", "nowhere"]);
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
    }

    #[test]
    fn options_that_only_a_command_uses_refuse_to_open_the_app() {
        // With no command each of these would open the window, which a script cannot close.
        let refused: [&[&str]; 6] = [
            &["--json"],
            &["--json", "./some/folder"],
            &["--json", "--data-dir", "C:/data"],
            &["--json", "--course", "is216"],
            &["--exclude-session", "1A Session"],
            &["--exclude-session", "1A Session", "./some/folder"],
        ];
        for args in refused {
            let err = parse(args)
                .gui_launch()
                .err()
                .unwrap_or_else(|| panic!("{args:?} should not open the app"));
            assert_eq!((err.kind, err.exit_code()), (ErrorKind::Usage, 2), "{args:?}");
            for command in ["info", "search", "course", "plan", "instructors", "hot", "backtest", "guide"] {
                assert!(err.message.contains(command), "{args:?} should name {command}: {}", err.message);
            }
            let listed = err.message.rsplit("Commands: ").next().unwrap_or_default().trim_end_matches('.');
            assert!(!listed.split(", ").any(|name| name == "gui"), "gui opens the app, so it is not listed: {}", err.message);
        }
        let json = parse(&["--json"]).gui_launch().err().expect("refused");
        assert!(json.message.starts_with("--json needs a command"), "{}", json.message);
        assert!(json.message.contains("smu_bid_analyser --json info"), "{}", json.message);
        let both = parse(&["--json", "--exclude-session", "1A Session"])
            .gui_launch()
            .err()
            .expect("refused");
        assert!(
            both.message.starts_with("--json and --exclude-session need a command"),
            "{}",
            both.message
        );
    }

    #[test]
    fn the_legacy_forms_still_open_the_app_and_commands_still_run() {
        assert!(matches!(parse(&["--data-dir", "C:/data"]).gui_launch(), Ok(Some(_))));
        assert!(matches!(parse(&["--course", "is216", "--tab", "planner"]).gui_launch(), Ok(Some(_))));
        assert!(matches!(parse(&["--json", "info"]).gui_launch(), Ok(None)));
        // `gui` names the app itself, so it opens whatever else is given.
        assert!(matches!(parse(&["--json", "gui"]).gui_launch(), Ok(Some(_))));
        // The legacy statistics flags run from the command runner, with or without --json.
        assert!(matches!(parse(&["--backtest"]).gui_launch(), Ok(None)));
        assert!(matches!(parse(&["--tune"]).gui_launch(), Ok(None)));
        assert!(matches!(parse(&["--json", "--backtest"]).gui_launch(), Ok(None)));
        assert!(matches!(parse(&["--json", "--tune"]).gui_launch(), Ok(None)));
    }

    #[test]
    fn plan_target_help_names_the_term_the_code_defaults_to() {
        let command = Cli::command();
        let plan = command.find_subcommand("plan").expect("plan is a command");
        let target = plan
            .get_arguments()
            .find(|arg| arg.get_long() == Some("target"))
            .expect("plan has --target");
        let help = target.get_help().expect("--target has help").to_string();
        assert!(help.contains("regular (non-summer) term"), "{help}");
        assert!(!help.contains("the term after the latest one"), "{help}");
    }

    #[test]
    fn data_dir_flag_is_global_and_beats_the_folder_argument() {
        let cli = parse(&["info", "--data-dir", "C:/data"]);
        assert_eq!(cli.data_dir_arg(), Some(PathBuf::from("C:/data")));
        assert!(matches!(cli.command, Some(Command::Info)));
        assert_eq!(
            parse(&["--data-dir", "A", "B"]).data_dir_arg(),
            Some(PathBuf::from("A"))
        );
    }

    #[test]
    fn help_hides_the_legacy_flags_and_lists_every_command() {
        let help = Cli::command().render_long_help().to_string();
        // Only option lines count: the examples legitimately mention `instructors --course`.
        for hidden in ["--backtest", "--tune", "--course", "--tab"] {
            assert!(
                !help.lines().any(|line| line.trim_start().starts_with(hidden)),
                "{hidden} should be hidden"
            );
        }
        for shown in [
            "gui",
            "info",
            "search",
            "course",
            "plan",
            "instructors",
            "hot",
            "backtest",
            "guide",
        ] {
            assert!(help.contains(shown), "{shown} should be listed");
        }
    }

    #[test]
    fn errors_have_exit_codes_and_the_documented_json_shape() {
        assert_eq!(CliError::usage("x").exit_code(), 2);
        assert_eq!(CliError::not_found("x").exit_code(), 3);
        assert_eq!(CliError::data("x").exit_code(), 1);
        let err = CliError::not_found("no course with code \"XYZ\"").with_suggestions(vec!["IS111".to_string()]);
        let value = serde_json::to_value(ErrorEnvelope { error: &err }).expect("errors serialise");
        assert_eq!(
            value,
            serde_json::json!({"error": {"kind": "not_found", "message": "no course with code \"XYZ\"", "suggestions": ["IS111"]}})
        );
    }

    #[test]
    fn window_and_term_arguments_parse_or_explain_the_formats() {
        assert_eq!(parse_window("R1W1").expect("valid"), WindowSel::ROUND1_W1);
        assert_eq!(parse_window("all").expect("valid"), WindowSel::default());
        let err = parse_window("R9W9").expect_err("invalid");
        assert_eq!(err.kind, ErrorKind::Usage);
        assert!(err.message.contains("R1AW2"), "{}", err.message);
        assert_eq!(
            parse_term("2026-27 T2").expect("valid"),
            Term {
                year: 2026,
                kind: TermKind::T2
            }
        );
        let err = parse_term("next year").expect_err("invalid");
        assert_eq!(err.kind, ErrorKind::Usage);
        assert!(err.message.contains("2026-27 T2"), "{}", err.message);
    }

    fn bid(
        year: u16,
        kind: TermKind,
        course: &str,
        section: &str,
        instructor: &str,
        school: &str,
        session: &str,
    ) -> BidRow {
        BidRow {
            term: Term { year, kind },
            session: Arc::from(session),
            round: RoundKind::Round1,
            window: 1,
            course_code: Arc::from(course),
            section: Arc::from(section),
            instructor: Arc::from(instructor),
            school: Arc::from(school),
            vacancy: 40,
            opening_vacancy: 10,
            before_proc: 10,
            dice: 0,
            after_proc: 0,
            enrolled: 10,
            median_bid: 20.0,
            min_bid: 15.0,
        }
    }

    fn sample_ctx() -> Ctx {
        let mut rows = Vec::new();
        for kind in [TermKind::T1, TermKind::T2] {
            rows.push(bid(
                2025,
                kind,
                "IS111",
                "G1",
                "Alice Tan",
                "SCIS",
                "Regular Academic Session",
            ));
            rows.push(bid(
                2025,
                kind,
                "IS111",
                "G2",
                "Bob Tan",
                "SCIS",
                "Regular Academic Session",
            ));
            rows.push(bid(2025, kind, "IS112", "G1", "Dan Ng", "SOSS", "1A Session"));
        }
        let descriptions = HashMap::from([
            (
                Arc::from("IS111"),
                (
                    Term {
                        year: 2025,
                        kind: TermKind::T2,
                    },
                    "Business Intelligence".to_string(),
                ),
            ),
            (
                Arc::from("IS112"),
                (
                    Term {
                        year: 2025,
                        kind: TermKind::T2,
                    },
                    "Data Mining".to_string(),
                ),
            ),
        ]);
        Ctx {
            data: AppData::new(rows, descriptions, LoadReport::default()),
            filters: GlobalFilters::default(),
            params: ModelParams::default(),
            json: false,
        }
    }

    #[test]
    fn course_lookup_is_forgiving_and_suggests_on_a_miss() {
        let ctx = sample_ctx();
        assert_eq!(ctx.course("is 111").expect("found").code.as_ref(), "IS111");
        let err = ctx.course("IS11").expect_err("not an exact code");
        assert_eq!(err.kind, ErrorKind::NotFound);
        assert!(err.suggestions.contains(&"IS111".to_string()), "{:?}", err.suggestions);
        assert_eq!(ctx.course("  ").expect_err("empty").kind, ErrorKind::Usage);
    }

    #[test]
    fn instructor_lookup_reports_ambiguity_and_unknown_names() {
        let ctx = sample_ctx();
        assert_eq!(ctx.instructor("IS111", "alice").expect("found"), "Alice Tan");
        let ambiguous = ctx.instructor("IS111", "tan").expect_err("two Tans");
        assert_eq!(ambiguous.kind, ErrorKind::NotFound);
        assert!(ambiguous.message.contains("more than one"), "{}", ambiguous.message);
        assert!(ambiguous.suggestions.contains(&"Bob Tan".to_string()));
        let unknown = ctx.instructor("IS111", "zed").expect_err("no Zed");
        assert!(
            unknown.message.starts_with("no instructor named"),
            "{}",
            unknown.message
        );
    }

    #[test]
    fn sessions_and_schools_match_without_regard_to_case() {
        let ctx = sample_ctx();
        let filters = session_filters(&ctx.data, &["1a session".to_string()]).expect("found");
        assert!(filters.excluded_sessions.contains("1A Session"));
        let err = session_filters(&ctx.data, &["nonsense".to_string()]).expect_err("unknown");
        assert_eq!(err.kind, ErrorKind::NotFound);
        assert!(err.suggestions.contains(&"1A Session".to_string()));
        assert_eq!(ctx.school("scis").expect("found"), "SCIS");
        assert_eq!(ctx.school("NOPE").expect_err("unknown").kind, ErrorKind::NotFound);
    }

    #[test]
    fn known_term_must_have_data() {
        let ctx = sample_ctx();
        assert_eq!(ctx.known_term("2025-26 T2").expect("in the data").label(), "2025-26 T2");
        let err = ctx.known_term("2030-31 T1").expect_err("not in the data");
        assert_eq!(err.kind, ErrorKind::NotFound);
        assert_eq!(
            err.suggestions,
            vec!["2025-26 T1".to_string(), "2025-26 T2".to_string()]
        );
    }

    #[test]
    fn a_usage_error_in_json_keeps_the_argument_it_is_about() {
        let err = Cli::try_parse_from(["smu_bid_analyser", "--json", "plan"]).expect_err("the code is missing");
        assert_eq!(
            clap_message(&err.to_string()),
            "the following required arguments were not provided: <CODE>"
        );
        assert_eq!(clap_message("error: bad\n\nUsage: x"), "bad");
    }

    #[test]
    fn guide_and_help_do_not_need_the_data_folder() {
        // The folder does not exist, so any command that loads data fails with exit 1.
        let missing = "/nonexistent/smu_bid_analyser_no_such_folder";
        assert!(execute(&parse(&["--data-dir", missing, "guide"])).is_ok());
        assert!(execute(&parse(&["--json", "--data-dir", missing, "guide"])).is_ok());
        let info = execute(&parse(&["--data-dir", missing, "info"])).expect_err("needs the data");
        assert_eq!(info.exit_code(), 1);
        let help = Cli::try_parse_from(["smu_bid_analyser", "help"]).expect_err("help is not a command");
        assert_eq!(help.kind(), clap::error::ErrorKind::DisplayHelp);
        let version = Cli::try_parse_from(["smu_bid_analyser", "--version"]).expect_err("version");
        assert_eq!(version.kind(), clap::error::ErrorKind::DisplayVersion);
    }

    #[test]
    fn the_guide_is_ascii_because_text_output_is_ascii() {
        assert!(GUIDE.is_ascii(), "LLM_GUIDE.md has non-ASCII characters; text output must be ASCII");
    }

    #[test]
    fn the_legacy_flags_are_refused_with_json_before_any_data_loads() {
        let backtest = execute(&parse(&["--json", "--backtest"])).expect_err("text only");
        assert_eq!(backtest.kind, ErrorKind::Usage);
        assert!(backtest.message.contains("backtest --json"), "{}", backtest.message);
        let tune = execute(&parse(&["--json", "--tune"])).expect_err("text only");
        assert_eq!(tune.kind, ErrorKind::Usage);
        assert!(tune.message.contains("--tune"), "{}", tune.message);
    }

    #[test]
    fn a_data_file_that_could_not_be_read_is_a_warning() {
        let file = |name: &str, error: Option<&str>| FileReport {
            name: name.to_string(),
            term: None,
            rows_kept: 0,
            rows_special_rounds: 0,
            rows_invalid: 0,
            duplicates: 0,
            error: error.map(str::to_string),
        };
        let report = LoadReport {
            files: vec![
                file("2025-26_T1.xlsx", None),
                file("2025-26_T2.xlsx", Some("not a workbook:\n  bad zip")),
            ],
            ..LoadReport::default()
        };
        let ctx = Ctx {
            data: AppData::new(Vec::new(), HashMap::new(), report),
            filters: GlobalFilters::default(),
            params: ModelParams::default(),
            json: false,
        };
        assert_eq!(
            ctx.data_warnings(),
            vec!["2025-26_T2.xlsx could not be read (not a workbook: bad zip), so its rows are left out.".to_string()]
        );
        assert!(sample_ctx().data_warnings().is_empty());
    }
}
