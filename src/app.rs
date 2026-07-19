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
use crate::deezer::{DeezerClient, Genre, Track};
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
    Genres(Vec<Genre>),
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
    /// Live category list (fallback genres → live genres once loaded, + playlists).
    pub categories: Vec<Category>,
    pub menu_index: usize,
    /// Type-to-filter text on the menu (also accepts a Deezer playlist id/URL).
    pub menu_filter: String,

    // Round in progress.
    pub round: Option<Round>,
    pub input: String,
    pub suggestions: Vec<Track>,
    pub suggestion_index: usize,
    /// When the current clip started playing, for the animated playback bar.
    pub play_started_at: Option<Instant>,
    search_gen: u64,
    pending_search_at: Option<Instant>,
    /// The category of the current/last round, for "play again".
    last_category: Option<Category>,

    // Stats.
    pub score: u32,
    pub streak: u32,
    pub rounds_played: u32,
    /// When the score last increased, to flash the header counter.
    pub score_flash_at: Option<Instant>,
    /// When the round ended, to animate the result popup.
    pub round_end_at: Option<Instant>,
    /// Points awarded for the last won round.
    pub last_points: u32,
}

impl App {
    pub fn new(cfg: Config, deezer: DeezerClient, audio: AudioHandle) -> (Self, Receiver<Msg>) {
        let (tx, rx) = mpsc::channel(64);
        let schedule = cfg.schedule_durations();
        let categories = cfg.default_categories();
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
            categories,
            menu_index: 0,
            menu_filter: String::new(),
            round: None,
            input: String::new(),
            suggestions: Vec::new(),
            suggestion_index: 0,
            play_started_at: None,
            search_gen: 0,
            pending_search_at: None,
            last_category: None,
            score: 0,
            streak: 0,
            rounds_played: 0,
            score_flash_at: None,
            round_end_at: None,
            last_points: 0,
        };
        (app, rx)
    }

    pub async fn run(mut self, mut terminal: DefaultTerminal, mut rx: Receiver<Msg>) -> Result<()> {
        let mut events = EventStream::new();
        let mut ticker = tokio::time::interval(Duration::from_millis(100));
        self.fetch_genres();

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

    /// The menu list for the current filter: an optional "custom playlist" entry
    /// (when the filter parses as a Deezer id/URL) followed by matching categories.
    pub fn menu_items(&self) -> Vec<MenuItem> {
        let mut items = Vec::new();
        if let Some(id) = parse_playlist_ref(&self.menu_filter) {
            items.push(MenuItem::CustomPlaylist(id));
        }
        let needle = self.menu_filter.trim().to_lowercase();
        for c in &self.categories {
            if needle.is_empty() || c.name.to_lowercase().contains(&needle) {
                items.push(MenuItem::Category(c.clone()));
            }
        }
        items
    }

    fn on_menu_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Up => self.menu_index = self.menu_index.saturating_sub(1),
            KeyCode::Down if self.menu_index + 1 < self.menu_items().len() => {
                self.menu_index += 1;
            }
            KeyCode::Enter => {
                if let Some(item) = self.menu_items().into_iter().nth(self.menu_index) {
                    self.start_round(item.into_category());
                }
            }
            KeyCode::Backspace => {
                self.menu_filter.pop();
                self.menu_index = 0;
            }
            KeyCode::Esc => {
                if self.menu_filter.is_empty() {
                    self.should_quit = true;
                } else {
                    self.menu_filter.clear();
                    self.menu_index = 0;
                }
            }
            KeyCode::Char(c) if !ctrl => {
                self.menu_filter.push(c);
                self.menu_index = 0;
            }
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
                if let Some(cat) = self.last_category.clone() {
                    self.start_round(cat);
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
            Msg::Genres(genres) => {
                // Replace the fallback genre list with Deezer's live one, then
                // append the always-present playlist categories.
                if !genres.is_empty() {
                    let mut categories: Vec<Category> = genres
                        .iter()
                        .map(|g| Category::chart(&g.name, g.id))
                        .collect();
                    categories.extend(self.cfg.playlists.iter().cloned());
                    self.categories = categories;
                    if self.menu_index >= self.categories.len() {
                        self.menu_index = 0;
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

    fn start_round(&mut self, category: Category) {
        self.last_category = Some(category.clone());
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
        self.round_end_at = Some(Instant::now());
        if won {
            let points = self.round.as_ref().map(|r| r.score_value()).unwrap_or(0);
            self.score += points;
            self.last_points = points;
            self.score_flash_at = Some(Instant::now());
            self.streak += 1;
        } else {
            self.last_points = 0;
            self.streak = 0;
        }
        self.screen = Screen::RoundEnd;
    }

    fn play_current_clip(&mut self) {
        let Some(round) = &self.round else { return };
        self.audio.play(round.preview.clone(), round.current_clip());
        self.play_started_at = Some(Instant::now());
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

    fn fetch_genres(&self) {
        let client = self.deezer.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            if let Ok(genres) = client.genres().await {
                let _ = tx.send(Msg::Genres(genres)).await;
            }
        });
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

/// A row on the menu: either a known category or a custom playlist parsed from
/// the filter text.
pub enum MenuItem {
    Category(Category),
    CustomPlaylist(i64),
}

impl MenuItem {
    pub fn label(&self) -> String {
        match self {
            MenuItem::Category(c) => c.name.clone(),
            MenuItem::CustomPlaylist(id) => format!("▶ Play Deezer playlist {id}"),
        }
    }

    fn into_category(self) -> Category {
        match self {
            MenuItem::Category(c) => c,
            MenuItem::CustomPlaylist(id) => Category::playlist(&format!("Playlist {id}"), id),
        }
    }
}

/// Parse a Deezer playlist reference from filter text: a `playlist/<id>` URL, or
/// a bare numeric id (≥ 6 digits, so short filter words aren't misread as ids).
fn parse_playlist_ref(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Some(pos) = s.find("playlist/") {
        let digits: String = s[pos + "playlist/".len()..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if !digits.is_empty() {
            return digits.parse().ok();
        }
    }
    if s.len() >= 6 && s.chars().all(|c| c.is_ascii_digit()) {
        return s.parse().ok();
    }
    None
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

#[cfg(test)]
mod tests {
    use super::parse_playlist_ref;

    #[test]
    fn parses_playlist_refs() {
        // Bare ids need at least 6 digits so short filter words aren't ids.
        assert_eq!(parse_playlist_ref("867825522"), Some(867825522));
        assert_eq!(parse_playlist_ref("12345"), None);
        assert_eq!(parse_playlist_ref("rock"), None);
        assert_eq!(parse_playlist_ref("80s"), None);
        // URL forms.
        assert_eq!(
            parse_playlist_ref("https://www.deezer.com/en/playlist/908622995"),
            Some(908622995)
        );
        assert_eq!(parse_playlist_ref("playlist/867825522"), Some(867825522));
    }
}
