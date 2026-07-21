//! Rendering. Each screen reads session state and turns interaction into session
//! intents (`handle_key` / `list_click`). Immediate-mode, so reads and calls are
//! interleaved — small bits of state are copied out before a mutating call.

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense, Stroke,
    StrokeKind, pos2, vec2,
};
use hitair_core::game::{GameMode, GuessLog, Outcome};
use hitair_core::profile::RecentGame;
use hitair_core::session::{Key, LobbyPhase, MaintenanceAction, Screen, Session};

use crate::theme::*;

const COL_W: f32 = 680.0;

pub fn draw(ui: &mut egui::Ui, session: &mut Session) {
    header(ui, session);
    hairline(ui);
    ui.add_space(8.0);

    column(ui, |ui| match session.screen {
        Screen::Setup => setup(ui, session),
        Screen::Home => home(ui, session),
        Screen::Menu => menu(ui, session),
        Screen::Loading => loading(ui),
        Screen::Playing => playing(ui, session),
        Screen::RoundEnd => result(ui, session),
        Screen::Profile => profile(ui, session),
        Screen::Settings => settings(ui, session),
        Screen::Whatsnew => whatsnew(ui),
        Screen::ChallengeMenu => challenge_menu(ui, session),
        Screen::HostConfig => host_config(ui, session),
        Screen::Browse => browse(ui, session),
        Screen::JoinCode => join_code(ui, session),
        Screen::Lobby => lobby(ui, session),
    });

    // Fulfil a pending "copy to clipboard" request (e.g. the lobby code).
    if let Some(text) = session.take_copy_request() {
        ui.ctx().copy_text(text);
    }

    toast(ui, session);
}

/// A transient status message (errors, "online unavailable", volume) pinned to
/// the bottom. It clears itself via the session's tick.
fn toast(ui: &egui::Ui, session: &Session) {
    let Some(status) = session.status.clone() else {
        return;
    };
    egui::Area::new("toast".into())
        .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -18.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::default()
                .fill(PANEL_HI)
                .stroke(Stroke::new(1.0, LINE))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(egui::Margin::symmetric(16, 10))
                .show(ui, |ui| {
                    ui.label(RichText::new(status).color(GOLD).size(14.0));
                });
        });
}

// --- shared chrome --------------------------------------------------------

fn header(ui: &mut egui::Ui, session: &mut Session) {
    let screen = session.screen;
    let (volume, streak, score) = (session.volume, session.streak, session.score);
    let round = session.rounds_played + 1;
    let name = session.profile.name.clone();
    let accent = accent_color(&session.profile.accent);
    // Pulse the score for ~1.5s after it increases.
    let flash = session
        .score_flash_at
        .map(|t| t.elapsed().as_millis())
        .is_some_and(|ms| ms < 1500 && (ms / 200).is_multiple_of(2));
    let score_color = if flash { TEXT } else { MINT };
    // The first-run wizard shows only the wordmark — no Back, stats, or profile.
    let bare = screen == Screen::Setup;
    let show_back = !matches!(screen, Screen::Home | Screen::Setup);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        if show_back && back_chip(ui).clicked() {
            session.handle_key(Key::Esc);
        }
        if show_back {
            ui.add_space(8.0);
        }
        ui.label(RichText::new("♪ hitair").color(CORAL).size(20.0).strong());

        if !bare {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                // Always-present entry to the profile (avatar + name).
                if screen != Screen::Profile && profile_chip(ui, &name, accent).clicked() {
                    session.open_profile();
                }
                if screen != Screen::Profile {
                    dot(ui);
                }
                let vol = format!("{}%", (volume * 100.0).round() as i32);
                stat(ui, if volume <= 0.001 { "🔇" } else { "🔊" }, &vol, MUTED);
                dot(ui);
                stat(ui, "round", &round.to_string(), MUTED);
                dot(ui);
                stat(ui, "streak", &streak.to_string(), GOLD);
                dot(ui);
                stat(ui, "score", &score.to_string(), score_color);
                // A visible entry point to online play (keyboard: Ctrl+O still works).
                if screen == Screen::Menu {
                    ui.add_space(14.0);
                    if online_chip(ui).clicked() {
                        session.open_challenge();
                    }
                }
            });
        }
    });
    ui.add_space(6.0);
}

/// A small "‹ Back" pill for the header (sends Esc).
fn back_chip(ui: &mut egui::Ui) -> egui::Response {
    let font = FontId::proportional(13.0);
    let galley = ui
        .painter()
        .layout_no_wrap("Back".into(), font.clone(), MUTED);
    let w = galley.size().x + 34.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
    let hov = resp.hovered();
    let p = ui.painter();
    p.rect_filled(
        rect,
        CornerRadius::same(8),
        if hov { PANEL_HI } else { PANEL },
    );
    p.rect_stroke(
        rect,
        CornerRadius::same(8),
        Stroke::new(1.0, LINE),
        StrokeKind::Inside,
    );
    let col = if hov { CORAL } else { MUTED };
    let cx = rect.left() + 14.0;
    let cy = rect.center().y;
    p.add(egui::Shape::convex_polygon(
        vec![
            pos2(cx + 2.0, cy - 4.0),
            pos2(cx - 3.0, cy),
            pos2(cx + 2.0, cy + 4.0),
        ],
        col,
        Stroke::NONE,
    ));
    p.text(
        pos2(cx + 10.0, cy),
        Align2::LEFT_CENTER,
        "Back",
        font,
        if hov { TEXT } else { MUTED },
    );
    resp
}

/// A coral "Play online" pill for the menu header.
fn online_chip(ui: &mut egui::Ui) -> egui::Response {
    let font = FontId::proportional(13.5);
    let galley = ui
        .painter()
        .layout_no_wrap("Play online".into(), font.clone(), INK);
    let w = galley.size().x + 26.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
    let p = ui.painter();
    let fill = if resp.hovered() {
        CORAL.gamma_multiply(1.12)
    } else {
        CORAL
    };
    p.rect_filled(rect, CornerRadius::same(8), fill);
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        "Play online",
        font,
        INK,
    );
    resp
}

/// A header pill: the player's avatar dot + name, opening the profile.
fn profile_chip(ui: &mut egui::Ui, name: &str, accent: Color32) -> egui::Response {
    let font = FontId::proportional(13.5);
    let galley = ui
        .painter()
        .layout_no_wrap(name.to_string(), font.clone(), TEXT);
    let dot_r = 8.0;
    let w = galley.size().x + dot_r * 2.0 + 30.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
    let hov = resp.hovered();
    let p = ui.painter();
    p.rect_filled(
        rect,
        CornerRadius::same(8),
        if hov { PANEL_HI } else { PANEL },
    );
    p.rect_stroke(
        rect,
        CornerRadius::same(8),
        Stroke::new(1.0, LINE),
        StrokeKind::Inside,
    );
    let cx = rect.left() + 12.0 + dot_r;
    let cy = rect.center().y;
    p.circle_filled(pos2(cx, cy), dot_r, accent);
    let initial = name
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();
    p.text(
        pos2(cx, cy - 0.5),
        Align2::CENTER_CENTER,
        initial,
        FontId::proportional(10.5),
        INK,
    );
    p.text(
        pos2(cx + dot_r + 8.0, cy),
        Align2::LEFT_CENTER,
        name,
        font,
        if hov { TEXT } else { MUTED },
    );
    resp
}

