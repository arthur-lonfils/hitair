//! Rendering. Each screen reads session state and turns interaction into session
//! intents (`handle_key` / `list_click`). Immediate-mode, so reads and calls are
//! interleaved — small bits of state are copied out before a mutating call.

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense, Stroke,
    StrokeKind, pos2, vec2,
};
use hitair_core::game::{GameMode, GuessLog, Outcome};
use hitair_core::session::{Key, Screen, Session};

use crate::theme::*;

const COL_W: f32 = 680.0;

pub fn draw(ui: &mut egui::Ui, session: &mut Session) {
    header(ui, session);
    hairline(ui);
    ui.add_space(8.0);

    column(ui, |ui| match session.screen {
        Screen::Menu => menu(ui, session),
        Screen::Loading => loading(ui),
        Screen::Playing => playing(ui, session),
        Screen::RoundEnd => result(ui, session),
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
    ui.label(RichText::new("Tune in.").color(TEXT).font(display(46.0)));
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

// --- loading --------------------------------------------------------------

fn loading(ui: &mut egui::Ui) {
    ui.add_space(150.0);
    ui.vertical_centered(|ui| {
        let t = ui.input(|i| i.time) as f32;
        let (rect, _) = ui.allocate_exact_size(vec2(90.0, 18.0), Sense::hover());
        let p = ui.painter();
        for i in 0..3 {
            let a = ((t * 3.2 - i as f32 * 0.6).sin() * 0.5 + 0.5).clamp(0.2, 1.0);
            let x = rect.center().x - 22.0 + i as f32 * 22.0;
            p.circle_filled(pos2(x, rect.center().y), 5.0, CORAL.gamma_multiply(a));
        }
        ui.add_space(16.0);
        ui.label(RichText::new("Finding a track…").color(MUTED).size(14.0));
    });
}

// --- playing (the signature reveal meter) ---------------------------------

fn playing(ui: &mut egui::Ui, session: &mut Session) {
    // Copy everything we need before any mutating session call.
    let Some(round) = session.round.as_ref() else {
        return;
    };
    let level = round.guess_number().saturating_sub(1);
    let total = round.total_levels();
    let clip_label = round.current_clip_label();
    let clip_secs = round.current_clip().as_secs_f32();
    let guesses = round.guesses.clone();
    let started = session.play_started_at;
    let mode = session.game_mode;
    let input = session.input.clone();
    let sel = session.suggestion_index;
    let sugg: Vec<(String, String)> = session
        .suggestions
        .iter()
        .map(|t| (t.title.clone(), t.artist_name().to_string()))
        .collect();

    ui.add_space(16.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("Guess {}/{}", level + 1, total))
                .color(CORAL)
                .size(15.0)
                .strong(),
        );
        ui.label(
            RichText::new(format!("· clip {clip_label}"))
                .color(MUTED)
                .size(14.0),
        );
        if mode != GameMode::Normal {
            chip(ui, mode.label(), GOLD);
        }
        if !session.audio_available {
            chip(ui, "no audio device", ROSE);
        }
    });
    ui.add_space(14.0);

    reveal_meter(ui, level, total, started, clip_secs);
    ui.add_space(16.0);

    ui.horizontal(|ui| {
        if primary_button(ui, "Replay").clicked() {
            session.handle_key(Key::Ctrl('r'));
        }
        if ghost_button(ui, "Skip").clicked() {
            session.handle_key(Key::Tab);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ghost_button(ui, "Vol +").clicked() {
                session.handle_key(Key::CtrlUp);
            }
            if ghost_button(ui, "Vol -").clicked() {
                session.handle_key(Key::CtrlDown);
            }
        });
    });
    ui.add_space(16.0);

    guesses_row(ui, &guesses);
    ui.add_space(10.0);
    input_well(ui, &input);
    ui.add_space(10.0);

    if sugg.is_empty() {
        let hint = if input.trim().chars().count() < 2 {
            "Start typing to search Deezer…"
        } else {
            "No matches — keep typing."
        };
        ui.label(RichText::new(hint).color(MUTED).size(13.5).italics());
        return;
    }
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, (title, artist)) in sugg.iter().enumerate() {
                if suggestion_row(ui, title, artist, i == sel).clicked() {
                    clicked = Some(i);
                }
            }
        });
    if let Some(i) = clicked {
        session.list_click(i);
    }
}

/// The signature: a reveal meter that grows as clips unlock, with a live
/// playhead sweeping the current clip. Segment ticks mark the guess levels.
fn reveal_meter(
    ui: &mut egui::Ui,
    level: usize,
    total: usize,
    started: Option<std::time::Instant>,
    clip_secs: f32,
) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
    let p = ui.painter();
    let r = CornerRadius::same(10);
    p.rect_filled(rect, r, WELL);
    p.rect_stroke(rect, r, Stroke::new(1.0, LINE), StrokeKind::Inside);

    let n = total.max(1) as f32;
    let frac = started
        .map(|s| (s.elapsed().as_secs_f32() / clip_secs.max(0.01)).clamp(0.0, 1.0))
        .unwrap_or(0.0);
    let fill_units = level as f32 + frac;
    let fill_x = rect.left() + (fill_units / n) * rect.width();
    let fill_rect = Rect::from_min_max(rect.min, pos2(fill_x.max(rect.left()), rect.max.y));

    // Soft glow, then the coral fill.
    p.rect_filled(fill_rect.expand(2.5), r, CORAL.gamma_multiply(0.16));
    p.rect_filled(fill_rect, r, CORAL);

    // Segment ticks (darker over the fill, faint over the track).
    for i in 1..total {
        let x = rect.left() + (i as f32 / n) * rect.width();
        p.vline(
            x,
            (rect.top() + 3.0)..=(rect.bottom() - 3.0),
            Stroke::new(1.0, Color32::from_black_alpha(70)),
        );
    }
    // Playhead.
    if fill_x > rect.left() + 1.0 {
        p.vline(
            fill_x,
            rect.top()..=rect.bottom(),
            Stroke::new(2.0, Color32::WHITE),
        );
        p.circle_filled(pos2(fill_x, rect.center().y), 3.5, Color32::WHITE);
    }
}

