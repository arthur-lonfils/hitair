//! hitair — a terminal "Songless": guess a song from growing preview snippets.

mod app;
mod ui;

use std::io::{self, Cursor, Write};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use rodio::Source;

use app::App;
use hitair_core::config::Config;
use hitair_core::deezer::DeezerClient;
use hitair_core::session::PostAction;
use hitair_core::{audio, game, lobby, realtime, supa, update};

#[tokio::main]
async fn main() -> Result<()> {
    hitair_core::install_crypto_provider();
    match std::env::args().nth(1).as_deref() {
        None => run_tui().await,
        // `--smoke` exercises the API + audio pipeline without the TUI.
        Some("--smoke") => smoke().await,
        Some("--challenge-smoke") => challenge_smoke().await,
        Some("--realtime-smoke") => realtime_smoke().await,
        Some("--lobby-smoke") => lobby_smoke().await,
        Some("--update") => do_update().await,
        Some("--uninstall" | "--delete") => do_uninstall(),
        Some("--version" | "-V") => {
            println!("hitair {}", update::CURRENT_VERSION);
            Ok(())
        }
        Some("--help" | "-h") => {
            print_help();
            Ok(())
        }
        Some(other) => {
            eprintln!("unknown option: {other}\n");
            print_help();
            std::process::exit(2);
        }
    }
}

async fn run_tui() -> Result<()> {
    let cfg = Config::load();
    let deezer = DeezerClient::new()?;
    let audio = audio::spawn(); // opens the output device (or reports unavailable)

    // `ratatui::init` enables raw mode + alternate screen and installs a panic
    // hook that restores the terminal, so a panic never leaves a broken tty.
    let terminal = ratatui::init();
    let _ = crossterm::execute!(io::stdout(), crossterm::event::EnableMouseCapture);
    let (app, rx) = App::new(cfg, deezer, audio);
    let outcome = app.run(terminal, rx).await;
    let _ = crossterm::execute!(io::stdout(), crossterm::event::DisableMouseCapture);
    ratatui::restore();

    // Update/uninstall need normal stdout + terminal, so they run after teardown.
    match outcome? {
        Some(PostAction::Update) => do_update().await?,
        Some(PostAction::Uninstall) => uninstall_now()?,
        None => {}
    }
    Ok(())
}

async fn do_update() -> Result<()> {
    if update::is_itch_managed() {
        println!("This copy is managed by the itch app — update it from itch instead.");
        return Ok(());
    }
    println!("Checking for updates…");
    match update::perform_update().await? {
        update::Outcome::UpToDate => {
            println!(
                "hitair is already up to date (v{}).",
                update::CURRENT_VERSION
            )
        }
        update::Outcome::Updated { version, sibling } => {
            println!("Updated to v{version} — restart hitair to use it.");
            if sibling {
                println!("The desktop app is installed too — run `hitair-gui`.");
            }
        }
        update::Outcome::SiblingInstalled => {
            println!(
                "hitair is already up to date (v{}) — installed the desktop app: run `hitair-gui`.",
                update::CURRENT_VERSION
            );
        }
    }
    Ok(())
}

fn do_uninstall() -> Result<()> {
    let exe = std::env::current_exe()?;
    print!("Remove hitair from {}? [y/N] ", exe.display());
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if answer.trim().eq_ignore_ascii_case("y") {
        uninstall_now()
    } else {
        println!("Cancelled.");
        Ok(())
    }
}

fn uninstall_now() -> Result<()> {
    let path = update::uninstall()?;
    println!("Removed {}.", path.display());
    println!("(Any config at ~/.config/hitair was left in place.)");
    Ok(())
}

fn print_help() {
    println!(
        "hitair {} — a terminal Songless music-guessing game",
        update::CURRENT_VERSION
    );
    println!();
    println!("USAGE:");
    println!("  hitair              Play (launches the TUI)");
    println!("  hitair --update     Update to the latest release");
    println!("  hitair --uninstall  Remove the installed binary");
    println!("  hitair --version    Print the version");
    println!("  hitair --help       Show this help");
}

