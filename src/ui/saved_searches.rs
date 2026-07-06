use egui::Ui;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::data::RoundKind;
use crate::data::stats::PredictionBands;
use chrono::Local;

#[derive(Serialize, Deserialize, Clone)]
pub struct SavedSearch {
    pub timestamp: String,
    pub course_code: String,
    pub description: String,
    pub round: Option<RoundKind>,
    pub window: Option<u8>,
    pub p10: f64,
    pub p30: f64,
    pub p50: f64,
    pub p70: f64,
    pub p90: f64,
}

pub struct SavedState {
    pub entries: Vec<SavedSearch>,
    file_path: PathBuf,
}

impl SavedState {
    pub fn load() -> Self {
        let mut path = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push("smu_bid_analyser");
        std::fs::create_dir_all(&path).ok();
        path.push("saved_searches.json");
        
        let entries = if let Ok(data) = std::fs::read_to_string(&path) {
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            Vec::new()
        };
        
        Self {
            entries,
            file_path: path,
        }
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(&self.entries) {
            std::fs::write(&self.file_path, json).ok();
        }
    }

    pub fn add_search(&mut self, course_code: &str, description: &str, round: Option<RoundKind>, window: Option<u8>, bands: &PredictionBands) {
        let entry = SavedSearch {
            timestamp: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            course_code: course_code.to_string(),
            description: description.to_string(),
            round,
            window,
            p10: bands.p10,
            p30: bands.p30,
            p50: bands.p50,
            p70: bands.p70,
            p90: bands.p90,
        };
        self.entries.insert(0, entry);
        self.save();
    }
    
    pub fn auto_save_click(&mut self, course_code: &str, description: &str) {
        // Prevent duplicate consecutive auto-saves
        if let Some(first) = self.entries.first() {
            if first.course_code == course_code && first.p50 == 0.0 && first.round.is_none() {
                return;
            }
        }
        
        let entry = SavedSearch {
            timestamp: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            course_code: course_code.to_string(),
            description: description.to_string(),
            round: None,
            window: None,
            p10: 0.0,
            p30: 0.0,
            p50: 0.0,
            p70: 0.0,
            p90: 0.0,
        };
        self.entries.insert(0, entry);
        
        if self.entries.len() > 100 {
            self.entries.truncate(100);
        }
        
        self.save();
    }

    pub fn ui(&mut self, ui: &mut Ui, prediction: &mut crate::ui::prediction::PredictionState, active_tab: &mut crate::ui::Tab) {
        ui.heading("Saved & History Searches");
        
        if ui.button("Clear All").clicked() {
            self.entries.clear();
            self.save();
        }
        
        ui.separator();
        
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut to_delete = None;
            
            for (i, entry) in self.entries.iter().enumerate() {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&entry.course_code).strong());
                        ui.label(&entry.description);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Delete").clicked() {
                                to_delete = Some(i);
                            }
                            ui.label(egui::RichText::new(&entry.timestamp).small());
                        });
                    });
                    
                    if entry.p50 > 0.0 {
                        let r_str = entry.round.as_ref().map(|r| r.to_string()).unwrap_or_else(|| "All".to_string());
                        let w_str = entry.window.map(|w| format!("{}", w)).unwrap_or_else(|| "All".to_string());
                        ui.label(format!("Filters: Round {}, Window {}", r_str, w_str));
                        
                        egui::Grid::new(format!("grid_{}", i)).striped(true).show(ui, |ui| {
                            ui.label("Worst (p10)");
                            ui.label("Pessimistic (p30)");
                            ui.label(egui::RichText::new("Median (p50)").strong());
                            ui.label("Optimistic (p70)");
                            ui.label("Best (p90)");
                            ui.end_row();
                            
                            ui.label(format!("${:.2}", entry.p10));
                            ui.label(format!("${:.2}", entry.p30));
                            ui.label(egui::RichText::new(format!("${:.2}", entry.p50)).strong());
                            ui.label(format!("${:.2}", entry.p70));
                            ui.label(format!("${:.2}", entry.p90));
                            ui.end_row();
                        });
                        
                        ui.add_space(5.0);
                        if ui.button("Load into Prediction").clicked() {
                            prediction.selected_course = Some(entry.course_code.clone());
                            prediction.selected_round = entry.round.clone();
                            prediction.selected_window = entry.window;
                            *active_tab = crate::ui::Tab::Prediction;
                        }
                    } else {
                        ui.label(egui::RichText::new("History log (Auto-saved)").italics());
                    }
                });
                ui.add_space(5.0);
            }
            
            if let Some(i) = to_delete {
                self.entries.remove(i);
                self.save();
            }
        });
    }
}