/// A theme-styled single-line text field bound to `buf`.
fn text_field(ui: &mut egui::Ui, buf: &mut String, hint: &str, width: f32) -> egui::Response {
    ui.add(
        egui::TextEdit::singleline(buf)
            .hint_text(hint)
            .desired_width(width)
            .margin(egui::Margin::symmetric(12, 9))
            .font(FontId::proportional(15.0)),
    )
}

/// True if the field lost focus because Enter was pressed (a submit).
fn submitted(ui: &egui::Ui, resp: &egui::Response) -> bool {
    resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

/// Keep this text field focused whenever nothing else is, so the user can just
/// start typing on a screen whose main job is text entry.
fn autofocus(ui: &egui::Ui, resp: &egui::Response) {
    if ui.memory(|m| m.focused().is_none()) {
        resp.request_focus();
    }
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
        let mode = session.game_mode.label();
        let field_w = (ui.available_width() - mode_w - 48.0).max(180.0);
        let mut filter = session.menu_filter.clone();
        let resp = text_field(ui, &mut filter, "Filter categories", field_w);
        if resp.changed() {
            session.set_menu_filter(filter);
        }
        if submitted(ui, &resp) {
            session.handle_key(Key::Enter); // start the highlighted category
        }
        autofocus(ui, &resp); // type-to-filter immediately, like the terminal
        ui.add_space(12.0);
        mode_pill(ui, session, mode, mode_w);
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

/// A `MODE  ◄ value ►` pill. The arrows send Left/Right, which each screen
/// interprets (the menu cycles the solo mode; Host config cycles its own).
fn mode_pill(ui: &mut egui::Ui, session: &mut Session, value: &str, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 40.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(10), PANEL);
    p.rect_stroke(
        rect,
        CornerRadius::same(10),
        Stroke::new(1.0, LINE),
        StrokeKind::Inside,
    );
    p.text(
        pos2(rect.left() + 14.0, rect.center().y),
        Align2::LEFT_CENTER,
        "MODE",
        FontId::proportional(10.5),
        MUTED,
    );
    let left_x = rect.left() + 64.0;
    let right_x = rect.right() - 18.0;
    p.text(
        pos2((left_x + right_x) / 2.0, rect.center().y),
        Align2::CENTER_CENTER,
        value,
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
    let checkpoints = round.checkpoint_secs();
    let guesses = round.guesses.clone();
    let is_anime = round.anime.is_some();
    let started = session.play_started_at;
    let countdown = session.countdown_remaining();
    let countdown_frac = session.countdown_second_fraction();
    // A lobby round uses the host's effect + shows the round number.
    let mode = session
        .lobby
        .as_ref()
        .map(|l| l.mode)
        .unwrap_or(session.game_mode);
    let lobby_round = session.lobby.as_ref().map(|l| (l.game.round, l.rounds));
    let input = session.input.clone();
    let sel = session.suggestion_index;
    let sugg: Vec<(String, String)> = session
        .suggestions
        .iter()
        .map(|t| (t.title.clone(), t.artist_name().to_string()))
        .collect();
    // Live "who's finished this round" (lobby only; you're not in the list yet).
    let finished: Option<(Vec<String>, usize, usize)> = session.lobby.as_ref().map(|l| {
        let names = l
            .game
            .submitted_names()
            .iter()
            .filter(|n| **n != session.player_name)
            .cloned()
            .collect::<Vec<_>>();
        (names, l.game.submitted_count(), l.players.len().max(1))
    });

    ui.add_space(16.0);
    ui.horizontal(|ui| {
        if let Some((r, n)) = lobby_round {
            ui.label(
                RichText::new(format!("Round {r}/{n}"))
                    .color(MINT)
                    .size(15.0)
                    .strong(),
            );
            ui.label(RichText::new("·").color(LINE).size(14.0));
        }
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

    // In a lobby, show who's already finished while you're still guessing.
    if let Some((names, done, total)) = &finished
        && *done > 0
    {
        let who = if names.is_empty() {
            String::new()
        } else {
            format!(" — {}", names.join(", "))
        };
        ui.add_space(6.0);
        ui.label(
            RichText::new(format!("{done}/{total} finished this round{who}"))
                .color(MINT)
                .size(13.5),
        );
    }
    ui.add_space(14.0);

    if let Some(secs) = countdown {
        countdown_view(ui, secs, countdown_frac);
        ui.add_space(16.0);
    } else {
        reveal_meter(ui, &checkpoints, level, started);
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
    }

    guesses_row(ui, &guesses);
    ui.add_space(10.0);
    let mut q = input.clone();
    let hint = if is_anime {
        "Name the song — or the anime it's from"
    } else {
        "Name the track…"
    };
    let search = text_field(ui, &mut q, hint, ui.available_width() - 26.0);
    if search.changed() {
        session.set_search(q);
    }
    if submitted(ui, &search) {
        session.handle_key(Key::Enter);
    }
    autofocus(ui, &search);
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

/// The pre-round "get ready" lead-in: a big number inside a ring that empties as
/// each second elapses. Replaces the meter + controls until the clip starts.
fn countdown_view(ui: &mut egui::Ui, secs: u32, frac: f32) {
    ui.add_space(6.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new("Get ready").color(MUTED).size(15.0).strong());
        ui.add_space(16.0);
        let d = 116.0;
        let (rect, _) = ui.allocate_exact_size(vec2(d, d), Sense::hover());
        let p = ui.painter();
        let c = rect.center();
        let r = d / 2.0 - 7.0;
        // A faint full track, with a coral arc that depletes over the second,
        // sweeping from 12 o'clock clockwise.
        p.circle_stroke(c, r, Stroke::new(5.0, WELL));
        let start = -std::f32::consts::FRAC_PI_2;
        let sweep = frac.clamp(0.0, 1.0) * std::f32::consts::TAU;
        let steps = 72;
        let mut prev = c + vec2(start.cos(), start.sin()) * r;
        for i in 1..=steps {
            let a = start + sweep * (i as f32 / steps as f32);
            let next = c + vec2(a.cos(), a.sin()) * r;
            p.line_segment([prev, next], Stroke::new(5.0, CORAL));
            prev = next;
        }
        p.text(
            c,
            Align2::CENTER_CENTER,
            secs.to_string(),
            display(48.0),
            TEXT,
        );
        ui.add_space(14.0);
        ui.label(
            RichText::new("Clip starts in a moment…")
                .color(MUTED)
                .size(13.0),
        );
    });
}

/// The signature: a reveal meter on the song's real timeline. Ticks sit at each
/// checkpoint's true time (0.5s, 1s, 2s … 15s — so they're *not* evenly spaced),
/// a faint fill marks the unlocked span, and a bright playhead sweeps the clip.
fn reveal_meter(
    ui: &mut egui::Ui,
    checkpoints: &[f32],
    level: usize,
    started: Option<std::time::Instant>,
) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
    let p = ui.painter();
    let r = CornerRadius::same(10);
    p.rect_filled(rect, r, WELL);
    p.rect_stroke(rect, r, Stroke::new(1.0, LINE), StrokeKind::Inside);

    let total = checkpoints.last().copied().unwrap_or(1.0).max(0.01); // longest clip
    let current = checkpoints.get(level).copied().unwrap_or(total); // unlocked so far
    // Seconds heard this pass (idle ⇒ fully heard). The audible window ≈ real
    // seconds for every effect, so wall-clock maps straight onto the timeline.
    let heard = started
        .map(|s| s.elapsed().as_secs_f32())
        .unwrap_or(f32::MAX)
        .min(current);
    let at = |secs: f32| rect.left() + (secs / total) * rect.width();
    let unlocked_x = at(current);
    let play_x = at(heard);
    let unlocked_rect = Rect::from_min_max(rect.min, pos2(unlocked_x.max(rect.left()), rect.max.y));
    let play_rect = Rect::from_min_max(rect.min, pos2(play_x.max(rect.left()), rect.max.y));

    // Faint fill marks how much is unlocked; the bright sweep is the live playhead.
    p.rect_filled(unlocked_rect, r, CORAL.gamma_multiply(0.22));
    p.rect_filled(play_rect.expand(2.5), r, CORAL.gamma_multiply(0.16));
    p.rect_filled(play_rect, r, CORAL);

    // A tick at each checkpoint's true time (the last one is the far edge).
    for &c in checkpoints.iter().rev().skip(1) {
        p.vline(
            at(c),
            (rect.top() + 3.0)..=(rect.bottom() - 3.0),
            Stroke::new(1.0, Color32::from_black_alpha(70)),
        );
    }
    // Playhead.
    if play_x > rect.left() + 1.0 {
        p.vline(
            play_x,
            rect.top()..=rect.bottom(),
            Stroke::new(2.0, Color32::WHITE),
        );
        p.circle_filled(pos2(play_x, rect.center().y), 3.5, Color32::WHITE);
    }
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
                // Amber == a partial hit: right artist, wrong song.
                GuessLog::WrongRightArtist(name) => {
                    chip(ui, &format!("{name} · right artist"), GOLD)
                }
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
    let cover = round.answer.cover().map(str::to_string);
    let anime = round
        .anime
        .as_ref()
        .map(|a| format!("{} · {}", a.theme, a.anime));
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
    ui.add_space(18.0);

    // Cover art (fades in via the async loader) beside the song info.
    ui.horizontal(|ui| {
        if let Some(url) = &cover {
            ui.add(
                egui::Image::new(url)
                    .fit_to_exact_size(vec2(150.0, 150.0))
                    .corner_radius(CornerRadius::same(14)),
            );
            ui.add_space(20.0);
        }
        ui.vertical(|ui| {
            ui.label(
                RichText::new("THE SONG WAS")
                    .color(MUTED)
                    .size(11.5)
                    .strong(),
            );
            ui.add_space(6.0);
            ui.label(RichText::new(title).color(TEXT).font(display(38.0)));
            ui.add_space(2.0);
            ui.label(RichText::new(artist).color(CORAL).size(19.0));
            if let Some(album) = album {
                ui.add_space(2.0);
                ui.label(RichText::new(album).color(MUTED).size(14.0));
            }
            if let Some(anime) = &anime {
                ui.add_space(6.0);
                ui.label(RichText::new(anime).color(GOLD).size(15.0).strong());
            }
        });
    });

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
    } else if points > 0 {
        // Missed the song but named the artist — a consolation.
        ui.add_space(18.0);
        ui.label(
            RichText::new(format!("Right artist  ·  +{points} points"))
                .color(GOLD)
                .size(18.0)
                .strong(),
        );
    }

    let paused = session.reveal_paused;
    let audio = session.audio_available;
    ui.add_space(26.0);
    ui.horizontal(|ui| {
        if primary_button(ui, "Next song").clicked() {
            session.handle_key(Key::Enter);
        }
        if ghost_button(ui, "Menu").clicked() {
            session.handle_key(Key::Char('m'));
        }
        // Pause / resume the reveal that's playing.
        if audio && ghost_button(ui, if paused { "Play" } else { "Pause" }).clicked() {
            session.toggle_reveal_pause();
        }
    });
}

fn plural<'a>(one: &'a str, many: &'a str, n: usize) -> &'a str {
    if n == 1 { one } else { many }
}

// --- first-run setup wizard -----------------------------------------------

fn setup(ui: &mut egui::Ui, session: &mut Session) {
    ui.add_space(46.0);
    ui.label(
        RichText::new("Welcome to hitair.")
            .color(TEXT)
            .font(display(52.0)),
    );
    ui.add_space(10.0);
    ui.label(
        RichText::new(
            "Set it up as a real app on your machine — an entry in your app menu \
             with its icon — so you can launch it like anything else.",
        )
        .color(MUTED)
        .size(16.0),
    );
    ui.add_space(26.0);

    for line in [
        "Adds hitair to your applications, with its icon",
        "Keeps a tidy copy in place, so nothing breaks if you move the download",
        "Undo it anytime in Settings → Desktop app",
    ] {
        ui.horizontal(|ui| {
            ui.label(RichText::new("·").color(CORAL).size(16.0).strong());
            ui.add_space(8.0);
            ui.label(RichText::new(line).color(TEXT).size(14.5));
        });
        ui.add_space(7.0);
    }
    ui.add_space(24.0);

    ui.horizontal(|ui| {
        if primary_button(ui, "Set up hitair").clicked() {
            session.confirm_setup();
        }
        if ghost_button(ui, "Skip for now").clicked() {
            session.skip_setup();
        }
    });
    ui.add_space(12.0);
    ui.label(
        RichText::new("You can also just play — hitair runs fine from wherever it is.")
            .color(MUTED)
            .size(13.0)
            .italics(),
    );
}

// --- home + settings ------------------------------------------------------

fn home(ui: &mut egui::Ui, session: &mut Session) {
    let name = session.profile.name.clone();
    let (rounds, wr) = {
        let s = &session.profile.stats;
        (s.rounds, (s.win_rate() * 100.0).round() as i32)
    };
    let sel = session.home_index;

    ui.add_space(40.0);
    ui.label(
        RichText::new("Guess the song.")
            .color(TEXT)
            .font(display(56.0)),
    );
    ui.add_space(6.0);
    let greeting = if rounds == 0 {
        format!("Welcome, {name}. A snippet grows with every miss — solo or online.")
    } else {
        format!("Welcome back, {name}  ·  {rounds} rounds  ·  {wr}% solved")
    };
    ui.label(RichText::new(greeting).color(MUTED).size(16.0));
    ui.add_space(30.0);

    let settings_sub =
        if session.update_available.is_some() && !hitair_core::update::is_itch_managed() {
            "Update available · effect, volume, and more"
        } else {
            "Default effect, volume, and more"
        };
    let actions = [
        ("Play solo", "Pick a category and start guessing"),
        ("Play online", "Host or join a live Challenge lobby"),
        ("Profile", "Your stats, history, and identity"),
        ("Settings", settings_sub),
    ];
    for (i, (title, sub)) in actions.iter().enumerate() {
        if action_row(ui, title, sub, i == sel).clicked() {
            session.home_select(i);
        }
    }
}

/// A big Home entry: title over a subtitle, a coral bar + chevron when active.
fn action_row(ui: &mut egui::Ui, title: &str, subtitle: &str, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 60.0), Sense::click());
    let hot = selected || resp.hovered();
    let p = ui.painter();
    p.rect_filled(
        rect,
        CornerRadius::same(12),
        if hot { PANEL_HI } else { PANEL },
    );
    p.rect_stroke(
        rect,
        CornerRadius::same(12),
        Stroke::new(1.0, if hot { CORAL.gamma_multiply(0.7) } else { LINE }),
        StrokeKind::Inside,
    );
    if selected {
        let bar = Rect::from_min_size(
            rect.left_top() + vec2(0.0, 10.0),
            vec2(3.0, rect.height() - 20.0),
        );
        p.rect_filled(bar, CornerRadius::same(2), CORAL);
    }
    p.text(
        pos2(rect.left() + 20.0, rect.center().y - 9.0),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(17.0),
        TEXT,
    );
    p.text(
        pos2(rect.left() + 20.0, rect.center().y + 11.0),
        Align2::LEFT_CENTER,
        subtitle,
        FontId::proportional(13.0),
        MUTED,
    );
    let (cx, cy) = (rect.right() - 24.0, rect.center().y);
    p.add(egui::Shape::line(
        vec![
            pos2(cx - 3.0, cy - 5.0),
            pos2(cx + 2.0, cy),
            pos2(cx - 3.0, cy + 5.0),
        ],
        Stroke::new(2.0, if hot { CORAL } else { MUTED }),
    ));
    resp
}

