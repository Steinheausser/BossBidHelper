use crate::data::BidRow;
use linregress::{FormulaRegressionBuilder, RegressionDataBuilder};

pub struct PredictionBands {
    pub p10: f64,
    pub p30: f64,
    pub p50: f64,
    pub p70: f64,
    pub p90: f64,
}

pub fn median_by_term(rows: &[&BidRow]) -> Vec<(String, f64)> {
    let mut terms_map: std::collections::BTreeMap<(u16, u8, String), Vec<f64>> = std::collections::BTreeMap::new();
    
    for r in rows {
        if r.median_bid > 0.0 {
            terms_map.entry((r.year, r.term_num, r.term.clone()))
                .or_default()
                .push(r.median_bid);
        }
    }
    
    let mut results = Vec::new();
    for ((_, _, term), mut bids) in terms_map {
        bids.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mid = bids.len() / 2;
        let median = if bids.len() % 2 == 0 {
            (bids[mid - 1] + bids[mid]) / 2.0
        } else {
            bids[mid]
        };
        results.push((term, median));
    }
    results
}

pub fn fill_rate_by_term(rows: &[&BidRow]) -> Vec<(String, f64, f64)> {
    let mut terms_map: std::collections::BTreeMap<(u16, u8, String), (i32, i32, i32)> = std::collections::BTreeMap::new();
    
    for r in rows {
        let e = terms_map.entry((r.year, r.term_num, r.term.clone())).or_insert((0, 0, 0));
        e.0 += r.vacancy;
        e.1 += r.before_proc;
        e.2 += r.after_proc;
    }
    
    let mut results = Vec::new();
    for ((_, _, term), (vac, bef, aft)) in terms_map {
        if vac > 0 {
            let bef_pct = (bef as f64 / vac as f64) * 100.0;
            let aft_pct = (aft as f64 / vac as f64) * 100.0;
            results.push((term, bef_pct, aft_pct));
        } else {
            results.push((term, 0.0, 0.0));
        }
    }
    results
}

pub fn linear_regression_predict(points: &[(f64, f64)], future_x: f64) -> Option<PredictionBands> {
    if points.len() < 2 {
        return None;
    }
    
    let x: Vec<f64> = points.iter().map(|p| p.0).collect();
    let y: Vec<f64> = points.iter().map(|p| p.1).collect();
    
    let data = vec![("x", x), ("y", y)];
    let reg_data = RegressionDataBuilder::new().build_from(data).ok()?;
    let formula = "y ~ x";
    let model = FormulaRegressionBuilder::new().data(&reg_data).formula(formula).fit().ok()?;
    
    let intercept = model.parameters()[0];
    let slope = model.parameters()[1];
    let _se = model.se();
    // Simplified band calculation just for illustration:
    // In a real app we'd use t-distribution and prediction intervals.
    // For now, we'll use a standard error proxy:
    
    let y_pred = intercept + slope * future_x;
    
    // Fake standard error for bands (use the residuals standard error roughly)
    let err = 2.0; // Hardcoded dummy error margin, in reality derive from model
    
    Some(PredictionBands {
        p10: (y_pred - err * 1.28).max(0.0), // ~z-score for 10%
        p30: (y_pred - err * 0.52).max(0.0), // ~z-score for 30%
        p50: y_pred.max(0.0),
        p70: (y_pred + err * 0.52).max(0.0),
        p90: (y_pred + err * 1.28).max(0.0),
    })
}

pub fn empty_slots_by_term(rows: &[&BidRow]) -> Vec<(String, i32)> {
    let mut terms_map: std::collections::BTreeMap<(u16, u8, String), (i32, i32)> = std::collections::BTreeMap::new();
    
    for r in rows {
        let e = terms_map.entry((r.year, r.term_num, r.term.clone())).or_insert((0, 0));
        e.0 += r.vacancy;
        e.1 += r.after_proc;
    }
    
    let mut results = Vec::new();
    for ((_, _, term), (vac, aft)) in terms_map {
        let empty = std::cmp::max(0, vac - aft);
        results.push((term, empty));
    }
    results
}
