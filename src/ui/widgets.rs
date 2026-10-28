use crate::data::filter::WindowSel;
use crate::data::model::Confidence;
use crate::data::{RoundKind, Term};
use crate::ui::theme;
use egui::{Color32, RichText, Ui};
use egui_plot::{GridMark, Plot};

pub fn money(x: f64) -> String {
    format!("e${:.2}", x)
}

pub fn pct(x: f64) -> String {
    format!("{:.0}%", x * 100.0)
}

/// Round + window pickers limited to the combinations in `options`.
/// Returns true when the selection changed.
pub fn window_selector(ui: &mut Ui, id: &str, options: &[(RoundKind, u8)], sel: &mut WindowSel, allow_all: bool) -> bool {
    let before = *sel;
    let mut rounds: Vec<RoundKind> = options.iter().map(|o| o.0).collect();
    rounds.dedup();

    ui.label("Round");
    egui::ComboBox::from_id_salt((id, "round"))
        .selected_text(sel.round.map_or("All rounds".to_string(), |r| r.to_string()))
        .show_ui(ui, |ui| {
            if allow_all {
                ui.selectable_value(&mut sel.round, None, "All rounds");
            }
            for r in &rounds {
                ui.selectable_value(&mut sel.round, Some(*r), r.to_string());
            }
        });
    if sel.round != before.round {
        // Default to the first window of the new round (the one with the most competition).
        sel.window = sel.round.and_then(|r| options.iter().find(|o| o.0 == r).map(|o| o.1));
        if allow_all && sel.round.is_none() {
            sel.window = None;
        }
    }

    if let Some(round) = sel.round {
        let windows: Vec<u8> = options.iter().filter(|o| o.0 == round).map(|o| o.1).collect();
        ui.label("Window");
        egui::ComboBox::from_id_salt((id, "window"))
            .selected_text(sel.window.map_or("All windows".to_string(), |w| format!("Window {w}")))
            .show_ui(ui, |ui| {
                if allow_all {
                    ui.selectable_value(&mut sel.window, None, "All windows");
                }
                for w in windows {
                    ui.selectable_value(&mut sel.window, Some(w), format!("Window {w}"));
                }
            });
    }
    *sel != before
}

/// Make sure `sel` is one of `options` (or "all" when allowed); prefer Round 1 Window 1.
pub fn normalise_selection(options: &[(RoundKind, u8)], sel: &mut WindowSel, allow_all: bool) {
    let valid = match (sel.round, sel.window) {
        (Some(r), Some(w)) => options.contains(&(r, w)),
        (Some(r), None) => allow_all && options.iter().any(|o| o.0 == r),
        (None, _) => allow_all,
    };
    if !valid {
        let pick = options.iter().find(|o| **o == (RoundKind::Round1, 1)).or(options.first());
        *sel = pick.map_or(WindowSel::default(), |&(r, w)| WindowSel { round: Some(r), window: Some(w) });
    }
}

pub fn stat_card(ui: &mut Ui, title: &str, value: impl Into<String>, subtitle: impl Into<String>, color: Option<Color32>) {
    theme::card(ui).show(ui, |ui| {
        ui.set_min_width(150.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(title).small().color(theme::muted(ui)));
            let mut v = RichText::new(value.into()).size(22.0).strong();
            if let Some(c) = color {
                v = v.color(c);
            }
            ui.label(v);
            let sub = subtitle.into();
            if !sub.is_empty() {
                ui.label(RichText::new(sub).small().color(theme::muted(ui)));
            }
        });
    });
}

pub fn confidence_badge(ui: &mut Ui, c: Confidence) {
    let color = match c {
        Confidence::High => theme::GOOD,
        Confidence::Medium => theme::WARN,
        Confidence::Low => theme::BAD,
    };
    egui::Frame::new()
        .fill(color.linear_multiply(0.18))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(format!("{} confidence", c.label())).small().strong().color(color));
        });
}

pub fn section_heading(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).size(16.0).strong());
    ui.add_space(2.0);
}

/// A plot whose x axis is a list of terms placed at 0, 1, 2, ...
pub fn term_plot<'a>(id: &str, terms: Vec<Term>, extra_label: Option<String>) -> Plot<'a> {
    let n = terms.len() + extra_label.is_some() as usize;
    let labels: Vec<String> = terms.iter().map(|t| t.label()).chain(extra_label).collect();
    Plot::new(id)
        .allow_scroll(false)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_boxed_zoom(false)
        .x_grid_spacer(move |_| (0..n).map(|i| GridMark { value: i as f64, step_size: 1.0 }).collect())
        .x_axis_formatter(move |mark, _| {
            let i = mark.value.round();
            if (mark.value - i).abs() < 1e-6 && i >= 0.0 {
                labels.get(i as usize).cloned().unwrap_or_default()
            } else {
                String::new()
            }
        })
        .y_axis_formatter(|mark, _| format!("{:.0}", mark.value))
}
