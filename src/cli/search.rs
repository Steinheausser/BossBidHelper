//! `search`: find courses by code or title.

use super::output::Table;
use super::{count_at_least_one, CliError, Ctx};
use crate::data::query::{self, CourseHit};
use clap::Args;
use serde::Serialize;

/// The most courses `query::search_courses` returns. A total of this size may be cut short.
const SEARCH_CAP: usize = 200;

#[derive(Debug, Args)]
pub struct SearchArgs {
    /// Words to look for in a course code or title. Matching is fuzzy: "data sci" finds "Data Science".
    /// Leave it out to list courses.
    pub query: Vec<String>,
    /// Only courses offered by this school, e.g. SIS. Case-insensitive. `info` lists the schools.
    #[arg(long, value_name = "SCHOOL")]
    pub school: Option<String>,
    /// Show at most this many courses.
    #[arg(long, default_value_t = 20, value_name = "N", value_parser = count_at_least_one)]
    pub limit: usize,
}

#[derive(Serialize)]
struct SearchOut<'a> {
    query: &'a str,
    school: Option<&'a str>,
    /// The most courses to list, as asked with --limit.
    limit: usize,
    /// How many courses match. A fuzzy query stops counting at 200.
    total_matches: usize,
    /// How many are in `results`.
    shown: usize,
    /// Things to know before trusting the numbers, such as a total that is only a lower bound.
    warnings: Vec<String>,
    results: Vec<CourseHit>,
}

pub fn run(ctx: &Ctx, args: &SearchArgs) -> Result<(), CliError> {
    let needle = args.query.join(" ");
    let school = args.school.as_deref().map(|name| ctx.school(name)).transpose()?;
    let school_filter = school.as_deref();
    let results = query::course_hits(&ctx.data, &needle, school_filter, args.limit);
    let total_matches = query::search_courses(&ctx.data, &needle, school_filter).len();
    let mut warnings = ctx.data_warnings();
    warnings.extend(cap_warning(total_matches, !needle.trim().is_empty()));
    let out = SearchOut {
        query: &needle,
        school: school_filter,
        limit: args.limit,
        total_matches,
        shown: results.len(),
        warnings,
        results,
    };
    ctx.warn_text(&out.warnings);
    ctx.emit(&out, || print_text(&out))
}

/// A fuzzy query keeps at most `SEARCH_CAP` matches, so a total that reaches the cap is only a lower bound.
fn cap_warning(total_matches: usize, fuzzy: bool) -> Option<String> {
    (fuzzy && total_matches >= SEARCH_CAP).then(|| {
        format!(
            "The query matches {SEARCH_CAP} or more courses, and the total is capped at {SEARCH_CAP}. Use a longer query to narrow it."
        )
    })
}

fn or_dash(text: &str) -> String {
    if text.trim().is_empty() {
        "-".to_string()
    } else {
        text.to_string()
    }
}

fn print_text(out: &SearchOut) {
    if !out.results.is_empty() {
        let mut table = Table::new(&["CODE", "TITLE", "SCHOOL", "LAST OFFERED"]).max_width(1, 50);
        for hit in &out.results {
            table.push(vec![
                hit.code.clone(),
                or_dash(&hit.description),
                or_dash(&hit.school),
                hit.last_term.clone(),
            ]);
        }
        println!("{}\n", table.render());
    }
    // A fuzzy query keeps at most SEARCH_CAP matches; an empty query lists every course.
    let fuzzy = !out.query.trim().is_empty();
    let capped = if fuzzy && out.total_matches >= SEARCH_CAP {
        "+"
    } else {
        ""
    };
    let more = if out.shown < out.total_matches {
        " (use --limit to show more)"
    } else {
        ""
    };
    println!("{} of {}{capped} matches{more}", out.shown, out.total_matches);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_total_at_the_cap_is_a_warning_for_a_fuzzy_query_only() {
        assert!(cap_warning(SEARCH_CAP - 1, true).is_none());
        let warning = cap_warning(SEARCH_CAP, true).expect("the total is capped");
        assert!(warning.contains("200 or more"), "{warning}");
        assert!(cap_warning(SEARCH_CAP, false).is_none(), "an empty query lists every course");
    }

    #[test]
    fn json_echoes_the_limit_and_always_has_warnings() {
        let out = SearchOut {
            query: "data",
            school: None,
            limit: 20,
            total_matches: 0,
            shown: 0,
            warnings: Vec::new(),
            results: Vec::new(),
        };
        let value = serde_json::to_value(&out).expect("serialises");
        assert_eq!(value["limit"], 20);
        assert_eq!(value["warnings"], serde_json::json!([]));
    }
}
