use std::path::Path;
use std::fs;
use polars::prelude::*;
use crate::data::{BidRow, RoundKind, AppData};
use std::collections::HashSet;

pub fn load_all_data(data_dir: &Path) -> Result<AppData, Box<dyn std::error::Error>> {
    let mut rows = Vec::new();
    let mut unique_courses_set = HashSet::new();
    
    if !data_dir.exists() {
        return Err(format!("Data directory {:?} does not exist", data_dir).into());
    }

    for entry in fs::read_dir(data_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("parquet") {
            let df = LazyFrame::scan_parquet(&path, ScanArgsParquet::default())?
                .collect()?;
            
            // Extract vectors for each column
            let term_s = df.column("Term")?.str()?;
            let year_series = df.column("Year")?.cast(&DataType::Int64)?; let year_s = year_series.i64()?;
            let term_num_series = df.column("TermNum")?.cast(&DataType::Int64)?; let term_num_s = term_num_series.i64()?;
            let round_kind_s = df.column("RoundKind")?.str()?;
            let window_num_series = df.column("WindowNum")?.cast(&DataType::Int64)?; let window_num_s = window_num_series.i64()?;
            let course_code_s = df.column("Course Code")?.str()?;
            let desc_s = df.column("Description")?.str()?;
            let section_s = df.column("Section")?.str()?;
            let instructor_s = df.column("Instructor")?.str()?;
            let school_s = df.column("School/Department")?.str()?;
            
            let vac_series = df.column("Vacancy")?.cast(&DataType::Float64)?; let vac_s = vac_series.f64()?;
            let open_vac_series = df.column("Opening Vacancy")?.cast(&DataType::Float64)?; let open_vac_s = open_vac_series.f64()?;
            let bef_proc_series = df.column("Before Process Vacancy")?.cast(&DataType::Float64)?; let bef_proc_s = bef_proc_series.f64()?;
            let dice_series = df.column("D.I.C.E")?.cast(&DataType::Float64)?; let dice_s = dice_series.f64()?;
            let aft_proc_series = df.column("After Process Vacancy")?.cast(&DataType::Float64)?; let aft_proc_s = aft_proc_series.f64()?;
            let enroll_series = df.column("Enrolled Students")?.cast(&DataType::Float64)?; let enroll_s = enroll_series.f64()?;
            let med_bid_series = df.column("Median Bid")?.cast(&DataType::Float64)?; let med_bid_s = med_bid_series.f64()?;
            let min_bid_series = df.column("Min Bid")?.cast(&DataType::Float64)?; let min_bid_s = min_bid_series.f64()?;
            let session_s = df.column("Session")?.str()?;

            for i in 0..df.height() {
                let round_kind_str = round_kind_s.get(i).unwrap_or("");
                if let Some(round_kind) = RoundKind::from_str(round_kind_str) {
                    let course_code = course_code_s.get(i).unwrap_or("").to_string();
                    let desc = desc_s.get(i).unwrap_or("").to_string();
                    
                    unique_courses_set.insert((course_code.clone(), desc.clone()));
                    
                    rows.push(BidRow {
                        term: term_s.get(i).unwrap_or("").to_string(),
                        year: year_s.get(i).unwrap_or(0) as u16,
                        term_num: term_num_s.get(i).unwrap_or(0) as u8,
                        round: round_kind,
                        window: window_num_s.get(i).unwrap_or(0) as u8,
                        course_code,
                        description: desc,
                        section: section_s.get(i).unwrap_or("").to_string(),
                        instructor: instructor_s.get(i).unwrap_or("").to_string(),
                        school: school_s.get(i).unwrap_or("").to_string(),
                        vacancy: vac_s.get(i).unwrap_or(0.0) as i32,
                        opening_vacancy: open_vac_s.get(i).unwrap_or(0.0) as i32,
                        before_proc: bef_proc_s.get(i).unwrap_or(0.0) as i32,
                        dice: dice_s.get(i).unwrap_or(0.0) as i32,
                        after_proc: aft_proc_s.get(i).unwrap_or(0.0) as i32,
                        enrolled: enroll_s.get(i).unwrap_or(0.0) as i32,
                        median_bid: med_bid_s.get(i).unwrap_or(0.0),
                        min_bid: min_bid_s.get(i).unwrap_or(0.0),
                        session: session_s.get(i).unwrap_or("").to_string(),
                    });
                }
            }
        }
    }
    
    let mut unique_courses: Vec<_> = unique_courses_set.into_iter().collect();
    unique_courses.sort_by(|a, b| a.0.cmp(&b.0));
    
    // Sort rows by year and term
    rows.sort_by(|a, b| {
        a.year.cmp(&b.year).then(a.term_num.cmp(&b.term_num))
    });

    Ok(AppData { rows, unique_courses })
}
