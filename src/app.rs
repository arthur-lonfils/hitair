//! Application state and the async event loop.
//!
//! The loop multiplexes three sources with `tokio::select!`: terminal key events
//! (`crossterm::EventStream`), results of async HTTP work (an mpsc channel), and
//! a 100ms tick used for search debouncing and toast expiry. All rodio calls go
//! through the `AudioHandle`, so nothing `!Send` is touched here.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc::{self, Receiver, Sender};

use crate::audio::AudioHandle;
use crate::config::{Category, CategorySource, Config};
use crate::deezer::{DeezerClient, Genre, Track};
use crate::game::{GuessLog, Outcome, Round};
use crate::supa::{self, SupaClient};
use crate::ui;

const DEBOUNCE: Duration = Duration::from_millis(250);
const TOAST_TTL: Duration = Duration::from_secs(4);
const MIN_QUERY_LEN: usize = 2;
const LEADERBOARD_POLL: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Menu,
    Loading,
    Playing,
    RoundEnd,
    // Online "Challenge" mode.
    ChallengeMenu,
    HostConfig,
    Browse,
    JoinCode,
    Leaderboard,
}

/// An action to run after the TUI tears down (needs normal stdout/terminal).
#[derive(Clone, Copy)]
pub enum PostAction {
    Update,
    Uninstall,
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
    UpdateAvailable(String),
    // Challenge mode.
    PublicParties(Vec<supa::Party>),
    PartyRoundReady {
        party: Box<supa::Party>,
        answer: Box<Track>,
        preview: Vec<u8>,
    },
    Leaderboard(Vec<supa::Score>),
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

    // Self-update / uninstall.
    /// Newer version available on GitHub, if a background check found one.
    pub update_available: Option<String>,
    /// Whether the uninstall confirmation overlay is showing.
    pub confirm_uninstall: bool,
    /// Action to run after the TUI exits.
    post_action: Option<PostAction>,

