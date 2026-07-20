//! hitair — desktop GUI frontend (eframe/egui) over `hitair_core::session::Session`.

use eframe::egui;

use hitair_core::audio;
use hitair_core::config::Config;
use hitair_core::deezer::DeezerClient;
use hitair_core::session::{Key, Msg, Session};
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
        Box::new(move |_cc| Ok(Box::new(HitairApp::new(cfg, deezer, audio)))),
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
        let (session, rx) = Session::new(cfg, deezer, audio);
        session.fetch_genres();
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

impl eframe::App for HitairApp {
    // eframe 0.35 wraps this in a CentralPanel and hands us the `Ui`.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.pump();

        ui.heading("hitair");
        ui.label("GUI skeleton — wiring check.");
        if ui.button("Quit").clicked() {
            self.session.handle_key(Key::Ctrl('c'));
        }

        // Keep polling async results + animating.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
        if self.session.should_quit {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