fn settings(ui: &mut egui::Ui, session: &mut Session) {
    ui.add_space(18.0);
    eyebrow(ui, "SETTINGS");
    ui.add_space(4.0);
    ui.label(
        RichText::new("Preferences.")
            .color(TEXT)
            .font(display(40.0)),
    );
    ui.add_space(18.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| settings_body(ui, session));
}

fn settings_body(ui: &mut egui::Ui, session: &mut Session) {
    setting_head(
        ui,
        "Default effect",
        "Applied to new solo rounds — remembered.",
    );
    let mode = session.game_mode.label();
    ui.horizontal(|ui| {
        ui.add_space(2.0);
        mode_pill(ui, session, mode, 230.0);
    });
    ui.add_space(18.0);

    let vol = (session.volume * 100.0).round() as i32;
    setting_head(ui, "Volume", "Also Ctrl+↑ / Ctrl+↓ anywhere — remembered.");
    ui.horizontal(|ui| {
        if ghost_button(ui, "Vol −").clicked() {
            session.handle_key(Key::CtrlDown);
        }
        if ghost_button(ui, "Vol +").clicked() {
            session.handle_key(Key::CtrlUp);
        }
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!("{vol}%"))
                .color(GOLD)
                .size(15.0)
                .strong(),
        );
    });
    ui.add_space(18.0);

    setting_head(ui, "Identity", "Your name and accent colour.");
    if ghost_button(ui, "Edit on Profile").clicked() {
        session.open_profile();
    }
    ui.add_space(18.0);

    if hitair_core::desktop::SUPPORTED {
        setting_head(
            ui,
            "Desktop app",
            "A launcher + icon in your applications menu.",
        );
        let label = if session.launcher_installed() {
            "Remove from Applications"
        } else {
            "Add to Applications"
        };
        if ghost_button(ui, label).clicked() {
            session.toggle_launcher();
        }
        ui.add_space(18.0);
    }

    // Maintenance — hidden under the itch app, which manages updates + removal.
    if !hitair_core::update::is_itch_managed() {
        let ver = hitair_core::update::CURRENT_VERSION;
        let sub = if session.update_ready {
            "Update downloaded — restart to apply.".to_string()
        } else if let Some(v) = &session.update_available {
            format!("v{ver} installed · v{v} available")
        } else {
            format!("v{ver} · you're on the latest")
        };
        setting_head(ui, "Maintenance", &sub);
        ui.horizontal(|ui| {
            if !session.update_ready
                && session.update_available.is_some()
                && primary_button(ui, "Update now").clicked()
            {
                session.request_maintenance(MaintenanceAction::Update);
            }
            if ghost_button(ui, "Restart").clicked() {
                session.request_maintenance(MaintenanceAction::Restart);
            }
            if ghost_button(ui, "Uninstall").clicked() {
                session.request_maintenance(MaintenanceAction::Uninstall);
            }
        });
        ui.add_space(18.0);
    }

    setting_head(
        ui,
        "What's new",
        "Release notes for this and past versions.",
    );
    if ghost_button(ui, "View changelog").clicked() {
        session.open_whatsnew();
    }
    ui.add_space(18.0);

    ui.add_space(6.0);
    ui.label(
        RichText::new("Your profile, stats, and preferences are saved locally.")
            .color(MUTED)
            .size(12.5),
    );
}

