//! hitair — desktop GUI frontend (eframe/egui) over `hitair_core::session::Session`.

mod input;
mod theme;
mod ui;

use eframe::egui;

use hitair_core::audio;
use hitair_core::config::Config;
use hitair_core::deezer::DeezerClient;
use hitair_core::session::{Msg, Session};
use tokio::sync::mpsc::Receiver;

fn main() -> anyhow::Result<()> {
    // A tokio runtime for the async work the session spawns. Entering its context
    // makes `tokio::spawn` work from the egui (winit) main thread; the runtime's
    // worker threads drive those tasks + I/O while winit owns the main loop.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let _guard = rt.enter();

    let cfg = Config::load();
    let deezer = DeezerClient::new()?;
    let audio = audio::spawn();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("hitair")
            .with_inner_size([900.0, 640.0])
            .with_min_inner_size([560.0, 460.0]),
        ..Default::default()
    };

    eframe::run_native(
        "hitair",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(HitairApp::new(cfg, deezer, audio)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("failed to launch: {e}"))?;
    Ok(())
}

struct HitairApp {
    session: Session,
    rx: Receiver<Msg>,
    lobby_rx: Option<Receiver<hitair_core::realtime::RtEvent>>,
}

impl HitairApp {
    fn new(cfg: Config, deezer: DeezerClient, audio: audio::AudioHandle) -> Self {
        let (mut session, rx) = Session::new(cfg, deezer, audio);
        session.fetch_genres();
        // Dev-only: seed a screen for design screenshots (HITAIR_GUI_PREVIEW=playing|result).
        if let Ok(which) = std::env::var("HITAIR_GUI_PREVIEW") {
            seed_preview(&mut session, &which);
        }
        Self {
            session,
            rx,
            lobby_rx: None,
        }
    }

    /// Drain async results into the session (mirrors the TUI's select loop).
    fn pump(&mut self) {
        if let Some(new_rx) = self.session.take_pending_lobby_rx() {
            self.lobby_rx = Some(new_rx);
        }
        if self.session.lobby.is_none() {
            self.lobby_rx = None;
        }
        while let Ok(msg) = self.rx.try_recv() {
            self.session.handle_msg(msg);
        }
        if let Some(rx) = &mut self.lobby_rx {
            while let Ok(evt) = rx.try_recv() {
                self.session.handle_rt_event(evt);
            }
        }
        self.session.on_tick();
    }
}

/// Seed a screen with fake data so its layout can be screenshotted without the
/// network or an audio device. Gated behind `HITAIR_GUI_PREVIEW`.
fn seed_preview(session: &mut Session, which: &str) {
    use hitair_core::deezer::{Album, Artist, Track};
    use hitair_core::game::Round;
    use hitair_core::session::Screen;
    use std::time::Instant;

    let track = |id: i64, title: &str, artist: &str| Track {
        id,
        title: title.into(),
        preview: String::new(),
        artist: Artist {
            id: 0,
            name: artist.into(),
        },
        album: Some(Album {
            id: 0,
            title: "After Hours".into(),
        }),
    };
    let answer = track(1, "Blinding Lights", "The Weeknd");
    let schedule = session.cfg.schedule_durations();
    let mut round = Round::new(answer.clone(), vec![0u8; 8], schedule);

    if which == "result" {
        round.submit_guess(&answer);
        session.round = Some(round);
        session.last_points = 6;
        session.score = 42;
        session.streak = 3;
        session.round_end_at = Some(Instant::now());
        session.screen = Screen::RoundEnd;
    } else {
        round.skip();
        round.skip();
        session.round = Some(round);
        session.play_started_at = Some(Instant::now());
        session.suggestions = vec![
            track(2, "Blinding Lights", "The Weeknd"),
            track(3, "Blinding Lights - Remix", "The Weeknd"),
            track(4, "Save Your Tears", "The Weeknd"),
            track(5, "Take My Breath", "The Weeknd"),
        ];
        session.input = "blind".into();
        session.suggestion_index = 0;
        session.screen = Screen::Playing;
    }
}

impl eframe::App for HitairApp {
    // eframe 0.35 wraps this in a CentralPanel and hands us the `Ui`.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.pump();
        input::feed(ui.ctx(), &mut self.session);
        ui::draw(ui, &mut self.session);

        // Keep polling async results + animating.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
        if self.session.should_quit {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
