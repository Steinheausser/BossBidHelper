use crate::data::{AppData, BidRow, RoundKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Filters that apply across every tab.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GlobalFilters {
    /// Sessions (e.g. "1A Session", "FT Workshop Session") the user has switched off.
    pub excluded_sessions: BTreeSet<String>,
}

impl GlobalFilters {
    pub fn allows(&self, r: &BidRow) -> bool {
        self.excluded_sessions.is_empty() || !self.excluded_sessions.contains(&*r.session)
    }
}

/// A bidding round/window selection; `None` means "all".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WindowSel {
    pub round: Option<RoundKind>,
    pub window: Option<u8>,
}

impl WindowSel {
    pub const ROUND1_W1: WindowSel = WindowSel { round: Some(RoundKind::Round1), window: Some(1) };

    pub fn matches(&self, r: &BidRow) -> bool {
        self.round.is_none_or(|rk| r.round == rk) && self.window.is_none_or(|w| r.window == w)
    }

    pub fn is_specific(&self) -> bool {
        self.round.is_some() && self.window.is_some()
    }

    pub fn label(&self) -> String {
        match (self.round, self.window) {
            (None, _) => "All rounds".to_string(),
            (Some(r), None) => format!("{} (all windows)", r),
            (Some(r), Some(w)) => format!("{} Window {}", r, w),
        }
    }

    /// Compact form such as "R1W1", "R1A" (round 1A, all windows) or "all". `parse` reads it back.
    pub fn short_label(&self) -> String {
        match (self.round, self.window) {
            (None, _) => "all".to_string(),
            (Some(r), None) => format!("R{}", round_code(r)),
            (Some(r), Some(w)) => format!("R{}W{}", round_code(r), w),
        }
    }

    /// Parses one of three forms, ignoring case and surrounding whitespace:
    /// - "all";
    /// - a compact form in one token with no spaces: "R1W1", "r1aw2", "1:1", "1A:2", or a round alone ("R1", "1A")
    ///   for all its windows;
    /// - a long form of words: "Round 1A Window 2", "round 1a win 2", "Round 1" ("Round1A Window2" also works).
    ///
    /// A window number is plain digits from 1 to `MAX_WINDOW`, with no sign and no leading zero. Spaces only
    /// separate the words of a long form, so a stray token such as "R1W1 2" is rejected, not read as "R1W12".
    pub fn parse(s: &str) -> Option<WindowSel> {
        let text = s.trim().to_ascii_lowercase();
        if text == "all" {
            return Some(WindowSel::default());
        }
        let words: Vec<&str> = text.split_whitespace().collect();
        match words.as_slice() {
            [] => None,
            [compact] => parse_compact(compact),
            _ => parse_long(&words),
        }
    }
}

/// The highest window number `WindowSel::parse` accepts. A BOSS round has only a few windows.
const MAX_WINDOW: u8 = 20;

/// One token such as "R1W1", "r1aw2", "1:1", "1A:2", "round1w1" or "R1".
fn parse_compact(s: &str) -> Option<WindowSel> {
    let rest = ["round", "rnd", "r"].into_iter().find_map(|p| s.strip_prefix(p)).unwrap_or(s);
    // Round number, plus an A or B suffix when there is one.
    let digits = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    let round_end = if matches!(rest.as_bytes().get(digits), Some(b'a' | b'b')) { digits + 1 } else { digits };
    let round = RoundKind::parse(&rest[..round_end])?;
    let after = &rest[round_end..];
    let window = if after.is_empty() {
        None
    } else {
        let num = after.strip_prefix(':').or_else(|| ["window", "win", "w"].into_iter().find_map(|p| after.strip_prefix(p)))?;
        Some(window_number(num)?)
    };
    Some(WindowSel { round: Some(round), window })
}

/// Words such as "Round 1A Window 2". Each label is a word of its own or glued to its value ("round1a").
fn parse_long(words: &[&str]) -> Option<WindowSel> {
    let (round_text, rest) = take_labelled(words, &["round", "rnd"])?;
    let round = RoundKind::parse(round_text)?;
    let window = match rest {
        [] => None,
        _ => {
            let (num, rest) = take_labelled(rest, &["window", "win"])?;
            if !rest.is_empty() {
                return None;
            }
            Some(window_number(num)?)
        }
    };
    Some(WindowSel { round: Some(round), window })
}

/// Reads a label and its value from the front of `words`: "round 1a" is two words, "round1a" is one.
/// Returns the value and the words after it.
fn take_labelled<'w, 'a>(words: &'w [&'a str], labels: &[&str]) -> Option<(&'a str, &'w [&'a str])> {
    let first = *words.first()?;
    let glued = labels.iter().find_map(|label| first.strip_prefix(label))?;
    if glued.is_empty() {
        Some((*words.get(1)?, &words[2..]))
    } else {
        Some((glued, &words[1..]))
    }
}

/// Plain digits from 1 to `MAX_WINDOW`. `u8::from_str` alone would also take "+1" and "01".
fn window_number(s: &str) -> Option<u8> {
    let plain = !s.starts_with('0') && s.bytes().all(|b| b.is_ascii_digit());
    s.parse::<u8>().ok().filter(|w| plain && (1..=MAX_WINDOW).contains(w))
}

fn round_code(r: RoundKind) -> &'static str {
    match r {
        RoundKind::Round1 => "1",
        RoundKind::Round1A => "1A",
        RoundKind::Round1B => "1B",
        RoundKind::Round2 => "2",
        RoundKind::Round2A => "2A",
    }
}