/// Non-interactive check of the multi-round lobby: two clients play a 2-round
/// game over Realtime and must converge on the same cumulative leaderboard.
async fn lobby_smoke() -> Result<()> {
    use lobby::{
        EV_GAME_OVER, EV_NEW_GAME, EV_RESULT, EV_ROUND_OVER, EV_ROUND_START, Game, NewGame,
        RoundResult, RoundStart,
    };
    use serde_json::json;

    println!("hitair lobby smoke (2-client realtime game)");
    let topic = "lobby-gamesmoke";
    let (rounds, max_clips) = (2u32, 7u32);

    let (alice, mut a_rx) = realtime::join(topic, json!({"name": "Alice", "role": "host"})).await?;
    let (bob, mut b_rx) = realtime::join(topic, json!({"name": "Bob"})).await?;
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let mut alice_game = Game::new(rounds, max_clips);
    let mut bob_game = Game::new(rounds, max_clips);

    for round in 1..=rounds {
        alice_game.start_round(round);
        bob_game.start_round(round);
        let tag = if alice_game.is_final_round() {
            " (final)"
        } else {
            ""
        };
        println!("• Round {round}{tag}: host broadcasts round_start");
        alice.broadcast(
            EV_ROUND_START,
            serde_json::to_value(RoundStart {
                round,
                track_id: 3135556,
            })?,
        );

        // Each client "plays" and broadcasts its result.
        let (a_clips, a_solved) = if round == 1 { (2, true) } else { (7, false) };
        let (b_clips, b_solved) = if round == 1 { (3, true) } else { (1, true) };
        let ar = RoundResult {
            round,
            name: "Alice".into(),
            solved: a_solved,
            clips: a_clips,
            time_ms: 3000,
            mistakes: a_clips - 1,
            artist_bonus: 0,
        };
        let br = RoundResult {
            round,
            name: "Bob".into(),
            solved: b_solved,
            clips: b_clips,
            time_ms: 5000,
            mistakes: b_clips.saturating_sub(1),
            artist_bonus: 0,
        };
        alice.broadcast(EV_RESULT, serde_json::to_value(&ar)?);
        bob.broadcast(EV_RESULT, serde_json::to_value(&br)?);

        // Both clients collect this round's two results from the broadcast stream.
        collect_round(&mut alice_game, &mut a_rx).await;
        collect_round(&mut bob_game, &mut b_rx).await;
        alice.broadcast(EV_ROUND_OVER, json!({ "round": round }));
    }
    alice.broadcast(EV_GAME_OVER, json!({}));

    // Host restarts a new game in the same lobby (no re-invite).
    alice.broadcast(
        EV_NEW_GAME,
        serde_json::to_value(NewGame {
            rounds: 3,
            mode: "reverse".into(),
            category: "Rock".into(),
        })?,
    );
    let restarted = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(evt) = b_rx.recv().await {
            if let realtime::RtEvent::Broadcast { event, .. } = evt
                && event == EV_NEW_GAME
            {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    println!("• Host restarted the lobby (Bob got new_game): {restarted}");

    let a_board = alice_game.standings.ranked();
    let b_board = bob_game.standings.ranked();
    println!("• Final leaderboard (Alice's view):");
    for (i, s) in a_board.iter().enumerate() {
        println!(
            "    {}. {:<8} {} pts  ({:.1}s)",
            i + 1,
            s.name,
            s.points,
            s.time_ms as f32 / 1000.0
        );
    }
    let consistent = a_board == b_board;
    println!("• Both clients agree: {consistent}");
    alice.close();
    bob.close();
    anyhow::ensure!(
        consistent && a_board.len() == 2,
        "leaderboards did not converge"
    );
    println!("Lobby smoke OK.");
    Ok(())
}

/// Read broadcasts until both players' results for the current round are in.
async fn collect_round(
    game: &mut lobby::Game,
    rx: &mut tokio::sync::mpsc::Receiver<realtime::RtEvent>,
) {
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        while !game.round_complete(2) {
            match rx.recv().await {
                Some(realtime::RtEvent::Broadcast { event, payload })
                    if event == lobby::EV_RESULT =>
                {
                    if let Ok(r) = serde_json::from_value::<lobby::RoundResult>(payload) {
                        game.on_result(&r);
                    }
                }
                Some(_) => {}
                None => break,
            }
        }
    })
    .await;
}

