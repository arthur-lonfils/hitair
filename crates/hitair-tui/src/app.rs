//! The terminal frontend: a thin adapter over `hitair_core::session::Session`.
//!
//! It owns the `crossterm`/`ratatui` event loop and translates terminal input
//! (keys, mouse clicks, scroll) into the session's frontend-agnostic intents,
//! then renders the session state each frame. All application logic lives in the
//! session; nothing here knows the rules of the game.

use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use futures::StreamExt;
use ratatui::DefaultTerminal;
use ratatui::layout::Rect;
use tokio::sync::mpsc::Receiver;

use hitair_core::audio::AudioHandle;
use hitair_core::config::Config;
use hitair_core::deezer::DeezerClient;
use hitair_core::realtime::RtEvent;
use hitair_core::session::{Key, Msg, PostAction, Session};

use crate::ui;

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

pub struct App {
    session: Session,
    /// Clickable regions from the last render, for mouse hit-testing.
    click_map: Vec<Click>,
}

impl App {
    pub fn new(cfg: Config, deezer: DeezerClient, audio: AudioHandle) -> (Self, Receiver<Msg>) {
        let (session, rx) = Session::new(cfg, deezer, audio);
        (
            App {
                session,
                click_map: Vec::new(),
            },
            rx,
        )
    }

    pub async fn run(
        mut self,
        mut terminal: DefaultTerminal,
        mut rx: Receiver<Msg>,
    ) -> Result<Option<PostAction>> {
        let mut events = EventStream::new();
        let mut ticker = tokio::time::interval(Duration::from_millis(100));
        self.session.fetch_genres();
        self.check_for_update();

        // The lobby's Realtime receiver lives here as a loop-local so the
        // `select!` arm borrows it (not `self`); the session hands it over via
        // `take_pending_lobby_rx` when a lobby connects, and it's dropped on exit.
        let mut lobby_rx: Option<Receiver<RtEvent>> = None;
        loop {
            if let Some(new_rx) = self.session.take_pending_lobby_rx() {
                lobby_rx = Some(new_rx);
            }
            if self.session.lobby.is_none() {
                lobby_rx = None;
            }

            let mut clicks = Vec::new();
            terminal.draw(|f| ui::draw(f, &self.session, &mut clicks))?;
            self.click_map = clicks;
            if self.session.should_quit {
                break;
            }
            tokio::select! {
                maybe_event = events.next() => match maybe_event {
                    Some(Ok(event)) => self.handle_event(event),
                    Some(Err(_)) | None => self.session.should_quit = true,
                },
                Some(msg) = rx.recv() => self.session.handle_msg(msg),
                Some(evt) = recv_rt(&mut lobby_rx) => self.session.handle_rt_event(evt),
                _ = ticker.tick() => self.session.on_tick(),
            }
        }
        Ok(self.session.post_action())
    }

    /// Check GitHub for a newer release in the background (fail-silent).
    /// Set `HITAIR_NO_UPDATE_CHECK` to skip.
    fn check_for_update(&self) {
        if std::env::var_os("HITAIR_NO_UPDATE_CHECK").is_some() {
            return;
        }
        let tx = self.session.sender();
        tokio::spawn(async move {
            if let Ok(Some(version)) = crate::update::latest_if_newer().await {
                let _ = tx.send(Msg::UpdateAvailable(version)).await;
            }
        });
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::Key(k) => {
                if k.kind != KeyEventKind::Release
                    && let Some(key) = to_key(k)
                {
                    self.session.handle_key(key);
                }
            }
            Event::Mouse(m) => self.handle_mouse(m),
            _ => {}
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
            MouseEventKind::ScrollUp => self.session.handle_key(Key::Up),
            MouseEventKind::ScrollDown => self.session.handle_key(Key::Down),
            _ => {}
        }
    }

    /// Route a click to the session. Each button maps to the key it stands in
    /// for on the screen it renders on.
    fn dispatch_click(&mut self, action: ClickAction) {
        match action {
            ClickAction::Replay => self.session.handle_key(Key::Ctrl('r')),
            ClickAction::Skip => self.session.handle_key(Key::Tab),
            ClickAction::VolumeUp => self.session.handle_key(Key::CtrlUp),
            ClickAction::VolumeDown => self.session.handle_key(Key::CtrlDown),
            ClickAction::ListItem(i) => self.session.list_click(i),
            ClickAction::NextRound => self.session.handle_key(Key::Enter),
            ClickAction::ResultToMenu => self.session.handle_key(Key::Char('m')),
            ClickAction::LobbyPrimary => self.session.handle_key(Key::Enter),
            ClickAction::LobbyLeave => self.session.handle_key(Key::Esc),
        }
    }
}

/// Translate a crossterm key event into the session's frontend-agnostic `Key`.
fn to_key(k: KeyEvent) -> Option<Key> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    Some(match k.code {
        KeyCode::Up if ctrl => Key::CtrlUp,
        KeyCode::Down if ctrl => Key::CtrlDown,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Tab => Key::Tab,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Char(c) if ctrl => Key::Ctrl(c.to_ascii_lowercase()),
        KeyCode::Char(c) => Key::Char(c),
        _ => return None,
    })
}

/// Await the next lobby Realtime event, or park forever when there's no lobby
/// (so the `select!` arm is simply inactive).
async fn recv_rt(rx: &mut Option<Receiver<RtEvent>>) -> Option<RtEvent> {
    match rx {
        Some(r) => r.recv().await,
        None => std::future::pending().await,
    }
}