pub fn course_rows<'a>(data: &'a AppData, course: &str, sel: WindowSel, filters: &GlobalFilters) -> Vec<&'a BidRow> {
    data.course_rows(course).filter(|r| sel.matches(r) && filters.allows(r)).collect()
}

/// Every (round, window) combination this course has been offered in, in bidding order.
pub fn windows_for_course(data: &AppData, course: &str) -> Vec<(RoundKind, u8)> {
    let set: BTreeSet<(RoundKind, u8)> = data.course_rows(course).map(|r| (r.round, r.window)).collect();
    set.into_iter().collect()
}

/// Every (round, window) combination in the whole dataset.
pub fn all_windows(data: &AppData) -> Vec<(RoundKind, u8)> {
    let mut v: Vec<_> = data.fill_prior.keys().copied().collect();
    v.sort();
    v
}

/// Instructors who have taught this course, most recent first.
pub fn instructors_for_course(data: &AppData, course: &str) -> Vec<String> {
    let mut latest: std::collections::HashMap<&str, crate::data::Term> = std::collections::HashMap::new();
    for r in data.course_rows(course) {
        if !r.instructor.is_empty() {
            let e = latest.entry(&r.instructor).or_insert(r.term);
            *e = (*e).max(r.term);
        }
    }
    let mut v: Vec<_> = latest.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    v.into_iter().map(|(n, _)| n.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(round: Option<RoundKind>, window: Option<u8>) -> Option<WindowSel> {
        Some(WindowSel { round, window })
    }

    #[test]
    fn window_parse_accepts_common_spellings() {
        let (r1, r1a, r2a) = (Some(RoundKind::Round1), Some(RoundKind::Round1A), Some(RoundKind::Round2A));
        assert_eq!(WindowSel::parse("all"), sel(None, None));
        assert_eq!(WindowSel::parse(" ALL "), sel(None, None));
        assert_eq!(WindowSel::parse("R1W1"), sel(r1, Some(1)));
        assert_eq!(WindowSel::parse("r1aw2"), sel(r1a, Some(2)));
        assert_eq!(WindowSel::parse("R2AW3"), sel(r2a, Some(3)));
        assert_eq!(WindowSel::parse("1:1"), sel(r1, Some(1)));
        assert_eq!(WindowSel::parse("1A:2"), sel(r1a, Some(2)));
        assert_eq!(WindowSel::parse("Round 1 Window 1"), sel(r1, Some(1)));
        assert_eq!(WindowSel::parse("round 1a window 2"), sel(r1a, Some(2)));
        assert_eq!(WindowSel::parse("R1"), sel(r1, None));
        assert_eq!(WindowSel::parse("Round 1A"), sel(r1a, None));
        assert_eq!(WindowSel::parse("1A"), sel(r1a, None));
        assert_eq!(WindowSel::parse("\tR1W1\n"), sel(r1, Some(1)));
        assert_eq!(WindowSel::parse("Round 1A Window 2 "), sel(r1a, Some(2)));
        assert_eq!(WindowSel::parse("Round1A Window2"), sel(r1a, Some(2)));
        assert_eq!(WindowSel::parse("rnd 2a win 3"), sel(r2a, Some(3)));
        assert_eq!(WindowSel::parse("round 1 window 1"), sel(r1, Some(1)));
        assert_eq!(WindowSel::parse("Round 1B Window 1"), sel(Some(RoundKind::Round1B), Some(1)));
        assert_eq!(WindowSel::parse("R1W10"), sel(r1, Some(10)));
        assert_eq!(WindowSel::parse("R1W20"), sel(r1, Some(20)));
    }

    #[test]
    fn window_parse_rejects_signs_leading_zeros_and_stray_tokens() {
        for s in [
            "R1W+1",
            "R1W-1",
            "R1W01",
            "R1W\u{FF11}",
            "R1W21",
            "R1W256",
            "Round 1 Window +1",
            "Round 1 Window 0",
            "R1W1 2",
            "R1W1 extra",
            "R1W1 R1W1",
            "R1 W1",
            "R1W 1",
            "1 A",
            "1 : 1",
            "Round 1A Window 2 2",
            "Round 1 Window 1 Window 2",
            "Round 1 Window",
            "Round 1 A Window 2",
            "Round 1 2",
            "Round Window 1",
        ] {
            assert_eq!(WindowSel::parse(s), None, "{s:?} should not parse");
        }
    }

    #[test]
    fn window_parse_rejects_garbage() {
        for s in ["", "garbage", "R", "R9W1", "R1W0", "R1W", "R1W1X", "2B", "1:x", "1 2", "Window 1", "1:1:1"] {
            assert_eq!(WindowSel::parse(s), None, "{s:?} should not parse");
        }
    }

    #[test]
    fn short_label_parses_back() {
        let rounds = [RoundKind::Round1, RoundKind::Round1A, RoundKind::Round1B, RoundKind::Round2, RoundKind::Round2A];
        for round in rounds {
            let whole = WindowSel { round: Some(round), window: None };
            assert_eq!(WindowSel::parse(&whole.short_label()), Some(whole), "{}", whole.short_label());
            for window in 1..=MAX_WINDOW {
                let w = WindowSel { round: Some(round), window: Some(window) };
                assert_eq!(WindowSel::parse(&w.short_label()), Some(w), "{}", w.short_label());
            }
        }
        assert_eq!(WindowSel::default().short_label(), "all");
        assert_eq!(WindowSel::ROUND1_W1.short_label(), "R1W1");
        assert_eq!(WindowSel::parse("R1A").map(|s| s.short_label()), Some("R1A".to_string()));
    }
}
