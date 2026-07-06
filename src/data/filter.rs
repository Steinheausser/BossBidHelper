use crate::data::{AppData, BidRow, RoundKind};

pub fn filter_rows<'a>(
    data: &'a AppData,
    course_code: &str,
    round: Option<&RoundKind>,
    window: Option<u8>,
    include_1a: bool,
    include_1b: bool,
) -> Vec<&'a BidRow> {
    data.rows.iter().filter(|r| {
        if r.course_code != course_code {
            return false;
        }
        if let Some(rk) = round {
            if &r.round != rk {
                return false;
            }
        }
        if let Some(w) = window {
            if r.window != w {
                return false;
            }
        }
        
        let is_1a = r.session.contains("1A");
        let is_1b = r.session.contains("1B");
        if is_1a && !include_1a {
            return false;
        }
        if is_1b && !include_1b {
            return false;
        }
        
        true
    }).collect()
}

pub fn filter_by_instructor<'a>(
    data: &'a AppData,
    round: Option<&RoundKind>,
    window: Option<u8>,
    school: Option<&str>,
) -> Vec<&'a BidRow> {
    data.rows.iter().filter(|r| {
        if let Some(rk) = round {
            if &r.round != rk {
                return false;
            }
        }
        if let Some(w) = window {
            if r.window != w {
                return false;
            }
        }
        if let Some(s) = school {
            if r.school != s {
                return false;
            }
        }
        true
    }).collect()
}
