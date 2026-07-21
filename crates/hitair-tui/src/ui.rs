//! All rendering. Pure function of `&App` — no state is mutated here.

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
};

use hitair_core::game::{GameMode, GuessLog, Outcome, Round};
use hitair_core::session::{LobbyPhase, LobbyState, Screen, Session};

use crate::app::{Click, ClickAction};

const ACCENT: Color = Color::Cyan;
const GOOD: Color = Color::Green;
const BAD: Color = Color::Red;
const WARN: Color = Color::Yellow;
const DIM: Color = Color::DarkGray;

pub fn draw(f: &mut Frame, app: &Session, clicks: &mut Vec<Click>) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(f.area());

    draw_header(f, chunks[0], app);
    match app.screen {
        Screen::Menu => draw_menu(f, chunks[1], app, clicks),
        Screen::Loading => draw_centered(f, chunks[1], "Loading…", WARN),
        Screen::Playing => draw_playing(f, chunks[1], app, clicks),
        Screen::RoundEnd => draw_round_end(f, chunks[1], app, clicks),
        Screen::Profile => draw_profile(f, chunks[1], app),
        Screen::ChallengeMenu => draw_challenge_menu(f, chunks[1], app, clicks),
        Screen::HostConfig => draw_host_config(f, chunks[1], app),
        Screen::Browse => draw_browse(f, chunks[1], app, clicks),
        Screen::JoinCode => draw_join(f, chunks[1], app),
        Screen::Lobby => draw_lobby(f, chunks[1], app, clicks),
    }
    draw_footer(f, chunks[2], app);

    if app.confirm_uninstall {
        draw_confirm_uninstall(f);
    }
}

/// Register each visible row of a list as clickable (row i → `ListItem(i)`).
fn register_rows(clicks: &mut Vec<Click>, area: Rect, count: usize) {
    for i in 0..count.min(area.height as usize) {
        clicks.push(Click {
            rect: Rect {
                x: area.x,
                y: area.y + i as u16,
                width: area.width,
                height: 1,
            },
            action: ClickAction::ListItem(i),
        });
    }
}

/// Render a row of `[ label ]` buttons and register their click rects.
fn button_row(f: &mut Frame, area: Rect, clicks: &mut Vec<Click>, buttons: &[(&str, ClickAction)]) {
    let mut spans = vec![Span::raw(" ")];
    let mut x = area.x.saturating_add(1);
    for (label, action) in buttons {
        let text = format!("[ {label} ]");
        let w = text.chars().count() as u16;
        if x.saturating_add(w) > area.x.saturating_add(area.width) {
            break;
        }
        clicks.push(Click {
            rect: Rect {
                x,
                y: area.y,
                width: w,
                height: 1,
            },
            action: *action,
        });
        spans.push(Span::styled(text, Style::default().fg(ACCENT)));
        spans.push(Span::raw("  "));
        x = x.saturating_add(w + 2);
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Centered modal asking to confirm uninstalling the binary.
fn draw_confirm_uninstall(f: &mut Frame) {
    let area = f.area();
    let w = 54.min(area.width);
    let h = 6.min(area.height);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BAD))
        .title(Span::styled(" Uninstall ", Style::default().fg(BAD)));
    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Remove the hitair binary from disk?",
            Style::default().fg(Color::White),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  y = uninstall      ·      any other key = cancel",
            Style::default().fg(DIM),
        )),
    ];
    f.render_widget(Paragraph::new(text).block(block), rect);
}

