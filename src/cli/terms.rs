//! `terms`: how big each term was across every course, from `query::term_overview`.

use super::output::{opt_money, pct, Table};
use super::{parse_window, CliError, Ctx};
use crate::data::query;
use crate::data::stats::TermOverview;
use clap::Args;
use serde::Serialize;

/// The line under the table that says what the seat columns count.
const SEATS_KEY: &str = "\
CLASS SEATS = class sizes added up. 1ST WINDOW = seats open in each section's first bidding window.
WON = seats taken by bids over every window. UNCLAIMED = seats still open after each section's last window.
FILL% = share of section-windows with seats open that filled. The BOSS export has no class days or times.";

#[derive(Debug, Args)]
pub struct TermsArgs {
    /// Round and window, e.g. R1W1, R1AW2 or 1:1. "R1" means every window of that round. The default, "all", counts every round and window.
    #[arg(long, default_value = "all", value_name = "W")]
    pub window: String,
    /// Only this school's sections, e.g. SCIS. Case-insensitive.
    #[arg(long, value_name = "SCHOOL")]
    pub school: Option<String>,
}

#[derive(Serialize)]
struct TermsOut {
    window: String,
    window_code: String,
    /// The school asked for with --school, or null for every school.
    school: Option<String>,
    /// Sessions left out with --exclude-session.
    excluded_sessions: Vec<String>,
    /// Oldest first. Empty when nothing matches.
    terms: Vec<TermOverview>,
    warnings: Vec<String>,
}

pub fn run(ctx: &Ctx, args: &TermsArgs) -> Result<(), CliError> {
    let sel = parse_window(&args.window)?;
    let school = args.school.as_deref().map(|s| ctx.school(s)).transpose()?;
    let terms = query::term_overview(&ctx.data, &ctx.filters, sel, school.as_deref());
    let warnings = ctx.data_warnings();
    ctx.warn_text(&warnings);
    let out = TermsOut {
        window: sel.label(),
        window_code: sel.short_label(),
        school,
        excluded_sessions: ctx.filters.excluded_sessions.iter().cloned().collect(),
        terms,
        warnings,
    };
    ctx.emit(&out, || print_text(&out))
}

fn print_text(out: &TermsOut) {
    let scope = match &out.school {
        Some(school) => format!("{school}, {}", out.window),
        None => format!("All schools, {}", out.window),
    };
    if out.terms.is_empty() {
        println!("No sections in {scope}.");
        return;
    }
    println!("{scope}\n");
    let mut table = Table::new(&[
        "TERM",
        "COURSES",
        "SECTIONS",
        "INSTRUCTORS",
        "WINDOWS",
        "CLASS SEATS",
        "1ST WINDOW",
        "WON",
        "UNCLAIMED",
        "FILL%",
        "CLEARING MEDIAN",
    ])
    .right_align(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    for t in &out.terms {
        table.push(vec![
            t.term.label(),
            t.courses.to_string(),
            t.sections.to_string(),
            t.instructors.to_string(),
            t.windows.to_string(),
            t.class_seats.to_string(),
            t.first_window_seats.to_string(),
            t.seats_won.to_string(),
            t.seats_unclaimed.to_string(),
            pct(t.fill_rate),
            opt_money(t.clearing_median),
        ]);
    }
    println!("{}\n", table.render());
    println!("{SEATS_KEY}");
}
