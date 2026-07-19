//! hitair — a terminal "Songless": guess a song from growing preview snippets.

mod app;
mod audio;
mod config;
mod deezer;
mod game;
mod ui;

use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use rodio::Source;

use app::App;
use config::Config;
use deezer::DeezerClient;

#[tokio::main]
async fn main() -> Result<()> {
    // `--smoke` exercises the API + audio pipeline without the TUI, so you can
    // validate networking and audio output on a new machine.
    if std::env::args().any(|a| a == "--smoke") {
        return smoke().await;
    }
    run_tui().await
}

async fn run_tui() -> Result<()> {
    let cfg = Config::load();
    let deezer = DeezerClient::new()?;
    let audio = audio::spawn(); // opens the output device (or reports unavailable)

    // `ratatui::init` enables raw mode + alternate screen and installs a panic
    // hook that restores the terminal, so a panic never leaves a broken tty.
    let terminal = ratatui::init();
    let (app, rx) = App::new(cfg, deezer, audio);
    let result = app.run(terminal, rx).await;
    ratatui::restore();
    result
}

/// Non-interactive check: fetch a chart track, download + decode its preview,
/// and play a 0.5s then a 2s clip if an output device is available.
async fn smoke() -> Result<()> {
    println!("hitair smoke test");

    let deezer = DeezerClient::new()?;
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
