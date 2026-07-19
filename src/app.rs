//! Application state and the async event loop.
//!
//! The loop multiplexes three sources with `tokio::select!`: terminal key events
//! (`crossterm::EventStream`), results of async HTTP work (an mpsc channel), and
//! a 100ms tick used for search debouncing and toast expiry. All rodio calls go
//! through the `AudioHandle`, so nothing `!Send` is touched here.

use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc::{self, Receiver, Sender};

use crate::audio::AudioHandle;
use crate::config::{Category, CategorySource, Config};
use crate::deezer::{DeezerClient, Track};
use crate::game::{Outcome, Round};
use crate::ui;

const DEBOUNCE: Duration = Duration::from_millis(250);
const TOAST_TTL: Duration = Duration::from_secs(4);
const MIN_QUERY_LEN: usize = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Menu,
    Loading,
    Playing,
    RoundEnd,
}

/// Results delivered back to the loop from spawned async tasks.
pub enum Msg {
    RoundReady {
        answer: Box<Track>,
        preview: Vec<u8>,
    },
    Search {
        generation: u64,
        tracks: Vec<Track>,
    },
    Error(String),
}

pub struct App {
    // Dependencies.
    pub cfg: Config,
    deezer: DeezerClient,
    audio: AudioHandle,
    tx: Sender<Msg>,
    schedule: Vec<Duration>,

    // Screen + global.
    pub screen: Screen,
    pub should_quit: bool,
    pub status: Option<String>,
    status_since: Option<Instant>,
    pub audio_available: bool,

    // Menu.
    pub menu_index: usize,

    // Round in progress.
    pub round: Option<Round>,
    pub input: String,
    pub suggestions: Vec<Track>,
    pub suggestion_index: usize,
    search_gen: u64,
    pending_search_at: Option<Instant>,
    last_category: Option<usize>,

    // Stats.
    pub score: u32,
    pub streak: u32,
    pub rounds_played: u32,
}

impl App {
    pub fn new(cfg: Config, deezer: DeezerClient, audio: AudioHandle) -> (Self, Receiver<Msg>) {
        let (tx, rx) = mpsc::channel(64);
        let schedule = cfg.schedule_durations();
        let audio_available = audio.available();
        let app = App {
            cfg,
            deezer,
            audio,
            tx,
            schedule,
            screen: Screen::Menu,
            should_quit: false,
            status: None,
            status_since: None,
            audio_available,
            menu_index: 0,
            round: None,
            input: String::new(),
            suggestions: Vec::new(),
            suggestion_index: 0,
            search_gen: 0,
            pending_search_at: None,
            last_category: None,
            score: 0,
            streak: 0,
            rounds_played: 0,
        };
        (app, rx)
    }

    pub async fn run(mut self, mut terminal: DefaultTerminal, mut rx: Receiver<Msg>) -> Result<()> {
        let mut events = EventStream::new();
        let mut ticker = tokio::time::interval(Duration::from_millis(100));

        loop {
            terminal.draw(|f| ui::draw(f, &self))?;
            if self.should_quit {
                break;
            }
            tokio::select! {
                maybe_event = events.next() => match maybe_event {
                    Some(Ok(event)) => self.handle_event(event),
                    Some(Err(_)) | None => self.should_quit = true,
                },
                Some(msg) = rx.recv() => self.handle_msg(msg),
                _ = ticker.tick() => self.on_tick(),
            }
        }
        Ok(())
    }

    // --- input ------------------------------------------------------------

