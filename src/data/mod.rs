pub mod loader;
pub mod filter;
pub mod stats;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RoundKind {
    Round1,
    Round1A,
    Round1B,
    Round2,
    Round2A,
}

impl std::fmt::Display for RoundKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoundKind::Round1 => write!(f, "Round 1"),
            RoundKind::Round1A => write!(f, "Round 1A"),
            RoundKind::Round1B => write!(f, "Round 1B"),
            RoundKind::Round2 => write!(f, "Round 2"),
            RoundKind::Round2A => write!(f, "Round 2A"),
        }
    }
}

impl RoundKind {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "Round1" => Some(RoundKind::Round1),
            "Round1A" => Some(RoundKind::Round1A),
            "Round1B" => Some(RoundKind::Round1B),
            "Round2" => Some(RoundKind::Round2),
            "Round2A" => Some(RoundKind::Round2A),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BidRow {
    pub term: String,
    pub year: u16,
    pub term_num: u8,
    pub round: RoundKind,
    pub window: u8,
    pub course_code: String,
    pub description: String,
    pub section: String,
    pub instructor: String,
    pub school: String,
    pub vacancy: i32,
    pub opening_vacancy: i32,
    pub before_proc: i32,
    pub dice: i32,
    pub after_proc: i32,
    pub enrolled: i32,
    pub median_bid: f64,
    pub min_bid: f64,
    pub session: String,
}

pub struct AppData {
    pub rows: Vec<BidRow>,
    pub unique_courses: Vec<(String, String)>, // (Course Code, Description)
}
