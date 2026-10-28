use crate::data::query::search_courses;
use crate::data::AppData;
use crate::ui::theme;
use egui::{RichText, Ui};

/// Fuzzy course search shown on the left of every tab.
#[derive(Default)]
pub struct Sidebar {
    query: String,
    last_query: Option<String>,
    results: Vec<usize>,
    school: Option<String>,
}

impl Sidebar {
    pub fn reset(&mut self) {
        self.last_query = None;
    }

    fn refresh(&mut self, data: &AppData) {
        let key = format!("{}\u{0}{}", self.query, self.school.as_deref().unwrap_or(""));
        if self.last_query.as_deref() == Some(&key) {
            return;
        }
        self.last_query = Some(key);

        self.results = search_courses(data, &self.query, self.school.as_deref());
    }

    /// Returns the course code the user clicked, if any.
    pub fn ui(&mut self, ui: &mut Ui, data: &AppData, selected: Option<&str>) -> Option<String> {
        let mut clicked = None;
        ui.add_space(8.0);
        ui.label(RichText::new("Courses").size(16.0).strong());
        ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .hint_text("🔍 Search code or title…")
                .desired_width(f32::INFINITY),
        );
        ui.horizontal(|ui| {
            ui.label(RichText::new("School").small());
            egui::ComboBox::from_id_salt("sidebar_school")
                .selected_text(self.school.as_deref().unwrap_or("All"))
                .width(ui.available_width() - 8.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.school, None, "All");
                    for s in &data.schools {
                        ui.selectable_value(&mut self.school, Some(s.to_string()), &**s);
                    }
                });
        });
        self.refresh(data);
        ui.label(RichText::new(format!("{} courses", self.results.len())).small().color(theme::muted(ui)));
        ui.separator();

        let latest = data.latest_term();
        let row_height = 40.0;
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, row_height, self.results.len(), |ui, range| {
            for &i in &self.results[range] {
                let c = &data.courses[i];
                let is_selected = selected == Some(&*c.code);
                let stale = latest.is_some_and(|t| c.last_term.year + 1 < t.year);
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), row_height - 4.0), egui::Sense::click());
                if is_selected {
                    ui.painter().rect_filled(rect, 6.0, theme::ACCENT.linear_multiply(0.22));
                    ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())), 2.0, theme::ACCENT);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 6.0, ui.visuals().widgets.hovered.weak_bg_fill);
                }
                let code_color = if is_selected { theme::ACCENT } else { ui.visuals().strong_text_color() };
                ui.painter().text(
                    rect.left_top() + egui::vec2(8.0, 3.0),
                    egui::Align2::LEFT_TOP,
                    &*c.code,
                    egui::FontId::proportional(14.0),
                    code_color,
                );
                let mut desc = c.description.clone();
                if desc.chars().count() > 46 {
                    desc = desc.chars().take(44).collect::<String>() + "…";
                }
                if stale {
                    desc = format!("{desc}  (last {})", c.last_term);
                }
                ui.painter().text(
                    rect.left_top() + egui::vec2(8.0, 20.0),
                    egui::Align2::LEFT_TOP,
                    desc,
                    egui::FontId::proportional(11.5),
                    theme::muted(ui),
                );
                let resp = resp.on_hover_text(format!("{} — {}\n{} · last offered {}", c.code, c.description, c.school, c.last_term));
                if resp.clicked() {
                    clicked = Some(c.code.to_string());
                }
            }
        });
        clicked
    }
}
