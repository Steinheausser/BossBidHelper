//! Output helpers shared by the commands: JSON, plain text tables and the number formats the app uses.
//!
//! Text output is ASCII only, because Windows consoles often cannot show anything else.

use super::CliError;
use serde::Serialize;

/// Writes `value` to stdout as pretty-printed JSON, followed by a newline.
pub fn print_json<T: Serialize>(value: &T) -> Result<(), CliError> {
    let text =
        serde_json::to_string_pretty(value).map_err(|err| CliError::data(format!("could not write JSON: {err}")))?;
    println!("{text}");
    Ok(())
}

/// Makes text printable ASCII. Curly quotes and dashes become their plain forms, other characters
/// become "?", and control characters become spaces. Newlines are kept.
pub fn ascii(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            ' '..='~' | '\n' => c,
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201C}' | '\u{201D}' => '"',
            '\u{2013}' | '\u{2014}' | '\u{2212}' => '-',
            '\u{00A0}' => ' ',
            c if c.is_control() => ' ',
            _ => '?',
        })
        .collect()
}

/// Money as the app shows it: "e$23.36".
pub fn money(x: f64) -> String {
    format!("e${x:.2}")
}

/// Money, or "-" when there is no value.
pub fn opt_money(x: Option<f64>) -> String {
    x.map_or_else(|| "-".to_string(), money)
}

/// A fraction as a percentage with one decimal: 0.786 is "78.6%".
pub fn pct(x: f64) -> String {
    format!("{:.1}%", x * 100.0)
}

/// A plain text table: a header line, a rule, then one line per row. Columns are separated by two
/// spaces. Numeric columns can be right-aligned, and long cells are cut with "...".
pub struct Table {
    headers: Vec<String>,
    right: Vec<bool>,
    max_width: Vec<Option<usize>>,
    rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(headers: &[&str]) -> Self {
        Table {
            headers: headers.iter().map(|h| ascii(h)).collect(),
            right: vec![false; headers.len()],
            max_width: vec![None; headers.len()],
            rows: Vec::new(),
        }
    }

    /// Right-aligns these columns, for numbers.
    pub fn right_align(mut self, columns: &[usize]) -> Self {
        for &column in columns {
            if let Some(right) = self.right.get_mut(column) {
                *right = true;
            }
        }
        self
    }

    /// Cuts the cells of this column to `width` characters. A cut cell ends with "...".
    pub fn max_width(mut self, column: usize, width: usize) -> Self {
        if let Some(max) = self.max_width.get_mut(column) {
            *max = Some(width);
        }
        self
    }

    /// Adds a row. Missing cells are left blank and extra cells are dropped.
    pub fn push(&mut self, mut cells: Vec<String>) {
        cells.resize(self.headers.len(), String::new());
        self.rows.push(cells);
    }

    /// The table as text, without a trailing newline.
    pub fn render(&self) -> String {
        let cells: Vec<Vec<String>> = self
            .rows
            .iter()
            .map(|row| row.iter().enumerate().map(|(c, text)| self.fit(c, text)).collect())
            .collect();

        let mut widths: Vec<usize> = self.headers.iter().map(|h| h.chars().count()).collect();
        for row in &cells {
            for (width, cell) in widths.iter_mut().zip(row) {
                *width = (*width).max(cell.chars().count());
            }
        }

        let line = |items: &[String]| -> String {
            let parts: Vec<String> = items
                .iter()
                .enumerate()
                .map(|(c, text)| {
                    let w = widths[c];
                    if self.right[c] {
                        format!("{text:>w$}")
                    } else {
                        format!("{text:<w$}")
                    }
                })
                .collect();
            parts.join("  ").trim_end().to_string()
        };

        let rule: Vec<String> = widths.iter().map(|&w| "-".repeat(w)).collect();
        let mut lines = vec![line(&self.headers), line(&rule)];
        lines.extend(cells.iter().map(|row| line(row)));
        lines.join("\n")
    }

    /// ASCII text for one cell, on one line and cut to the column's maximum width.
    fn fit(&self, column: usize, text: &str) -> String {
        let text = ascii(text).replace('\n', " ");
        match self.max_width[column] {
            Some(max) if text.chars().count() > max => {
                if max > 3 {
                    let head: String = text.chars().take(max - 3).collect();
                    format!("{head}...")
                } else {
                    text.chars().take(max).collect()
                }
            }
            _ => text,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_and_percentages_read_like_the_app() {
        assert_eq!(money(23.36), "e$23.36");
        assert_eq!(opt_money(None), "-");
        assert_eq!(opt_money(Some(10.0)), "e$10.00");
        assert_eq!(pct(0.786), "78.6%");
        assert_eq!(pct(1.0), "100.0%");
    }

    #[test]
    fn text_is_ascii_only_and_keeps_newlines() {
        assert_eq!(
            ascii("Ian\u{2019}s \u{201C}bid\u{201D} \u{2013} caf\u{e9}\tok"),
            "Ian's \"bid\" - caf? ok"
        );
        assert_eq!(ascii("first\nsecond"), "first\nsecond");
        assert!(ascii("\u{1F600} \u{00A0}").is_ascii());
    }

    #[test]
    fn table_aligns_numbers_and_cuts_long_text() {
        let mut table = Table::new(&["CODE", "TITLE", "N"]).right_align(&[2]).max_width(1, 10);
        table.push(vec!["IS111".into(), "Business Intelligence".into(), "5".into()]);
        table.push(vec!["IS2".into(), "Short".into(), "120".into()]);
        let expected = [
            "CODE   TITLE         N",
            "-----  ----------  ---",
            "IS111  Busines...    5",
            "IS2    Short       120",
        ]
        .join("\n");
        assert_eq!(table.render(), expected);
    }

    #[test]
    fn table_puts_a_multi_line_cell_on_one_line() {
        let mut table = Table::new(&["ERROR"]);
        table.push(vec!["Data folder does not exist:\nC:/nowhere".into()]);
        let text = "Data folder does not exist: C:/nowhere";
        assert_eq!(table.render(), format!("ERROR\n{}\n{text}", "-".repeat(text.len())));
    }
}
