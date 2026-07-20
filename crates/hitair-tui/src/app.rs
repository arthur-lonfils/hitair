//! Application state and the async event loop.
//!
//! The loop multiplexes three sources with `tokio::select!`: terminal key events
//! (`crossterm::EventStream`), results of async HTTP work (an mpsc channel), and
//! a 100ms tick used for search debouncing and toast expiry. All rodio calls go
//! through the `AudioHandle`, so nothing `!Send` is touched here.

use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{
    Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use futures::StreamExt;
use ratatui::DefaultTerminal;
use ratatui::layout::Rect;
use tokio::sync::mpsc::{self, Receiver, Sender};

use hitair_core::audio::AudioHandle;
use hitair_core::config::{Category, CategorySource, Config};
use hitair_core::deezer::{DeezerClient, Genre, Track};
use hitair_core::game::{GameMode, GuessLog, Outcome, Round};
use hitair_core::lobby::{self, RoundResult, RoundStart};
use hitair_core::realtime::{self, PresenceEntry, RtEvent, RtHandle};
use hitair_core::supa::{self, SupaClient};

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
    // Online "Challenge" mode.
    ChallengeMenu,
    HostConfig,
    Browse,
    JoinCode,
    /// Live multi-round lobby (waiting room / between rounds / game over).
    Lobby,
}

/// What the live lobby is showing between the actual rounds.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LobbyPhase {
    /// Before the first round; players gather, host configures & starts.
    Waiting,
    /// A round just ended: reveal + running board while others finish.
    Between,
    /// Joined mid-game: watching, will play when the host starts the next game.
    Spectating,
    /// All rounds done: final standings; host can start a fresh game.
    GameOver,
}

/// An action to run after the TUI tears down (needs normal stdout/terminal).
#[derive(Clone, Copy)]
pub enum PostAction {
    Update,
    Uninstall,
}

/// A clickable on-screen region, reported by the renderer each frame and
/// hit-tested against mouse clicks.
#[derive(Clone, Copy)]
pub struct Click {
    pub rect: Rect,
    pub action: ClickAction,
}

#[derive(Clone, Copy)]
pub enum ClickAction {
    Replay,
    Skip,
    VolumeUp,
    VolumeDown,
    /// A list row (its meaning depends on the current screen).
    ListItem(usize),
    NextRound,
    ResultToMenu,
    /// The host's primary lobby button (start / next round / new game).
    LobbyPrimary,
    /// Leave the current lobby.
    LobbyLeave,
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
    // Challenge / live lobby.
    PublicParties(Vec<supa::Party>),
    /// Connected to a lobby's Realtime channel (host or joiner).
    LobbyReady {
        code: String,
        handle: RtHandle,
        rx: Receiver<RtEvent>,
        is_host: bool,
        rounds: u32,
        mode: GameMode,
        category_label: String,
        /// Host only: the pool to draw round songs from.
        category: Option<Category>,
    },
    /// A round's song (from a `round_start` broadcast) resolved + downloaded.
    LobbyRoundReady {
        round: u32,
        answer: Box<Track>,
        preview: Vec<u8>,
    },
    /// Host only: the category's playable song pool, cached for picking rounds.
    LobbyPool(Vec<Track>),
    Error(String),
}

/// State for a live multi-round lobby. All clients keep the same cumulative
/// `game`; only the host drives round progression.
pub struct LobbyState {
    pub code: String,
    pub is_host: bool,
    /// Active player names from Realtime presence.
    pub players: Vec<String>,
    /// Names of members waiting out a running game (late joiners).
    pub spectators: Vec<String>,
    pub game: lobby::Game,
    pub phase: LobbyPhase,
    pub rounds: u32,
    pub mode: GameMode,
    pub category_label: String,
    /// Host only: where round songs are drawn from, and the cached pool.
    pub category: Option<Category>,
    pub pool: Vec<Track>,
    /// The just-revealed song (shown between rounds / at game over).
    pub last_answer: Option<Track>,
    pub round_started_at: Option<Instant>,
    /// Whether we've already broadcast our result for the current round.
    pub my_result_sent: bool,
    /// True once we've received the `new_game` that started the current game —
    /// i.e. we're a participant, not a late-joining spectator.
    pub playing_this_game: bool,
    /// True while we're spectating a game we joined mid-way through.
    pub spectating: bool,
}