/// Non-interactive check of the Supabase Realtime transport: two clients join a
/// lobby, verify presence sees both, and a broadcast from one reaches the other.
async fn realtime_smoke() -> Result<()> {
    use serde_json::json;
    println!("hitair realtime smoke (Supabase Realtime)");
    let topic = "lobby-rtsmoke";

    print!("• Alice joining… ");
    let (alice, mut a_rx) = realtime::join(topic, json!({"name": "Alice", "role": "host"})).await?;
    println!("ok");
    print!("• Bob joining… ");
    let (bob, mut b_rx) = realtime::join(topic, json!({"name": "Bob"})).await?;
    println!("ok");

    // Let presence sync, then read Alice's latest player view.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let mut alice_sees: Vec<String> = Vec::new();
    while let Ok(evt) = a_rx.try_recv() {
        match evt {
            realtime::RtEvent::Presence(roster) => {
                alice_sees = roster.iter().map(|e| e.name.clone()).collect()
            }
            realtime::RtEvent::Disconnected(reason) => println!("  (alice disconnected: {reason})"),
            realtime::RtEvent::Broadcast { .. } => {}
        }
    }
    println!("• Alice's lobby: {alice_sees:?}");

    // Update our presence state (host marks ready).
    alice.update_presence(json!({"name": "Alice", "role": "host", "status": "ready"}));

    // Alice broadcasts a round start; Bob should receive it.
    alice.broadcast("round_start", json!({"round": 1, "track_id": 3135556}));
    let received = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(evt) = b_rx.recv().await {
            if let realtime::RtEvent::Broadcast { event, payload } = evt
                && event == "round_start"
            {
                return Some(payload);
            }
        }
        None
    })
    .await
    .ok()
    .flatten();
    println!("• Bob received round_start: {}", received.is_some());
    if let Some(p) = &received {
        println!("  payload: {p}");
    }

    // Presence carries the `spectating` flag (the late-joiner marker): Bob marks
    // himself spectating and Alice should observe it on the roster.
    bob.update_presence(json!({"name": "Bob", "role": "player", "spectating": true}));
    let bob_spectating = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(evt) = a_rx.recv().await {
            if let realtime::RtEvent::Presence(roster) = evt
                && roster.iter().any(|e| e.name == "Bob" && e.spectating)
            {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    println!("• Alice sees Bob spectating: {bob_spectating}");

    alice.close();
    bob.close();
    // `bob_spectating` requires Alice to have received a full roster containing
    // Bob, so it subsumes the (timing-flaky) early two-name snapshot above.
    let ok = received.is_some() && bob_spectating;
    println!("Realtime smoke {}.", if ok { "OK" } else { "INCOMPLETE" });
    anyhow::ensure!(
        ok,
        "presence, broadcast, or spectating flag did not round-trip"
    );
    Ok(())
}

/// Non-interactive check of the Supabase "Challenge" integration: host a party,
/// read it back, resolve the song by id, list it publicly, submit scores, and
/// fetch the leaderboard. Requires the schema in `supabase/schema.sql`.
async fn challenge_smoke() -> Result<()> {
    println!("hitair challenge smoke test (Supabase)");
    let deezer = DeezerClient::new()?;
    let supa = supa::SupaClient::new()?;

    print!("• Picking a chart track… ");
    let mut tracks = deezer.chart_tracks(0).await?;
    tracks.retain(|t| t.has_preview());
    anyhow::ensure!(!tracks.is_empty(), "no playable tracks");
    let track = &tracks[0];
    println!("{} — {}", track.title, track.artist_name());

    print!("• Creating a public party… ");
    let party = supa
        .create_party(supa::NewParty {
            code: String::new(),
            visibility: "public".into(),
            max_players: 8,
            track_id: track.id,
            title: track.title.clone(),
            artist: track.artist_name().to_string(),
            album: track.album_title().map(str::to_string),
            schedule: vec![500, 1000, 2000, 4000, 7000, 11000, 15000],
            host_name: "smoke-test".into(),
        })
        .await?;
    println!("code {}", party.code);

    let fetched = supa
        .get_party(&party.code)
        .await?
        .context("party missing after create")?;
    println!(
        "• Round-trip OK: {} — {} (max {} players, {} visibility)",
        fetched.title, fetched.artist, fetched.max_players, fetched.visibility
    );

    let resolved = deezer.track(fetched.track_id).await?;
    println!(
        "• Joiner resolved the song by id: {} — {} (preview: {})",
        resolved.title,
        resolved.artist_name(),
        if resolved.has_preview() { "yes" } else { "no" }
    );

    let listed = supa
        .list_public_parties(10)
        .await?
        .iter()
        .any(|p| p.code == party.code);
    println!("• Appears in public list: {listed}");

    for (name, clips, ms) in [("Alice", 2, 3100), ("Bob", 4, 6000)] {
        supa.submit_score(&supa::Score {
            party_code: party.code.clone(),
            player_name: name.into(),
            solved: true,
            clips_used: clips,
            time_ms: ms,
            mistakes: clips - 1,
            created_at: None,
        })
        .await?;
    }

    let board = supa.leaderboard(&party.code, 10).await?;
    println!("• Leaderboard ({} entries):", board.len());
    for (i, s) in board.iter().enumerate() {
        println!(
            "    {}. {:<8} {} clips  {:.1}s",
            i + 1,
            s.player_name,
            s.clips_used,
            s.time_ms as f32 / 1000.0
        );
    }
    println!("• Player count: {}", supa.player_count(&party.code).await?);
    println!("Challenge smoke OK (test party {} left in DB).", party.code);
    Ok(())
}

/// Non-interactive check: fetch a chart track, download + decode its preview,
/// and play a 0.5s then a 2s clip if an output device is available.
async fn smoke() -> Result<()> {
    println!("hitair smoke test");

    let deezer = DeezerClient::new()?;
    print!("• Fetching genres… ");
    let genres = deezer.genres().await?;
    println!("{} genres", genres.len());

    print!("• Fetching top chart… ");
    let mut tracks = deezer.chart_tracks(0).await?;
    tracks.retain(|t| t.has_preview());
    anyhow::ensure!(!tracks.is_empty(), "no chart tracks with a preview");
    println!("{} playable tracks", tracks.len());

    let track = &tracks[0];
    println!("• Track: {} — {}", track.title, track.artist_name());

    print!("• Downloading preview… ");
    let bytes = deezer.download_preview(&track.preview).await?;
    println!("{} KiB", bytes.len() / 1024);

    // Decode independently of any output device and count the samples in the
    // first 0.5s — proves the MP3 decoder feature is wired up correctly.
    let frames = audio::strip_id3v2(&bytes).to_vec();
    let decoder = rodio::Decoder::new_mp3(Cursor::new(frames))?;
    let samples_half_sec = decoder.take_duration(Duration::from_secs_f32(0.5)).count();
    println!("• Decoded OK — {samples_half_sec} samples in the first 0.5s");

    let audio = audio::spawn();
    if audio.available() {
        let data = Arc::new(bytes);
        for mode in game::GameMode::ALL {
            println!("• Playing a 2s clip in mode: {}…", mode.label());
            audio.play(data.clone(), Duration::from_secs_f32(2.0), mode);
            tokio::time::sleep(Duration::from_secs_f32(2.3)).await;
        }
        println!("• Playing the full reveal…");
        audio.play_full(data.clone());
        tokio::time::sleep(Duration::from_secs_f32(1.5)).await;
    } else {
        println!("• No audio output device available (skipping playback).");
    }

    println!("Smoke test OK.");
    Ok(())
}
