//! Rendering. Each screen reads session state and turns interaction into session
//! intents (`handle_key` / `list_click`). Immediate-mode, so reads and calls are
//! interleaved — small bits of state are copied out before a mutating call.

use egui::{Align, Align2, CornerRadius, FontId, Layout, Rect, RichText, Sense, Stroke, vec2};
use hitair_core::session::{Key, Screen, Session};

use crate::theme::*;

const COL_W: f32 = 680.0;

pub fn draw(ui: &mut egui::Ui, session: &mut Session) {
    header(ui, session);
    hairline(ui);
    ui.add_space(8.0);

    column(ui, |ui| match session.screen {
        Screen::Menu => menu(ui, session),
        Screen::Loading => centered_note(ui, "Loading…", CORAL),
        other => placeholder(ui, session, other),
    });
}

// --- shared chrome --------------------------------------------------------

fn header(ui: &mut egui::Ui, session: &Session) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        ui.label(RichText::new("♪ hitair").color(CORAL).size(20.0).strong());
        ui.label(RichText::new("guess the song").color(MUTED).size(12.5));

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(14.0);
            let vol = format!("{}%", (session.volume * 100.0).round() as i32);
            stat(
                ui,
                if session.volume <= 0.001 {
                    "🔇"
                } else {
                    "🔊"
                },
                &vol,
                MUTED,
            );
            dot(ui);
            stat(ui, "streak", &session.streak.to_string(), GOLD);
            dot(ui);
            stat(ui, "score", &session.score.to_string(), MINT);
        });
    });
    ui.add_space(6.0);
}

fn stat(ui: &mut egui::Ui, label: &str, value: &str, color: egui::Color32) {
    ui.label(RichText::new(value).color(color).size(15.0).strong());
    ui.label(RichText::new(label).color(MUTED).size(12.0));
}

fn dot(ui: &mut egui::Ui) {
    ui.label(RichText::new("·").color(LINE).size(15.0));
}

fn hairline(ui: &mut egui::Ui) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 1.0), Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, Stroke::new(1.0, LINE));
}

/// A left-aligned, centered content column of a fixed max width.
fn column(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let avail = ui.available_width();
    let w = avail.min(COL_W);
    let pad = ((avail - w) / 2.0).max(0.0);
    ui.horizontal_top(|ui| {
        ui.add_space(pad);
        ui.allocate_ui_with_layout(
            vec2(w, ui.available_height()),
            Layout::top_down(Align::Min),
            |ui| {
                ui.set_width(w);
                add(ui);
            },
        );
    });
}

fn eyebrow(ui: &mut egui::Ui, text: &str) {
    // Track the eyebrow with spaced caps for a "channel label" feel.
    let spaced: String = text
        .chars()
        .flat_map(|c| [c, ' '])
        .collect::<String>()
        .trim_end()
        .to_string();
    ui.label(RichText::new(spaced).color(CORAL).size(12.0).strong());
}

// --- menu -----------------------------------------------------------------

fn menu(ui: &mut egui::Ui, session: &mut Session) {
    ui.add_space(18.0);
    eyebrow(ui, "SOLO");
    ui.add_space(4.0);
    ui.label(RichText::new("Tune in.").color(TEXT).size(38.0).strong());
    ui.add_space(2.0);
    ui.label(
        RichText::new(
            "Pick a category — then name the track from a snippet that grows with every miss.",
        )
        .color(MUTED)
        .size(15.0),
    );
    ui.add_space(18.0);

    // Controls: search well (left) + game-mode selector (right).
    ui.horizontal(|ui| {
        let mode_w = 230.0;
        search_well(
            ui,
            ui.available_width() - mode_w - 12.0,
            &session.menu_filter,
        );
        ui.add_space(12.0);
        mode_selector(ui, session, mode_w);
    });
    ui.add_space(14.0);

    // Category list.
    let items: Vec<String> = session.menu_items().iter().map(|it| it.label()).collect();
    let selected = session.menu_index;
    if items.is_empty() {
        ui.add_space(8.0);
        ui.label(
            RichText::new("No matches — type a Deezer playlist id or URL to play a custom list.")
                .color(MUTED)
                .italics(),
        );
        return;
    }
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, label) in items.iter().enumerate() {
                if list_row(ui, label, i == selected).clicked() {
                    clicked = Some(i);
                }
            }
        });
    if let Some(i) = clicked {
        session.list_click(i);
    }
}

