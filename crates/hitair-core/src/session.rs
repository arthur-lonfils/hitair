//! The UI-agnostic application controller.
//!
//! `Session` owns all app state and every state transition — the round
//! lifecycle, search, the online lobby, audio, and the `Msg`/`RtEvent` handling —
//! but knows nothing about a terminal or a window. Frontends drive it by feeding
//! semantic input (`handle_key`, `list_click`) and the async pumps
//! (`handle_msg`, `handle_rt_event`, `on_tick`), and render from its public
//! state. All rodio calls go through the `AudioHandle`, so nothing `!Send` is
//! touched here.

use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::sync::mpsc::{self, Receiver, Sender};

use crate::anime::{self, AnimeTag};
use crate::audio::AudioHandle;
use crate::config::{self, Category, CategorySource, Config};
use crate::deezer::{DeezerClient, Genre, Track};
use crate::game::{GameMode, GuessLog, Outcome, Round};
use crate::lobby::{self, RoundResult, RoundStart};
use crate::profile::{Profile, RoundRecord};
use crate::realtime::{self, PresenceEntry, RtEvent, RtHandle};
use crate::supa::{self, SupaClient};

const DEBOUNCE: Duration = Duration::from_millis(250);
const TOAST_TTL: Duration = Duration::from_secs(4);
const MIN_QUERY_LEN: usize = 2;
/// Home actions: Play solo, Play online, Profile, Settings.
pub const HOME_ACTIONS: usize = 4;

/// A frontend-agnostic key press. Terminal/GUI frontends translate their native
/// key events into this so the same handlers serve both.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Esc,
    Tab,
    Backspace,
    /// A plain character (no Ctrl).
    Char(char),
    /// Ctrl + a (lowercased) letter.
    Ctrl(char),
    /// Ctrl + arrows — reserved for volume.
    CtrlUp,
    CtrlDown,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// Landing screen: profile summary + the main entry points.
    Home,
    Menu,
    Loading,
    Playing,
    RoundEnd,
    /// Player profile: identity + lifetime stats + recent games.
    Profile,
    /// Preferences: default effect, volume, and maintenance.
    Settings,
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