fn draw_header(f: &mut Frame, area: Rect, app: &Session) {
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(DIM));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let cols =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(inner);

    let title = Line::from(vec![
        Span::styled(
            "  ♪ hitair ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled("— guess the song", Style::default().fg(DIM)),
    ]);
    f.render_widget(Paragraph::new(title), cols[0]);

    // Flash the score for ~1.5s after it increases (pulse every 200ms).
    let flash_ms = app.score_flash_at.map(|t| t.elapsed().as_millis());
    let flashing = flash_ms.is_some_and(|ms| ms < 1500);
    let pulse_on = flash_ms.is_some_and(|ms| (ms / 200).is_multiple_of(2));
    let score_style = if flashing && pulse_on {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(GOOD).add_modifier(Modifier::BOLD)
    };

    let vol_icon = if app.volume <= 0.001 { "🔇" } else { "🔊" };
    let mut stats = vec![
        Span::styled(
            format!("{vol_icon} {}%  ·  ", (app.volume * 100.0).round() as i32),
            Style::default().fg(DIM),
        ),
        Span::styled("Score ", Style::default().fg(DIM)),
        Span::styled(app.score.to_string(), score_style),
    ];
    if flashing {
        stats.push(Span::styled(" ▲", Style::default().fg(GOOD)));
    }
    stats.push(Span::styled("  ·  Streak ", Style::default().fg(DIM)));
    stats.push(Span::styled(
        app.streak.to_string(),
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    ));
    stats.push(Span::styled(
        format!("  ·  Round {}  ", app.rounds_played + 1),
        Style::default().fg(DIM),
    ));
    f.render_widget(
        Paragraph::new(Line::from(stats)).alignment(Alignment::Right),
        cols[1],
    );
}

fn draw_menu(f: &mut Frame, area: Rect, app: &Session, clicks: &mut Vec<Click>) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(DIM))
        .title(Span::styled(
            " Pick a category ",
            Style::default().fg(ACCENT),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);
    let top = Layout::horizontal([Constraint::Min(0), Constraint::Length(30)]).split(rows[0]);

    // Type-to-filter line.
    let filter = Line::from(vec![
        Span::styled(" Filter ", Style::default().fg(DIM)),
        Span::styled(app.menu_filter.clone(), Style::default().fg(Color::White)),
        Span::styled("▏", Style::default().fg(ACCENT)),
    ]);
    f.render_widget(Paragraph::new(filter), top[0]);

    // Game-mode selector (change with ← / →).
    let mode = Line::from(vec![
        Span::styled("Mode ◄ ", Style::default().fg(DIM)),
        Span::styled(
            app.game_mode.label(),
            Style::default().fg(WARN).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ► ", Style::default().fg(DIM)),
    ]);
    f.render_widget(Paragraph::new(mode).alignment(Alignment::Right), top[1]);

    // Filtered category list.
    let items = app.menu_items();
    if items.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "  No matches — type a Deezer playlist id or URL to play a custom list.",
                Style::default().fg(DIM),
            )),
            rows[1],
        );
        return;
    }
    let list_items: Vec<ListItem> = items
        .iter()
        .map(|it| ListItem::new(Line::from(it.label())))
        .collect();
    let list = List::new(list_items)
        .highlight_symbol("› ")
        .highlight_style(Style::default().fg(GOOD).add_modifier(Modifier::BOLD));
    let mut state = ListState::default();
    state.select(Some(app.menu_index.min(items.len() - 1)));
    register_rows(clicks, rows[1], items.len());
    f.render_stateful_widget(list, rows[1], &mut state);
}

