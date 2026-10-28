use crate::data::filter::WindowSel;
use crate::data::model::Confidence;
use crate::data::{RoundKind, Term};
use crate::ui::prediction::LEVELS;
use crate::ui::widgets::{money, pct};
use crate::ui::theme;
use chrono::Local;
use egui::{RichText, Ui};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
pub struct SavedSearch {
    pub timestamp: String,
    pub course_code: String,
    pub description: String,
    pub round: Option<RoundKind>,
    pub window: Option<u8>,
    #[serde(default)]
    pub instructor: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    /// (probability of a seat, recommended bid)
    #[serde(default)]
    pub bids: Vec<(f64, f64)>,
    #[serde(default)]
    pub confidence: Option<String>,
    // Fields written by v0.1, kept so old files still load.
    #[serde(default)]
    pub p10: f64,
    #[serde(default)]
    pub p30: f64,
    #[serde(default)]
    pub p50: f64,
    #[serde(default)]
    pub p70: f64,
    #[serde(default)]
    pub p90: f64,
}

impl SavedSearch {
    pub fn has_prediction(&self) -> bool {
        !self.bids.is_empty() || self.p50 > 0.0
    }

    fn history(course_code: &str, description: &str) -> Self {
        Self {
            timestamp: now(),
            course_code: course_code.to_string(),
            description: description.to_string(),
            round: None,
            window: None,
            instructor: None,
            target: None,
            bids: Vec::new(),
            confidence: None,
            p10: 0.0,
            p30: 0.0,
            p50: 0.0,
            p70: 0.0,
            p90: 0.0,
        }
    }
}

fn now() -> String {
    Local::now().format("%Y-%m-%d %H:%M").to_string()
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

        let entries = std::fs::read_to_string(&path).ok().and_then(|d| serde_json::from_str(&d).ok()).unwrap_or_default();
        Self { entries, file_path: path }
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(&self.entries) {
            std::fs::write(&self.file_path, json).ok();
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_prediction(
        &mut self,
        course_code: &str,
        description: &str,
        sel: WindowSel,
        instructor: Option<String>,
        target: Term,
        bids: &[f64],
        confidence: Confidence,
    ) {
        let mut entry = SavedSearch::history(course_code, description);
        entry.round = sel.round;
        entry.window = sel.window;
        entry.instructor = instructor;
        entry.target = Some(target.label());
        entry.bids = LEVELS.iter().copied().zip(bids.iter().copied()).collect();
        entry.confidence = Some(confidence.label().to_string());
        self.entries.insert(0, entry);
        self.save();
    }

    pub fn auto_save_click(&mut self, course_code: &str, description: &str) {
        // Keep one history entry per course, most recent first.
        self.entries.retain(|e| e.has_prediction() || e.course_code != course_code);
        self.entries.insert(0, SavedSearch::history(course_code, description));
        let mut history = 0;
        self.entries.retain(|e| {
            if e.has_prediction() {
                return true;
            }
            history += 1;
            history <= 50
        });
        self.save();
    }

    /// Returns the entry the user asked to open.
    pub fn ui(&mut self, ui: &mut Ui) -> Option<SavedSearch> {
        let mut open = None;
        let mut delete = None;
        let mut clear_history = false;

        egui::ScrollArea::vertical().auto_shrink([true, false]).show(ui, |ui| {
            ui.heading("Saved bid plans");
            let plans: Vec<usize> = (0..self.entries.len()).filter(|&i| self.entries[i].has_prediction()).collect();
            if plans.is_empty() {
                ui.label(RichText::new("Nothing saved yet. Use \"★ Save this plan\" in the Bid Planner.").color(theme::muted(ui)));
            }
            for i in plans {
                let e = &self.entries[i];
                theme::card(ui).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&e.course_code).strong().color(theme::ACCENT));
                        ui.label(&e.description);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Delete").clicked() {
                                delete = Some(i);
                            }
                            if ui.small_button("Open").clicked() {
                                open = Some(e.clone());
                            }
                            ui.label(RichText::new(&e.timestamp).small().color(theme::muted(ui)));
                        });
                    });
                    let window = match (e.round, e.window) {
                        (Some(r), Some(w)) => format!("{r} Window {w}"),
                        (Some(r), None) => r.to_string(),
                        _ => "All rounds".to_string(),
                    };
                    let mut meta = vec![window];
                    meta.extend(e.instructor.clone());
                    meta.extend(e.target.as_ref().map(|t| format!("for {t}")));
                    meta.extend(e.confidence.as_ref().map(|c| format!("{c} confidence")));
                    ui.label(RichText::new(meta.join(" · ")).small().color(theme::muted(ui)));

                    egui::Grid::new(("saved_grid", i)).spacing([18.0, 2.0]).show(ui, |ui| {
                        if !e.bids.is_empty() {
                            for (p, _) in &e.bids {
                                ui.label(RichText::new(format!("{} chance", pct(*p))).small());
                            }
                            ui.end_row();
                            for (p, b) in &e.bids {
                                let t = RichText::new(money(*b));
                                ui.label(if (*p - 0.8).abs() < 1e-9 { t.strong().color(theme::ACCENT) } else { t });
                            }
                        } else {
                            for h in ["p10", "p30", "p50", "p70", "p90"] {
                                ui.label(RichText::new(h).small());
                            }
                            ui.end_row();
                            for v in [e.p10, e.p30, e.p50, e.p70, e.p90] {
                                ui.label(money(v));
                            }
                        }
                        ui.end_row();
                    });
                    if e.bids.is_empty() {
                        ui.label(RichText::new("Saved by v0.1 (old linear model); re-open to get an updated plan.").small().color(theme::WARN));
                    }
                });
                ui.add_space(4.0);
            }

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.heading("Recently viewed");
                if ui.small_button("Clear").clicked() {
                    clear_history = true;
                }
            });
            ui.horizontal_wrapped(|ui| {
                for e in self.entries.iter().filter(|e| !e.has_prediction()) {
                    if ui.button(&e.course_code).on_hover_text(format!("{}\nviewed {}", e.description, e.timestamp)).clicked() {
                        open = Some(e.clone());
                    }
                }
            });
        });

        if let Some(i) = delete {
            self.entries.remove(i);
            self.save();
        }
        if clear_history {
            self.entries.retain(|e| e.has_prediction());
            self.save();
        }
        open
    }
}