fn search_well(ui: &mut egui::Ui, width: f32, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 40.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(10), WELL);
    p.rect_stroke(
        rect,
        CornerRadius::same(10),
        Stroke::new(1.0, LINE),
        egui::StrokeKind::Inside,
    );
    // Drawn magnifier icon.
    let c = egui::pos2(rect.left() + 20.0, rect.center().y);
    p.circle_stroke(c, 5.5, Stroke::new(1.6, MUTED));
    p.line_segment(
        [c + vec2(4.0, 4.0), c + vec2(8.0, 8.0)],
        Stroke::new(1.6, MUTED),
    );

    let x = rect.left() + 38.0;
    let shown = if text.is_empty() {
        "Filter categories"
    } else {
        text
    };
    let color = if text.is_empty() { MUTED } else { TEXT };
    let galley = p.text(
        egui::pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        shown,
        FontId::proportional(15.0),
        color,
    );
    if !text.is_empty() {
        p.vline(
            galley.right() + 2.0,
            (rect.center().y - 8.0)..=(rect.center().y + 8.0),
            Stroke::new(1.5, CORAL),
        );
    }
}

fn mode_selector(ui: &mut egui::Ui, session: &mut Session, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 40.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(10), PANEL);
    p.rect_stroke(
        rect,
        CornerRadius::same(10),
        Stroke::new(1.0, LINE),
        egui::StrokeKind::Inside,
    );
    p.text(
        egui::pos2(rect.left() + 14.0, rect.center().y),
        Align2::LEFT_CENTER,
        "MODE",
        FontId::proportional(10.5),
        MUTED,
    );
    let left_x = rect.left() + 64.0;
    let right_x = rect.right() - 18.0;
    p.text(
        egui::pos2((left_x + right_x) / 2.0, rect.center().y),
        Align2::CENTER_CENTER,
        session.game_mode.label(),
        FontId::proportional(14.5),
        GOLD,
    );
    if arrow(ui, left_x, rect.center().y, false).clicked() {
        session.handle_key(Key::Left);
    }
    if arrow(ui, right_x, rect.center().y, true).clicked() {
        session.handle_key(Key::Right);
    }
}

/// A small clickable triangle (drawn, so it needs no glyph coverage).
fn arrow(ui: &mut egui::Ui, cx: f32, cy: f32, right: bool) -> egui::Response {
    let r = Rect::from_center_size(egui::pos2(cx, cy), vec2(26.0, 34.0));
    let resp = ui.interact(r, ui.id().with(("arrow", cx as i32, right)), Sense::click());
    let color = if resp.hovered() { CORAL } else { MUTED };
    let (w, h) = (5.0, 6.0);
    let pts = if right {
        vec![
            egui::pos2(cx - w * 0.5, cy - h),
            egui::pos2(cx + w, cy),
            egui::pos2(cx - w * 0.5, cy + h),
        ]
    } else {
        vec![
            egui::pos2(cx + w * 0.5, cy - h),
            egui::pos2(cx - w, cy),
            egui::pos2(cx + w * 0.5, cy + h),
        ]
    };
    ui.painter()
        .add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
    resp
}

/// A full-width clickable list row. Selected rows get a coral accent + lift.
fn list_row(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
    let p = ui.painter();
    let bg = if selected {
        PANEL_HI
    } else if resp.hovered() {
        PANEL
    } else {
        INK
    };
    p.rect_filled(rect, CornerRadius::same(10), bg);
    if selected {
        let bar = Rect::from_min_size(
            rect.left_top() + vec2(0.0, 9.0),
            vec2(3.0, rect.height() - 18.0),
        );
        p.rect_filled(bar, CornerRadius::same(2), CORAL);
    }
    p.text(
        egui::pos2(rect.left() + 18.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(15.5),
        if selected { TEXT } else { MUTED },
    );
    resp
}

// --- placeholders (built out next) ---------------------------------------

fn placeholder(ui: &mut egui::Ui, _session: &Session, screen: Screen) {
    let name = match screen {
        Screen::Playing => "Playing",
        Screen::RoundEnd => "Result",
        Screen::ChallengeMenu => "Challenge",
        Screen::HostConfig => "Host a lobby",
        Screen::Browse => "Browse lobbies",
        Screen::JoinCode => "Join by code",
        Screen::Lobby => "Live lobby",
        _ => "…",
    };
    centered_note(ui, &format!("{name} — coming next"), MUTED);
}

fn centered_note(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    ui.add_space(120.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(text).color(color).size(20.0).strong());
    });
}