// --- what's new -----------------------------------------------------------

fn whatsnew(ui: &mut egui::Ui) {
    ui.add_space(18.0);
    eyebrow(ui, "WHAT'S NEW");
    ui.add_space(4.0);
    ui.label(RichText::new("What's new.").color(TEXT).font(display(40.0)));
    ui.add_space(4.0);
    ui.label(
        RichText::new(format!(
            "You're on hitair v{}.",
            hitair_core::update::CURRENT_VERSION
        ))
        .color(MUTED)
        .size(14.0),
    );
    ui.add_space(16.0);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for rel in hitair_core::changelog::releases() {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("v{}", rel.version))
                            .color(CORAL)
                            .size(18.0)
                            .strong(),
                    );
                    if !rel.date.is_empty() {
                        ui.label(RichText::new("·").color(LINE).size(14.0));
                        ui.label(RichText::new(&rel.date).color(MUTED).size(12.5));
                    }
                });
                ui.add_space(4.0);
                // Join wrapped bullet lines back into one label so egui can rewrap.
                let mut bullet = String::new();
                for line in rel.body.lines() {
                    let t = line.trim();
                    if let Some(h) = t.strip_prefix("### ") {
                        flush_bullet(ui, &mut bullet);
                        ui.add_space(3.0);
                        ui.label(
                            RichText::new(h.to_uppercase())
                                .color(MINT)
                                .size(11.0)
                                .strong(),
                        );
                    } else if let Some(b) = t.strip_prefix("- ") {
                        flush_bullet(ui, &mut bullet);
                        bullet.push_str(b);
                    } else if t.is_empty() {
                        flush_bullet(ui, &mut bullet);
                    } else {
                        if !bullet.is_empty() {
                            bullet.push(' ');
                        }
                        bullet.push_str(t);
                    }
                }
                flush_bullet(ui, &mut bullet);
                ui.add_space(16.0);
            }
        });
}

