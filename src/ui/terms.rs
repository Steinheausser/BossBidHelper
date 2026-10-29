use crate::data::filter::{all_windows, GlobalFilters, WindowSel};
use crate::data::query::term_overview;
use crate::data::stats::TermOverview;
use crate::ui::widgets::{money, pct, section_heading, stat_card, term_plot, window_selector};
use crate::ui::{memo, theme, Ctx};
use egui::{Color32, RichText, Ui};
use egui_plot::{Bar, BarChart, Corner, Legend};

/// Everything the table depends on; it is recomputed only when one of these changes.
#[derive(PartialEq)]
struct CacheKey {
    filters: GlobalFilters,
    sel: WindowSel,
    school: Option<String>,
}

#[derive(Default)]
pub struct TermsState {
    sel: WindowSel,
    school: Option<String>,
    cache: Option<(CacheKey, Vec<TermOverview>)>,
}

impl TermsState {
    pub fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let data = cx.data;
        ui.heading("Terms at a glance");
        ui.label(
            RichText::new(
                "How much was on offer each term across every course: classes, instructors and seats, and how much of it bidding took up. \
                 The BOSS results have no class days or times, so the timetable isn't shown.",
            )
            .color(theme::muted(ui)),
        );
        ui.add_space(4.0);

        theme::card(ui).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                window_selector(ui, "terms", &all_windows(data), &mut self.sel, true);
                ui.separator();
                ui.label("School");
                egui::ComboBox::from_id_salt("terms_school")
                    .selected_text(self.school.as_deref().unwrap_or("All"))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.school, None, "All");
                        for s in &data.schools {
                            ui.selectable_value(&mut self.school, Some(s.to_string()), &**s);
                        }
                    });
            });
        });

        let key = CacheKey { filters: cx.filters.clone(), sel: self.sel, school: self.school.clone() };
        let terms = memo(&mut self.cache, key, |k| term_overview(data, &k.filters, k.sel, k.school.as_deref()));
        let Some(last) = terms.last() else {
            ui.label("No sections match these filters.");
            return;
        };

        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            stat_card(ui, &format!("Courses · {}", last.term), last.courses.to_string(), format!("{} sections", last.sections), None);
            stat_card(ui, "Instructors", last.instructors.to_string(), "", None);
            stat_card(ui, "Class seats", last.class_seats.to_string(), format!("{} open in the first window", last.first_window_seats), None);
            stat_card(ui, "Seats won by bids", last.seats_won.to_string(), format!("{} left unclaimed", last.seats_unclaimed), Some(theme::ACCENT));
            stat_card(ui, "Sections that filled", pct(last.fill_rate), format!("{} of {} section-windows", last.filled, last.available), None);
        });

        section_heading(ui, "Seats per term");
        let labels: Vec<_> = terms.iter().map(|t| t.term).collect();
        let muted = theme::muted(ui);
        let bars = |name: &str, color: Color32, offset: f64, value: fn(&TermOverview) -> i64| {
            let bars = terms.iter().enumerate().map(|(i, t)| Bar::new(i as f64 + offset, value(t) as f64).width(0.25)).collect();
            BarChart::new(bars).name(name).color(color)
        };
        // Headroom above the tallest bar keeps the legend clear of the bars.
        let tallest = terms.iter().map(|t| t.first_window_seats.max(t.seats_won)).max().unwrap_or(0) as f64;
        term_plot("terms_seats", labels, None)
            .height(220.0)
            .legend(Legend::default().position(Corner::LeftTop))
            .include_y(0.0)
            .include_y(tallest * 1.6)
            .label_formatter(|name, v| if name.is_empty() { String::new() } else { format!("{name}\n{:.0} seats", v.y) })
            .show(ui, |plot| {
                plot.bar_chart(bars("Open in first window", muted, -0.27, |t| t.first_window_seats));
                plot.bar_chart(bars("Won by bids", theme::ACCENT, 0.0, |t| t.seats_won));
                plot.bar_chart(bars("Left unclaimed", theme::WARN, 0.27, |t| t.seats_unclaimed));
            });

        section_heading(ui, "Every term");
        egui::ScrollArea::both().auto_shrink([true, false]).show(ui, |ui| {
            egui::Grid::new("terms_grid").striped(true).min_col_width(40.0).show(ui, |ui| {
                let headers = [
                    ("Term", ""),
                    ("Courses", ""),
                    ("Sections", "Classes: a course's section in one session, counted once however many windows it was in."),
                    ("Instructors", ""),
                    ("Windows", "Bidding windows held."),
                    ("Class seats", "Class sizes added up."),
                    ("1st window", "Seats open in each section's first bidding window."),
                    ("Won", "Seats taken by bids, over every window."),
                    ("Unclaimed", "Seats still open after each section's last window."),
                    ("Filled", "Share of section-windows with seats open that filled."),
                    ("Median clearing", "Median clearing price of the section-windows that filled."),
                ];
                for (h, tip) in headers {
                    let label = ui.label(RichText::new(h).strong());
                    if !tip.is_empty() {
                        label.on_hover_text(tip);
                    }
                }
                ui.end_row();
                for t in terms.iter().rev() {
                    ui.label(t.term.label());
                    ui.label(t.courses.to_string());
                    ui.label(t.sections.to_string());
                    ui.label(t.instructors.to_string());
                    ui.label(t.windows.to_string());
                    ui.label(t.class_seats.to_string());
                    ui.label(t.first_window_seats.to_string());
                    ui.label(t.seats_won.to_string());
                    ui.label(t.seats_unclaimed.to_string());
                    ui.label(pct(t.fill_rate));
                    ui.label(t.clearing_median.map(money).unwrap_or_else(|| "–".into()));
                    ui.end_row();
                }
            });
        });
    }
}
