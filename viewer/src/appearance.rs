use eframe::egui::{self, Color32, FontId, RichText, Stroke, TextStyle};

pub const INK: Color32 = Color32::from_rgb(23, 43, 42);
pub const MUTED: Color32 = Color32::from_rgb(78, 103, 101);
pub const ACCENT: Color32 = Color32::from_rgb(18, 100, 86);
pub const LINE: Color32 = Color32::from_rgb(216, 229, 226);
pub const TINT: Color32 = Color32::from_rgb(234, 244, 241);
pub const CANVAS: Color32 = Color32::from_rgb(236, 243, 242);
pub const PAPER: Color32 = Color32::WHITE;

pub fn install(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Light);
    ctx.style_mut(|style| {
        let visuals = &mut style.visuals;
        visuals.panel_fill = PAPER;
        visuals.window_fill = PAPER;
        visuals.window_corner_radius = 12.into();
        visuals.menu_corner_radius = 8.into();
        visuals.window_stroke = Stroke::new(1_f32, LINE);
        visuals.faint_bg_color = Color32::from_rgb(246, 250, 249);
        visuals.extreme_bg_color = Color32::from_rgb(243, 248, 247);
        visuals.code_bg_color = TINT;
        visuals.hyperlink_color = ACCENT;
        visuals.selection.bg_fill = Color32::from_rgb(210, 237, 228);
        visuals.selection.stroke = Stroke::new(1_f32, INK);
        let widgets = &mut visuals.widgets;
        widgets.noninteractive.fg_stroke.color = INK;
        widgets.noninteractive.bg_stroke.color = LINE;
        widgets.inactive.fg_stroke.color = INK;
        widgets.inactive.bg_fill = TINT;
        widgets.inactive.weak_bg_fill = TINT;
        widgets.inactive.bg_stroke = Stroke::NONE;
        widgets.hovered.fg_stroke.color = INK;
        widgets.hovered.bg_fill = visuals.selection.bg_fill;
        widgets.hovered.weak_bg_fill = visuals.selection.bg_fill;
        widgets.hovered.bg_stroke = Stroke::new(1_f32, ACCENT);
        widgets.active.fg_stroke.color = INK;
        widgets.active.bg_fill = visuals.selection.bg_fill;
        widgets.active.weak_bg_fill = visuals.selection.bg_fill;
        widgets.active.bg_stroke = Stroke::new(1_f32, ACCENT);
        for widget in [
            &mut widgets.noninteractive,
            &mut widgets.inactive,
            &mut widgets.hovered,
            &mut widgets.active,
            &mut widgets.open,
        ] {
            widget.corner_radius = 7.into();
            widget.expansion = 0.;
        }
        style.spacing.button_padding = egui::vec2(10., 5.);
        style.spacing.item_spacing = egui::vec2(8., 7.);
        style
            .text_styles
            .insert(TextStyle::Heading, FontId::proportional(19.));
    });
}

pub fn panel(fill: Color32, x: i8, y: i8) -> egui::Frame {
    egui::Frame::new()
        .fill(fill)
        .inner_margin(egui::Margin::symmetric(x, y))
        .stroke(Stroke::new(1_f32, LINE))
}

pub fn badge(ui: &mut egui::Ui, text: impl Into<RichText>) {
    egui::Frame::new()
        .fill(TINT)
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(text.into().color(ACCENT).small());
        });
}
