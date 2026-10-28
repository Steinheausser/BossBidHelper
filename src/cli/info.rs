//! `info`: what the data folder holds and how it loaded.

use super::output::{ascii, Table};
use super::{CliError, Ctx};
use crate::data::query::{self, DataSummary};
use serde::Serialize;

#[derive(Serialize)]
struct InfoOut<'a> {
    #[serde(flatten)]
    summary: &'a DataSummary,
    /// Sessions left out with --exclude-session.
    excluded_sessions: Vec<&'a str>,
    /// Data files that could not be read. The other files are still used.
    warnings: Vec<String>,
}

pub fn run(ctx: &Ctx) -> Result<(), CliError> {
    let summary = query::data_summary(&ctx.data);
    let excluded: Vec<&str> = ctx.filters.excluded_sessions.iter().map(String::as_str).collect();
    let warnings = ctx.data_warnings();
    ctx.warn_text(&warnings);
    let out = InfoOut {
        summary: &summary,
        excluded_sessions: excluded.clone(),
        warnings,
    };
    ctx.emit(&out, || print_text(&summary, &excluded))
}

fn list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(", ")
    }
}

fn print_text(s: &DataSummary, excluded: &[&str]) {
    let with_errors = s.files.iter().filter(|f| f.error.is_some()).count();
    let terms = match (s.terms.first(), s.terms.last()) {
        (Some(first), Some(last)) => format!("{first} to {last} ({} terms)", s.terms.len()),
        _ => "none".to_string(),
    };
    let excluded = if excluded.is_empty() {
        "none".to_string()
    } else {
        excluded.join(", ")
    };
    let lines = [
        ("Data folder", s.data_dir.clone()),
        ("Files", format!("{} ({with_errors} with errors)", s.files.len())),
        ("Load time", format!("{} ms", s.load_millis)),
        ("Rows", s.total_rows.to_string()),
        ("Courses", s.courses.to_string()),
        ("Terms", terms),
        (
            "Latest term",
            s.latest_term.clone().unwrap_or_else(|| "none".to_string()),
        ),
        (
            "Bidding for",
            format!("{} (default target term)", s.default_target_term),
        ),
        ("Sessions", list_or_none(&s.sessions)),
        ("Schools", list_or_none(&s.schools)),
        ("Excluded", excluded),
    ];
    for (label, value) in lines {
        println!("{:<14}{}", format!("{label}:"), ascii(&value));
    }

    println!();
    let mut table = Table::new(&["FILE", "TERM", "KEPT", "SPECIAL RND", "INVALID", "DUPES", "ERROR"])
        .right_align(&[2, 3, 4, 5])
        .max_width(0, 40)
        .max_width(6, 60);
    for f in &s.files {
        table.push(vec![
            f.name.clone(),
            f.term.clone().unwrap_or_else(|| "-".to_string()),
            f.rows_kept.to_string(),
            f.rows_special_rounds.to_string(),
            f.rows_invalid.to_string(),
            f.duplicates.to_string(),
            f.error.clone().unwrap_or_else(|| "-".to_string()),
        ]);
    }
    println!("{}", table.render());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{AppData, LoadReport};
    use std::collections::HashMap;

    #[test]
    fn json_has_the_warnings_array_even_when_there_is_nothing_to_warn_about() {
        let data = AppData::new(Vec::new(), HashMap::new(), LoadReport::default());
        let summary = query::data_summary(&data);
        let out = InfoOut {
            summary: &summary,
            excluded_sessions: Vec::new(),
            warnings: Vec::new(),
        };
        let value = serde_json::to_value(&out).expect("serialises");
        assert_eq!(value["warnings"], serde_json::json!([]));
        assert!(value.get("files").is_some() && value.get("excluded_sessions").is_some(), "{value}");
    }
}
