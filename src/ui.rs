//! All rendering. Pure function of `&App` — no state is mutated here.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Wrap};

use crate::app::{App, Screen};
use crate::game::{GuessLog, Outcome, Round};

const ACCENT: Color = Color::Cyan;
const GOOD: Color = Color::Green;
const BAD: Color = Color::Red;
const WARN: Color = Color::Yellow;
const DIM: Color = Color::DarkGray;

pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(f.area());

    draw_header(f, chunks[0], app);
    match app.screen {
        Screen::Menu => draw_menu(f, chunks[1], app),
        Screen::Loading => draw_centered(f, chunks[1], "Loading a track…", WARN),
        Screen::Playing => draw_playing(f, chunks[1], app),
        Screen::RoundEnd => draw_round_end(f, chunks[1], app),
    }
    draw_footer(f, chunks[2], app);
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

    let stats = Line::from(vec![
        Span::styled("Score ", Style::default().fg(DIM)),
        Span::styled(
            app.score.to_string(),
            Style::default().fg(GOOD).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  Streak ", Style::default().fg(DIM)),
        Span::styled(
            app.streak.to_string(),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  ·  Round {}  ", app.rounds_played + 1),
            Style::default().fg(DIM),
        ),
    ]);
    f.render_widget(Paragraph::new(stats).alignment(Alignment::Right), cols[1]);
}

fn draw_menu(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(DIM))
        .title(Span::styled(
            " Pick a category ",
            Style::default().fg(ACCENT),
        ));

    let items: Vec<ListItem> = app
        .cfg
        .categories
        .iter()
        .map(|c| ListItem::new(Line::from(c.name.clone())))
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_symbol("› ")
        .highlight_style(Style::default().fg(GOOD).add_modifier(Modifier::BOLD));

    let mut state = ListState::default();
    state.select(Some(app.menu_index));
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_playing(f: &mut Frame, area: Rect, app: &App) {
    let Some(round) = &app.round else { return };

    let rows = Layout::vertical([
        Constraint::Length(1), // status line
        Constraint::Length(1), // progress bar
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

    // Segmented reveal bar: one cell per level, filled up to the current one.
    f.render_widget(Paragraph::new(progress_bar(round, rows[1].width)), rows[1]);

    // Previous guesses.
    f.render_widget(
        Paragraph::new(guesses_line(round)).wrap(Wrap { trim: true }),
        rows[2],
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
    f.render_widget(Paragraph::new(input).block(input_block), rows[3]);

    // Suggestions.
    let sugg_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(DIM))
        .title(Span::styled(
            " Suggestions (↑↓ to pick, Enter to guess) ",
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
            rows[4],
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
        let list = List::new(items)
            .block(sugg_block)
            .highlight_symbol("› ")
            .highlight_style(Style::default().fg(GOOD).add_modifier(Modifier::BOLD));
        let mut state = ListState::default();
        state.select(Some(app.suggestion_index));
        f.render_stateful_widget(list, rows[4], &mut state);
    }
}

fn draw_round_end(f: &mut Frame, area: Rect, app: &App) {
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
        lines.push(Line::from(Span::styled(
            format!("  +{} points", round.score_value()),
            Style::default().fg(GOOD),
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
    f.render_widget(Paragraph::new(lines).block(block), area);
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
    let help = match app.screen {
        Screen::Menu => " ↑↓ move   Enter play   q quit",
        Screen::Loading => " Esc cancel",
        Screen::Playing => {
            " Type to search   ↑↓ pick   Enter guess   Ctrl+R replay   Tab skip   Esc menu"
        }
        Screen::RoundEnd => " Enter next song   m menu   q quit",
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

fn progress_bar(round: &Round, width: u16) -> Line<'static> {
    let total = round.total_levels().max(1);
    let filled = (round.level + 1).min(total);
    // Budget the width so segment blocks + separating spaces never overflow.
    let avail = (width as usize).saturating_sub(1);
    let per = (avail / total).saturating_sub(1).max(1);
    let mut spans = vec![Span::raw(" ")];
    for i in 0..total {
        let ch = "█".repeat(per);
        let style = if i < filled {
            Style::default().fg(GOOD)
        } else {
            Style::default().fg(DIM)
        };
        spans.push(Span::styled(ch, style));
        spans.push(Span::raw(" "));
    }
    Line::from(spans)
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
