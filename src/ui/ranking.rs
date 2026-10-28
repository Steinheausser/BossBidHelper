use crate::data::filter::{all_windows, GlobalFilters, WindowSel};
use crate::data::query::{rank_instructors, InstructorQuery, InstructorSort, DEFAULT_MIN_COMPARED};
use crate::data::stats::InstructorStats;
use crate::ui::overview::premium_label;
use crate::ui::widgets::{money, pct, window_selector};
use crate::ui::{theme, Ctx};
use egui::{RichText, Ui};

/// Everything the ranking depends on; the list is recomputed only when one of these changes.
#[derive(PartialEq)]
struct CacheKey {
    filters: GlobalFilters,
    sel: WindowSel,
    school: Option<String>,
    course: Option<String>,
    min_compared: usize,
    sort: InstructorSort,
    search: String,
}

pub struct RankingState {
    sel: WindowSel,
    school: Option<String>,
    this_course_only: bool,
    min_compared: usize,
    sort: InstructorSort,
    search: String,
    cache: Option<(CacheKey, Vec<InstructorStats>)>,
}

impl Default for RankingState {
    fn default() -> Self {
        Self {
            sel: WindowSel::ROUND1_W1,
            school: None,
            this_course_only: false,
            min_compared: DEFAULT_MIN_COMPARED,
            sort: InstructorSort::MostInDemand,
            search: String::new(),
            cache: None,
        }
    }
}

impl RankingState {
    pub fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let data = cx.data;
        ui.heading("Instructor demand");
        ui.label(
            RichText::new(
                "Each section is compared with the other sections of the same course, term and window, so popular courses don't drown out popular instructors. \
                 +40% means getting into their section typically cost 40% more than the course's other sections. Scores are shrunk toward 0 when there are few comparisons.",
            )
            .color(theme::muted(ui)),
        );
        ui.add_space(4.0);

        let course = cx.selection.course.clone();
        theme::card(ui).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                window_selector(ui, "ranking", &all_windows(data), &mut self.sel, true);
                ui.separator();
                ui.label("School");
                egui::ComboBox::from_id_salt("ranking_school")
                    .selected_text(self.school.as_deref().unwrap_or("All"))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.school, None, "All");
                        for s in &data.schools {
                            ui.selectable_value(&mut self.school, Some(s.to_string()), &**s);
                        }
                    });
                if let Some(c) = &course {
                    ui.separator();
                    ui.checkbox(&mut self.this_course_only, format!("Only {c}"));
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Sort");
                ui.selectable_value(&mut self.sort, InstructorSort::MostInDemand, "Most in demand");
                ui.selectable_value(&mut self.sort, InstructorSort::EasiestToGet, "Easiest to get");
                ui.selectable_value(&mut self.sort, InstructorSort::ClearingPrice, "Clearing price");
                ui.selectable_value(&mut self.sort, InstructorSort::Sections, "Most sections");
                ui.separator();
                ui.label("Min. comparisons");
                ui.add(egui::DragValue::new(&mut self.min_compared).range(0..=30));
                ui.separator();
                ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("🔍 Instructor name").desired_width(180.0));
            });
        });

        let course_filter = if self.this_course_only { course } else { None };
        let key = CacheKey {
            filters: cx.filters.clone(),
            sel: self.sel,
            school: self.school.clone(),
            course: course_filter.clone(),
            min_compared: self.min_compared,
            sort: self.sort,
            search: self.search.clone(),
        };
        if self.cache.as_ref().is_none_or(|(k, _)| *k != key) {
            let query = InstructorQuery {
                sel: self.sel,
                school: self.school.as_deref(),
                course: course_filter.as_deref(),
                search: Some(self.search.as_str()),
                min_compared: self.min_compared,
                sort: self.sort,
            };
            let list = rank_instructors(data, cx.filters, &query);
            self.cache = Some((key, list));
        }
        let list = &self.cache.as_ref().unwrap().1;

        ui.label(RichText::new(format!("{} instructors", list.len())).small().color(theme::muted(ui)));
        let max_abs = list.iter().map(|s| s.premium.abs()).fold(0.1, f64::max);
        egui::ScrollArea::both().auto_shrink([true, false]).show(ui, |ui| {
            egui::Grid::new("ranking_grid").striped(true).min_col_width(40.0).show(ui, |ui| {
                for h in ["#", "Instructor", "School", "vs. peers", "", "Compared", "Sections", "Filled", "Median clearing", "Courses", "Last taught"] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();
                for (i, s) in list.iter().take(500).enumerate() {
                    ui.label((i + 1).to_string());
                    ui.label(&s.name);
                    ui.label(&s.school);
                    premium_label(ui, s.premium, s.compared);
                    premium_bar(ui, s.premium, max_abs);
                    ui.label(s.compared.to_string());
                    ui.label(s.sections.to_string());
                    ui.label(pct(s.fill_rate));
                    ui.label(s.median_clearing.map(money).unwrap_or_else(|| "–".into()));
                    ui.label(s.courses.to_string());
                    ui.label(s.last_term.label());
                    ui.end_row();
                }
            });
        });
    }
}

fn premium_bar(ui: &mut Ui, premium: f64, max_abs: f64) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(80.0, 12.0), egui::Sense::hover());
    let mid = rect.center().x;
    let half = rect.width() / 2.0;
    let len = (premium / max_abs).clamp(-1.0, 1.0) as f32 * half;
    let color = if premium >= 0.0 { theme::BAD } else { theme::GOOD };
    let bar = egui::Rect::from_min_max(egui::pos2(mid.min(mid + len), rect.top() + 2.0), egui::pos2(mid.max(mid + len), rect.bottom() - 2.0));
    ui.painter().line_segment([egui::pos2(mid, rect.top()), egui::pos2(mid, rect.bottom())], egui::Stroke::new(1.0, theme::muted(ui)));
    ui.painter().rect_filled(bar, 2.0, color.linear_multiply(0.8));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_minimum_comparisons_start_where_the_cli_starts() {
        assert_eq!(RankingState::default().min_compared, 10);
        assert_eq!(RankingState::default().min_compared, DEFAULT_MIN_COMPARED);
    }
}