/// Results delivered back to the loop from spawned async tasks.
pub enum Msg {
    RoundReady {
        answer: Box<Track>,
        preview: Vec<u8>,
        /// Set for anime rounds: the anime the song is from.
        anime: Option<AnimeTag>,
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

pub struct Session {
    // Dependencies.
    pub cfg: Config,
    deezer: DeezerClient,
    audio: AudioHandle,
    tx: Sender<Msg>,
    schedule: Vec<Duration>,

    /// Persistent local player profile (identity + lifetime stats + history).
    pub profile: Profile,

    // Screen + global.
    pub screen: Screen,
    /// Screen to return to when leaving the profile (it's reachable from anywhere).
    profile_return: Screen,
    /// Highlighted action on the Home screen (for keyboard nav).
    pub home_index: usize,
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
    /// Action to run after the frontend exits.
    post_action: Option<PostAction>,

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
    /// True while the Host config screen is editing an existing lobby's settings
    /// rather than creating a new lobby.
    pub editing_lobby: bool,
    /// Whether the in-flight Loading was started from Challenge (for back-routing).
    loading_is_challenge: bool,

    // Live lobby.
    pub lobby: Option<LobbyState>,
    /// Host: awaiting confirmation to skip to the next round while players are
    /// still guessing the current one.
    pub confirm_skip_round: bool,
    /// Handle for broadcasting on the current lobby channel.
    rt: Option<RtHandle>,
    /// Receiver handed to the event loop on the next iteration (see `run`).
    pending_lobby_rx: Option<Receiver<RtEvent>>,
}

impl Session {
    pub fn new(cfg: Config, deezer: DeezerClient, audio: AudioHandle) -> (Self, Receiver<Msg>) {
        let (tx, rx) = mpsc::channel(64);
        let schedule = cfg.schedule_durations();
        let categories = cfg.default_categories();
        let audio_available = audio.available();
        let profile = Profile::load();
        let player_name = profile.name.clone();
        let volume = profile.volume.clamp(0.0, 1.0);
        let game_mode = if profile.mode.is_empty() {
            GameMode::Normal
        } else {
            GameMode::from_tag(&profile.mode)
        };
        let app = Session {
            cfg,
            deezer,
            audio,
            tx,
            schedule,
            profile,
            screen: Screen::Home,
            profile_return: Screen::Home,
            home_index: 0,
            should_quit: false,
            status: None,
            status_since: None,
            audio_available,
            volume,
            game_mode,
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
            player_name,
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
            editing_lobby: false,
            loading_is_challenge: false,
            lobby: None,
            confirm_skip_round: false,
            rt: None,
            pending_lobby_rx: None,
        };
        (app, rx)
    }

    /// A clone of the `Msg` sender, so a frontend can spawn its own async tasks
    /// (e.g. the update check) that feed results back into the session.
    pub fn sender(&self) -> Sender<Msg> {
        self.tx.clone()
    }

    /// Hand the frontend the lobby's Realtime receiver (set when a lobby
    /// connects) so its event loop can `select!` on it as a loop-local.
    pub fn take_pending_lobby_rx(&mut self) -> Option<Receiver<RtEvent>> {
        self.pending_lobby_rx.take()
    }

    /// The action the frontend should run after tearing down (update/uninstall).
    pub fn post_action(&self) -> Option<PostAction> {
        self.post_action
    }

    // --- input ------------------------------------------------------------

    /// Handle a frontend-agnostic key press. Screen-independent shortcuts
    /// (quit, volume, the uninstall overlay) are handled first, then the press
    /// is routed to the current screen.
    pub fn handle_key(&mut self, key: Key) {
        // Ctrl-C always quits.
        if key == Key::Ctrl('c') {
            self.should_quit = true;
            return;
        }
        // Global volume: Ctrl+Up / Ctrl+Down (works on any screen).
        match key {
            Key::CtrlUp => return self.adjust_volume(0.1),
            Key::CtrlDown => return self.adjust_volume(-0.1),
            _ => {}
        }
        // The uninstall confirmation overlay intercepts all input.
        if self.confirm_uninstall {
            match key {
                Key::Char('y') | Key::Char('Y') => {
                    self.post_action = Some(PostAction::Uninstall);
                    self.should_quit = true;
                }
                _ => self.confirm_uninstall = false,
            }
            return;
        }
        match self.screen {
            Screen::Home => self.on_home_key(key),
            Screen::Settings => self.on_settings_key(key),
            Screen::Menu => self.on_menu_key(key),
            Screen::Loading => {
                if key == Key::Esc {
                    self.screen = if self.loading_is_challenge {
                        Screen::ChallengeMenu
                    } else {
                        Screen::Menu
                    };
                }
            }
            Screen::Playing => self.on_playing_key(key),
            Screen::RoundEnd => self.on_roundend_key(key),
            Screen::Profile => self.on_profile_key(key),
            Screen::ChallengeMenu => self.on_challenge_menu_key(key),
            Screen::HostConfig => self.on_host_config_key(key),
            Screen::Browse => self.on_browse_key(key),
            Screen::JoinCode => self.on_join_key(key),
            Screen::Lobby => self.on_lobby_key(key),
        }
    }

    /// Activate list row `i` on the current screen (select it, then Enter).
    pub fn list_click(&mut self, i: usize) {
        match self.screen {
            Screen::Home => {
                self.home_index = i;
                self.on_home_key(Key::Enter);
            }
            Screen::Menu => {
                self.menu_index = i;
                self.on_menu_key(Key::Enter);
            }
            Screen::Playing => {
                self.suggestion_index = i;
                self.on_playing_key(Key::Enter);
            }
            Screen::ChallengeMenu if !self.editing_name && i < 3 => {
                self.challenge_index = i;
                self.on_challenge_menu_key(Key::Enter);
            }
            Screen::Browse => {
                self.browse_index = i;
                self.on_browse_key(Key::Enter);
            }
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

    /// Set the menu filter text (e.g. bound to a GUI text field).
    pub fn set_menu_filter(&mut self, filter: String) {
        self.menu_filter = filter;
        self.menu_index = 0;
    }

    /// Set the guess-search query and (debounced) kick off a Deezer search.
    pub fn set_search(&mut self, query: String) {
        self.input = query;
        self.queue_search();
    }

    /// Open the online Challenge menu (no-op if online play is unavailable).
    pub fn open_challenge(&mut self) {
        self.open_challenge_menu();
    }

    /// Open the profile screen, remembering where to return to.
    pub fn open_profile(&mut self) {
        if self.screen != Screen::Profile {
            self.profile_return = self.screen;
        }
        self.screen = Screen::Profile;
    }

    fn on_profile_key(&mut self, key: Key) {
        if key == Key::Esc {
            self.screen = self.profile_return;
        }
    }

    /// Live-edit the display name (bound to a text field). Keeps `player_name`
    /// and the profile in step; persist with `commit_profile` when done.
    pub fn set_display_name(&mut self, name: String) {
        self.player_name = name.chars().take(20).collect();
        self.profile.name = self.player_name.clone();
    }

    /// Settle the name (fall back to a default if blank) and persist the profile.
    pub fn commit_profile(&mut self) {
        if self.player_name.trim().is_empty() {
            self.player_name = default_player_name();
            self.profile.name = self.player_name.clone();
        }
        self.profile.save();
    }

    /// Choose the profile accent colour (a theme key) and persist immediately.
    pub fn set_accent(&mut self, key: &str) {
        self.profile.accent = key.to_string();
        self.profile.save();
    }

    // --- home + settings --------------------------------------------------

    fn on_home_key(&mut self, key: Key) {
        match key {
            Key::Up => self.home_index = self.home_index.saturating_sub(1),
            Key::Down if self.home_index + 1 < HOME_ACTIONS => self.home_index += 1,
            Key::Enter => self.home_select(self.home_index),
            Key::Esc => self.should_quit = true,
            _ => {}
        }
    }

    /// Run a Home action (0 solo · 1 online · 2 profile · 3 settings).
    pub fn home_select(&mut self, i: usize) {
        match i {
            0 => self.play_solo(),
            1 => self.open_challenge_menu(),
            2 => self.open_profile(),
            3 => self.open_settings(),
            _ => {}
        }
    }

    /// Enter the solo category picker.
    pub fn play_solo(&mut self) {
        self.host_selecting = false;
        self.menu_filter.clear();
        self.menu_index = 0;
        self.screen = Screen::Menu;
    }

    pub fn open_settings(&mut self) {
        self.screen = Screen::Settings;
    }

    fn on_settings_key(&mut self, key: Key) {
        match key {
            Key::Esc => self.screen = Screen::Home,
            Key::Left => self.set_default_mode(self.game_mode.prev()),
            Key::Right => self.set_default_mode(self.game_mode.next()),
            _ => {}
        }
    }

    /// Set the default solo effect and remember it in the profile.
    pub fn set_default_mode(&mut self, mode: GameMode) {
        self.game_mode = mode;
        self.profile.mode = mode.tag().to_string();
        self.profile.save();
    }

    fn on_menu_key(&mut self, key: Key) {
        match key {
            Key::Up => self.menu_index = self.menu_index.saturating_sub(1),
            Key::Down if self.menu_index + 1 < self.menu_items().len() => {
                self.menu_index += 1;
            }
            Key::Enter => {
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
            Key::Left => {
                self.set_default_mode(self.game_mode.prev());
                self.set_status(format!("Game mode: {}", self.game_mode.label()));
            }
            Key::Right => {
                self.set_default_mode(self.game_mode.next());
                self.set_status(format!("Game mode: {}", self.game_mode.label()));
            }
            Key::Ctrl('o') => self.open_challenge_menu(),
            Key::Ctrl('p') => self.open_profile(),
            Key::Ctrl('u') => {
                if self.update_available.is_some() {
                    self.post_action = Some(PostAction::Update);
                    self.should_quit = true;
                } else {
                    self.set_status("You're on the latest version.".into());
                }
            }
            Key::Ctrl('x') => self.confirm_uninstall = true,
            Key::Backspace => {
                self.menu_filter.pop();
                self.menu_index = 0;
            }
            Key::Esc => {
                if self.host_selecting {
                    self.host_selecting = false;
                    self.screen = Screen::ChallengeMenu;
                } else if self.menu_filter.is_empty() {
                    self.screen = Screen::Home;
                } else {
                    self.menu_filter.clear();
                    self.menu_index = 0;
                }
            }
            Key::Char(c) => {
                self.menu_filter.push(c);
                self.menu_index = 0;
            }
            _ => {}
        }
    }

    fn on_playing_key(&mut self, key: Key) {
        match key {
            Key::Esc => {
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
            Key::Enter => {
                // On an anime round, typing the anime name (then Enter) counts —
                // checked before the highlighted track suggestion.
                let guess = self.input.trim().to_string();
                if self.round.as_ref().is_some_and(|r| r.anime_named(&guess)) {
                    self.win_anime_guess();
                } else if let Some(track) = self.suggestions.get(self.suggestion_index).cloned() {
                    self.make_guess(track);
                }
            }
            Key::Tab => self.skip(),
            Key::Up => self.suggestion_index = self.suggestion_index.saturating_sub(1),
            Key::Down if self.suggestion_index + 1 < self.suggestions.len() => {
                self.suggestion_index += 1;
            }
            Key::Ctrl('r') | Key::Ctrl('p') => self.play_current_clip(),
            Key::Backspace => {
                self.input.pop();
                self.queue_search();
            }
            Key::Char(c) => {
                self.input.push(c);
                self.queue_search();
            }
            _ => {}
        }
    }

    fn on_roundend_key(&mut self, key: Key) {
        match key {
            Key::Enter => {
                if let Some(cat) = self.last_category.clone() {
                    self.start_round(cat);
                }
            }
            Key::Char('m') | Key::Esc => {
                self.audio.stop();
                self.round = None;
                self.screen = Screen::Menu;
            }
            Key::Char('q') => self.should_quit = true,
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

    fn on_challenge_menu_key(&mut self, key: Key) {
        if self.editing_name {
            match key {
                Key::Enter | Key::Esc => {
                    if self.player_name.trim().is_empty() {
                        self.player_name = default_player_name();
                    }
                    self.editing_name = false;
                    self.profile.name = self.player_name.clone();
                    self.profile.save();
                }
                Key::Backspace => {
                    self.player_name.pop();
                }
                Key::Char(c) if self.player_name.chars().count() < 20 => {
                    self.player_name.push(c);
                }
                _ => {}
            }
            return;
        }
        match key {
            Key::Up => self.challenge_index = self.challenge_index.saturating_sub(1),
            Key::Down if self.challenge_index + 1 < 3 => self.challenge_index += 1,
            Key::Char('n') => self.editing_name = true,
            Key::Enter => match self.challenge_index {
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
            Key::Esc => self.screen = Screen::Home,
            _ => {}
        }
    }

    fn on_host_config_key(&mut self, key: Key) {
        match key {
            Key::Left => self.host_mode = self.host_mode.prev(),
            Key::Right => self.host_mode = self.host_mode.next(),
            Key::Up if self.host_rounds < 20 => self.host_rounds += 1,
            Key::Down if self.host_rounds > 1 => self.host_rounds -= 1,
            Key::Char('v') => self.host_public = !self.host_public,
            Key::Char('+') | Key::Char('=') if self.host_max < 64 => self.host_max += 1,
            Key::Char('-') | Key::Char('_') if self.host_max > 1 => self.host_max -= 1,
            Key::Char('c') => self.change_pool(),
            Key::Enter => {
                if self.editing_lobby {
                    self.apply_lobby_settings();
                } else {
                    self.host_lobby();
                }
            }
            Key::Esc => {
                if self.editing_lobby {
                    self.editing_lobby = false;
                    self.screen = Screen::Lobby;
                } else {
                    self.host_selecting = false;
                    self.screen = Screen::ChallengeMenu;
                }
            }
            _ => {}
        }
    }

    /// Go pick a different song pool (returns to Host config on selection).
    pub fn change_pool(&mut self) {
        self.host_selecting = true;
        self.menu_filter.clear();
        self.menu_index = 0;
        self.screen = Screen::Menu;
    }

    /// Host: re-open the config to change this lobby's settings (waiting/game-over
    /// only). Pre-fills the config from the current lobby.
    pub fn open_lobby_settings(&mut self) {
        let Some(lobby) = &self.lobby else { return };
        if !lobby.is_host || !matches!(lobby.phase, LobbyPhase::Waiting | LobbyPhase::GameOver) {
            return;
        }
        self.host_rounds = lobby.rounds;
        self.host_mode = lobby.mode;
        self.host_category = lobby.category.clone();
        self.editing_lobby = true;
        self.screen = Screen::HostConfig;
    }

    /// Apply edited settings to the running lobby: update local state, re-fetch
    /// the pool if the category changed, and update the Browse ad.
    fn apply_lobby_settings(&mut self) {
        self.editing_lobby = false;
        self.screen = Screen::Lobby;
        let (rounds, mode, public, max) = (
            self.host_rounds,
            self.host_mode,
            self.host_public,
            self.host_max,
        );
        let Some(category) = self.host_category.clone() else {
            return;
        };
        let changed = self
            .lobby
            .as_ref()
            .is_some_and(|l| l.category_label != category.name);
        if let Some(lobby) = &mut self.lobby {
            lobby.rounds = rounds.max(1);
            lobby.mode = mode;
            lobby.category = Some(category.clone());
            lobby.category_label = category.name.clone();
        }
        if changed {
            self.fetch_lobby_pool(); // new pool for the new category
        }
        // Reflect the changes in the Browse ad (best-effort).
        if let (Some(supa), Some(code)) = (
            self.supa.clone(),
            self.lobby.as_ref().map(|l| l.code.clone()),
        ) {
            let title = format!("Live lobby · {}", category.name);
            let visibility = if public { "public" } else { "private" }.to_string();
            tokio::spawn(async move {
                let _ = supa
                    .update_party(&code, &visibility, max as i32, &title)
                    .await;
            });
        }
    }

    fn on_join_key(&mut self, key: Key) {
        match key {
            Key::Enter => {
                let code = self.join_input.trim().to_uppercase();
                if !code.is_empty() {
                    self.join_lobby(code);
                }
            }
            Key::Backspace => {
                self.join_input.pop();
            }
            Key::Esc => self.screen = Screen::ChallengeMenu,
            Key::Char(c) if c.is_ascii_alphanumeric() && self.join_input.len() < 12 => {
                self.join_input.push(c.to_ascii_uppercase());
            }
            _ => {}
        }
    }

    fn on_browse_key(&mut self, key: Key) {
        match key {
            Key::Up => self.browse_index = self.browse_index.saturating_sub(1),
            Key::Down if self.browse_index + 1 < self.browse.len() => self.browse_index += 1,
            Key::Char('r') => self.refresh_public_parties(),
            Key::Enter => {
                if let Some(party) = self.browse.get(self.browse_index).cloned() {
                    self.join_lobby(party.code);
                }
            }
            Key::Esc => self.screen = Screen::ChallengeMenu,
            _ => {}
        }
    }

    // --- live lobby -------------------------------------------------------

    fn on_lobby_key(&mut self, key: Key) {
        // A pending "skip while players are still guessing?" confirmation swallows
        // the next key: Enter/y goes ahead, anything else backs out.
        if self.confirm_skip_round {
            match key {
                Key::Enter | Key::Char('y') => self.do_advance_lobby(),
                _ => self.confirm_skip_round = false,
            }
            return;
        }
        match key {
            Key::Enter => self.lobby_primary_action(),
            Key::Char('s') => self.open_lobby_settings(),
            Key::Esc => self.leave_lobby(),
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
    /// If players are still guessing this round, ask to confirm the skip first.
    fn advance_lobby(&mut self) {
        if !self.lobby_round_complete() {
            self.confirm_skip_round = true;
            return;
        }
        self.do_advance_lobby();
    }

    /// Whether every active (non-spectating) player has submitted this round.
    fn lobby_round_complete(&self) -> bool {
        self.lobby
            .as_ref()
            .map(|l| l.game.round_complete(l.players.len()))
            .unwrap_or(true)
    }

    /// Actually advance — start the next round or end the game.
    fn do_advance_lobby(&mut self) {
        self.confirm_skip_round = false;
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
        // Remove the Browse ad if we're the host or the last one here, so empty
        // lobbies don't linger. (A hard crash can still leak a row — rare.)
        if let Some(lobby) = &self.lobby {
            let alone = lobby.players.len() + lobby.spectators.len() <= 1;
            if (lobby.is_host || alone)
                && let Some(supa) = self.supa.clone()
            {
                let code = lobby.code.clone();
                tokio::spawn(async move {
                    let _ = supa.delete_party(&code).await;
                });
            }
        }
        self.editing_lobby = false;
        self.confirm_skip_round = false;
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

    pub fn handle_rt_event(&mut self, evt: RtEvent) {
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
            .filter(|g| matches!(g, GuessLog::Wrong(_) | GuessLog::WrongRightArtist(_)))
            .count() as u32;
        let artist_bonus = round.artist_bonus;
        let points = round.awarded_points();
        let rec_title = round.answer.title.clone();
        let rec_artist = round.answer.artist_name().to_string();
        let answer = round.answer.clone();
        let Some(lobby) = self.lobby.as_mut() else {
            return;
        };
        if lobby.my_result_sent {
            return; // already scored this round (e.g. Esc after a win)
        }
        let category = lobby.category_label.clone();
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
            artist_bonus,
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
        let category = if category.is_empty() {
            "Challenge".to_string()
        } else {
            category
        };
        self.profile.record_round(RoundRecord {
            title: rec_title,
            artist: rec_artist,
            category,
            won,
            clips,
            points,
        });
        self.profile.save();
        self.screen = Screen::Lobby;
    }

    // --- async results ----------------------------------------------------

    pub fn handle_msg(&mut self, msg: Msg) {
        match msg {
            Msg::RoundReady {
                answer,
                preview,
                anime,
            } => {
                // Ignore if the player already backed out of loading.
                if self.screen != Screen::Loading {
                    return;
                }
                let mut round = Round::new(*answer, preview, self.schedule.clone());
                round.anime = anime;
                self.round = Some(round);
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

    pub fn on_tick(&mut self) {
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
                Ok((answer, preview, anime)) => Msg::RoundReady {
                    answer: Box::new(answer),
                    preview,
                    anime,
                },
                Err(e) => Msg::Error(format!("Couldn't load “{}”: {e}", category.name)),
            };
            let _ = tx.send(msg).await;
        });
    }

    /// The player named the anime this song is from — count it as a solve.
    fn win_anime_guess(&mut self) {
        if let Some(round) = self.round.as_mut() {
            round.outcome = Outcome::Won;
        }
        self.reset_turn();
        self.finish(true);
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
        // If the clip is still actively playing, we carry it past the checkpoint
        // rather than restarting; decide before advancing (the check reads the
        // *current* clip length).
        let extend = self.skip_should_continue();
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
            // Still playing → push the live clip's end out to the new checkpoint,
            // uninterrupted (play_started_at stays put: it's the same clip).
            _ if extend => {
                if let Some(round) = &self.round {
                    self.audio.extend(round.current_clip());
                }
            }
            // Already stopped at the checkpoint (or a non-continuable effect) →
            // restart the song from the beginning up to the new checkpoint.
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
        // Solve points, or — failing that — the right-artist consolation.
        let points = self.round.as_ref().map(|r| r.awarded_points()).unwrap_or(0);
        self.score += points;
        self.last_points = points;
        if points > 0 {
            self.score_flash_at = Some(Instant::now());
        }
        self.streak = if won { self.streak + 1 } else { 0 };
        let category = self
            .last_category
            .as_ref()
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "Solo".into());
        self.record_to_profile(won, points, &category);
        self.screen = Screen::RoundEnd;
    }

    /// Fold the just-finished round into the persistent profile and save it.
    fn record_to_profile(&mut self, won: bool, points: u32, category: &str) {
        let Some(record) = self.round.as_ref().map(|round| {
            let clips = if won {
                round.guess_number() as u32
            } else {
                round.total_levels() as u32
            };
            RoundRecord {
                title: round.answer.title.clone(),
                artist: round.answer.artist_name().to_string(),
                category: category.to_string(),
                won,
                clips,
                points,
            }
        }) else {
            return;
        };
        self.profile.record_round(record);
        self.profile.save();
    }

    fn adjust_volume(&mut self, delta: f32) {
        self.volume = (self.volume + delta).clamp(0.0, 1.0);
        self.audio.set_volume(self.volume);
        self.profile.volume = self.volume;
        self.profile.save();
        self.set_status(format!("Volume {}%", (self.volume * 100.0).round() as i32));
    }

    /// The effect in force: the host's in a lobby round, else the menu one.
    fn active_mode(&self) -> GameMode {
        self.lobby
            .as_ref()
            .map(|l| l.mode)
            .unwrap_or(self.game_mode)
    }

    fn play_current_clip(&mut self) {
        let Some(round) = &self.round else { return };
        self.audio.play(
            round.preview.clone(),
            round.current_clip(),
            self.active_mode(),
        );
        self.play_started_at = Some(Instant::now());
    }

    /// Whether a skip should *continue* the live clip (extend it) rather than
    /// restart from zero. True only when it's still audibly playing — a safe
    /// margin before the checkpoint so we never extend a clip about to stop
    /// itself — under an effect that can extend seamlessly (Normal, Muffled). The
    /// speed/reverse effects, and a clip that's already reached its checkpoint,
    /// restart instead.
    fn skip_should_continue(&self) -> bool {
        if !matches!(self.active_mode(), GameMode::Normal | GameMode::Muffled) {
            return false;
        }
        let (Some(round), Some(started)) = (self.round.as_ref(), self.play_started_at) else {
            return false;
        };
        started.elapsed() + Duration::from_millis(150) < round.current_clip()
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

    pub fn fetch_genres(&self) {
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
async fn load_round(
    client: &DeezerClient,
    category: &Category,
) -> Result<(Track, Vec<u8>, Option<AnimeTag>)> {
    // Anime rounds come from AnimeThemes so the anime counts as a guess; if that
    // pipeline is unavailable, fall through to the Deezer anime playlist below.
    if matches!(category.source, CategorySource::Anime)
        && let Ok((track, preview, tag)) = anime::resolve_anime_round(client).await
    {
        return Ok((track, preview, Some(tag)));
    }

    let mut tracks = match category.source {
        CategorySource::Chart(genre) => client.chart_tracks(genre).await?,
        CategorySource::Playlist(id) => client.playlist_tracks(id).await?,
        CategorySource::Anime => {
            client
                .playlist_tracks(config::ANIME_DEEZER_PLAYLIST)
                .await?
        }
    };
    tracks.retain(|t| t.has_preview());
    anyhow::ensure!(!tracks.is_empty(), "no playable tracks found");

    let index = (rand::random::<u64>() % tracks.len() as u64) as usize;
    let answer = tracks.swap_remove(index);
    let preview = client.download_preview(&answer.preview).await?;
    Ok((answer, preview, None))
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
        // Lobbies play anime song-only (the anime tag isn't broadcast).
        CategorySource::Anime => {
            deezer
                .playlist_tracks(config::ANIME_DEEZER_PLAYLIST)
                .await?
        }
    };
    tracks.retain(|t| t.has_preview());
    anyhow::ensure!(!tracks.is_empty(), "no playable tracks in that category");
    Ok(tracks)
}

/// Default leaderboard name from the OS username.
fn default_player_name() -> String {
    crate::profile::default_name()
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
