use egui::Ui;
use std::sync::Arc;
use crate::data::{AppData, RoundKind};
use crate::ui::widgets::round_selector;

pub struct RankingState {
    pub selected_round: Option<RoundKind>,
    pub selected_window: Option<u8>,
    pub selected_school: Option<String>,
    pub schools: Vec<String>,
    pub sort_ascending: bool,
}

struct ProfessorStats {
    name: String,
    school: String,
    median_bid: f64,
    data_points: usize,
}

impl RankingState {
    pub fn new(data: Arc<AppData>) -> Self {
        let mut schools = std::collections::HashSet::new();
        for r in &data.rows {
            if !r.school.is_empty() {
                schools.insert(r.school.clone());
            }
        }
        let mut schools_vec: Vec<_> = schools.into_iter().collect();
        schools_vec.sort();
        
        Self {
            selected_round: None,
            selected_window: None,
            selected_school: None,
            schools: schools_vec,
            sort_ascending: false,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, data: &AppData, _include_1a: bool, _include_1b: bool) {
        ui.heading("Professor Ranking");
        
        ui.horizontal(|ui| {
            round_selector(ui, &mut self.selected_round, &mut self.selected_window);
            
            ui.label("School:");
            egui::ComboBox::from_id_salt("school_combo")
                .selected_text(self.selected_school.as_deref().unwrap_or("All"))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.selected_school, None, "All");
                    for s in &self.schools {
                        ui.selectable_value(&mut self.selected_school, Some(s.clone()), s);
                    }
                });
        });
        
        ui.separator();
        
        let filtered = crate::data::filter::filter_by_instructor(data, self.selected_round.as_ref(), self.selected_window, self.selected_school.as_deref());
        
        let mut prof_map: std::collections::HashMap<String, (String, Vec<f64>)> = std::collections::HashMap::new();
        for r in filtered {
            if r.median_bid > 0.0 && !r.instructor.is_empty() {
                let e = prof_map.entry(r.instructor.clone()).or_insert_with(|| (r.school.clone(), Vec::new()));
                e.1.push(r.median_bid);
            }
        }
        
        let mut stats = Vec::new();
        for (name, (school, mut bids)) in prof_map {
            bids.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let mid = bids.len() / 2;
            let median = if bids.len() % 2 == 0 {
                (bids[mid - 1] + bids[mid]) / 2.0
            } else {
                bids[mid]
            };
            stats.push(ProfessorStats {
                name,
                school,
                median_bid: median,
                data_points: bids.len(),
            });
        }
        
        if self.sort_ascending {
            stats.sort_by(|a, b| a.median_bid.partial_cmp(&b.median_bid).unwrap());
        } else {
            stats.sort_by(|a, b| b.median_bid.partial_cmp(&a.median_bid).unwrap());
        }
        
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.sort_ascending, "Sort Ascending (Lowest Bid First)");
        });
        
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("ranking_grid").striped(true).show(ui, |ui| {
                ui.label(egui::RichText::new("Rank").strong());
                ui.label(egui::RichText::new("Professor").strong());
                ui.label(egui::RichText::new("School").strong());
                ui.label(egui::RichText::new("Median Bid").strong());
                ui.label(egui::RichText::new("Data Points").strong());
                ui.end_row();
                
                for (i, prof) in stats.iter().enumerate() {
                    ui.label(format!("{}", i + 1));
                    ui.label(&prof.name);
                    ui.label(&prof.school);
                    ui.label(format!("${:.2}", prof.median_bid));
                    ui.label(format!("{}", prof.data_points));
                    ui.end_row();
                }
            });
        });
    }
}
