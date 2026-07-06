use egui::Ui;
use crate::data::RoundKind;

pub fn round_selector(ui: &mut Ui, selected_round: &mut Option<RoundKind>, selected_window: &mut Option<u8>) {
    ui.horizontal(|ui| {
        ui.label("Round:");
        
        let mut changed = false;
        
        egui::ComboBox::from_id_salt("round_combo")
            .selected_text(selected_round.as_ref().map(|r| r.to_string()).unwrap_or_else(|| "All".to_string()))
            .show_ui(ui, |ui| {
                if ui.selectable_value(selected_round, None, "All").clicked() {
                    *selected_window = None;
                    changed = true;
                }
                if ui.selectable_value(selected_round, Some(RoundKind::Round1), "Round 1").clicked() {
                    *selected_window = None;
                    changed = true;
                }
                if ui.selectable_value(selected_round, Some(RoundKind::Round1A), "Round 1A").clicked() {
                    *selected_window = None;
                    changed = true;
                }
                if ui.selectable_value(selected_round, Some(RoundKind::Round1B), "Round 1B").clicked() {
                    *selected_window = None;
                    changed = true;
                }
                if ui.selectable_value(selected_round, Some(RoundKind::Round2), "Round 2").clicked() {
                    *selected_window = None;
                    changed = true;
                }
                if ui.selectable_value(selected_round, Some(RoundKind::Round2A), "Round 2A").clicked() {
                    *selected_window = None;
                    changed = true;
                }
            });

        if let Some(rk) = selected_round {
            ui.label("Window:");
            
            let max_win = match rk {
                RoundKind::Round1 => 1,
                RoundKind::Round1A => 3,
                RoundKind::Round1B => 2,
                RoundKind::Round2 => 3,
                RoundKind::Round2A => 3,
            };
            
            egui::ComboBox::from_id_salt("window_combo")
                .selected_text(selected_window.map(|w| format!("Window {}", w)).unwrap_or_else(|| "All".to_string()))
                .show_ui(ui, |ui| {
                    ui.selectable_value(selected_window, None, "All");
                    for w in 1..=max_win {
                        ui.selectable_value(selected_window, Some(w), format!("Window {}", w));
                    }
                });
        }
    });
}