fn draw_playing(f: &mut Frame, area: Rect, app: &Session, clicks: &mut Vec<Click>) {
    let Some(round) = &app.round else { return };

    let rows = Layout::vertical([
        Constraint::Length(1), // status line
        Constraint::Length(1), // progress bar
        Constraint::Length(1), // buttons
        Constraint::Length(2), // guesses so far
        Constraint::Length(3), // input box
        Constraint::Min(3),    // suggestions
    ])
    .split(area);

    // Status line: guess counter + clip length (+ audio warning).
    let mut status = Vec::new();
    if let Some(lobby) = &app.lobby {
        status.push(Span::styled(
            format!(" Round {}/{}", lobby.game.round, lobby.rounds),
            Style::default().fg(GOOD).add_modifier(Modifier::BOLD),
        ));
        status.push(Span::styled("   ·  ", Style::default().fg(DIM)));
    }
    status.extend([
        Span::styled(
            format!(" Guess {}/{}", round.guess_number(), round.total_levels()),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled("   ·   Clip ", Style::default().fg(DIM)),
        Span::styled(
            round.current_clip_label(),
            Style::default().fg(WARN).add_modifier(Modifier::BOLD),
        ),
    ]);
    let mode = app.lobby.as_ref().map(|l| l.mode).unwrap_or(app.game_mode);
    if mode != GameMode::Normal {
        status.push(Span::styled(
            format!("   ·   {}", mode.label()),
            Style::default().fg(WARN),
        ));
    }
    if !app.audio_available {
        status.push(Span::styled(
            "   (no audio device — visual only)",
            Style::default().fg(BAD),
        ));
    }
    // In a lobby, show who's already finished while you keep guessing.
    if let Some(lobby) = &app.lobby {
        let done = lobby.game.submitted_count();
        if done > 0 {
            let others: Vec<&str> = lobby
                .game
                .submitted_names()
                .iter()
                .filter(|n| n.as_str() != app.player_name)
                .map(String::as_str)
                .collect();
            let total = lobby.players.len().max(1);
            let who = if others.is_empty() {
                String::new()
            } else {
                format!(": {}", others.join(", "))
            };
            status.push(Span::styled(
                format!("   ·   {done}/{total} finished{who}"),
                Style::default().fg(GOOD),
            ));
        }
    }
    f.render_widget(Paragraph::new(Line::from(status)), rows[0]);

    // Animated playback position within the current clip.
    f.render_widget(
        Paragraph::new(playback_bar(round, app.play_started_at, rows[1].width)),
        rows[1],
    );

    // Playback controls (clickable).
    button_row(
        f,
        rows[2],
        clicks,
        &[
            ("Replay", ClickAction::Replay),
            ("Skip", ClickAction::Skip),
            ("Vol -", ClickAction::VolumeDown),
            ("Vol +", ClickAction::VolumeUp),
        ],
    );

    // Previous guesses.
    f.render_widget(
        Paragraph::new(guesses_line(round)).wrap(Wrap { trim: true }),
        rows[3],
    );

    // Input box.
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(
            " Type the title/artist ",
            Style::default().fg(DIM),
        ));
    let input = Line::from(vec![
        Span::styled(app.input.clone(), Style::default().fg(Color::White)),
        Span::styled("▏", Style::default().fg(ACCENT)), // caret
    ]);
    f.render_widget(Paragraph::new(input).block(input_block), rows[4]);

    // Suggestions.
    let sugg_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(DIM))
        .title(Span::styled(
            " Suggestions (↑↓ or click to pick, Enter to guess) ",
            Style::default().fg(DIM),
        ));

    if app.suggestions.is_empty() {
        let hint = if app.input.trim().chars().count() < 2 {
            "Start typing to search Deezer…"
        } else {
            "No matches — keep typing."
        };
        f.render_widget(
            Paragraph::new(Span::styled(hint, Style::default().fg(DIM))).block(sugg_block),
            rows[5],
        );
    } else {
        let items: Vec<ListItem> = app
            .suggestions
            .iter()
            .map(|t| {
                ListItem::new(Line::from(vec![
                    Span::styled(t.title.clone(), Style::default().fg(Color::White)),
                    Span::styled(format!("  — {}", t.artist_name()), Style::default().fg(DIM)),
                ]))
            })
            .collect();
        let inner = sugg_block.inner(rows[5]);
        let list = List::new(items)
            .block(sugg_block)
            .highlight_symbol("› ")
            .highlight_style(Style::default().fg(GOOD).add_modifier(Modifier::BOLD));
        let mut state = ListState::default();
        state.select(Some(app.suggestion_index));
        register_rows(clicks, inner, app.suggestions.len());
        f.render_stateful_widget(list, rows[5], &mut state);
    }
}

