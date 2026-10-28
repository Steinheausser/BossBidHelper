use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Theme};

pub const ACCENT: Color32 = Color32::from_rgb(88, 145, 247);
pub const GOOD: Color32 = Color32::from_rgb(63, 185, 80);
pub const WARN: Color32 = Color32::from_rgb(222, 160, 50);
pub const BAD: Color32 = Color32::from_rgb(240, 90, 80);
pub const PURPLE: Color32 = Color32::from_rgb(170, 120, 240);

pub fn apply(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Heading, FontId::new(22.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(14.5, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(14.5, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(13.5, FontFamily::Monospace)),
            (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size.y = 26.0;
        let v = &mut style.visuals;
        v.selection.bg_fill = ACCENT.linear_multiply(0.55);
        v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        v.hyperlink_color = ACCENT;
        v.window_corner_radius = CornerRadius::same(10);
        v.menu_corner_radius = CornerRadius::same(8);
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open, &mut v.widgets.noninteractive] {
            w.corner_radius = CornerRadius::same(6);
        }
    });
    ctx.style_mut_of(Theme::Dark, |s| {
        s.visuals.panel_fill = Color32::from_rgb(24, 26, 31);
        s.visuals.window_fill = Color32::from_rgb(30, 33, 39);
        s.visuals.extreme_bg_color = Color32::from_rgb(17, 18, 22);
        s.visuals.faint_bg_color = Color32::from_rgb(32, 35, 42);
    });
    ctx.style_mut_of(Theme::Light, |s| {
        s.visuals.panel_fill = Color32::from_rgb(246, 247, 250);
        s.visuals.faint_bg_color = Color32::from_rgb(236, 239, 245);
    });
}

/// A rounded panel used for stat cards and sections.
pub fn card(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(12))
}

pub fn muted(ui: &egui::Ui) -> Color32 {
    ui.visuals().weak_text_color()
}