/// Render the accumulated bullet (wrapping) and clear it.
fn flush_bullet(ui: &mut egui::Ui, buf: &mut String) {
    if !buf.trim().is_empty() {
        ui.label(
            RichText::new(format!("•  {}", buf.trim()))
                .color(TEXT)
                .size(13.5),
        );
    }
    buf.clear();
}

/// A setting's title + one-line description.
fn setting_head(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.label(RichText::new(title).color(TEXT).size(15.5).strong());
    ui.label(RichText::new(subtitle).color(MUTED).size(12.5));
    ui.add_space(7.0);
}

// --- profile --------------------------------------------------------------

fn profile(ui: &mut egui::Ui, session: &mut Session) {
    // Copy everything out of the profile up front, so the editing calls below
    // (which borrow the session mutably) don't clash with reading stats.
    let accent = accent_color(&session.profile.accent);
    let accent_key = session.profile.accent.clone();
    let (rounds, win_rate, best_streak, total_points);
    let mut cats: Vec<(String, u32, u32)>;
    let recent: Vec<RecentGame>;
    {
        let s = &session.profile.stats;
        rounds = s.rounds;
        win_rate = (s.win_rate() * 100.0).round() as i32;
        best_streak = s.best_streak;
        total_points = s.total_points;
        cats = s
            .by_category
            .iter()
            .map(|(k, c)| (k.clone(), c.rounds, c.wins))
            .collect();
        recent = s.recent.iter().take(14).cloned().collect();
    }
    cats.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    cats.truncate(6);

    ui.add_space(18.0);
    eyebrow(ui, "PROFILE");
    ui.add_space(8.0);

    // Identity: avatar, editable name, accent picker.
    let mut name = session.profile.name.clone();
    ui.horizontal(|ui| {
        avatar(ui, &name, accent, 58.0);
        ui.add_space(16.0);
        ui.vertical(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut name)
                    .hint_text("Your name")
                    .desired_width(300.0)
                    .margin(egui::Margin::symmetric(10, 7))
                    .font(FontId::proportional(22.0)),
            );
            if resp.changed() {
                session.set_display_name(name.clone());
            }
            if resp.lost_focus() {
                session.commit_profile();
            }
            ui.add_space(6.0);
            let sub = if rounds == 0 {
                "New here — play a round to start your stats.".to_string()
            } else {
                format!("{rounds} rounds · {win_rate}% solved · best streak {best_streak}")
            };
            ui.label(RichText::new(sub).color(MUTED).size(14.0));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                for key in ACCENTS {
                    if accent_dot(ui, accent_color(key), *key == accent_key).clicked() {
                        session.set_accent(key);
                    }
                }
            });
        });
    });
    ui.add_space(22.0);

    if rounds == 0 {
        return; // nothing to chart yet
    }

    // Lifetime stat tiles.
    let tw = (ui.available_width() - 30.0) / 4.0;
    ui.horizontal(|ui| {
        stat_tile(ui, tw, "ROUNDS", &rounds.to_string(), TEXT);
        stat_tile(ui, tw, "SOLVED", &format!("{win_rate}%"), MINT);
        stat_tile(ui, tw, "BEST STREAK", &best_streak.to_string(), GOLD);
        stat_tile(ui, tw, "POINTS", &total_points.to_string(), CORAL);
    });
    ui.add_space(22.0);

    // Two columns so category + recent both fit without scrolling far.
    ui.columns(2, |cols| {
        {
            let ui = &mut cols[0];
            eyebrow(ui, "BY CATEGORY");
            ui.add_space(10.0);
            if cats.is_empty() {
                ui.label(
                    RichText::new("Play a few to fill this in.")
                        .color(MUTED)
                        .size(13.0),
                );
            }
            for (cat, r, w) in &cats {
                cat_bar(ui, cat, *r, *w, accent);
            }
        }
        {
            let ui = &mut cols[1];
            eyebrow(ui, "RECENT");
            ui.add_space(10.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for g in &recent {
                        recent_row(ui, g);
                    }
                });
        }
    });
}

/// A filled accent circle with the name's initial — the player's avatar.
fn avatar(ui: &mut egui::Ui, name: &str, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let p = ui.painter();
    p.circle_filled(rect.center(), size / 2.0, color);
    let initial = name
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        initial,
        display(size * 0.46),
        INK,
    );
}

/// A clickable accent-colour swatch, ringed when it's the current choice.
fn accent_dot(ui: &mut egui::Ui, color: Color32, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    let p = ui.painter();
    p.circle_filled(rect.center(), 9.0, color);
    if selected {
        p.circle_stroke(rect.center(), 12.5, Stroke::new(2.0, color));
    } else if resp.hovered() {
        p.circle_stroke(rect.center(), 12.5, Stroke::new(1.0, LINE));
    }
    resp
}

/// A labelled stat card: small caps label over a big display-font value.
fn stat_tile(ui: &mut egui::Ui, w: f32, label: &str, value: &str, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(w, 74.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(12), PANEL);
    p.rect_stroke(
        rect,
        CornerRadius::same(12),
        Stroke::new(1.0, LINE),
        StrokeKind::Inside,
    );
    p.text(
        pos2(rect.left() + 16.0, rect.top() + 19.0),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(10.5),
        MUTED,
    );
    p.text(
        pos2(rect.left() + 15.0, rect.bottom() - 24.0),
        Align2::LEFT_CENTER,
        value,
        display(26.0),
        color,
    );
}

/// One category row: name, a win-rate bar, and wins/rounds — width-responsive so
/// it reads well full-width or in a half-width column.
fn cat_bar(ui: &mut egui::Ui, name: &str, rounds: u32, wins: u32, accent: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    let cy = rect.center().y;
    let p = ui.painter();
    let name_w = (rect.width() * 0.46).min(180.0);
    p.text(
        pos2(rect.left() + 2.0, cy),
        Align2::LEFT_CENTER,
        truncate(name, ((name_w / 8.5) as usize).max(6)),
        FontId::proportional(13.5),
        TEXT,
    );
    let (bar_left, bar_right) = (rect.left() + name_w + 8.0, rect.right() - 44.0);
    let track = Rect::from_min_max(pos2(bar_left, cy - 4.5), pos2(bar_right, cy + 4.5));
    p.rect_filled(track, CornerRadius::same(5), WELL);
    let rate = if rounds > 0 {
        wins as f32 / rounds as f32
    } else {
        0.0
    };
    let fill_r = bar_left + (bar_right - bar_left) * rate;
    if fill_r > bar_left + 1.0 {
        p.rect_filled(
            Rect::from_min_max(pos2(bar_left, cy - 4.5), pos2(fill_r, cy + 4.5)),
            CornerRadius::same(5),
            accent,
        );
    }
    p.text(
        pos2(rect.right(), cy),
        Align2::RIGHT_CENTER,
        format!("{wins}/{rounds}"),
        FontId::proportional(13.0),
        MUTED,
    );
}