impl LobbyState {
    /// Best-first standings for display.
    pub fn board(&self) -> Vec<lobby::Standing> {
        self.game.standings.ranked()
    }
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
    /// Output volume, 0.0..=1.0.
    pub volume: f32,
    /// Selected game mode (audio effect) for solo rounds.
    pub game_mode: GameMode,

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
    /// Clickable regions from the last render, for mouse hit-testing.
    pub click_map: Vec<Click>,

    // Challenge (online) — all optional; None `supa` means offline-only.
    supa: Option<SupaClient>,
    pub player_name: String,
    pub editing_name: bool,
    pub challenge_index: usize,
    pub join_input: String,
    pub browse: Vec<supa::Party>,
    pub browse_index: usize,
    /// True while the category menu is being used to pick a song pool to host.
    pub host_selecting: bool,
    pub host_category: Option<Category>,
    pub host_public: bool,
    pub host_max: u32,
    /// Number of rounds the host will run.
    pub host_rounds: u32,
    /// Audio effect the host applies to every round.
    pub host_mode: GameMode,
    /// Whether the in-flight Loading was started from Challenge (for back-routing).
    loading_is_challenge: bool,

    // Live lobby.
    pub lobby: Option<LobbyState>,
    /// Handle for broadcasting on the current lobby channel.
    rt: Option<RtHandle>,
    /// Receiver handed to the event loop on the next iteration (see `run`).
    pending_lobby_rx: Option<Receiver<RtEvent>>,
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
            volume: 1.0,
            game_mode: GameMode::Normal,
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
            click_map: Vec::new(),
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
            host_rounds: 5,
            host_mode: GameMode::Normal,
            loading_is_challenge: false,
            lobby: None,
            rt: None,
            pending_lobby_rx: None,
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

