//! hitair — desktop GUI frontend (eframe/egui) over `hitair_core::session::Session`.
//!
//! On Windows, release builds use the `windows` subsystem so launching the app
//! never opens a console window behind the game (debug builds keep the console
//! for logging).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod input;
mod smoke;
mod theme;
mod ui;

use std::sync::{Arc, Mutex};

use eframe::egui;

use hitair_core::audio;
use hitair_core::config::Config;
use hitair_core::deezer::DeezerClient;
use hitair_core::session::{MaintenanceAction, Msg, Session};
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
        // Maintenance tool: render the app icon to a high-res PNG (source for the
        // packaged `.icns`/`.ico`). `hitair-gui --emit-icon path.png [size]`.
        Some("--emit-icon") => {
            let path = std::env::args()
                .nth(2)
                .unwrap_or_else(|| "icon.png".to_string());
            let n: usize = std::env::args()
                .nth(3)
                .and_then(|s| s.parse().ok())
                .unwrap_or(1024);
            let img = image::RgbaImage::from_raw(n as u32, n as u32, disc_rgba(n))
                .ok_or_else(|| anyhow::anyhow!("failed to build icon buffer"))?;
            img.save(&path)?;
            println!("wrote {path} ({n}×{n})");
            return Ok(());
        }
        // Non-interactive integration smokes (dev tools; hit the real network/audio).
        Some(flag @ ("--smoke" | "--challenge-smoke" | "--realtime-smoke" | "--lobby-smoke")) => {
            let flag = flag.to_string();
            return tokio::runtime::Runtime::new()?.block_on(smoke::run(&flag));
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
/// The window icon is 256px; the same art scales to any size for `.icns`/`.ico`.
fn app_icon() -> egui::IconData {
    const N: u32 = 256;
    egui::IconData {
        rgba: disc_rgba(N as usize),
        width: N,
        height: N,
    }
}

/// The record-disc icon rasterised into RGBA at `n`×`n`. All feature sizes scale
/// with `n` so it stays proportional from 16px favicons to 1024px `.icns` tiles.
fn disc_rgba(n: usize) -> Vec<u8> {
    let s = n as f32 / 256.0; // scale relative to the reference 256px art
    let mut rgba = vec![0u8; n * n * 4];
    let c = (n as f32 - 1.0) / 2.0;
    let r_out = 118.0 * s;
    let coral = [0xFFu8, 0x6A, 0x5D];
    let ink = [0x18u8, 0x12, 0x20];
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = (x as f32 - c, y as f32 - c);
            let d = (dx * dx + dy * dy).sqrt();
            if d > r_out {
                continue; // transparent outside the disc
            }
            let mut col = coral;
            if (d % (15.0 * s)) < (2.0 * s) {
                col = ink; // grooves
            }
            if d < 34.0 * s {
                col = coral; // center label
            }
            if d < 9.0 * s {
                col = ink; // spindle hole
            }
            let feather = 1.5 * s;
            let a = ((r_out - d).clamp(0.0, feather) / feather * 255.0) as u8;
            let i = (y * n + x) * 4;
            rgba[i..i + 4].copy_from_slice(&[col[0], col[1], col[2], a]);
        }
    }
    rgba
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

/// Relaunch this app as a fresh, detached process.
///
/// On macOS, if we're running from inside a `.app` bundle, relaunch the *bundle*
/// via `open -n` so it comes to the foreground as a proper app — spawning the
/// bare executable there starts a background process that never surfaces a window
/// (which is why in-app Restart appeared to "not reopen"). Everywhere else we
/// spawn the executable directly, detached from our stdio and process group so it
/// cleanly outlives us.
fn relaunch(exe: &std::path::Path) {
    use std::process::{Command, Stdio};
    #[cfg(target_os = "macos")]
    if let Some(app) = macos_app_bundle(exe) {
        let _ = Command::new("/usr/bin/open").arg("-n").arg(app).spawn();
        return;
    }
    let mut cmd = Command::new(exe);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0); // detach from our process group / controlling terminal
    }
    let _ = cmd.spawn();
}

/// If `exe` is `…/<Name>.app/Contents/MacOS/<bin>`, return the `.app` path.
#[cfg(target_os = "macos")]
fn macos_app_bundle(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    let macos = exe.parent()?; // …/Contents/MacOS
    let contents = macos.parent()?; // …/Contents
    let app = contents.parent()?; // …/<Name>.app
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension()? == "app")
        .then(|| app.to_path_buf())
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
        // First launch opens on the setup wizard (see `Session::new`), which runs
        // the install when the user opts in — see `take_setup_request` below.
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

    /// Relaunch the (possibly just-updated) app and close this instance.
    fn restart(&self, ctx: &egui::Context) {
        if let Ok(exe) = std::env::current_exe() {
            relaunch(&exe);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Update-progress toast (bottom-left) + the uninstall confirm dialog, on any
    /// screen. The "update available" + uninstall/restart *actions* live in
    /// Settings → Maintenance. Hidden entirely under the itch app.
    fn draw_update_ui(&mut self, ui: &mut egui::Ui) {
        if update::is_itch_managed() {
            return;
        }
        let ctx = ui.ctx().clone();
        let phase = *self.update_phase.lock().unwrap();

        if phase != UpdatePhase::Idle {
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
                            egui::RichText::new("Update ready — restart to apply (Settings).")
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
                    UpdatePhase::Idle => {}
                });
        }

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
                played_songs: Vec::new(),
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
        "whatsnew" => session.screen = Screen::Whatsnew,
        "setup" => session.screen = Screen::Setup,
        "countdown" => {
            let answer = track(1, "Blinding Lights", "The Weeknd");
            let round = Round::new(answer, vec![0u8; 8], session.cfg.schedule_durations());
            session.round = Some(round);
            session.screen = Screen::Playing;
            session.preview_start_countdown();
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
        // The first-run wizard asked us to install hitair as a real desktop app.
        if self.session.take_setup_request()
            && let Ok(exe) = std::env::current_exe()
        {
            let msg = match hitair_core::setup::run(&exe, &icon_png()) {
                Ok(summary) => summary,
                Err(e) => format!("Couldn't finish setup: {e}"),
            };
            self.session.set_status(msg);
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
        // Reflect update progress + run any maintenance action from Settings.
        if *self.update_phase.lock().unwrap() == UpdatePhase::Done {
            self.session.update_ready = true;
        }
        if let Some(action) = self.session.take_maintenance() {
            match action {
                MaintenanceAction::Update => self.start_update(),
                MaintenanceAction::Uninstall => self.confirm_uninstall = true,
                MaintenanceAction::Restart => self.restart(ui.ctx()),
            }
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