fn draw_round_end(f: &mut Frame, area: Rect, app: &Session, clicks: &mut Vec<Click>) {
    let Some(round) = &app.round else { return };
    let won = round.outcome == Outcome::Won;

    let mut lines = Vec::new();
    lines.push(Line::from(""));
    if won {
        lines.push(Line::from(Span::styled(
            format!(
                "  ✓ Correct in {} {}!",
                round.guess_number(),
                plural("guess", "guesses", round.guess_number())
            ),
            Style::default().fg(GOOD).add_modifier(Modifier::BOLD),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  ✗ Out of guesses.",
            Style::default().fg(BAD).add_modifier(Modifier::BOLD),
        )));
    }
    lines.push(Line::from(""));

    let answer = &round.answer;
    lines.push(Line::from(vec![
        Span::styled("  The song was:  ", Style::default().fg(DIM)),
        Span::styled(
            answer.title.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Artist:        ", Style::default().fg(DIM)),
        Span::styled(
            answer.artist_name().to_string(),
            Style::default().fg(ACCENT),
        ),
    ]));
    if let Some(album) = answer.album_title() {
        lines.push(Line::from(vec![
            Span::styled("  Album:         ", Style::default().fg(DIM)),
            Span::styled(album.to_string(), Style::default().fg(DIM)),
        ]));
    }
    if let Some(anime) = &round.anime {
        lines.push(Line::from(vec![
            Span::styled("  From:          ", Style::default().fg(DIM)),
            Span::styled(
                format!("{} · {}", anime.theme, anime.anime),
                Style::default().fg(WARN).add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    if won {
        lines.push(Line::from(""));
        // Animated points popup: sparkles fan out and the text pulses briefly.
        let ms = app
            .round_end_at
            .map(|t| t.elapsed().as_millis())
            .unwrap_or(9999);
        let spark = "✦ ".repeat(((ms / 90).min(5)) as usize);
        let style = if ms < 1600 && (ms / 220).is_multiple_of(2) {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(GOOD).add_modifier(Modifier::BOLD)
        };
        lines.push(Line::from(Span::styled(
            format!("  {spark}+{} points {spark}", app.last_points),
            style,
        )));
    } else if app.last_points > 0 {
        // Didn't get the song, but named the artist — a consolation.
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  ♪ Right artist — +{} points", app.last_points),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if won { GOOD } else { BAD }))
        .title(Span::styled(
            " Result ",
            Style::default().fg(if won { GOOD } else { BAD }),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let parts = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(inner);
    f.render_widget(Paragraph::new(lines), parts[0]);
    button_row(
        f,
        parts[1],
        clicks,
        &[
            ("Next song", ClickAction::NextRound),
            ("Menu", ClickAction::ResultToMenu),
        ],
    );
}

fn challenge_block(title: &'static str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(DIM))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(ACCENT),
        ))
}

fn draw_profile(f: &mut Frame, area: Rect, app: &Session) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(
            " Profile ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let s = &app.profile.stats;
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", app.profile.name),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
    ];

    if s.rounds == 0 {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  New here — play a round to start your stats.",
            Style::default().fg(DIM),
        )));
        f.render_widget(Paragraph::new(lines), inner);
        return;
    }

    let wr = (s.win_rate() * 100.0).round() as i32;
    lines.push(Line::from(Span::styled(
        format!(
            "  {} rounds  ·  {}% solved  ·  best streak {}  ·  {} pts",
            s.rounds, wr, s.best_streak, s.total_points
        ),
        Style::default().fg(DIM),
    )));
    lines.push(Line::from(""));

    // By category (a 10-cell win-rate bar).
    lines.push(Line::from(Span::styled(
        "  By category",
        Style::default().fg(ACCENT),
    )));
    let mut cats: Vec<_> = s.by_category.iter().collect();
    cats.sort_by(|a, b| b.1.rounds.cmp(&a.1.rounds).then(a.0.cmp(b.0)));
    for (name, c) in cats.iter().take(6) {
        let filled = (c.win_rate() * 10.0).round() as usize;
        let bar = format!("{}{}", "█".repeat(filled), "░".repeat(10 - filled));
        lines.push(Line::from(vec![
            Span::styled(format!("   {:<18} ", clip(name, 18)), Style::default()),
            Span::styled(bar, Style::default().fg(ACCENT)),
            Span::styled(
                format!("  {}/{}", c.wins, c.rounds),
                Style::default().fg(DIM),
            ),
        ]));
    }
    lines.push(Line::from(""));

    // Recent rounds.
    lines.push(Line::from(Span::styled(
        "  Recent",
        Style::default().fg(ACCENT),
    )));
    for g in s.recent.iter().take(8) {
        let (mark, mc) = if g.won { ("✓", GOOD) } else { ("✗", BAD) };
        let meta = if g.won {
            format!("   {} · +{}", g.category, g.points)
        } else {
            format!("   {} · missed", g.category)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {mark} "), Style::default().fg(mc)),
            Span::styled(
                clip(&format!("{} — {}", g.title, g.artist), 38),
                Style::default().fg(Color::White),
            ),
            Span::styled(meta, Style::default().fg(DIM)),
        ]));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

/// Truncate to `n` chars with an ellipsis (for fixed-width TUI rows).
fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
    }
}

fn draw_challenge_menu(f: &mut Frame, area: Rect, app: &Session, clicks: &mut Vec<Click>) {
    let block = challenge_block("Challenge — online");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).split(inner);

    let name_line = if app.editing_name {
        Line::from(vec![
            Span::styled(" Name: ", Style::default().fg(DIM)),
            Span::styled(app.player_name.clone(), Style::default().fg(Color::White)),
            Span::styled("▏", Style::default().fg(ACCENT)),
        ])
    } else {
        Line::from(vec![
            Span::styled(" Playing as ", Style::default().fg(DIM)),
            Span::styled(
                app.player_name.clone(),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled("   (n to rename)", Style::default().fg(DIM)),
        ])
    };
    f.render_widget(Paragraph::new(name_line), rows[0]);

    let items = ["Host a lobby", "Browse public lobbies", "Join by code"];
    let list_items: Vec<ListItem> = items
        .iter()
        .map(|s| ListItem::new(Line::from(*s)))
        .collect();
    let list = List::new(list_items)
        .highlight_symbol("› ")
        .highlight_style(Style::default().fg(GOOD).add_modifier(Modifier::BOLD));
    let mut state = ListState::default();
    state.select(Some(app.challenge_index));
    register_rows(clicks, rows[1], items.len());
    f.render_stateful_widget(list, rows[1], &mut state);
}

fn draw_host_config(f: &mut Frame, area: Rect, app: &Session) {
    let block = challenge_block(if app.editing_lobby {
        "Lobby settings"
    } else {
        "Host a live lobby"
    });
    let category = app
        .host_category
        .as_ref()
        .map(|c| c.name.as_str())
        .unwrap_or("—");
    let visibility = if app.host_public {
        "Public  (anyone can browse & join)"
    } else {
        "Private (join by code only)"
    };
    let field = |label: &'static str, value: String, hint: &'static str| {
        Line::from(vec![
            Span::styled(format!("  {label:<13}"), Style::default().fg(DIM)),
            Span::styled(
                value,
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("   {hint}"), Style::default().fg(DIM)),
        ])
    };
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Song pool:   ", Style::default().fg(DIM)),
            Span::styled(
                category.to_string(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("   c change", Style::default().fg(DIM)),
        ]),
        field("Rounds:", app.host_rounds.to_string(), "↑↓ adjust"),
        field("Game mode:", app.host_mode.label().to_string(), "←→ cycle"),
        field("Visibility:", visibility.to_string(), "v toggle"),
        field("Max players:", app.host_max.to_string(), "+ / − adjust"),
        Line::from(""),
        Line::from(Span::styled(
            if app.editing_lobby {
                "  Enter to save the changes · Esc to cancel."
            } else {
                "  Enter to open the lobby — friends join, then you launch the rounds."
            },
            Style::default().fg(DIM),
        )),
    ];
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_browse(f: &mut Frame, area: Rect, app: &Session, clicks: &mut Vec<Click>) {
    let block = challenge_block("Public lobbies");
    if app.browse.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "  No public lobbies right now — host one!   (r to refresh)",
                Style::default().fg(DIM),
            ))
            .block(block),
            area,
        );
        return;
    }
    // The song stays hidden — it's the challenge. Show code, host, capacity.
    let items: Vec<ListItem> = app
        .browse
        .iter()
        .map(|p| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{}  ", p.code),
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("host {} · up to {} players", p.host_name, p.max_players),
                    Style::default().fg(DIM),
                ),
            ]))
        })
        .collect();
    let inner = block.inner(area);
    let list = List::new(items)
        .block(block)
        .highlight_symbol("› ")
        .highlight_style(Style::default().fg(GOOD).add_modifier(Modifier::BOLD));
    let mut state = ListState::default();
    state.select(Some(app.browse_index.min(app.browse.len() - 1)));
    register_rows(clicks, inner, app.browse.len());
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_join(f: &mut Frame, area: Rect, app: &Session) {
    let block = challenge_block("Join by code");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Enter the party code your host shared:",
            Style::default().fg(DIM),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            Span::styled(
                app.join_input.clone(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("▏", Style::default().fg(ACCENT)),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_lobby(f: &mut Frame, area: Rect, app: &Session, clicks: &mut Vec<Click>) {
    let Some(lobby) = &app.lobby else { return };
    let block = challenge_block("Live lobby");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(inner);

    // Header: code, player count, and game config.
    let phase_word = match lobby.phase {
        LobbyPhase::Waiting => "Waiting to start",
        LobbyPhase::Between => "Between rounds",
        LobbyPhase::Spectating => "Spectating",
        LobbyPhase::GameOver => "Game over",
    };
    let mut config = format!("{} · {} rounds", lobby.mode.label(), lobby.rounds);
    if !lobby.category_label.is_empty() {
        config.push_str(&format!(" · {}", lobby.category_label));
    }
    let mut count = format!("   ·   {} playing", lobby.players.len());
    if !lobby.spectators.is_empty() {
        count.push_str(&format!(" · {} waiting", lobby.spectators.len()));
    }
    let header = vec![
        Line::from(vec![
            Span::styled(" Code ", Style::default().fg(DIM)),
            Span::styled(
                lobby.code.clone(),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(count, Style::default().fg(DIM)),
            Span::styled(
                format!("   ·   {phase_word}"),
                Style::default().fg(WARN).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled(format!(" {config}"), Style::default().fg(DIM))),
    ];
    f.render_widget(Paragraph::new(header), rows[0]);

    match lobby.phase {
        LobbyPhase::Waiting => draw_lobby_waiting(f, rows[1], app, lobby),
        LobbyPhase::Between | LobbyPhase::GameOver | LobbyPhase::Spectating => {
            draw_lobby_board(f, rows[1], app, lobby)
        }
    }

    // A pending "skip while players are still guessing?" replaces the buttons.
    if app.confirm_skip_round {
        let waiting = lobby
            .players
            .len()
            .saturating_sub(lobby.game.submitted_count());
        let prompt = Line::from(vec![
            Span::styled(
                format!(" ⚠ {waiting} still guessing — "),
                Style::default().fg(WARN).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Enter",
                Style::default().fg(GOOD).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" skip anyway  ·  ", Style::default().fg(DIM)),
            Span::styled("Esc", Style::default().fg(BAD).add_modifier(Modifier::BOLD)),
            Span::styled(" keep waiting", Style::default().fg(DIM)),
        ]);
        f.render_widget(Paragraph::new(prompt), rows[2]);
        return;
    }

    // Host controls + Leave button.
    let mut buttons: Vec<(&str, ClickAction)> = Vec::new();
    if lobby.is_host {
        let primary = match lobby.phase {
            LobbyPhase::Waiting => {
                if lobby.pool.is_empty() {
                    "Loading songs…"
                } else {
                    "Start game"
                }
            }
            LobbyPhase::Between => {
                if lobby.game.is_final_round() {
                    "See final scores"
                } else {
                    "Next round"
                }
            }
            LobbyPhase::GameOver => "New game",
            LobbyPhase::Spectating => "Spectating", // host never spectates
        };
        buttons.push((primary, ClickAction::LobbyPrimary));
    }
    buttons.push(("Leave", ClickAction::LobbyLeave));
    button_row(f, rows[2], clicks, &buttons);
}

/// Lines listing members waiting out a running game (late joiners).
fn spectator_lines(lobby: &LobbyState, app: &Session) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if lobby.spectators.is_empty() {
        return lines;
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Waiting to join (next game)",
        Style::default().fg(DIM),
    )));
    for name in &lobby.spectators {
        let me = *name == app.player_name;
        let mut spans = vec![
            Span::styled("    ◦ ", Style::default().fg(DIM)),
            Span::styled(name.clone(), Style::default().fg(WARN)),
        ];
        if me {
            spans.push(Span::styled("  (you)", Style::default().fg(WARN)));
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// The waiting room: the live roster and a hint about who acts next.
fn draw_lobby_waiting(f: &mut Frame, area: Rect, app: &Session, lobby: &LobbyState) {
    let mut lines = vec![Line::from(Span::styled(
        "  Players",
        Style::default().fg(DIM),
    ))];
    if lobby.players.is_empty() {
        lines.push(Line::from(Span::styled(
            "    (connecting…)",
            Style::default().fg(DIM),
        )));
    }
    for name in &lobby.players {
        let me = *name == app.player_name;
        let mut spans = vec![
            Span::styled("    • ", Style::default().fg(DIM)),
            Span::styled(
                name.clone(),
                if me {
                    Style::default().fg(GOOD).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                },
            ),
        ];
        if me {
            spans.push(Span::styled("  (you)", Style::default().fg(GOOD)));
        }
        lines.push(Line::from(spans));
    }
    lines.extend(spectator_lines(lobby, app));
    lines.push(Line::from(""));
    let hint = if lobby.is_host {
        "  You're the host — press Enter / Start game when everyone's in."
    } else {
        "  Waiting for the host to start the game…"
    };
    lines.push(Line::from(Span::styled(hint, Style::default().fg(WARN))));
    f.render_widget(Paragraph::new(lines), area);
}

/// Between rounds / game over / spectating: the running standings + the reveal.
fn draw_lobby_board(f: &mut Frame, area: Rect, app: &Session, lobby: &LobbyState) {
    let mut lines = Vec::new();
    if lobby.phase == LobbyPhase::Spectating {
        lines.push(Line::from(Span::styled(
            "  Spectating — a game is in progress. You'll join when the host starts the next one.",
            Style::default().fg(WARN).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
    }
    if let Some(answer) = &lobby.last_answer {
        lines.push(Line::from(vec![
            Span::styled("  Round ", Style::default().fg(DIM)),
            Span::styled(
                format!("{}/{}", lobby.game.round, lobby.rounds),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ·  song was  ", Style::default().fg(DIM)),
            Span::styled(
                answer.display(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(""));
    }

    let board = lobby.board();
    if board.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No scores yet.",
            Style::default().fg(DIM),
        )));
    } else {
        for (i, s) in board.iter().enumerate() {
            let me = s.name == app.player_name;
            let name_style = if me {
                Style::default().fg(GOOD).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let mut spans = vec![
                Span::styled(format!("  {:>2}. ", i + 1), Style::default().fg(DIM)),
                Span::styled(format!("{:<16}", s.name), name_style),
                Span::styled(
                    format!("{:>3} pts", s.points),
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("   {} solved", s.solves), Style::default().fg(DIM)),
            ];
            if me {
                spans.push(Span::styled("   ← you", Style::default().fg(GOOD)));
            }
            lines.push(Line::from(spans));
        }
    }

    if lobby.phase == LobbyPhase::Between {
        lines.push(Line::from(""));
        let waiting = lobby
            .players
            .len()
            .saturating_sub(lobby.game.submitted_count());
        let note = if waiting > 0 {
            format!("  Waiting on {waiting} more player(s)…")
        } else {
            "  Everyone's in.".to_string()
        };
        lines.push(Line::from(Span::styled(note, Style::default().fg(DIM))));
    }
    lines.extend(spectator_lines(lobby, app));
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &Session) {
    if let Some(status) = &app.status {
        f.render_widget(
            Paragraph::new(Span::styled(
                format!(" {status}"),
                Style::default().fg(WARN),
            )),
            area,
        );
        return;
    }
    // On the menu, surface an available update as a highlighted banner.
    if app.screen == Screen::Menu
        && !app.host_selecting
        && let Some(version) = &app.update_available
    {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!(" ⬆ Update v{version} available"),
                    Style::default().fg(GOOD).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    " — Ctrl+U to update   ·   Ctrl+X uninstall   ·   Esc quit",
                    Style::default().fg(DIM),
                ),
            ])),
            area,
        );
        return;
    }
    let help = match app.screen {
        Screen::Menu if app.host_selecting => {
            " Pick a song to host   ↑↓ move   Enter choose   Esc back"
        }
        Screen::Menu => {
            " Type filter   ↑↓ move   ←→ mode   Enter play   Ctrl+O online   Ctrl+P profile   Esc quit"
        }
        Screen::Loading => " Esc cancel",
        Screen::Playing => {
            " Type/click to pick   Enter guess   Ctrl+R replay   Tab skip   Ctrl+↑↓ vol   Esc menu"
        }
        Screen::RoundEnd => " Enter next song   m menu   q quit",
        Screen::Profile => " Your stats & history   Esc back",
        Screen::ChallengeMenu => " ↑↓ move   Enter select   n rename   Esc back",
        Screen::HostConfig if app.editing_lobby => {
            " ↑↓ rounds   ←→ mode   v public/private   +− players   c pool   Enter save   Esc cancel"
        }
        Screen::HostConfig => {
            " ↑↓ rounds   ←→ mode   v public/private   +− players   c pool   Enter open   Esc back"
        }
        Screen::Browse => " ↑↓ move   Enter join   r refresh   Esc back",
        Screen::JoinCode => " Type the code   Enter join   Esc back",
        Screen::Lobby if app.lobby.as_ref().is_some_and(|l| l.is_host) => {
            " Enter host action   s settings   Esc leave lobby"
        }
        Screen::Lobby => " Waiting for the host…   Esc leave lobby",
    };
    f.render_widget(
        Paragraph::new(Span::styled(help, Style::default().fg(DIM))),
        area,
    );
}

fn draw_centered(f: &mut Frame, area: Rect, text: &str, color: Color) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(DIM));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let msg = Paragraph::new(Span::styled(
        text,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))
    .alignment(Alignment::Center);
    // Vertically center within the inner rect.
    let mid = Layout::vertical([
        Constraint::Percentage(45),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(inner);
    f.render_widget(msg, mid[1]);
}

/// How many of `bar_w` cells are filled at `elapsed`/`total` seconds, plus
/// whether the clip has finished. Clamped so it never over/under-fills.
fn playback_fill(elapsed: f32, total: f32, bar_w: usize) -> (usize, bool) {
    let ratio = if total > 0.0 {
        (elapsed / total).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let filled = ((ratio * bar_w as f32).round() as usize).min(bar_w);
    let done = elapsed >= total - 0.01;
    (filled, done)
}

/// Animated playback position within the current clip. `started` is when the
/// clip began playing; the bar fills as it plays and caps when the clip ends.
fn playback_bar(round: &Round, started: Option<Instant>, width: u16) -> Line<'static> {
    let total = round.current_clip().as_secs_f32();
    let elapsed = started
        .map(|s| s.elapsed().as_secs_f32())
        .unwrap_or(0.0)
        .min(total);
    let done = elapsed >= total - 0.01;
    let icon = if done { "↺" } else { "▶" };
    let head = if done { DIM } else { GOOD };
    let left = format!(" {icon} {elapsed:>4.1}s ");
    let right = format!(" {total:.1}s");

    // Reserve room for the labels and the two bracket glyphs.
    let used = left.chars().count() + right.chars().count() + 2;
    let bar_w = (width as usize).saturating_sub(used).max(1);
    let (filled, _) = playback_fill(elapsed, total, bar_w);

    Line::from(vec![
        Span::styled(left, Style::default().fg(head)),
        Span::styled("▕", Style::default().fg(DIM)),
        Span::styled("█".repeat(filled), Style::default().fg(head)),
        Span::styled("░".repeat(bar_w - filled), Style::default().fg(DIM)),
        Span::styled("▏", Style::default().fg(DIM)),
        Span::styled(right, Style::default().fg(DIM)),
    ])
}

fn guesses_line(round: &Round) -> Line<'static> {
    if round.guesses.is_empty() {
        return Line::from(Span::styled("  No guesses yet.", Style::default().fg(DIM)));
    }
    let mut spans = vec![Span::styled("  ", Style::default())];
    for (i, g) in round.guesses.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ·  ", Style::default().fg(DIM)));
        }
        match g {
            GuessLog::Wrong(name) => {
                spans.push(Span::styled(format!("✗ {name}"), Style::default().fg(BAD)));
            }
            GuessLog::WrongRightArtist(name) => {
                // Cyan == the artist colour on the reveal: "right artist, wrong song".
                spans.push(Span::styled(
                    format!("♪ {name}"),
                    Style::default().fg(ACCENT),
                ));
            }
            GuessLog::Skipped => {
                spans.push(Span::styled("⏭ skipped", Style::default().fg(WARN)));
            }
        }
    }
    Line::from(spans)
}

fn plural<'a>(one: &'a str, many: &'a str, n: usize) -> &'a str {
    if n == 1 { one } else { many }
}

#[cfg(test)]
mod tests {
    use super::playback_fill;

    #[test]
    fn playback_bar_fills_over_the_clip() {
        // Empty at the start.
        assert_eq!(playback_fill(0.0, 2.0, 20), (0, false));
        // Half way through.
        assert_eq!(playback_fill(1.0, 2.0, 20), (10, false));
        // Full and flagged done at the end.
        assert_eq!(playback_fill(2.0, 2.0, 20), (20, true));
        // Overshoot stays clamped and done.
        assert_eq!(playback_fill(9.0, 2.0, 20), (20, true));
    }
}
