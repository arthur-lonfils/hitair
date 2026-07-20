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
            .with_app_id("hitair")
            .with_icon(std::sync::Arc::new(app_icon()))
            .with_inner_size([900.0, 640.0])
            .with_min_inner_size([560.0, 460.0]),
        ..Default::default()
    };

    eframe::run_native(
        "hitair",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            egui_extras::install_image_loaders(&cc.egui_ctx); // album art on the reveal
            Ok(Box::new(HitairApp::new(cfg, deezer, audio)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("failed to launch: {e}"))?;
    Ok(())
}

/// A procedural "record" app icon — a coral disc with ink grooves + center hole.
fn app_icon() -> egui::IconData {
    const N: usize = 256;
    let mut rgba = vec![0u8; N * N * 4];
    let c = (N as f32 - 1.0) / 2.0;
    let r_out = 118.0;
    let coral = [0xFFu8, 0x6A, 0x5D];
    let ink = [0x18u8, 0x12, 0x20];
    for y in 0..N {
        for x in 0..N {
            let (dx, dy) = (x as f32 - c, y as f32 - c);
            let d = (dx * dx + dy * dy).sqrt();
            if d > r_out {
                continue; // transparent outside the disc
            }
            let mut col = coral;
            if (d as i32 % 15) < 2 {
                col = ink; // grooves
            }
            if d < 34.0 {
                col = coral; // center label
            }
            if d < 9.0 {
                col = ink; // spindle hole
            }
            let a = ((r_out - d).clamp(0.0, 1.5) / 1.5 * 255.0) as u8;
            let i = (y * N + x) * 4;
            rgba[i..i + 4].copy_from_slice(&[col[0], col[1], col[2], a]);
        }
    }
    egui::IconData {
        rgba,
        width: N as u32,
        height: N as u32,
    }
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
    use hitair_core::config::Category;
    use hitair_core::deezer::{Album, Artist, Track};
    use hitair_core::game::{GameMode, Round};
    use hitair_core::lobby::{Game, RoundResult};
    use hitair_core::session::{LobbyPhase, LobbyState, Screen};
    use hitair_core::supa::Party;
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
            cover_big: String::new(),
            cover_medium: String::new(),
        }),
    };

    match which {
        "result" => {
            let mut answer = track(1, "Blinding Lights", "The Weeknd");
            // A resolvable image just to preview the reveal art layout.
            if let Some(a) = answer.album.as_mut() {
                a.cover_big = "https://picsum.photos/id/1062/500/500.jpg".into();
            }
            let mut round = Round::new(
                answer.clone(),
                vec![0u8; 8],
                session.cfg.schedule_durations(),
            );
            round.submit_guess(&answer);
            session.round = Some(round);
            session.last_points = 6;
            session.score = 42;
            session.streak = 3;
            session.round_end_at = Some(Instant::now());
            session.screen = Screen::RoundEnd;
        }
        "challenge" => session.screen = Screen::ChallengeMenu,
        "host" => {
            session.host_category = Some(Category::chart("Rock", 152));
            session.host_rounds = 5;
            session.host_mode = GameMode::Reverse;
            session.screen = Screen::HostConfig;
        }
        "browse" => {
            let mk = |code: &str, host: &str, max: i32| Party {
                code: code.into(),
                visibility: "public".into(),
                max_players: max,
                track_id: 0,
                title: String::new(),
                artist: String::new(),
                album: None,
                schedule: vec![],
                host_name: host.into(),
                created_at: None,
            };
            session.browse = vec![
                mk("7Q2F9K", "mara", 8),
                mk("K3P9Z2", "ivo", 6),
                mk("VX4M8Q", "you", 12),
            ];
            session.screen = Screen::Browse;
        }
        "join" => {
            session.join_input = "7Q2F9K".into();
            session.screen = Screen::JoinCode;
        }
        "lobby" => {
            let mut game = Game::new(5, 7);
            game.start_round(2);
            for (n, solved, clips, ms, mis) in [
                ("You", true, 2u32, 3200u32, 1u32),
                ("Mara", true, 4, 5100, 3),
                ("Ivo", false, 7, 9000, 6),
            ] {
                game.on_result(&RoundResult {
                    round: 2,
                    name: n.into(),
                    solved,
                    clips,
                    time_ms: ms,
                    mistakes: mis,
                });
            }
            let ans = track(9, "Instant Crush", "Daft Punk");
            session.player_name = "You".into();
            session.lobby = Some(LobbyState {
                code: "7Q2F9K".into(),
                is_host: true,
                players: vec!["You".into(), "Mara".into(), "Ivo".into()],
                spectators: vec!["Late".into()],
                game,
                phase: LobbyPhase::Between,
                rounds: 5,
                mode: GameMode::Reverse,
                category_label: "Rock".into(),
                category: None,
                pool: vec![ans.clone()],
                last_answer: Some(ans),
                round_started_at: None,
                my_result_sent: true,
                playing_this_game: true,
                spectating: false,
            });
            session.screen = Screen::Lobby;
        }
        _ => {
            let answer = track(1, "Blinding Lights", "The Weeknd");
            let mut round = Round::new(answer, vec![0u8; 8], session.cfg.schedule_durations());
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