    // Challenge (online) — all optional; None `supa` means offline-only.
    supa: Option<SupaClient>,
    pub player_name: String,
    pub editing_name: bool,
    pub challenge_index: usize,
    pub join_input: String,
    pub browse: Vec<supa::Party>,
    pub browse_index: usize,
    /// True while the category menu is being used to pick a song to host.
    pub host_selecting: bool,
    pub host_category: Option<Category>,
    pub host_public: bool,
    pub host_max: u32,
    /// The party whose round is currently being played (None = solo round).
    pub active_party: Option<supa::Party>,
    /// Whether the in-flight Loading was started from Challenge (for back-routing).
    loading_is_challenge: bool,
    party_started_at: Option<Instant>,
    pub leaderboard: Vec<supa::Score>,
    leaderboard_at: Option<Instant>,
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
            update_available: None,
            confirm_uninstall: false,
            post_action: None,
            supa: SupaClient::new().ok(),
            player_name: default_player_name(),
            editing_name: false,
            challenge_index: 0,
            join_input: String::new(),
            browse: Vec::new(),
            browse_index: 0,
            host_selecting: false,
            host_category: None,
            host_public: true,
            host_max: 8,
            active_party: None,
            loading_is_challenge: false,
            party_started_at: None,
            leaderboard: Vec::new(),
            leaderboard_at: None,
        };
        (app, rx)
    }

    pub async fn run(
        mut self,
        mut terminal: DefaultTerminal,
        mut rx: Receiver<Msg>,
    ) -> Result<Option<PostAction>> {
        let mut events = EventStream::new();
        let mut ticker = tokio::time::interval(Duration::from_millis(100));
        self.fetch_genres();
        self.check_for_update();

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
        Ok(self.post_action)
    }

    /// Check GitHub for a newer release in the background (fail-silent).
    /// Set `HITAIR_NO_UPDATE_CHECK` to skip.
    fn check_for_update(&self) {
        if std::env::var_os("HITAIR_NO_UPDATE_CHECK").is_some() {
            return;
        }
        let tx = self.tx.clone();
        tokio::spawn(async move {
            if let Ok(Some(version)) = crate::update::latest_if_newer().await {
                let _ = tx.send(Msg::UpdateAvailable(version)).await;
            }
        });
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
        // The uninstall confirmation overlay intercepts all input.
        if self.confirm_uninstall {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.post_action = Some(PostAction::Uninstall);
                    self.should_quit = true;
                }
                _ => self.confirm_uninstall = false,
            }
            return;
        }
        match self.screen {
            Screen::Menu => self.on_menu_key(key),
            Screen::Loading => {
                if key.code == KeyCode::Esc {
                    self.screen = if self.loading_is_challenge {
                        Screen::ChallengeMenu
                    } else {
                        Screen::Menu
                    };
                }
            }
            Screen::Playing => self.on_playing_key(key),
            Screen::RoundEnd => self.on_roundend_key(key),
            Screen::ChallengeMenu => self.on_challenge_menu_key(key),
            Screen::HostConfig => self.on_host_config_key(key),
            Screen::Browse => self.on_browse_key(key),
            Screen::JoinCode => self.on_join_key(key),
            Screen::Leaderboard => self.on_leaderboard_key(key),
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
                    if self.host_selecting {
                        self.host_category = Some(item.into_category());
                        self.host_selecting = false;
                        self.screen = Screen::HostConfig;
                    } else {
                        self.start_round(item.into_category());
                    }
                }
            }
            KeyCode::Char('o') if ctrl => self.open_challenge_menu(),
            KeyCode::Char('u') if ctrl => {
                if self.update_available.is_some() {
                    self.post_action = Some(PostAction::Update);
                    self.should_quit = true;
                } else {
                    self.set_status("You're on the latest version.".into());
                }
            }
            KeyCode::Char('x') if ctrl => self.confirm_uninstall = true,
            KeyCode::Backspace => {
                self.menu_filter.pop();
                self.menu_index = 0;
            }
            KeyCode::Esc => {
                if self.host_selecting {
                    self.host_selecting = false;
                    self.screen = Screen::ChallengeMenu;
                } else if self.menu_filter.is_empty() {
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
                self.screen = if self.active_party.take().is_some() {
                    Screen::ChallengeMenu
                } else {
                    Screen::Menu
                };
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

    // --- challenge (online) -----------------------------------------------

    fn open_challenge_menu(&mut self) {
        if self.supa.is_none() {
            self.set_status("Online play is unavailable.".into());
            return;
        }
        self.challenge_index = 0;
        self.editing_name = false;
        self.screen = Screen::ChallengeMenu;
    }

    fn on_challenge_menu_key(&mut self, key: KeyEvent) {
        if self.editing_name {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    if self.player_name.trim().is_empty() {
                        self.player_name = default_player_name();
                    }
                    self.editing_name = false;
                }
                KeyCode::Backspace => {
                    self.player_name.pop();
                }
                KeyCode::Char(c) if self.player_name.chars().count() < 20 => {
                    self.player_name.push(c);
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Up => self.challenge_index = self.challenge_index.saturating_sub(1),
            KeyCode::Down if self.challenge_index + 1 < 3 => self.challenge_index += 1,
            KeyCode::Char('n') => self.editing_name = true,
            KeyCode::Enter => match self.challenge_index {
                0 => {
                    // Host: pick a song via the category menu.
                    self.host_selecting = true;
                    self.menu_filter.clear();
                    self.menu_index = 0;
                    self.screen = Screen::Menu;
                }
                1 => {
                    self.browse.clear();
                    self.browse_index = 0;
                    self.screen = Screen::Browse;
                    self.refresh_public_parties();
                }
                2 => {
                    self.join_input.clear();
                    self.screen = Screen::JoinCode;
                }
                _ => {}
            },
            KeyCode::Esc => self.screen = Screen::Menu,
            _ => {}
        }
    }

    fn on_host_config_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Left | KeyCode::Right | KeyCode::Char('v') => {
                self.host_public = !self.host_public;
            }
            KeyCode::Up | KeyCode::Char('+') | KeyCode::Char('=') if self.host_max < 64 => {
                self.host_max += 1;
            }
            KeyCode::Down | KeyCode::Char('-') if self.host_max > 1 => self.host_max -= 1,
            KeyCode::Enter => self.start_host_party(),
            KeyCode::Esc => {
                // Back to picking a song.
                self.host_selecting = true;
                self.screen = Screen::Menu;
            }
            _ => {}
        }
    }

    fn on_join_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                let code = self.join_input.trim().to_uppercase();
                if !code.is_empty() {
                    self.join_by_code(code);
                }
            }
            KeyCode::Backspace => {
                self.join_input.pop();
            }
            KeyCode::Esc => self.screen = Screen::ChallengeMenu,
            KeyCode::Char(c) if c.is_ascii_alphanumeric() && self.join_input.len() < 12 => {
                self.join_input.push(c.to_ascii_uppercase());
            }
            _ => {}
        }
    }

    fn on_browse_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => self.browse_index = self.browse_index.saturating_sub(1),
            KeyCode::Down if self.browse_index + 1 < self.browse.len() => self.browse_index += 1,
            KeyCode::Char('r') => self.refresh_public_parties(),
            KeyCode::Enter => {
                if let Some(party) = self.browse.get(self.browse_index).cloned() {
                    self.join_party(party);
                }
            }
            KeyCode::Esc => self.screen = Screen::ChallengeMenu,
            _ => {}
        }
    }

    fn on_leaderboard_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('r') => self.refresh_leaderboard(),
            KeyCode::Enter | KeyCode::Esc | KeyCode::Char('m') => {
                self.active_party = None;
                self.round = None;
                self.leaderboard.clear();
                self.screen = Screen::ChallengeMenu;
            }
            _ => {}
        }
    }

    fn start_host_party(&mut self) {
        let (Some(supa), Some(category)) = (self.supa.clone(), self.host_category.clone()) else {
            return;
        };
        self.active_party = None;
        self.loading_is_challenge = true;
        self.screen = Screen::Loading;
        let deezer = self.deezer.clone();
        let tx = self.tx.clone();
        let public = self.host_public;
        let max = self.host_max as i32;
        let host = self.player_name.clone();
        let schedule_ms: Vec<i32> = self.schedule.iter().map(|d| d.as_millis() as i32).collect();
        tokio::spawn(async move {
            let msg =
                match load_host_party(&deezer, &supa, &category, public, max, &host, schedule_ms)
                    .await
                {
                    Ok((party, answer, preview)) => Msg::PartyRoundReady {
                        party: Box::new(party),
                        answer: Box::new(answer),
                        preview,
                    },
                    Err(e) => Msg::Error(format!("Couldn't host: {e}")),
                };
            let _ = tx.send(msg).await;
        });
    }

    fn join_by_code(&mut self, code: String) {
        let Some(supa) = self.supa.clone() else {
            return;
        };
        self.active_party = None;
        self.loading_is_challenge = true;
        self.screen = Screen::Loading;
        let deezer = self.deezer.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let msg = match load_join_party(&deezer, &supa, &code).await {
                Ok((party, answer, preview)) => Msg::PartyRoundReady {
                    party: Box::new(party),
                    answer: Box::new(answer),
                    preview,
                },
                Err(e) => Msg::Error(format!("{e}")),
            };
            let _ = tx.send(msg).await;
        });
    }

    fn join_party(&mut self, party: supa::Party) {
        self.active_party = None;
        self.loading_is_challenge = true;
        self.screen = Screen::Loading;
        let deezer = self.deezer.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let msg = match resolve_party_round(&deezer, &party).await {
                Ok((answer, preview)) => Msg::PartyRoundReady {
                    party: Box::new(party),
                    answer: Box::new(answer),
                    preview,
                },
                Err(e) => Msg::Error(format!("Couldn't join: {e}")),
            };
            let _ = tx.send(msg).await;
        });
    }

    fn refresh_public_parties(&self) {
        let Some(supa) = self.supa.clone() else {
            return;
        };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            match supa.list_public_parties(20).await {
                Ok(list) => {
                    let _ = tx.send(Msg::PublicParties(list)).await;
                }
                Err(e) => {
                    let _ = tx
                        .send(Msg::Error(format!("Couldn't list parties: {e}")))
                        .await;
                }
            }
        });
    }

    fn refresh_leaderboard(&mut self) {
        let (Some(supa), Some(party)) = (self.supa.clone(), self.active_party.clone()) else {
            return;
        };
        self.leaderboard_at = Some(Instant::now());
        let tx = self.tx.clone();
        tokio::spawn(async move {
            if let Ok(scores) = supa.leaderboard(&party.code, 50).await {
                let _ = tx.send(Msg::Leaderboard(scores)).await;
            }
        });
    }

    fn finish_party_round(&mut self, won: bool) {
        self.audio.stop();
        self.rounds_played += 1;
        let (Some(round), Some(party), Some(supa)) = (
            self.round.as_ref(),
            self.active_party.clone(),
            self.supa.clone(),
        ) else {
            self.screen = Screen::ChallengeMenu;
            return;
        };
        let clips_used = if won {
            round.guess_number() as i32
        } else {
            round.total_levels() as i32
        };
        let mistakes = round
            .guesses
            .iter()
            .filter(|g| matches!(g, GuessLog::Wrong(_)))
            .count() as i32;
        let time_ms = self
            .party_started_at
            .map(|t| t.elapsed().as_millis() as i32)
            .unwrap_or(0);
        let score = supa::Score {
            party_code: party.code.clone(),
            player_name: self.player_name.clone(),
            solved: won,
            clips_used,
            time_ms,
            mistakes,
            created_at: None,
        };
        self.leaderboard.clear();
        self.leaderboard_at = Some(Instant::now());
        self.screen = Screen::Leaderboard;
        let tx = self.tx.clone();
        let code = party.code.clone();
        tokio::spawn(async move {
            let _ = supa.submit_score(&score).await;
            if let Ok(scores) = supa.leaderboard(&code, 50).await {
                let _ = tx.send(Msg::Leaderboard(scores)).await;
            }
        });
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
            Msg::UpdateAvailable(version) => self.update_available = Some(version),
            Msg::PublicParties(list) => {
                self.browse = list;
                if self.browse_index >= self.browse.len() {
                    self.browse_index = 0;
                }
            }
            Msg::PartyRoundReady {
                party,
                answer,
                preview,
            } => {
                if self.screen != Screen::Loading {
                    return;
                }
                let schedule: Vec<Duration> = party
                    .schedule
                    .iter()
                    .map(|ms| Duration::from_millis((*ms).max(0) as u64))
                    .collect();
                let schedule = if schedule.is_empty() {
                    self.schedule.clone()
                } else {
                    schedule
                };
                self.active_party = Some(*party);
                self.round = Some(Round::new(*answer, preview, schedule));
                self.reset_turn();
                self.party_started_at = Some(Instant::now());
                self.screen = Screen::Playing;
                self.play_current_clip();
            }
            Msg::Leaderboard(scores) => self.leaderboard = scores,
            Msg::Error(err) => {
                self.set_status(err);
                if self.screen == Screen::Loading {
                    self.screen = if self.loading_is_challenge {
                        Screen::ChallengeMenu
                    } else {
                        Screen::Menu
                    };
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
        // Poll the party leaderboard so it feels live.
        if self.screen == Screen::Leaderboard
            && self
                .leaderboard_at
                .is_none_or(|t| t.elapsed() >= LEADERBOARD_POLL)
        {
            self.refresh_leaderboard();
        }
    }

    // --- actions ----------------------------------------------------------

    fn start_round(&mut self, category: Category) {
        self.last_category = Some(category.clone());
        self.round = None;
        self.active_party = None;
        self.loading_is_challenge = false;
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
            Outcome::Won => self.finish(true),
            Outcome::Lost => self.finish(false),
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
            Outcome::Lost => self.finish(false),
            _ => self.play_current_clip(),
        }
    }

    /// Route a finished round to the solo result screen or the party leaderboard.
    fn finish(&mut self, won: bool) {
        if self.active_party.is_some() {
            self.finish_party_round(won);
        } else {
            self.finish_round(won);
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

/// Resolve a party's song by id (fresh preview URL) and download its audio.
async fn resolve_party_round(
    deezer: &DeezerClient,
    party: &supa::Party,
) -> Result<(Track, Vec<u8>)> {
    let answer = deezer.track(party.track_id).await?;
    anyhow::ensure!(answer.has_preview(), "this song has no preview available");
    let preview = deezer.download_preview(&answer.preview).await?;
    Ok((answer, preview))
}

/// Look up a party by code, verify it isn't full, then resolve its song.
async fn load_join_party(
    deezer: &DeezerClient,
    supa: &SupaClient,
    code: &str,
) -> Result<(supa::Party, Track, Vec<u8>)> {
    let party = supa
        .get_party(code)
        .await?
        .context("no party with that code")?;
    let count = supa.player_count(&party.code).await.unwrap_or(0);
    anyhow::ensure!((count as i32) < party.max_players, "that party is full");
    let (answer, preview) = resolve_party_round(deezer, &party).await?;
    Ok((party, answer, preview))
}

/// Pick a random song from a category, create the party, and load the audio.
#[allow(clippy::too_many_arguments)]
async fn load_host_party(
    deezer: &DeezerClient,
    supa: &SupaClient,
    category: &Category,
    public: bool,
    max_players: i32,
    host_name: &str,
    schedule_ms: Vec<i32>,
) -> Result<(supa::Party, Track, Vec<u8>)> {
    let mut tracks = match category.source {
        CategorySource::Chart(genre) => deezer.chart_tracks(genre).await?,
        CategorySource::Playlist(id) => deezer.playlist_tracks(id).await?,
    };
    tracks.retain(|t| t.has_preview());
    anyhow::ensure!(!tracks.is_empty(), "no playable tracks in that category");

    let index = (rand::random::<u64>() % tracks.len() as u64) as usize;
    let track = tracks.swap_remove(index);
    let party = supa
        .create_party(supa::NewParty {
            code: String::new(),
            visibility: if public { "public" } else { "private" }.into(),
            max_players,
            track_id: track.id,
            title: track.title.clone(),
            artist: track.artist_name().to_string(),
            album: track.album_title().map(str::to_string),
            schedule: schedule_ms,
            host_name: host_name.to_string(),
        })
        .await?;
    let preview = deezer.download_preview(&track.preview).await?;
    Ok((party, track, preview))
}

/// Default leaderboard name from the OS username.
fn default_player_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "player".into())
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