    fn handle_event(&mut self, event: Event) {
        let Event::Key(key) = event else { return };
        if key.kind == KeyEventKind::Release {
            return;
        }
        // Ctrl-C always quits.
        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
            self.should_quit = true;
            return;
        }
        match self.screen {
            Screen::Menu => self.on_menu_key(key),
            Screen::Loading => {
                if key.code == KeyCode::Esc {
                    self.screen = Screen::Menu;
                }
            }
            Screen::Playing => self.on_playing_key(key),
            Screen::RoundEnd => self.on_roundend_key(key),
        }
    }

    fn on_menu_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.menu_index = self.menu_index.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j')
                if self.menu_index + 1 < self.cfg.categories.len() =>
            {
                self.menu_index += 1;
            }
            KeyCode::Enter => self.start_round(self.menu_index),
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            _ => {}
        }
    }

    fn on_playing_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                self.audio.stop();
                self.round = None;
                self.reset_turn();
                self.screen = Screen::Menu;
            }
            KeyCode::Enter => {
                if let Some(track) = self.suggestions.get(self.suggestion_index).cloned() {
                    self.make_guess(track);
                }
            }
            KeyCode::Tab => self.skip(),
            KeyCode::Up => self.suggestion_index = self.suggestion_index.saturating_sub(1),
            KeyCode::Down if self.suggestion_index + 1 < self.suggestions.len() => {
                self.suggestion_index += 1;
            }
            KeyCode::Char('r') | KeyCode::Char('p') if ctrl => self.play_current_clip(),
            KeyCode::Backspace => {
                self.input.pop();
                self.queue_search();
            }
            KeyCode::Char(c) if !ctrl => {
                self.input.push(c);
                self.queue_search();
            }
            _ => {}
        }
    }

    fn on_roundend_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                if let Some(index) = self.last_category {
                    self.start_round(index);
                }
            }
            KeyCode::Char('m') | KeyCode::Esc => {
                self.round = None;
                self.screen = Screen::Menu;
            }
            KeyCode::Char('q') => self.should_quit = true,
            _ => {}
        }
    }

    // --- async results ----------------------------------------------------

    fn handle_msg(&mut self, msg: Msg) {
        match msg {
            Msg::RoundReady { answer, preview } => {
                // Ignore if the player already backed out of loading.
                if self.screen != Screen::Loading {
                    return;
                }
                self.round = Some(Round::new(*answer, preview, self.schedule.clone()));
                self.reset_turn();
                self.screen = Screen::Playing;
                self.play_current_clip();
            }
            Msg::Search { generation, tracks } => {
                // Drop stale results from superseded keystrokes.
                if generation == self.search_gen {
                    self.suggestions = tracks;
                    if self.suggestion_index >= self.suggestions.len() {
                        self.suggestion_index = 0;
                    }
                }
            }
            Msg::Error(err) => {
                self.set_status(err);
                if self.screen == Screen::Loading {
                    self.screen = Screen::Menu;
                }
            }
        }
    }

    fn on_tick(&mut self) {
        if let Some(since) = self.status_since
            && since.elapsed() > TOAST_TTL
        {
            self.status = None;
            self.status_since = None;
        }
        if let Some(at) = self.pending_search_at
            && at.elapsed() >= DEBOUNCE
        {
            self.pending_search_at = None;
            self.fire_search();
        }
    }

    // --- actions ----------------------------------------------------------

    fn start_round(&mut self, category_index: usize) {
        let Some(category) = self.cfg.categories.get(category_index).cloned() else {
            return;
        };
        self.last_category = Some(category_index);
        self.round = None;
        self.reset_turn();
        self.screen = Screen::Loading;

        let client = self.deezer.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let msg = match load_round(&client, &category).await {
                Ok((answer, preview)) => Msg::RoundReady {
                    answer: Box::new(answer),
                    preview,
                },
                Err(e) => Msg::Error(format!("Couldn't load “{}”: {e}", category.name)),
            };
            let _ = tx.send(msg).await;
        });
    }

    fn make_guess(&mut self, guess: Track) {
        let outcome = {
            let Some(round) = self.round.as_mut() else {
                return;
            };
            round.submit_guess(&guess);
            round.outcome
        };
        self.reset_turn();
        match outcome {
            Outcome::Won => self.finish_round(true),
            Outcome::Lost => self.finish_round(false),
            Outcome::Playing => self.play_current_clip(),
        }
    }

    fn skip(&mut self) {
        let outcome = {
            let Some(round) = self.round.as_mut() else {
                return;
            };
            round.skip();
            round.outcome
        };
        self.reset_turn();
        match outcome {
            Outcome::Lost => self.finish_round(false),
            _ => self.play_current_clip(),
        }
    }

    fn finish_round(&mut self, won: bool) {
        self.audio.stop();
        self.rounds_played += 1;
        if won {
            if let Some(round) = &self.round {
                self.score += round.score_value();
            }
            self.streak += 1;
        } else {
            self.streak = 0;
        }
        self.screen = Screen::RoundEnd;
    }

    fn play_current_clip(&mut self) {
        if let Some(round) = &self.round {
            self.audio.play(round.preview.clone(), round.current_clip());
        }
    }

    fn queue_search(&mut self) {
        self.suggestion_index = 0;
        if self.input.trim().chars().count() < MIN_QUERY_LEN {
            self.suggestions.clear();
            self.pending_search_at = None;
        } else {
            self.pending_search_at = Some(Instant::now());
        }
    }

    fn fire_search(&mut self) {
        let query = self.input.trim().to_string();
        if query.chars().count() < MIN_QUERY_LEN {
            return;
        }
        self.search_gen += 1;
        let generation = self.search_gen;
        let client = self.deezer.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            if let Ok(tracks) = client.search(&query).await {
                let _ = tx.send(Msg::Search { generation, tracks }).await;
            }
        });
    }

    fn reset_turn(&mut self) {
        self.input.clear();
        self.suggestions.clear();
        self.suggestion_index = 0;
        self.pending_search_at = None;
    }

    fn set_status(&mut self, msg: String) {
        self.status = Some(msg);
        self.status_since = Some(Instant::now());
    }
}

/// Fetch the category's tracks, pick a random one that has a preview, and
/// download its audio. Runs entirely off the UI thread.
async fn load_round(client: &DeezerClient, category: &Category) -> Result<(Track, Vec<u8>)> {
    let mut tracks = match category.source {
        CategorySource::Chart(genre) => client.chart_tracks(genre).await?,
        CategorySource::Playlist(id) => client.playlist_tracks(id).await?,
    };
    tracks.retain(|t| t.has_preview());
    anyhow::ensure!(!tracks.is_empty(), "no playable tracks found");

    let index = (rand::random::<u64>() % tracks.len() as u64) as usize;
    let answer = tracks.swap_remove(index);
    let preview = client.download_preview(&answer.preview).await?;
    Ok((answer, preview))
}
