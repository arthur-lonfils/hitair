//! The hitair visual identity: an after-hours, analog-audio palette and the
//! egui style derived from it. Deep plum-ink field, a warm coral "signal" glow
//! as the accent, mint for success — deliberately not the stock dark-UI look.

use egui::{Color32, CornerRadius, FontId, Margin, Stroke, TextStyle, vec2};

pub const INK: Color32 = Color32::from_rgb(0x15, 0x10, 0x19); // background
pub const WELL: Color32 = Color32::from_rgb(0x0F, 0x0B, 0x14); // deepest (inputs)
pub const PANEL: Color32 = Color32::from_rgb(0x20, 0x1A, 0x2C); // surface
pub const PANEL_HI: Color32 = Color32::from_rgb(0x2C, 0x24, 0x3D); // hover surface
pub const LINE: Color32 = Color32::from_rgb(0x38, 0x2F, 0x4C); // hairlines
pub const CORAL: Color32 = Color32::from_rgb(0xFF, 0x6A, 0x5D); // primary "signal"
pub const MINT: Color32 = Color32::from_rgb(0x5F, 0xD6, 0xA6); // success / correct
pub const GOLD: Color32 = Color32::from_rgb(0xF5, 0xC5, 0x6B); // points / streak
pub const ROSE: Color32 = Color32::from_rgb(0xE8, 0x65, 0x7F); // wrong / bad
pub const TEXT: Color32 = Color32::from_rgb(0xF3, 0xED, 0xF7); // primary text
pub const MUTED: Color32 = Color32::from_rgb(0x92, 0x86, 0xA6); // secondary text

/// Apply the hitair style to an egui context (call once at startup).
pub fn apply(ctx: &egui::Context) {
    fonts(ctx);
    ctx.all_styles_mut(build);
}

/// A display-face `FontId` (Space Grotesk) for headings and song titles.
pub fn display(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("display".into()))
}

/// Embed Inter (UI/body) and Space Grotesk (display), keeping egui's default
/// fonts as fallbacks so symbols/emoji (♪, 🔊) still resolve.
fn fonts(ctx: &egui::Context) {
    use egui::{FontData, FontDefinitions, FontFamily};
    use std::sync::Arc;

    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "inter".to_owned(),
        Arc::new(FontData::from_static(include_bytes!("../assets/Inter.ttf"))),
    );
    fonts.font_data.insert(
        "space".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/SpaceGrotesk.ttf"
        ))),
    );
    // Inter first for proportional text (default fonts remain as fallback).
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "inter".to_owned());
    // A dedicated display family: Space Grotesk, falling back to Inter.
    fonts.families.insert(
        FontFamily::Name("display".into()),
        vec!["space".to_owned(), "inter".to_owned()],
    );
    ctx.set_fonts(fonts);
}

fn build(style: &mut egui::Style) {
    // A deliberate type scale.
    use egui::FontFamily::{Monospace, Proportional};
    style.text_styles = [
        (TextStyle::Heading, FontId::new(30.0, Proportional)),
        (TextStyle::Body, FontId::new(15.0, Proportional)),
        (TextStyle::Button, FontId::new(15.0, Proportional)),
        (TextStyle::Small, FontId::new(12.5, Proportional)),
        (TextStyle::Monospace, FontId::new(14.0, Monospace)),
    ]
    .into();

    let s = &mut style.spacing;
    s.item_spacing = vec2(10.0, 10.0);
    s.button_padding = vec2(14.0, 9.0);
    s.window_margin = Margin::same(0);
    s.menu_margin = Margin::same(8);
    s.interact_size.y = 30.0;
    s.scroll.bar_width = 8.0;

    let v = &mut style.visuals;
    v.dark_mode = true;
    v.override_text_color = Some(TEXT);
    v.panel_fill = INK;
    v.window_fill = PANEL;
    v.extreme_bg_color = WELL;
    v.faint_bg_color = PANEL;
    v.hyperlink_color = CORAL;
    v.selection.bg_fill = CORAL.gamma_multiply(0.28);
    v.selection.stroke = Stroke::new(1.0, CORAL);
    v.window_stroke = Stroke::new(1.0, LINE);

    let round = CornerRadius::same(10);
    let w = &mut v.widgets;
    for wv in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        wv.corner_radius = round;
    }
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    w.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);

    w.inactive.bg_fill = PANEL;
    w.inactive.weak_bg_fill = PANEL;
    w.inactive.bg_stroke = Stroke::new(1.0, LINE);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);

    w.hovered.bg_fill = PANEL_HI;
    w.hovered.weak_bg_fill = PANEL_HI;
    w.hovered.bg_stroke = Stroke::new(1.2, CORAL.gamma_multiply(0.7));
    w.hovered.fg_stroke = Stroke::new(1.0, TEXT);

    w.active.bg_fill = CORAL;
    w.active.weak_bg_fill = CORAL;
    w.active.bg_stroke = Stroke::new(1.0, CORAL);
    w.active.fg_stroke = Stroke::new(1.0, INK);
}
