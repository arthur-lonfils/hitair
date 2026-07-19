//! hitair — a terminal "Songless": guess a song from growing preview snippets.

mod app;
mod audio;
mod config;
mod deezer;
mod game;
mod ui;
mod update;

use std::io::{self, Cursor, Write};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use rodio::Source;

use app::{App, PostAction};
use config::Config;
use deezer::DeezerClient;

#[tokio::main]
async fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        None => run_tui().await,
        // `--smoke` exercises the API + audio pipeline without the TUI.
        Some("--smoke") => smoke().await,
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
    let (app, rx) = App::new(cfg, deezer, audio);
    let outcome = app.run(terminal, rx).await;
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
    println!("Checking for updates…");
    match update::perform_update().await? {
        update::Outcome::UpToDate => {
            println!(
                "hitair is already up to date (v{}).",
                update::CURRENT_VERSION
            )
        }
        update::Outcome::Updated(v) => println!("Updated to v{v} — restart hitair to use it."),
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
        for secs in [0.5f32, 2.0] {
            println!("• Playing {secs}s clip…");
            audio.play(data.clone(), Duration::from_secs_f32(secs));
            tokio::time::sleep(Duration::from_secs_f32(secs + 0.3)).await;
        }
    } else {
        println!("• No audio output device available (skipping playback).");
    }

    println!("Smoke test OK.");
    Ok(())
}
