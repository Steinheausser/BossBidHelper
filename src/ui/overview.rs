use egui::Ui;
use egui_plot::{Line, Plot, PlotPoints, MarkerShape, Points};
use std::sync::Arc;
use crate::data::{AppData, RoundKind};
use crate::ui::widgets::round_selector;
use crate::ui::saved_searches::SavedState;
use crate::ui::Tab;
use crate::ui::prediction::PredictionState;

pub struct OverviewState {
    pub search_query: String,
    pub last_query: String,
    pub search_results: Vec<(String, String)>,
    pub selected_course: Option<String>,
    pub selected_round: Option<RoundKind>,
    pub selected_window: Option<u8>,
}

impl OverviewState {
    pub fn new(data: Arc<AppData>) -> Self {
        Self {
            search_query: String::new(),
            last_query: String::new(),
            search_results: data.unique_courses.clone(), // Start with all
            selected_course: None,
            selected_round: None,
            selected_window: None,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, data: &AppData, saved: &mut SavedState, include_1a: bool, include_1b: bool, active_tab: &mut Tab, prediction: &mut PredictionState) {
        ui.heading("Course Overview");
        egui::ScrollArea::both().id_salt("overview_scroll").show(ui, |ui| {
        
        let text_edit = ui.text_edit_singleline(&mut self.search_query);
        if text_edit.changed() || self.search_query != self.last_query {
            self.last_query = self.search_query.clone();
            
            let query = self.search_query.to_lowercase();
            if query.is_empty() {
                self.search_results = data.unique_courses.clone();
            } else {
                let mut matcher = nucleo::Matcher::new(nucleo::Config::DEFAULT);
                let mut matches = Vec::new();
                for (code, desc) in &data.unique_courses {
                    let haystack = format!("{} {}", code, desc).to_lowercase();
                    let haystack_utf32 = nucleo::Utf32String::from(haystack.as_str());
                    let query_utf32 = nucleo::Utf32String::from(query.as_str());
                    if let Some(score) = matcher.fuzzy_match(haystack_utf32.slice(..), query_utf32.slice(..)) {
                        matches.push((score, code.clone(), desc.clone()));
                    }
                }
                matches.sort_by(|a, b| b.0.cmp(&a.0));
                self.search_results = matches.into_iter().map(|(_, c, d)| (c, d)).take(50).collect();
            }
        }

        ui.separator();
        
        ui.columns(2, |cols| {
            cols[0].heading("Search Results");
            egui::ScrollArea::vertical().id_salt("search_scroll").max_height(200.0).show(&mut cols[0], |ui| {
                for (code, desc) in &self.search_results {
                    let label = format!("{} - {}", code, desc);
                    if ui.selectable_label(self.selected_course.as_ref() == Some(code), label).clicked() {
                        self.selected_course = Some(code.clone());
                        
                        // Auto-save the click to history
                        saved.auto_save_click(code, desc);
                        
                        // Sync with prediction tab
                        prediction.selected_course = Some(code.clone());
                    }
                }
            });
            
            if let Some(course) = &self.selected_course {
                cols[1].heading(format!("Details for {}", course));
                if cols[1].button("Open in Price Prediction ->").clicked() {
                    *active_tab = Tab::Prediction;
                }
                round_selector(&mut cols[1], &mut self.selected_round, &mut self.selected_window);
            }
        });

        ui.separator();

        if let Some(course) = &self.selected_course {
            let filtered = crate::data::filter::filter_rows(data, course, self.selected_round.as_ref(), self.selected_window, include_1a, include_1b);
            
            let stats = crate::data::stats::median_by_term(&filtered);
            
            if stats.is_empty() {
                ui.label("No data available for the selected filters.");
            } else {
                let pts_vec: Vec<[f64; 2]> = stats.iter().enumerate().map(|(i, &(_, m))| [i as f64, m]).collect();
                let line = Line::new(PlotPoints::new(pts_vec.clone())).name("Median Bid");
                
                let terms: Vec<String> = stats.iter().map(|(t, _)| t.clone()).collect();
                
                Plot::new("overview_plot")
                    .view_aspect(2.0)
                    .allow_drag(true)
                    .allow_scroll(true)
                    .x_axis_formatter(move |x, _range| {
                        let idx = x.value.round() as usize;
                        if idx < terms.len() {
                            terms[idx].clone()
                        } else {
                            String::new()
                        }
                    })
                    .label_formatter(|name, val| format!("{}\nMedian: ${:.2}", name, val.y))
                    .show(ui, |plot_ui| {
                        plot_ui.line(line);
                        plot_ui.points(Points::new(PlotPoints::new(pts_vec)).radius(4.0).shape(MarkerShape::Circle));
                    });
            }
        }
        });
    }
}
