//! hitair — desktop GUI frontend (eframe/egui) over `hitair_core::session::Session`.

mod input;
mod theme;
mod ui;

use std::sync::{Arc, Mutex};

use eframe::egui;

use hitair_core::audio;
use hitair_core::config::Config;
use hitair_core::deezer::DeezerClient;
use hitair_core::session::{Msg, Screen, Session};
use hitair_core::update;
use tokio::sync::mpsc::Receiver;

fn main() -> anyhow::Result<()> {
    // Must run before any TLS: the GUI links two rustls providers (see the core fn).
    hitair_core::install_crypto_provider();
    match std::env::args().nth(1).as_deref() {
        Some("--version" | "-V") => {
            println!("hitair-gui {}", update::CURRENT_VERSION);
            return Ok(());
        }
        Some("--help" | "-h") => {
            println!(
                "hitair-gui {} — the hitair desktop app",
                update::CURRENT_VERSION
            );
            println!(
                "\nUSAGE:\n  hitair-gui           Launch the game\n  hitair-gui --version Print the version\n  hitair-gui --help    Show this help"
            );
            println!("\n(Update/uninstall live in-app on the menu; the terminal app is `hitair`.)");
            return Ok(());
        }
        _ => {}
    }

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

/// The app icon encoded as PNG bytes, for the desktop-launcher install.
fn icon_png() -> Vec<u8> {
    let icon = app_icon();
    let mut out = std::io::Cursor::new(Vec::new());
    if let Some(img) = image::RgbaImage::from_raw(icon.width, icon.height, icon.rgba) {
        let _ = img.write_to(&mut out, image::ImageFormat::Png);
    }
    out.into_inner()
}

#[derive(Clone, Copy, PartialEq)]
enum UpdatePhase {
    Idle,
    Running,
    Done,
    Failed,
}

struct HitairApp {
    session: Session,
    rx: Receiver<Msg>,
    lobby_rx: Option<Receiver<hitair_core::realtime::RtEvent>>,
    update_phase: Arc<Mutex<UpdatePhase>>,
    confirm_uninstall: bool,
}

impl HitairApp {
    fn new(cfg: Config, deezer: DeezerClient, audio: audio::AudioHandle) -> Self {
        let (mut session, rx) = Session::new(cfg, deezer, audio);
        session.fetch_genres();
        check_for_update(session.sender());
        // Dev-only: seed a screen for design screenshots (HITAIR_GUI_PREVIEW=playing|result).
        if let Ok(which) = std::env::var("HITAIR_GUI_PREVIEW") {
            seed_preview(&mut session, &which);
        }
        // One-time: add hitair to the desktop app menu (Linux) so it's searchable
        // and launchable like a normal app. Skipped under the itch app, which does
        // its own shortcut handling.
        if hitair_core::desktop::SUPPORTED
            && !session.profile.launcher_setup_done
            && !update::is_itch_managed()
        {
            if let Ok(exe) = std::env::current_exe() {
                let _ = hitair_core::desktop::install(&exe, &icon_png());
            }
            session.profile.launcher_setup_done = true;
            session.profile.save();
        }
        Self {
            session,
            rx,
            lobby_rx: None,
            update_phase: Arc::new(Mutex::new(UpdatePhase::Idle)),
            confirm_uninstall: false,
        }
    }

    /// Kick off the self-update (updates this binary + the terminal sibling),
    /// tracking progress for the menu banner.
    fn start_update(&mut self) {
        *self.update_phase.lock().unwrap() = UpdatePhase::Running;
        let phase = self.update_phase.clone();
        tokio::spawn(async move {
            let ok = update::perform_update().await.is_ok();
            *phase.lock().unwrap() = if ok {
                UpdatePhase::Done
            } else {
                UpdatePhase::Failed
            };
        });
    }

    fn do_uninstall(&mut self, ctx: &egui::Context) {
        let _ = update::uninstall();
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// The update banner + uninstall link + confirm dialog, shown on Home.
    /// Hidden entirely under the itch app, which manages install + updates itself.
    fn draw_update_ui(&mut self, ui: &mut egui::Ui) {
        if self.session.screen != Screen::Home || update::is_itch_managed() {
            return;
        }
        let ctx = ui.ctx().clone();
        let phase = *self.update_phase.lock().unwrap();
        let available = self.session.update_available.clone();

        egui::Area::new("update-bar".into())
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(18.0, -14.0))
            .show(&ctx, |ui| match phase {
                UpdatePhase::Running => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            egui::RichText::new("Updating…")
                                .color(theme::GOLD)
                                .size(13.0),
                        );
                    });
                }
                UpdatePhase::Done => {
                    ui.label(
                        egui::RichText::new("Updated — restart hitair-gui to apply.")
                            .color(theme::MINT)
                            .size(13.0),
                    );
                }
                UpdatePhase::Failed => {
                    ui.label(
                        egui::RichText::new("Update failed — try again later.")
                            .color(theme::ROSE)
                            .size(13.0),
                    );
                }
                UpdatePhase::Idle => {
                    if let Some(v) = &available {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("⬆ Update to v{v} available"))
                                    .color(theme::GOLD)
                                    .size(13.0),
                            );
                            if ui
                                .button(egui::RichText::new("Update now").size(13.0))
                                .clicked()
                            {
                                self.start_update();
                            }
                        });
                    }
                }
            });

        egui::Area::new("uninstall-link".into())
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-18.0, -14.0))
            .show(&ctx, |ui| {
                let btn = egui::Button::new(
                    egui::RichText::new("Uninstall")
                        .color(theme::MUTED)
                        .size(12.0),
                )
                .frame(false);
                if ui.add(btn).clicked() {
                    self.confirm_uninstall = true;
                }
            });

        if self.confirm_uninstall {
            egui::Window::new(egui::RichText::new("Uninstall").color(theme::ROSE))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(&ctx, |ui| {
                    ui.label("Remove hitair-gui from disk?");
                    ui.label(
                        egui::RichText::new("(The terminal app and your config stay in place.)")
                            .color(theme::MUTED)
                            .size(12.0),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(egui::RichText::new("Uninstall").color(theme::ROSE))
                            .clicked()
                        {
                            self.do_uninstall(&ctx);
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_uninstall = false;
                        }
                    });
                });
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
        "update" => {
            session.update_available = Some("9.9.9".into());
            session.rounds_played = 4;
            session.screen = Screen::Menu;
        }
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
            for (n, solved, clips, ms, mis, bonus) in [
                ("You", true, 2u32, 3200u32, 1u32, 0u32),
                ("Mara", true, 4, 5100, 3, 0),
                ("Ivo", false, 7, 9000, 6, 3), // right artist, wrong song
            ] {
                game.on_result(&RoundResult {
                    round: 2,
                    name: n.into(),
                    solved,
                    clips,
                    time_ms: ms,
                    mistakes: mis,
                    artist_bonus: bonus,
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
        "profile" => {
            session.profile.name = "Arthur".into();
            session.profile.accent = "violet".into();
            let games = [
                ("Blinding Lights", "The Weeknd", "Pop", true, 6u32),
                ("Gurenge", "LiSA", "Anime (Japan)", true, 5),
                ("Bohemian Rhapsody", "Queen", "Rock", false, 0),
                ("Idol", "YOASOBI", "Anime (Japan)", true, 7),
                ("Kaikai Kitan", "Eve", "Anime (Japan)", false, 2),
                ("Levitating", "Dua Lipa", "Pop", true, 5),
                ("Take On Me", "a-ha", "80s Hits", true, 6),
                ("Smells Like Teen Spirit", "Nirvana", "Rock", true, 4),
                ("Numb", "Linkin Park", "Rock", true, 3),
            ];
            for (title, artist, category, won, points) in games {
                session
                    .profile
                    .record_round(hitair_core::profile::RoundRecord {
                        title: title.into(),
                        artist: artist.into(),
                        category: category.into(),
                        won,
                        clips: if won { 3 } else { 7 },
                        points,
                    });
            }
            session.player_name = session.profile.name.clone();
            session.screen = Screen::Profile;
        }
        "home" => {
            session.profile.name = "Arthur".into();
            session.profile.accent = "violet".into();
            for (won, pts) in [(true, 6u32), (true, 5), (false, 0), (true, 7)] {
                session
                    .profile
                    .record_round(hitair_core::profile::RoundRecord {
                        title: "x".into(),
                        artist: "y".into(),
                        category: "Pop".into(),
                        won,
                        clips: 3,
                        points: pts,
                    });
            }
            session.player_name = session.profile.name.clone();
            session.screen = Screen::Home;
        }
        "settings" => {
            session.game_mode = GameMode::Reverse;
            session.volume = 0.7;
            session.screen = Screen::Settings;
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

/// Background check for a newer release (fail-silent). Sets `update_available`
/// via the session's `Msg` channel. Skipped when `HITAIR_NO_UPDATE_CHECK` is set,
/// or when the itch.io app is managing updates.
fn check_for_update(tx: tokio::sync::mpsc::Sender<Msg>) {
    if std::env::var_os("HITAIR_NO_UPDATE_CHECK").is_some() || update::is_itch_managed() {
        return;
    }
    tokio::spawn(async move {
        if let Ok(Some(version)) = update::latest_if_newer().await {
            let _ = tx.send(Msg::UpdateAvailable(version)).await;
        }
    });
}

impl eframe::App for HitairApp {
    // eframe 0.35 wraps this in a CentralPanel and hands us the `Ui`.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.pump();
        // App-level shortcuts (update/uninstall) — handled here, not by the
        // session, so they don't get routed as a quit.
        let ctrl = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
        // Self-update / uninstall are disabled under the itch app (it owns them).
        if !update::is_itch_managed() {
            if ctrl
                && ui.input(|i| i.key_pressed(egui::Key::U))
                && self.session.update_available.is_some()
            {
                self.start_update();
            }
            if ctrl && ui.input(|i| i.key_pressed(egui::Key::X)) {
                self.confirm_uninstall = true;
            }
        }
        // A Settings toggle asked us to add/remove the desktop launcher.
        if let Some(install) = self.session.take_launcher_request()
            && let Ok(exe) = std::env::current_exe()
        {
            let _ = if install {
                hitair_core::desktop::install(&exe, &icon_png())
            } else {
                hitair_core::desktop::remove()
            };
        }
        input::feed(ui.ctx(), &mut self.session);
        ui::draw(ui, &mut self.session);
        self.draw_update_ui(ui);

        // Keep polling async results + animating.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
        if self.session.should_quit {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