fn input_well(ui: &mut egui::Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::hover());
    let p = ui.painter();
    let r = CornerRadius::same(12);
    p.rect_filled(rect, r, WELL);
    p.rect_stroke(
        rect,
        r,
        Stroke::new(1.2, CORAL.gamma_multiply(0.5)),
        StrokeKind::Inside,
    );
    let x = rect.left() + 16.0;
    let shown = if text.is_empty() {
        "Name the track…"
    } else {
        text
    };
    let color = if text.is_empty() { MUTED } else { TEXT };
    let galley = p.text(
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        shown,
        FontId::proportional(16.0),
        color,
    );
    let caret_x = if text.is_empty() {
        x
    } else {
        galley.right() + 2.0
    };
    p.vline(
        caret_x,
        (rect.center().y - 10.0)..=(rect.center().y + 10.0),
        Stroke::new(1.5, CORAL),
    );
}

fn guesses_row(ui: &mut egui::Ui, guesses: &[GuessLog]) {
    if guesses.is_empty() {
        ui.label(RichText::new("No guesses yet.").color(MUTED).size(13.0));
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for g in guesses {
            match g {
                GuessLog::Wrong(name) => chip(ui, &format!("× {name}"), ROSE),
                GuessLog::Skipped => chip(ui, "skipped", MUTED),
            }
        }
    });
}

fn suggestion_row(ui: &mut egui::Ui, title: &str, artist: &str, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::click());
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
            rect.left_top() + vec2(0.0, 8.0),
            vec2(3.0, rect.height() - 16.0),
        );
        p.rect_filled(bar, CornerRadius::same(2), CORAL);
    }
    p.text(
        pos2(rect.left() + 18.0, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(15.0),
        if selected { TEXT } else { MUTED },
    );
    p.text(
        pos2(rect.right() - 16.0, rect.center().y),
        Align2::RIGHT_CENTER,
        artist,
        FontId::proportional(13.0),
        MUTED,
    );
    resp
}

// --- result ---------------------------------------------------------------

fn result(ui: &mut egui::Ui, session: &mut Session) {
    let Some(round) = session.round.as_ref() else {
        return;
    };
    let won = round.outcome == Outcome::Won;
    let guesses = round.guess_number();
    let title = round.answer.title.clone();
    let artist = round.answer.artist_name().to_string();
    let album = round.answer.album_title().map(str::to_string);
    let points = session.last_points;

    ui.add_space(30.0);
    let (verdict, color) = if won {
        (
            format!(
                "Nailed it — {guesses} {}.",
                plural("guess", "guesses", guesses)
            ),
            MINT,
        )
    } else {
        ("Out of guesses.".to_string(), ROSE)
    };
    ui.label(RichText::new(verdict).color(color).size(16.0).strong());
    ui.add_space(16.0);

    ui.label(
        RichText::new("THE SONG WAS")
            .color(MUTED)
            .size(11.5)
            .strong(),
    );
    ui.add_space(6.0);
    ui.label(RichText::new(title).color(TEXT).font(display(40.0)));
    ui.add_space(2.0);
    ui.label(RichText::new(artist).color(CORAL).size(19.0));
    if let Some(album) = album {
        ui.add_space(2.0);
        ui.label(RichText::new(album).color(MUTED).size(14.0));
    }

    if won {
        ui.add_space(18.0);
        let t = session
            .round_end_at
            .map(|s| s.elapsed().as_secs_f32())
            .unwrap_or(9.0);
        let pop = 1.0 + (1.0 - (t * 6.0).min(1.0)).powi(2) * 0.4; // brief scale-in
        ui.label(
            RichText::new(format!("+{points} points"))
                .color(GOLD)
                .font(display(24.0 * pop)),
        );
    }

    ui.add_space(26.0);
    ui.horizontal(|ui| {
        if primary_button(ui, "Next song").clicked() {
            session.handle_key(Key::Enter);
        }
        if ghost_button(ui, "Menu").clicked() {
            session.handle_key(Key::Char('m'));
        }
    });
}

fn plural<'a>(one: &'a str, many: &'a str, n: usize) -> &'a str {
    if n == 1 { one } else { many }
}

// --- small widgets --------------------------------------------------------

fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).color(INK).size(14.5).strong())
            .fill(CORAL)
            .corner_radius(CornerRadius::same(10))
            .min_size(vec2(0.0, 38.0)),
    )
}

fn ghost_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).color(TEXT).size(14.5))
            .fill(PANEL)
            .stroke(Stroke::new(1.0, LINE))
            .corner_radius(CornerRadius::same(10))
            .min_size(vec2(0.0, 38.0)),
    )
}

fn chip(ui: &mut egui::Ui, label: &str, color: Color32) {
    let font = FontId::proportional(12.5);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), color);
    let size = galley.size() + vec2(20.0, 8.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(9), color.gamma_multiply(0.14));
    p.text(rect.center(), Align2::CENTER_CENTER, label, font, color);
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