/// One recent-round row: outcome dot, song, then category · points.
fn recent_row(ui: &mut egui::Ui, g: &RecentGame) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    let cy = rect.center().y;
    let p = ui.painter();
    p.circle_filled(
        pos2(rect.left() + 6.0, cy),
        4.0,
        if g.won { MINT } else { ROSE },
    );
    let title_chars = ((rect.width() - 66.0) / 7.4) as usize;
    p.text(
        pos2(rect.left() + 22.0, cy),
        Align2::LEFT_CENTER,
        truncate(&format!("{} — {}", g.title, g.artist), title_chars.max(10)),
        FontId::proportional(13.5),
        TEXT,
    );
    let (meta, meta_col) = if g.won {
        (format!("+{}", g.points), GOLD)
    } else {
        ("missed".to_string(), MUTED)
    };
    p.text(
        pos2(rect.right(), cy),
        Align2::RIGHT_CENTER,
        meta,
        FontId::proportional(12.5),
        meta_col,
    );
}

/// Shorten a string to `max` chars with an ellipsis.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
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

// --- challenge menu -------------------------------------------------------

fn challenge_menu(ui: &mut egui::Ui, session: &mut Session) {
    let name = session.player_name.clone();
    let idx = session.challenge_index;

    ui.add_space(18.0);
    eyebrow(ui, "CHALLENGE");
    ui.add_space(4.0);
    ui.label(
        RichText::new("Play together.")
            .color(TEXT)
            .font(display(46.0)),
    );
    ui.add_space(2.0);
    ui.label(
        RichText::new(
            "Host a live lobby or join friends — everyone races the same song, round after round.",
        )
        .color(MUTED)
        .size(15.0),
    );
    ui.add_space(18.0);

    ui.horizontal(|ui| {
        ui.label(RichText::new("Playing as").color(MUTED).size(14.0));
        ui.add_space(6.0);
        let mut nm = name.clone();
        let resp = ui.add(
            egui::TextEdit::singleline(&mut nm)
                .char_limit(20)
                .desired_width(240.0)
                .margin(egui::Margin::symmetric(12, 8))
                .font(FontId::proportional(15.0)),
        );
        if resp.changed() {
            session.player_name = nm;
        }
    });
    ui.add_space(16.0);

    let opts = [
        (
            "Host a lobby",
            "Pick a pool, set the rounds & mode, share a code.",
        ),
        ("Browse public lobbies", "Jump into an open game."),
        ("Join by code", "Enter a friend's code."),
    ];
    let mut clicked = None;
    for (i, (title, sub)) in opts.iter().enumerate() {
        if option_card(ui, title, sub, i == idx).clicked() {
            clicked = Some(i);
        }
    }
    if let Some(i) = clicked {
        session.list_click(i);
    }
}

// --- host config ----------------------------------------------------------

fn host_config(ui: &mut egui::Ui, session: &mut Session) {
    let category = session
        .host_category
        .as_ref()
        .map(|c| c.name.clone())
        .unwrap_or_else(|| "—".into());
    let rounds = session.host_rounds.to_string();
    let mode = session.host_mode.label();
    let public = session.host_public;
    let max = session.host_max.to_string();
    let editing = session.editing_lobby;

    ui.add_space(18.0);
    eyebrow(ui, if editing { "SETTINGS" } else { "HOST" });
    ui.add_space(4.0);
    ui.label(
        RichText::new(if editing {
            "Lobby settings."
        } else {
            "Set up the lobby."
        })
        .color(TEXT)
        .font(display(44.0)),
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Song pool").color(MUTED).size(14.0));
        ui.add_space(6.0);
        ui.label(RichText::new(category).color(TEXT).size(15.0).strong());
        ui.add_space(12.0);
        if ghost_button(ui, "Change").clicked() {
            session.change_pool();
        }
    });
    ui.add_space(16.0);

    ui.horizontal(|ui| {
        stepper(ui, session, "ROUNDS", &rounds, Key::Down, Key::Up, 232.0);
        ui.add_space(12.0);
        mode_pill(ui, session, mode, 232.0);
    });
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        toggle_pill(
            ui,
            session,
            "VISIBILITY",
            if public { "Public" } else { "Private" },
            Key::Char('v'),
            232.0,
        );
        ui.add_space(12.0);
        stepper(
            ui,
            session,
            "MAX PLAYERS",
            &max,
            Key::Char('-'),
            Key::Char('+'),
            232.0,
        );
    });
    ui.add_space(24.0);

    ui.horizontal(|ui| {
        let (ok, cancel) = if editing {
            ("Save changes", "Cancel")
        } else {
            ("Open lobby", "Back")
        };
        if primary_button(ui, ok).clicked() {
            session.handle_key(Key::Enter);
        }
        if ghost_button(ui, cancel).clicked() {
            session.handle_key(Key::Esc);
        }
    });
    ui.add_space(10.0);
    ui.label(
        RichText::new(if editing {
            "Changes apply to this lobby and the next game you start."
        } else {
            "Friends join with your code, then you launch each round."
        })
        .color(MUTED)
        .size(13.5),
    );
}

// --- browse ---------------------------------------------------------------

fn browse(ui: &mut egui::Ui, session: &mut Session) {
    let parties: Vec<(String, String, i32)> = session
        .browse
        .iter()
        .map(|p| (p.code.clone(), p.host_name.clone(), p.max_players))
        .collect();
    let idx = session.browse_index;

    ui.add_space(18.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Public lobbies")
                .color(TEXT)
                .font(display(38.0)),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ghost_button(ui, "Back").clicked() {
                session.handle_key(Key::Esc);
            }
            if ghost_button(ui, "Refresh").clicked() {
                session.handle_key(Key::Char('r'));
            }
        });
    });
    ui.add_space(14.0);

    if parties.is_empty() {
        ui.add_space(8.0);
        ui.label(
            RichText::new("No public lobbies right now — host one, or refresh.")
                .color(MUTED)
                .size(14.0)
                .italics(),
        );
        return;
    }
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, (code, host, max)) in parties.iter().enumerate() {
                let sub = format!("host {host} · up to {max} players");
                if lobby_card(ui, code, &sub, i == idx).clicked() {
                    clicked = Some(i);
                }
            }
        });
    if let Some(i) = clicked {
        session.list_click(i);
    }
}

// --- join by code ---------------------------------------------------------

