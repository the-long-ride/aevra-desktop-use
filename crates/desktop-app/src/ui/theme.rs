//! Native equivalents of apps/web-react/src/styles/tokens.css.
use eframe::egui::{self, Color32, FontFamily, FontId, Stroke, TextStyle};

pub struct Palette {
    pub canvas: Color32,
    pub soft: Color32,
    pub card: Color32,
    pub ink: Color32,
    pub muted: Color32,
    pub line: Color32,
    pub accent: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
}

pub fn palette(dark: bool) -> Palette {
    let rgb = Color32::from_rgb;
    if dark {
        Palette {
            canvas: rgb(23, 21, 21),
            soft: rgb(30, 27, 27),
            card: rgb(41, 37, 37),
            ink: rgb(253, 252, 252),
            muted: rgb(170, 165, 165),
            line: rgb(64, 60, 60),
            accent: rgb(10, 132, 255),
            success: rgb(104, 213, 133),
            warning: rgb(255, 179, 64),
            danger: rgb(255, 112, 103),
        }
    } else {
        Palette {
            canvas: rgb(253, 252, 252),
            soft: rgb(248, 247, 247),
            card: rgb(241, 238, 238),
            ink: rgb(32, 29, 29),
            muted: rgb(100, 98, 98),
            line: rgb(224, 222, 222),
            accent: rgb(0, 122, 255),
            success: rgb(29, 113, 55),
            warning: rgb(142, 84, 0),
            danger: rgb(185, 35, 28),
        }
    }
}

pub fn colors(ui: &egui::Ui) -> Palette {
    palette(ui.visuals().dark_mode)
}

pub fn install(ctx: &egui::Context) {
    for (theme, dark) in [(egui::Theme::Light, false), (egui::Theme::Dark, true)] {
        let p = palette(dark);
        let mut style = egui::Style {
            visuals: if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            },
            ..egui::Style::default()
        };
        let v = &mut style.visuals;
        v.panel_fill = p.canvas;
        v.window_fill = p.canvas;
        v.extreme_bg_color = p.soft;
        v.faint_bg_color = p.soft;
        v.override_text_color = Some(p.ink);
        v.weak_text_color = Some(p.muted);
        v.selection.bg_fill = p.card;
        v.selection.stroke = Stroke::new(1.0, p.accent);
        v.hyperlink_color = p.accent;
        v.warn_fg_color = p.warning;
        v.error_fg_color = p.danger;
        v.window_corner_radius = 4.into();
        v.window_stroke = Stroke::new(1.0, p.line);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.line);
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.ink);
        for widget in [
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            widget.corner_radius = 4.into();
            widget.fg_stroke = Stroke::new(1.0, p.ink);
            widget.bg_stroke = Stroke::new(1.0, p.muted);
            widget.bg_fill = p.soft;
            widget.weak_bg_fill = p.soft;
        }
        v.widgets.hovered.bg_fill = p.card;
        v.widgets.hovered.weak_bg_fill = p.card;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.ink);
        v.widgets.active.bg_stroke = Stroke::new(1.5, p.accent);
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
        style.spacing.interact_size = egui::vec2(32.0, 32.0);
        style.text_styles = [
            (TextStyle::Heading, 22.0),
            (TextStyle::Body, 13.0),
            (TextStyle::Button, 13.0),
            (TextStyle::Small, 11.0),
            (TextStyle::Monospace, 12.0),
        ]
        .into_iter()
        .map(|(key, size)| (key, FontId::new(size, FontFamily::Monospace)))
        .collect();
        ctx.set_style_of(theme, style);
    }
}

pub fn section(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.heading(title);
    ui.label(egui::RichText::new(description).color(colors(ui).muted));
    ui.add_space(8.0);
}

pub fn badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .stroke(Stroke::new(1.0, colors(ui).line))
        .inner_margin(egui::Margin::symmetric(6, 3))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(11.0).color(color));
        });
}

pub fn empty(ui: &mut egui::Ui, title: &str, detail: &str) {
    ui.add_space(32.0);
    ui.vertical_centered(|ui| {
        ui.strong(title);
        ui.label(egui::RichText::new(detail).color(colors(ui).muted));
    });
}

/// A full-width content group; spacing separates tasks, borders bind related controls.
pub fn group(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(colors(ui).canvas)
        .stroke(Stroke::new(1.0, colors(ui).line))
        .corner_radius(4)
        .inner_margin(16)
}

pub fn group_title(ui: &mut egui::Ui, title: &str) {
    ui.label(egui::RichText::new(title).size(16.0).strong());
    ui.add_space(4.0);
}
