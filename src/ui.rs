//! All rendering. Pure function of `&App` — no state is mutated here.

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
};

use crate::app::{App, Click, ClickAction, Screen};
use crate::game::{GuessLog, Outcome, Round};

const ACCENT: Color = Color::Cyan;
const GOOD: Color = Color::Green;
const BAD: Color = Color::Red;
const WARN: Color = Color::Yellow;
const DIM: Color = Color::DarkGray;

pub fn draw(f: &mut Frame, app: &App, clicks: &mut Vec<Click>) {
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
        Screen::ChallengeMenu => draw_challenge_menu(f, chunks[1], app, clicks),
        Screen::HostConfig => draw_host_config(f, chunks[1], app),
        Screen::Browse => draw_browse(f, chunks[1], app, clicks),
        Screen::JoinCode => draw_join(f, chunks[1], app),
        Screen::Leaderboard => draw_leaderboard(f, chunks[1], app),
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

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
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

fn draw_menu(f: &mut Frame, area: Rect, app: &App, clicks: &mut Vec<Click>) {
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

    // Type-to-filter line.
    let filter = Line::from(vec![
        Span::styled(" Filter ", Style::default().fg(DIM)),
        Span::styled(app.menu_filter.clone(), Style::default().fg(Color::White)),
        Span::styled("▏", Style::default().fg(ACCENT)),
    ]);
    f.render_widget(Paragraph::new(filter), rows[0]);

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

fn draw_playing(f: &mut Frame, area: Rect, app: &App, clicks: &mut Vec<Click>) {
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
    let mut status = vec![
        Span::styled(
            format!(" Guess {}/{}", round.guess_number(), round.total_levels()),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled("   ·   Clip ", Style::default().fg(DIM)),
        Span::styled(
            round.current_clip_label(),
            Style::default().fg(WARN).add_modifier(Modifier::BOLD),
        ),
    ];
    if !app.audio_available {
        status.push(Span::styled(
            "   (no audio device — visual only)",
            Style::default().fg(BAD),
        ));
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

fn draw_round_end(f: &mut Frame, area: Rect, app: &App, clicks: &mut Vec<Click>) {
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

fn draw_challenge_menu(f: &mut Frame, area: Rect, app: &App, clicks: &mut Vec<Click>) {
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

    let items = ["Host a party", "Browse public parties", "Join by code"];
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

fn draw_host_config(f: &mut Frame, area: Rect, app: &App) {
    let block = challenge_block("Host a party");
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
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Category:     ", Style::default().fg(DIM)),
            Span::styled(
                category.to_string(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Visibility:   ", Style::default().fg(DIM)),
            Span::styled(
                visibility,
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled("   ←→ toggle", Style::default().fg(DIM)),
        ]),
        Line::from(vec![
            Span::styled("  Max players:  ", Style::default().fg(DIM)),
            Span::styled(
                app.host_max.to_string(),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled("   ↑↓ adjust", Style::default().fg(DIM)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Enter to lock a random song, create the party, and play.",
            Style::default().fg(DIM),
        )),
    ];
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_browse(f: &mut Frame, area: Rect, app: &App, clicks: &mut Vec<Click>) {
    let block = challenge_block("Public parties");
    if app.browse.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "  No public parties right now — host one!   (r to refresh)",
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

fn draw_join(f: &mut Frame, area: Rect, app: &App) {
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

fn draw_leaderboard(f: &mut Frame, area: Rect, app: &App) {
    let block = challenge_block("Leaderboard");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).split(inner);

    let (code, song) = app
        .active_party
        .as_ref()
        .map(|p| (p.code.clone(), format!("{} — {}", p.title, p.artist)))
        .unwrap_or_default();
    let header = vec![
        Line::from(vec![
            Span::styled(" Party ", Style::default().fg(DIM)),
            Span::styled(
                code,
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled("    The song was: ", Style::default().fg(DIM)),
            Span::styled(
                song,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled(" (updates live)", Style::default().fg(DIM))),
    ];
    f.render_widget(Paragraph::new(header), rows[0]);

    if app.leaderboard.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "  Waiting for results…",
                Style::default().fg(DIM),
            )),
            rows[1],
        );
        return;
    }
    let mut lines = Vec::new();
    for (i, s) in app.leaderboard.iter().enumerate() {
        let is_me = s.player_name == app.player_name;
        let result = if s.solved {
            format!("{} clips · {:.1}s", s.clips_used, s.time_ms as f32 / 1000.0)
        } else {
            "did not solve".to_string()
        };
        let name_style = if is_me {
            Style::default().fg(GOOD).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let result_style = if s.solved {
            Style::default().fg(ACCENT)
        } else {
            Style::default().fg(BAD)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {:>2}. ", i + 1), Style::default().fg(DIM)),
            Span::styled(format!("{:<16}", s.player_name), name_style),
            Span::styled(result, result_style),
            if is_me {
                Span::styled("   ← you", Style::default().fg(GOOD))
            } else {
                Span::raw("")
            },
        ]));
    }
    f.render_widget(Paragraph::new(lines), rows[1]);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
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
            " Type to filter   ↑↓ move   Enter play   Ctrl+O online   Ctrl+U update   Esc quit"
        }
        Screen::Loading => " Esc cancel",
        Screen::Playing => {
            " Type/click to pick   Enter guess   Ctrl+R replay   Tab skip   Ctrl+↑↓ vol   Esc menu"
        }
        Screen::RoundEnd => " Enter next song   m menu   q quit",
        Screen::ChallengeMenu => " ↑↓ move   Enter select   n rename   Esc back",
        Screen::HostConfig => {
            " ←→ public/private   ↑↓ max players   Enter create & play   Esc back"
        }
        Screen::Browse => " ↑↓ move   Enter join   r refresh   Esc back",
        Screen::JoinCode => " Type the code   Enter join   Esc back",
        Screen::Leaderboard => " r refresh   Enter/Esc back to Challenge",
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