fn join_code(ui: &mut egui::Ui, session: &mut Session) {
    let code = session.join_input.clone();
    ui.add_space(18.0);
    eyebrow(ui, "CHALLENGE");
    ui.add_space(4.0);
    ui.label(
        RichText::new("Join by code.")
            .color(TEXT)
            .font(display(44.0)),
    );
    ui.add_space(2.0);
    ui.label(
        RichText::new("Type the code your host shared.")
            .color(MUTED)
            .size(15.0),
    );
    ui.add_space(20.0);
    let mut c = code.clone();
    let resp = ui.add(
        egui::TextEdit::singleline(&mut c)
            .hint_text("ABC123")
            .char_limit(12)
            .desired_width(260.0)
            .margin(egui::Margin::symmetric(14, 12))
            .font(display(26.0)),
    );
    if resp.changed() {
        session.join_input = c.to_uppercase();
    }
    if submitted(ui, &resp) {
        session.handle_key(Key::Enter);
    }
    autofocus(ui, &resp);
    ui.add_space(18.0);
    ui.horizontal(|ui| {
        if primary_button(ui, "Join").clicked() {
            session.handle_key(Key::Enter);
        }
        if ghost_button(ui, "Back").clicked() {
            session.handle_key(Key::Esc);
        }
    });
}

// --- live lobby -----------------------------------------------------------

fn lobby(ui: &mut egui::Ui, session: &mut Session) {
    let Some(l) = session.lobby.as_ref() else {
        return;
    };
    let is_host = l.is_host;
    let phase = l.phase;
    let code = l.code.clone();
    let players = l.players.clone();
    let spectators = l.spectators.clone();
    let rounds = l.rounds;
    let mode = l.mode.label();
    let category = l.category_label.clone();
    let round = l.game.round;
    let is_final = l.game.is_final_round();
    let pool_empty = l.pool.is_empty();
    let submitted = l.game.submitted_count();
    let last_answer = l
        .last_answer
        .as_ref()
        .map(|t| format!("{} — {}", t.title, t.artist_name()));
    let board: Vec<(String, u32, u32)> = l
        .board()
        .iter()
        .map(|s| (s.name.clone(), s.points, s.solves))
        .collect();
    let me = session.player_name.clone();

    ui.add_space(16.0);
    lobby_header(
        ui,
        &code,
        players.len(),
        spectators.len(),
        phase,
        mode,
        rounds,
        &category,
    );
    ui.add_space(14.0);

    match phase {
        LobbyPhase::Waiting => {
            roster(ui, "PLAYERS", &players, &me, MINT);
            if !spectators.is_empty() {
                roster(ui, "WAITING TO JOIN", &spectators, &me, GOLD);
            }
            ui.add_space(12.0);
            let hint = if is_host {
                "You're the host — start when everyone's in."
            } else {
                "Waiting for the host to start the game…"
            };
            ui.label(RichText::new(hint).color(GOLD).size(14.0));
        }
        _ => {
            if phase == LobbyPhase::Spectating {
                ui.label(
                    RichText::new("Spectating — you'll join when the host starts the next game.")
                        .color(GOLD)
                        .size(14.0)
                        .strong(),
                );
                ui.add_space(8.0);
            }
            if let Some(answer) = &last_answer {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("Round {round}/{rounds}"))
                            .color(CORAL)
                            .size(14.0)
                            .strong(),
                    );
                    ui.label(RichText::new("· song was").color(MUTED).size(13.5));
                    ui.label(RichText::new(answer).color(TEXT).size(14.0).strong());
                });
                ui.add_space(10.0);
            }
            board_view(ui, &board, &me);
            if phase == LobbyPhase::Between {
                ui.add_space(8.0);
                let waiting = players.len().saturating_sub(submitted);
                let note = if waiting > 0 {
                    format!("Waiting on {waiting} more player(s)…")
                } else {
                    "Everyone's in.".to_string()
                };
                ui.label(RichText::new(note).color(MUTED).size(13.5));
            }
            if !spectators.is_empty() {
                ui.add_space(8.0);
                roster(ui, "WAITING TO JOIN", &spectators, &me, GOLD);
            }
        }
    }

    ui.add_space(18.0);
    // Host tried to move on while people are still guessing — confirm the skip.
    if is_host && session.confirm_skip_round {
        let waiting = players.len().saturating_sub(submitted);
        ui.label(
            RichText::new(format!(
                "{waiting} player(s) still guessing — skip to the next round anyway?"
            ))
            .color(GOLD)
            .size(14.5)
            .strong(),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if primary_button(ui, "Skip anyway").clicked() {
                session.handle_key(Key::Enter);
            }
            if ghost_button(ui, "Keep waiting").clicked() {
                session.confirm_skip_round = false;
            }
        });
    } else {
        ui.horizontal(|ui| {
            if is_host {
                let primary = match phase {
                    LobbyPhase::Waiting if pool_empty => "Loading songs…",
                    LobbyPhase::Waiting => "Start game",
                    LobbyPhase::GameOver => "New game",
                    LobbyPhase::Between if is_final => "See final scores",
                    LobbyPhase::Between => "Next round",
                    LobbyPhase::Spectating => "Spectating",
                };
                if primary_button(ui, primary).clicked() {
                    session.handle_key(Key::Enter);
                }
                // Change rounds / mode / pool / visibility between games.
                if matches!(phase, LobbyPhase::Waiting | LobbyPhase::GameOver)
                    && ghost_button(ui, "Settings").clicked()
                {
                    session.open_lobby_settings();
                }
            }
            if ghost_button(ui, "Leave").clicked() {
                session.handle_key(Key::Esc);
            }
            if ghost_button(ui, "Copy code").clicked() {
                session.handle_key(Key::Char('c'));
            }
            // Pause/resume the reveal that plays between rounds.
            if phase != LobbyPhase::Waiting
                && session.audio_available
                && ghost_button(
                    ui,
                    if session.reveal_paused {
                        "Play"
                    } else {
                        "Pause"
                    },
                )
                .clicked()
            {
                session.toggle_reveal_pause();
            }
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn lobby_header(
    ui: &mut egui::Ui,
    code: &str,
    playing: usize,
    waiting: usize,
    phase: LobbyPhase,
    mode: &str,
    rounds: u32,
    category: &str,
) {
    let phase_word = match phase {
        LobbyPhase::Waiting => "Waiting to start",
        LobbyPhase::Between => "Between rounds",
        LobbyPhase::Spectating => "Spectating",
        LobbyPhase::GameOver => "Game over",
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new("CODE").color(MUTED).size(11.0).strong());
        ui.label(RichText::new(code).color(CORAL).size(18.0).strong());
        ui.add_space(8.0);
        let mut count = format!("· {playing} playing");
        if waiting > 0 {
            count.push_str(&format!(" · {waiting} waiting"));
        }
        ui.label(RichText::new(count).color(MUTED).size(13.5));
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!("· {phase_word}"))
                .color(GOLD)
                .size(13.5)
                .strong(),
        );
    });
    let mut cfg = format!("{mode} · {rounds} rounds");
    if !category.is_empty() {
        cfg.push_str(&format!(" · {category}"));
    }
    ui.label(RichText::new(cfg).color(MUTED).size(13.0));
}

