use egui::Ui;
use egui_plot::{Line, Plot, PlotPoints, Polygon, MarkerShape, Points};
use crate::data::{AppData, RoundKind};
use crate::ui::widgets::round_selector;
use crate::ui::saved_searches::SavedState;

pub struct PredictionState {
    pub selected_course: Option<String>,
    pub selected_round: Option<RoundKind>,
    pub selected_window: Option<u8>,
}

impl PredictionState {
    pub fn new() -> Self {
        Self {
            selected_course: None,
            selected_round: None,
            selected_window: None,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, data: &AppData, saved: &mut SavedState, include_1a: bool, include_1b: bool) {
        ui.heading("Price Prediction & Fill Rates");
        egui::ScrollArea::both().id_salt("prediction_scroll").show(ui, |ui| {
        if let Some(course) = &self.selected_course {
            ui.label(format!("Predicting for: {}", course));
            round_selector(ui, &mut self.selected_round, &mut self.selected_window);
            
            let filtered = crate::data::filter::filter_rows(data, course, self.selected_round.as_ref(), self.selected_window, include_1a, include_1b);
            let stats = crate::data::stats::median_by_term(&filtered);
            let fill_rates = crate::data::stats::fill_rate_by_term(&filtered);
            let empty_slots = crate::data::stats::empty_slots_by_term(&filtered);
            
            if stats.len() < 2 {
                ui.label("Need at least 2 terms of data to run linear regression prediction.");
                return;
            }
            
            let pts: Vec<(f64, f64)> = stats.iter().enumerate().map(|(i, &(_, m))| (i as f64, m)).collect();
            let future_x = pts.len() as f64;
            
            let bands_opt = crate::data::stats::linear_regression_predict(&pts, future_x);
            
            if let Some(bands) = &bands_opt {
                ui.horizontal(|ui| {
                    ui.label(format!("Predicted Median (p50): ${:.2}", bands.p50));
                    ui.label(format!("Worst Case (p10): ${:.2}", bands.p10));
                    if ui.button("Save to Saved Searches").clicked() {
                        let desc = data.unique_courses.iter().find(|(c, _)| c == course).map(|(_, d)| d.clone()).unwrap_or_default();
                        saved.add_search(course, &desc, self.selected_round.clone(), self.selected_window, bands);
                    }
                });
            }
            
            ui.separator();
            
            // Draw Charts
            let terms: Vec<String> = stats.iter().map(|(t, _)| t.clone()).collect();
            let mut all_terms = terms.clone();
            all_terms.push("Next Term (Pred)".to_string());
            
            Plot::new("prediction_plot")
                .view_aspect(2.0)
                .allow_drag(true)
                .allow_scroll(true)
                .x_axis_formatter(move |x, _range| {
                    let idx = x.value.round() as usize;
                    if idx < all_terms.len() {
                        all_terms[idx].clone()
                    } else {
                        String::new()
                    }
                })
                .show(ui, |plot_ui| {
                    // Actual Medians
                    let pts_vec: Vec<[f64; 2]> = pts.iter().map(|p| [p.0, p.1]).collect();
                    plot_ui.line(Line::new(PlotPoints::new(pts_vec.clone())).name("Actual Median").color(egui::Color32::WHITE));
                    plot_ui.points(Points::new(PlotPoints::new(pts_vec)).radius(4.0).color(egui::Color32::WHITE).shape(MarkerShape::Circle));
                    
                    if let Some(bands) = bands_opt {
                        let last_pt = pts.last().unwrap();
                        
                        // Polygon for 10-90 band
                        let poly10_90 = vec![[last_pt.0, last_pt.1], [future_x, bands.p90], [future_x, bands.p10], [last_pt.0, last_pt.1]];
                        plot_ui.polygon(Polygon::new(PlotPoints::new(poly10_90)).fill_color(egui::Color32::from_rgba_unmultiplied(200, 100, 100, 50)).name("10-90% Band"));
                        
                        // Polygon for 30-70 band
                        let poly30_70 = vec![[last_pt.0, last_pt.1], [future_x, bands.p70], [future_x, bands.p30], [last_pt.0, last_pt.1]];
                        plot_ui.polygon(Polygon::new(PlotPoints::new(poly30_70)).fill_color(egui::Color32::from_rgba_unmultiplied(100, 200, 100, 100)).name("30-70% Band"));
                        
                        // Line for 50
                        plot_ui.line(Line::new(PlotPoints::new(vec![[last_pt.0, last_pt.1], [future_x, bands.p50]])).name("Predicted Median").color(egui::Color32::GREEN));
                    }
                });
                
            ui.separator();
            ui.heading("Fill Rates");
            let terms2 = terms.clone();
            Plot::new("fill_rate_plot")
                .view_aspect(3.0)
                .allow_drag(true)
                .allow_scroll(true)
                .x_axis_formatter(move |x, _range| {
                    let idx = x.value.round() as usize;
                    if idx < terms2.len() {
                        terms2[idx].clone()
                    } else {
                        String::new()
                    }
                })
                .show(ui, |plot_ui| {
                    let bef_pts: PlotPoints = fill_rates.iter().enumerate().map(|(i, (_, b, _))| [i as f64, *b]).collect();
                    let aft_pts: PlotPoints = fill_rates.iter().enumerate().map(|(i, (_, _, a))| [i as f64, *a]).collect();
                    
                    plot_ui.line(Line::new(bef_pts).name("Before Process % Fill").color(egui::Color32::LIGHT_BLUE));
                    plot_ui.line(Line::new(aft_pts).name("After Process % Fill").color(egui::Color32::LIGHT_RED));
                });
                
            ui.separator();
            ui.heading("Empty Slots by Term");
            Plot::new("empty_slots_plot")
                .view_aspect(3.0)
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
                .show(ui, |plot_ui| {
                    let empty_pts: PlotPoints = empty_slots.iter().enumerate().map(|(i, (_, e))| [i as f64, *e as f64]).collect();
                    let pts_vec: Vec<[f64; 2]> = empty_slots.iter().enumerate().map(|(i, (_, e))| [i as f64, *e as f64]).collect();
                    
                    plot_ui.line(Line::new(empty_pts).name("Empty Slots").color(egui::Color32::YELLOW));
                    plot_ui.points(Points::new(PlotPoints::new(pts_vec)).radius(4.0).color(egui::Color32::YELLOW).shape(MarkerShape::Circle));
                });

        } else {
            ui.label("Select a course in the Overview tab first.");
        }
        });
    }
}