        // The lobby's Realtime receiver lives here as a loop-local so the
        // `select!` arm borrows it (not `self`); it's handed over via a field
        // when a lobby connects, and dropped when we leave.
        let mut lobby_rx: Option<Receiver<RtEvent>> = None;
        loop {
            if let Some(new_rx) = self.pending_lobby_rx.take() {
                lobby_rx = Some(new_rx);
            }
            if self.lobby.is_none() {
                lobby_rx = None;
            }

            let mut clicks = Vec::new();
            terminal.draw(|f| ui::draw(f, &self, &mut clicks))?;
            self.click_map = clicks;
            if self.should_quit {
                break;
            }
            tokio::select! {
                maybe_event = events.next() => match maybe_event {
                    Some(Ok(event)) => self.handle_event(event),
                    Some(Err(_)) | None => self.should_quit = true,
                },
                Some(msg) = rx.recv() => self.handle_msg(msg),
                Some(evt) = recv_rt(&mut lobby_rx) => self.handle_rt_event(evt),
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
        let key = match event {
            Event::Key(k) => k,
            Event::Mouse(m) => return self.handle_mouse(m),
            _ => return,
        };
        if key.kind == KeyEventKind::Release {
            return;
        }
        // Ctrl-C always quits.
        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
            self.should_quit = true;
            return;
        }
        // Global volume: Ctrl+Up / Ctrl+Down (works on any screen).
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Up => return self.adjust_volume(0.1),
                KeyCode::Down => return self.adjust_volume(-0.1),
                _ => {}
            }
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
            Screen::Lobby => self.on_lobby_key(key),
        }
    }

    fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let action = self
                    .click_map
                    .iter()
                    .find(|c| {
                        mouse.column >= c.rect.x
                            && mouse.column < c.rect.x.saturating_add(c.rect.width)
                            && mouse.row >= c.rect.y
                            && mouse.row < c.rect.y.saturating_add(c.rect.height)
                    })
                    .map(|c| c.action);
                if let Some(action) = action {
                    self.dispatch_click(action);
                }
            }
            MouseEventKind::ScrollUp => self.scroll(true),
            MouseEventKind::ScrollDown => self.scroll(false),
            _ => {}
        }
    }

    fn dispatch_click(&mut self, action: ClickAction) {
        match action {
            ClickAction::Replay => self.play_current_clip(),
            ClickAction::Skip => self.skip(),
            ClickAction::VolumeUp => self.adjust_volume(0.1),
            ClickAction::VolumeDown => self.adjust_volume(-0.1),
            ClickAction::ListItem(i) => self.click_list_item(i),
            ClickAction::NextRound => self.on_roundend_key(key_press(KeyCode::Enter)),
            ClickAction::ResultToMenu => self.on_roundend_key(key_press(KeyCode::Char('m'))),
            ClickAction::LobbyPrimary => self.lobby_primary_action(),
            ClickAction::LobbyLeave => self.leave_lobby(),
        }
    }

    /// Clicking a list row = selecting it and pressing Enter on that screen.
    fn click_list_item(&mut self, i: usize) {
        match self.screen {
            Screen::Menu => {
                self.menu_index = i;
                self.on_menu_key(key_press(KeyCode::Enter));
            }
            Screen::Playing => {
                self.suggestion_index = i;
                self.on_playing_key(key_press(KeyCode::Enter));
            }
            Screen::ChallengeMenu if !self.editing_name && i < 3 => {
                self.challenge_index = i;
                self.on_challenge_menu_key(key_press(KeyCode::Enter));
            }
            Screen::Browse => {
                self.browse_index = i;
                self.on_browse_key(key_press(KeyCode::Enter));
            }
            _ => {}
        }
    }

    /// Scroll wheel navigates the focused list (same as Up/Down).
    fn scroll(&mut self, up: bool) {
        let ev = key_press(if up { KeyCode::Up } else { KeyCode::Down });
        match self.screen {
            Screen::Menu => self.on_menu_key(ev),
            Screen::Playing => self.on_playing_key(ev),
            Screen::ChallengeMenu => self.on_challenge_menu_key(ev),
            Screen::Browse => self.on_browse_key(ev),
            Screen::HostConfig => self.on_host_config_key(ev),
            _ => {}
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
            KeyCode::Left => {
                self.game_mode = self.game_mode.prev();
                self.set_status(format!("Game mode: {}", self.game_mode.label()));
            }
            KeyCode::Right => {
                self.game_mode = self.game_mode.next();
                self.set_status(format!("Game mode: {}", self.game_mode.label()));
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
                if self.lobby.is_some() {
                    // Forfeit this round but stay in the lobby.
                    self.finish_lobby_round(false);
                } else {
                    self.audio.stop();
                    self.round = None;
                    self.reset_turn();
                    self.screen = Screen::Menu;
                }
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
                self.audio.stop();
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
            KeyCode::Left => self.host_mode = self.host_mode.prev(),
            KeyCode::Right => self.host_mode = self.host_mode.next(),
            KeyCode::Up if self.host_rounds < 20 => self.host_rounds += 1,
            KeyCode::Down if self.host_rounds > 1 => self.host_rounds -= 1,
            KeyCode::Char('v') => self.host_public = !self.host_public,
            KeyCode::Char('+') | KeyCode::Char('=') if self.host_max < 64 => self.host_max += 1,
            KeyCode::Char('-') | KeyCode::Char('_') if self.host_max > 1 => self.host_max -= 1,
            KeyCode::Enter => self.host_lobby(),
            KeyCode::Esc => {
                // Back to picking a song pool.
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
                    self.join_lobby(code);
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
                    self.join_lobby(party.code);
                }
            }
            KeyCode::Esc => self.screen = Screen::ChallengeMenu,
            _ => {}
        }
    }

    // --- live lobby -------------------------------------------------------

    fn on_lobby_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.lobby_primary_action(),
            KeyCode::Esc => self.leave_lobby(),
            _ => {}
        }
    }

    /// The host's context-sensitive primary action (Enter / big button).
    fn lobby_primary_action(&mut self) {
        let Some(lobby) = &self.lobby else { return };
        if !lobby.is_host {
            return; // players just wait for the host
        }
        match lobby.phase {
            LobbyPhase::Waiting | LobbyPhase::GameOver => self.start_lobby_game(),
            LobbyPhase::Between => self.advance_lobby(),
            LobbyPhase::Spectating => {} // host never spectates
        }
    }

    /// Host: (re)start a game — broadcast the config, then round 1.
    fn start_lobby_game(&mut self) {
        let Some(lobby) = &self.lobby else { return };
        if lobby.pool.is_empty() {
            self.set_status("Still loading songs — try again in a moment.".into());
            return;
        }
        let (rounds, mode, label) = (lobby.rounds, lobby.mode, lobby.category_label.clone());
        if let Some(rt) = &self.rt {
            rt.broadcast(
                lobby::EV_NEW_GAME,
                serde_json::to_value(lobby::NewGame {
                    rounds,
                    mode: mode.tag().to_string(),
                    category: label,
                })
                .unwrap_or_default(),
            );
        }
        if let Some(lobby) = &mut self.lobby {
            lobby.game = lobby::Game::new(rounds, self.schedule.len() as u32);
        }
        self.start_lobby_round(1);
    }

    /// Host: pick a random song from the pool and broadcast the round start.
    fn start_lobby_round(&mut self, round: u32) {
        let Some(lobby) = &self.lobby else { return };
        let Some(track) = pick_random(&lobby.pool) else {
            self.set_status("No playable songs in that category.".into());
            return;
        };
        let track_id = track.id;
        if let Some(rt) = &self.rt {
            rt.broadcast(
                lobby::EV_ROUND_START,
                serde_json::to_value(RoundStart { round, track_id }).unwrap_or_default(),
            );
        }
        // The host also receives its own broadcast (self:true) and loads it,
        // so no special-casing here.
    }

    /// Host: move past the between-rounds board — next round, or end the game.
    fn advance_lobby(&mut self) {
        let Some(lobby) = &self.lobby else { return };
        if lobby.game.is_final_round() {
            if let Some(rt) = &self.rt {
                rt.broadcast(lobby::EV_GAME_OVER, serde_json::json!({}));
            }
            if let Some(lobby) = &mut self.lobby {
                lobby.phase = LobbyPhase::GameOver;
            }
            self.audio.stop();
        } else {
            let next = lobby.game.round + 1;
            self.start_lobby_round(next);
        }
    }

    fn leave_lobby(&mut self) {
        if let Some(rt) = self.rt.take() {
            rt.close();
        }
        self.lobby = None;
        self.round = None;
        self.audio.stop();
        self.reset_turn();
        self.screen = Screen::ChallengeMenu;
    }

    /// Host: fetch the category's playable songs to draw rounds from.
    fn fetch_lobby_pool(&self) {
        let Some(category) = self.lobby.as_ref().and_then(|l| l.category.clone()) else {
            return;
        };
        let deezer = self.deezer.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            if let Ok(tracks) = load_pool(&deezer, &category).await {
                let _ = tx.send(Msg::LobbyPool(tracks)).await;
            }
        });
    }

    /// A `round_start` arrived: resolve the song by id and download its audio.
    fn load_lobby_round(&mut self, round: u32, track_id: i64) {
        self.round = None;
        self.reset_turn();
        self.screen = Screen::Loading;
        let deezer = self.deezer.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let msg = match resolve_round_song(&deezer, track_id).await {
                Ok((answer, preview)) => Msg::LobbyRoundReady {
                    round,
                    answer: Box::new(answer),
                    preview,
                },
                Err(e) => Msg::Error(format!("Couldn't load the round: {e}")),
            };
            let _ = tx.send(msg).await;
        });
    }

    fn handle_rt_event(&mut self, evt: RtEvent) {
        match evt {
            RtEvent::Presence(roster) => self.on_presence(roster),
            RtEvent::Broadcast { event, payload } => self.on_lobby_broadcast(&event, payload),
            RtEvent::Disconnected(reason) => {
                if self.lobby.is_some() {
                    self.set_status(format!("Lobby disconnected: {reason}"));
                    self.leave_lobby();
                }
            }
        }
    }

    /// The lobby roster changed: split it into active players and spectators.
    /// The host re-announces a running game so fresh joiners know to spectate.
    fn on_presence(&mut self, roster: Vec<PresenceEntry>) {
        let announce = {
            let Some(lobby) = &mut self.lobby else { return };
            lobby.players = roster
                .iter()
                .filter(|e| !e.spectating)
                .map(|e| e.name.clone())
                .collect();
            lobby.spectators = roster
                .iter()
                .filter(|e| e.spectating)
                .map(|e| e.name.clone())
                .collect();
            lobby.is_host && lobby.game.round >= 1 && lobby.phase != LobbyPhase::GameOver
        };
        if announce {
            let (round, rounds) = self
                .lobby
                .as_ref()
                .map(|l| (l.game.round, l.rounds))
                .unwrap_or((0, 0));
            if let Some(rt) = &self.rt {
                rt.broadcast(
                    lobby::EV_GAME_STATE,
                    serde_json::json!({ "round": round, "rounds": rounds }),
                );
            }
        }
    }

    /// Publish our presence, marking whether we're currently spectating.
    fn update_my_presence(&self, spectating: bool) {
        if let (Some(rt), Some(lobby)) = (&self.rt, &self.lobby) {
            rt.update_presence(serde_json::json!({
                "name": self.player_name,
                "role": if lobby.is_host { "host" } else { "player" },
                "spectating": spectating,
            }));
        }
    }

    /// A game event arrived on the lobby channel. Every client runs the same
    /// logic; only the host ever *sends* start/next/game-over.
    fn on_lobby_broadcast(&mut self, event: &str, payload: serde_json::Value) {
        if self.lobby.is_none() {
            return;
        }
        let max_clips = self.schedule.len() as u32;
        match event {
            lobby::EV_NEW_GAME => {
                if let Ok(ng) = serde_json::from_value::<lobby::NewGame>(payload) {
                    if let Some(lobby) = &mut self.lobby {
                        lobby.rounds = ng.rounds.max(1);
                        lobby.mode = GameMode::from_tag(&ng.mode);
                        lobby.category_label = ng.category;
                        lobby.game = lobby::Game::new(lobby.rounds, max_clips);
                        lobby.phase = LobbyPhase::Waiting;
                        lobby.last_answer = None;
                        lobby.my_result_sent = false;
                        // A fresh game — everyone present now plays it.
                        lobby.playing_this_game = true;
                        lobby.spectating = false;
                    }
                    self.audio.stop();
                    self.round = None;
                    self.screen = Screen::Lobby;
                    self.update_my_presence(false);
                }
            }
            lobby::EV_ROUND_START => {
                if let Ok(rs) = serde_json::from_value::<RoundStart>(payload) {
                    let participate = {
                        let Some(lobby) = &mut self.lobby else { return };
                        lobby.game.start_round(rs.round);
                        lobby.is_host || lobby.playing_this_game
                    };
                    if participate {
                        self.load_lobby_round(rs.round, rs.track_id);
                    } else {
                        self.enter_spectating();
                    }
                }
            }
            lobby::EV_GAME_STATE => {
                // A game is running. If we didn't start it, we're a late joiner
                // and only spectate until the host starts the next game.
                let rounds = payload.get("rounds").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                if let Some(lobby) = &mut self.lobby
                    && rounds > 0
                {
                    lobby.rounds = rounds;
                }
                self.enter_spectating();
            }
            lobby::EV_RESULT => {
                if let Ok(r) = serde_json::from_value::<RoundResult>(payload)
                    && let Some(lobby) = &mut self.lobby
                {
                    lobby.game.on_result(&r);
                }
            }
            lobby::EV_GAME_OVER => {
                if let Some(lobby) = &mut self.lobby {
                    lobby.phase = LobbyPhase::GameOver;
                }
                self.audio.stop();
                self.round = None;
                self.screen = Screen::Lobby;
            }
            _ => {}
        }
    }

    /// Become a spectator of the currently-running game (no-op for participants
    /// and the host, or if we're already spectating).
    fn enter_spectating(&mut self) {
        let newly = {
            let Some(lobby) = &mut self.lobby else { return };
            if lobby.is_host || lobby.playing_this_game || lobby.spectating {
                false
            } else {
                lobby.spectating = true;
                lobby.phase = LobbyPhase::Spectating;
                true
            }
        };
        if newly {
            self.round = None;
            self.screen = Screen::Lobby;
            self.update_my_presence(true);
        }
    }

    /// Host: advertise a lobby (a `parties` row for Browse) and connect to its
    /// Realtime channel. The row's `track_id` is a placeholder — songs are chosen
    /// per round and pushed over broadcast, not stored.
    fn host_lobby(&mut self) {
        let (Some(supa), Some(category)) = (self.supa.clone(), self.host_category.clone()) else {
            self.set_status("Online play is unavailable.".into());
            return;
        };
        self.loading_is_challenge = true;
        self.screen = Screen::Loading;
        let tx = self.tx.clone();
        let public = self.host_public;
        let max = self.host_max as i32;
        let host = self.player_name.clone();
        let name = self.player_name.clone();
        let rounds = self.host_rounds;
        let mode = self.host_mode;
        let label = category.name.clone();
        let schedule_ms: Vec<i32> = self.schedule.iter().map(|d| d.as_millis() as i32).collect();
        tokio::spawn(async move {
            let created = supa
                .create_party(supa::NewParty {
                    code: String::new(),
                    visibility: if public { "public" } else { "private" }.into(),
                    max_players: max,
                    track_id: 0, // placeholder: lobby songs are per-round
                    title: format!("Live lobby · {label}"),
                    artist: "hitair".into(),
                    album: None,
                    schedule: schedule_ms,
                    host_name: host,
                })
                .await;
            let party = match created {
                Ok(p) => p,
                Err(e) => {
                    let _ = tx.send(Msg::Error(format!("Couldn't host: {e}"))).await;
                    return;
                }
            };
            connect_lobby(
                tx,
                party.code,
                true,
                name,
                rounds,
                mode,
                label,
                Some(category),
            )
            .await;
        });
    }

    /// Join an existing lobby by its code (from Browse or typed in).
    fn join_lobby(&mut self, code: String) {
        let Some(supa) = self.supa.clone() else {
            self.set_status("Online play is unavailable.".into());
            return;
        };
        self.loading_is_challenge = true;
        self.screen = Screen::Loading;
        let tx = self.tx.clone();
        let name = self.player_name.clone();
        tokio::spawn(async move {
            // Validate the code exists (typo protection); the row also carries
            // the host's chosen capacity for a friendlier error.
            match supa.get_party(&code).await {
                Ok(Some(_)) => {}
                Ok(None) => {
                    let _ = tx
                        .send(Msg::Error(format!("No lobby with code {code}.")))
                        .await;
                    return;
                }
                Err(e) => {
                    let _ = tx.send(Msg::Error(format!("Couldn't join: {e}"))).await;
                    return;
                }
            }
            connect_lobby(
                tx,
                code,
                false,
                name,
                0,
                GameMode::Normal,
                String::new(),
                None,
            )
            .await;
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

    /// A lobby round ended (solved, out of guesses, or forfeited): reveal the
    /// song, record + broadcast our result, and show the running board.
    fn finish_lobby_round(&mut self, won: bool) {
        // Spectators never play a round, so they never finish one.
        if self.lobby.as_ref().is_some_and(|l| l.spectating) {
            return;
        }
        if let Some(round) = &self.round {
            self.audio.play_full(round.preview.clone());
        }
        self.rounds_played += 1;
        let name = self.player_name.clone();
        let Some(round) = self.round.as_ref() else {
            return;
        };
        let clips = if won {
            round.guess_number() as u32
        } else {
            round.total_levels() as u32
        };
        let mistakes = round
            .guesses
            .iter()
            .filter(|g| matches!(g, GuessLog::Wrong(_)))
            .count() as u32;
        let answer = round.answer.clone();
        let Some(lobby) = self.lobby.as_mut() else {
            return;
        };
        if lobby.my_result_sent {
            return; // already scored this round (e.g. Esc after a win)
        }
        let time_ms = lobby
            .round_started_at
            .map(|t| t.elapsed().as_millis() as u32)
            .unwrap_or(0);
        let result = RoundResult {
            round: lobby.game.round,
            name,
            solved: won,
            clips,
            time_ms,
            mistakes,
        };
        // Record locally for instant feedback; the echoed broadcast dedups.
        lobby.game.on_result(&result);
        lobby.last_answer = Some(answer);
        lobby.my_result_sent = true;
        lobby.phase = LobbyPhase::Between;
        if let Some(rt) = &self.rt {
            rt.broadcast(
                lobby::EV_RESULT,
                serde_json::to_value(&result).unwrap_or_default(),
            );
        }
        self.screen = Screen::Lobby;
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
            Msg::LobbyReady {
                code,
                handle,
                rx,
                is_host,
                rounds,
                mode,
                category_label,
                category,
            } => {
                self.rt = Some(handle);
                self.pending_lobby_rx = Some(rx);
                self.lobby = Some(LobbyState {
                    code,
                    is_host,
                    players: Vec::new(),
                    spectators: Vec::new(),
                    game: lobby::Game::new(rounds.max(1), self.schedule.len() as u32),
                    phase: LobbyPhase::Waiting,
                    rounds,
                    mode,
                    category_label,
                    category,
                    pool: Vec::new(),
                    last_answer: None,
                    round_started_at: None,
                    my_result_sent: false,
                    // The host started this game; a joiner opts in on the next
                    // `new_game`. Until then a joiner mid-game only spectates.
                    playing_this_game: is_host,
                    spectating: false,
                });
                self.screen = Screen::Lobby;
                if is_host {
                    self.fetch_lobby_pool();
                }
            }
            Msg::LobbyPool(tracks) => {
                if let Some(lobby) = &mut self.lobby {
                    lobby.pool = tracks;
                }
            }
            Msg::LobbyRoundReady {
                round,
                answer,
                preview,
            } => {
                // Ignore stale round audio (e.g. host already advanced) or if we
                // left the lobby while it was downloading.
                let Some(lobby) = &mut self.lobby else { return };
                if lobby.game.round != round {
                    return;
                }
                lobby.round_started_at = Some(Instant::now());
                lobby.my_result_sent = false;
                self.round = Some(Round::new(*answer, preview, self.schedule.clone()));
                self.reset_turn();
                self.screen = Screen::Playing;
                self.play_current_clip();
            }
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
    }

    // --- actions ----------------------------------------------------------

    fn start_round(&mut self, category: Category) {
        self.last_category = Some(category.clone());
        self.round = None;
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

    /// Route a finished round to the solo result screen or the live-lobby board.
    fn finish(&mut self, won: bool) {
        if self.lobby.is_some() {
            self.finish_lobby_round(won);
        } else {
            self.finish_round(won);
        }
    }

    fn finish_round(&mut self, won: bool) {
        // Reveal: play the whole preview, not just the last clip.
        if let Some(round) = &self.round {
            self.audio.play_full(round.preview.clone());
        }
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

    fn adjust_volume(&mut self, delta: f32) {
        self.volume = (self.volume + delta).clamp(0.0, 1.0);
        self.audio.set_volume(self.volume);
        self.set_status(format!("Volume {}%", (self.volume * 100.0).round() as i32));
    }

    fn play_current_clip(&mut self) {
        let Some(round) = &self.round else { return };
        // A lobby round uses the host's chosen effect; solo uses the menu one.
        let mode = self
            .lobby
            .as_ref()
            .map(|l| l.mode)
            .unwrap_or(self.game_mode);
        self.audio
            .play(round.preview.clone(), round.current_clip(), mode);
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

/// Await the next lobby Realtime event, or park forever when there's no lobby
/// (so the `select!` arm is simply inactive).
async fn recv_rt(rx: &mut Option<Receiver<RtEvent>>) -> Option<RtEvent> {
    match rx {
        Some(r) => r.recv().await,
        None => std::future::pending().await,
    }
}

/// Join a lobby's Realtime channel, then hand the app the transport handles.
#[allow(clippy::too_many_arguments)]
async fn connect_lobby(
    tx: Sender<Msg>,
    code: String,
    is_host: bool,
    name: String,
    rounds: u32,
    mode: GameMode,
    category_label: String,
    category: Option<Category>,
) {
    let topic = format!("lobby-{code}");
    let presence = serde_json::json!({
        "name": name,
        "role": if is_host { "host" } else { "player" },
    });
    let msg = match realtime::join(&topic, presence).await {
        Ok((handle, rx)) => Msg::LobbyReady {
            code,
            handle,
            rx,
            is_host,
            rounds,
            mode,
            category_label,
            category,
        },
        Err(e) => Msg::Error(format!("Couldn't connect to the lobby: {e}")),
    };
    let _ = tx.send(msg).await;
}

/// A random pick from the host's song pool.
fn pick_random(pool: &[Track]) -> Option<&Track> {
    if pool.is_empty() {
        return None;
    }
    let i = (rand::random::<u64>() % pool.len() as u64) as usize;
    pool.get(i)
}

/// Resolve a round's song by id (fresh preview URL) and download its audio.
async fn resolve_round_song(deezer: &DeezerClient, track_id: i64) -> Result<(Track, Vec<u8>)> {
    let answer = deezer.track(track_id).await?;
    anyhow::ensure!(answer.has_preview(), "this song has no preview available");
    let preview = deezer.download_preview(&answer.preview).await?;
    Ok((answer, preview))
}

/// The host's song pool: a category's tracks that actually have a preview.
async fn load_pool(deezer: &DeezerClient, category: &Category) -> Result<Vec<Track>> {
    let mut tracks = match category.source {
        CategorySource::Chart(genre) => deezer.chart_tracks(genre).await?,
        CategorySource::Playlist(id) => deezer.playlist_tracks(id).await?,
    };
    tracks.retain(|t| t.has_preview());
    anyhow::ensure!(!tracks.is_empty(), "no playable tracks in that category");
    Ok(tracks)
}

/// A synthetic key-press event, used to route mouse clicks through the existing
/// keyboard handlers.
fn key_press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
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