fn roster(ui: &mut egui::Ui, label: &str, names: &[String], me: &str, accent: Color32) {
    ui.label(RichText::new(label).color(MUTED).size(11.0).strong());
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        for name in names {
            let is_me = name == me;
            let text = if is_me {
                format!("● {name} (you)")
            } else {
                format!("● {name}")
            };
            let color = if is_me { accent } else { TEXT };
            chip(ui, &text, color);
        }
        if names.is_empty() {
            ui.label(RichText::new("(connecting…)").color(MUTED).size(13.0));
        }
    });
    ui.add_space(6.0);
}

fn board_view(ui: &mut egui::Ui, board: &[(String, u32, u32)], me: &str) {
    if board.is_empty() {
        ui.label(RichText::new("No scores yet.").color(MUTED).size(14.0));
        return;
    }
    for (i, (name, points, solves)) in board.iter().enumerate() {
        let is_me = name == me;
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(
            rect,
            CornerRadius::same(10),
            if is_me { PANEL_HI } else { INK },
        );
        if is_me {
            let bar = Rect::from_min_size(
                rect.left_top() + vec2(0.0, 8.0),
                vec2(3.0, rect.height() - 16.0),
            );
            p.rect_filled(bar, CornerRadius::same(2), MINT);
        }
        p.text(
            pos2(rect.left() + 16.0, rect.center().y),
            Align2::LEFT_CENTER,
            format!("{}", i + 1),
            FontId::proportional(13.0),
            MUTED,
        );
        p.text(
            pos2(rect.left() + 44.0, rect.center().y),
            Align2::LEFT_CENTER,
            if is_me {
                format!("{name}  (you)")
            } else {
                name.clone()
            },
            FontId::proportional(15.0),
            TEXT,
        );
        p.text(
            pos2(rect.right() - 16.0, rect.center().y),
            Align2::RIGHT_CENTER,
            format!("{solves} solved"),
            FontId::proportional(12.5),
            MUTED,
        );
        p.text(
            pos2(rect.right() - 96.0, rect.center().y),
            Align2::RIGHT_CENTER,
            format!("{points} pts"),
            display(16.0),
            GOLD,
        );
    }
}

// --- online widgets -------------------------------------------------------

fn option_card(ui: &mut egui::Ui, title: &str, sub: &str, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 60.0), Sense::click());
    let p = ui.painter();
    let bg = if selected {
        PANEL_HI
    } else if resp.hovered() {
        PANEL
    } else {
        INK
    };
    p.rect_filled(rect, CornerRadius::same(12), bg);
    p.rect_stroke(
        rect,
        CornerRadius::same(12),
        Stroke::new(1.0, LINE),
        StrokeKind::Inside,
    );
    if selected {
        let bar = Rect::from_min_size(
            rect.left_top() + vec2(0.0, 12.0),
            vec2(3.0, rect.height() - 24.0),
        );
        p.rect_filled(bar, CornerRadius::same(2), CORAL);
    }
    p.text(
        pos2(rect.left() + 20.0, rect.center().y - 9.0),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(16.5),
        TEXT,
    );
    p.text(
        pos2(rect.left() + 20.0, rect.center().y + 11.0),
        Align2::LEFT_CENTER,
        sub,
        FontId::proportional(13.0),
        MUTED,
    );
    ui.add_space(4.0);
    resp
}

fn lobby_card(ui: &mut egui::Ui, code: &str, sub: &str, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
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
            rect.left_top() + vec2(0.0, 10.0),
            vec2(3.0, rect.height() - 20.0),
        );
        p.rect_filled(bar, CornerRadius::same(2), CORAL);
    }
    p.text(
        pos2(rect.left() + 20.0, rect.center().y),
        Align2::LEFT_CENTER,
        code,
        display(19.0),
        CORAL,
    );
    p.text(
        pos2(rect.left() + 130.0, rect.center().y),
        Align2::LEFT_CENTER,
        sub,
        FontId::proportional(13.5),
        MUTED,
    );
    ui.add_space(6.0);
    resp
}

fn stepper(
    ui: &mut egui::Ui,
    session: &mut Session,
    label: &str,
    value: &str,
    dec: Key,
    inc: Key,
    width: f32,
) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 40.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(10), PANEL);
    p.rect_stroke(
        rect,
        CornerRadius::same(10),
        Stroke::new(1.0, LINE),
        StrokeKind::Inside,
    );
    p.text(
        pos2(rect.left() + 14.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(10.5),
        MUTED,
    );
    let plus_x = rect.right() - 20.0;
    let minus_x = rect.right() - 96.0;
    p.text(
        pos2((plus_x + minus_x) / 2.0, rect.center().y),
        Align2::CENTER_CENTER,
        value,
        FontId::proportional(15.5),
        TEXT,
    );
    if step_btn(ui, minus_x, rect.center().y, false).clicked() {
        session.handle_key(dec);
    }
    if step_btn(ui, plus_x, rect.center().y, true).clicked() {
        session.handle_key(inc);
    }
}

fn step_btn(ui: &mut egui::Ui, cx: f32, cy: f32, plus: bool) -> egui::Response {
    let r = Rect::from_center_size(pos2(cx, cy), vec2(26.0, 26.0));
    let resp = ui.interact(r, ui.id().with(("step", cx as i32, plus)), Sense::click());
    let hov = resp.hovered();
    let p = ui.painter();
    p.rect_filled(r, CornerRadius::same(7), if hov { PANEL_HI } else { WELL });
    p.rect_stroke(
        r,
        CornerRadius::same(7),
        Stroke::new(1.0, LINE),
        StrokeKind::Inside,
    );
    let col = if hov { CORAL } else { TEXT };
    let c = r.center();
    let s = 6.0;
    p.hline((c.x - s)..=(c.x + s), c.y, Stroke::new(1.8, col));
    if plus {
        p.vline(c.x, (c.y - s)..=(c.y + s), Stroke::new(1.8, col));
    }
    resp
}

fn toggle_pill(
    ui: &mut egui::Ui,
    session: &mut Session,
    label: &str,
    value: &str,
    key: Key,
    width: f32,
) {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 40.0), Sense::click());
    let hov = resp.hovered();
    let p = ui.painter();
    p.rect_filled(
        rect,
        CornerRadius::same(10),
        if hov { PANEL_HI } else { PANEL },
    );
    p.rect_stroke(
        rect,
        CornerRadius::same(10),
        Stroke::new(1.0, LINE),
        StrokeKind::Inside,
    );
    p.text(
        pos2(rect.left() + 14.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(10.5),
        MUTED,
    );
    p.text(
        pos2(rect.right() - 16.0, rect.center().y),
        Align2::RIGHT_CENTER,
        value,
        FontId::proportional(14.5),
        CORAL,
    );
    if resp.clicked() {
        session.handle_key(key);
    }
}
