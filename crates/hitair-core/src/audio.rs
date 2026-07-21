//! Audio playback lives on its own OS thread.
//!
//! rodio's device handle is `!Send`, so a dedicated thread owns it and receives
//! commands over an `std::sync::mpsc` channel. We add clips straight to the
//! mixer (not a `Player`/`Sink`): each clip carries a **generation** stamp and
//! self-stops in its `periodic_access` callback once a newer clip supersedes it.
//! This avoids `Player::clear()`, whose `to_clear` counter can leak onto a fresh
//! clip and silently skip it — the bug behind "skip doesn't play the next clip".
//! The same callback applies live volume.

use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use rodio::buffer::SamplesBuffer;
use rodio::mixer::Mixer;
use rodio::{Decoder, DeviceSinkBuilder, Source};

use crate::game::GameMode;

const ACCESS: Duration = Duration::from_millis(20);

enum AudioCmd {
    /// Play `bytes` from the start for `duration` (or the whole preview if `None`)
    /// under the given game mode, replacing whatever is currently playing.
    Play {
        bytes: Arc<Vec<u8>>,
        duration: Option<Duration>,
        mode: GameMode,
    },
    /// Push the *currently playing* clip's stop point out to `until`, without
    /// restarting it — so a skip keeps the song rolling past the old checkpoint.
    /// A no-op if nothing is playing or the effect can't extend seamlessly.
    Extend(Duration),
    SetVolume(f32),
    /// Pause/resume the current clip in place (used for the reveal on the board);
    /// a paused clip holds its position and emits silence until resumed.
    SetPaused(bool),
    Stop,
    Quit,
}

/// Cheap-to-clone sender used by the app to drive playback.
#[derive(Clone)]
pub struct AudioHandle {
    tx: Sender<AudioCmd>,
    available: bool,
}

impl AudioHandle {
    /// Play the first `duration` of `bytes` under `mode`, from the start.
    pub fn play(&self, bytes: Arc<Vec<u8>>, duration: Duration, mode: GameMode) {
        let _ = self.tx.send(AudioCmd::Play {
            bytes,
            duration: Some(duration),
            mode,
        });
    }

    /// Extend the currently playing clip to end at `until` instead of its current
    /// stop point — how a skip continues the song rather than restarting it.
    pub fn extend(&self, until: Duration) {
        let _ = self.tx.send(AudioCmd::Extend(until));
    }

    /// Play the entire preview normally (for the end-of-round reveal).
    pub fn play_full(&self, bytes: Arc<Vec<u8>>) {
        let _ = self.tx.send(AudioCmd::Play {
            bytes,
            duration: None,
            mode: GameMode::Normal,
        });
    }

    /// Set the output volume (0.0..=1.0), applied live to any playing clip.
    pub fn set_volume(&self, volume: f32) {
        let _ = self.tx.send(AudioCmd::SetVolume(volume));
    }

    /// Pause or resume the current clip in place (for the end-of-round reveal).
    pub fn set_paused(&self, paused: bool) {
        let _ = self.tx.send(AudioCmd::SetPaused(paused));
    }

    pub fn stop(&self) {
        let _ = self.tx.send(AudioCmd::Stop);
    }

    /// Whether a real output device was opened.
    pub fn available(&self) -> bool {
        self.available
    }
}

impl Drop for AudioHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(AudioCmd::Quit);
    }
}

/// Spawn the audio actor. Opening the device happens on the actor thread; this
/// call blocks briefly until it reports whether a device was available.
pub fn spawn() -> AudioHandle {
    let (tx, rx) = mpsc::channel::<AudioCmd>();
    let (ready_tx, ready_rx) = mpsc::channel::<bool>();

    thread::Builder::new()
        .name("hitair-audio".into())
        .spawn(move || audio_loop(rx, ready_tx))
        .expect("spawning audio thread");

    let available = ready_rx.recv().unwrap_or(false);
    AudioHandle { tx, available }
}

/// Return the slice of `bytes` following any leading ID3v2 tag.
///
/// Symphonia's MP3 reader can fail with an out-of-bounds error on ID3v2.4 tags,
/// which Deezer previews use. The tag length is self-describing (a 4-byte
/// syncsafe integer), so we compute the offset and hand the decoder raw frames.
pub fn strip_id3v2(bytes: &[u8]) -> &[u8] {
    if bytes.len() >= 10 && &bytes[0..3] == b"ID3" {
        let footer = if bytes[5] & 0x10 != 0 { 10 } else { 0 };
        let size = ((bytes[6] as usize & 0x7f) << 21)
            | ((bytes[7] as usize & 0x7f) << 14)
            | ((bytes[8] as usize & 0x7f) << 7)
            | (bytes[9] as usize & 0x7f);
        let end = 10 + size + footer;
        if end <= bytes.len() {
            return &bytes[end..];
        }
    }
    bytes
}

fn audio_loop(rx: Receiver<AudioCmd>, ready_tx: Sender<bool>) {
    // The device handle must stay alive for the whole loop.
    let mut sink = match DeviceSinkBuilder::open_default_sink() {
        Ok(sink) => {
            let _ = ready_tx.send(true);
            sink
        }
        Err(_) => {
            let _ = ready_tx.send(false);
            return;
        }
    };
    sink.log_on_drop(false);

    // Bumped on every new clip / stop; the previous clip self-stops when it sees
    // its generation is stale.
    let generation = Arc::new(AtomicU64::new(0));
    let volume = Arc::new(Mutex::new(1.0f32));
    // Whether the current clip is paused in place (reveal pause/resume on the
    // board). Reset whenever a new clip starts or playback stops.
    let paused = Arc::new(AtomicBool::new(false));
    // The movable stop point of the current continuable clip (Normal/Muffled),
    // shared with its playback thread so `Extend` can push it out without a
    // restart. `None` when nothing continuable is playing (idle, a fixed-window
    // effect, or the full reveal).
    let mut stop_at: Option<Arc<Mutex<Duration>>> = None;

    while let Ok(cmd) = rx.recv() {
        match cmd {
            AudioCmd::Play {
                bytes,
                duration,
                mode,
            } => {
                let my_gen = generation.fetch_add(1, Ordering::SeqCst) + 1;
                stop_at = None;
                paused.store(false, Ordering::SeqCst); // a fresh clip starts playing
                let cursor = Cursor::new(strip_id3v2(bytes.as_ref()).to_vec());
                let Ok(decoder) = Decoder::new_mp3(cursor) else {
                    continue;
                };
                let mixer = sink.mixer();
                // Full-song reveal: play the whole thing, unmodified.
                let Some(d) = duration else {
                    add_clip(mixer, decoder, &generation, my_gen, &volume, &paused);
                    continue;
                };
                // Keep the audible window ~= `d` real seconds for every mode.
                match mode {
                    // Normal & Muffled leave the timeline intact, so they play the
                    // whole source and self-stop at a *movable* point — a skip can
                    // then extend the clip mid-play instead of restarting it.
                    GameMode::Normal => {
                        let until = Arc::new(Mutex::new(d));
                        add_clip_until(mixer, decoder, &generation, my_gen, &volume, until.clone());
                        stop_at = Some(until);
                    }
                    GameMode::Muffled => {
                        let until = Arc::new(Mutex::new(d));
                        add_clip_until(
                            mixer,
                            decoder.low_pass(500),
                            &generation,
                            my_gen,
                            &volume,
                            until.clone(),
                        );
                        stop_at = Some(until);
                    }
                    // The speed/reverse effects need a fixed window (reverse must
                    // know its length up front), so they restart on skip.
                    GameMode::Fast => add_clip(
                        mixer,
                        decoder.take_duration(d * 2).speed(2.0),
                        &generation,
                        my_gen,
                        &volume,
                        &paused,
                    ),
                    GameMode::Slow => add_clip(
                        mixer,
                        decoder.take_duration(d / 2).speed(0.5),
                        &generation,
                        my_gen,
                        &volume,
                        &paused,
                    ),
                    GameMode::Reverse => {
                        let base = decoder.take_duration(d);
                        let channels = base.channels();
                        let sample_rate = base.sample_rate();
                        let samples: Vec<f32> = base.collect();
                        // Reverse frame-wise so left/right stay paired.
                        let frame = channels.get() as usize;
                        let reversed: Vec<f32> = samples
                            .chunks_exact(frame)
                            .rev()
                            .flatten()
                            .copied()
                            .collect();
                        add_clip(
                            mixer,
                            SamplesBuffer::new(channels, sample_rate, reversed),
                            &generation,
                            my_gen,
                            &volume,
                            &paused,
                        );
                    }
                }
            }
            // Skip while still playing: carry the live clip past its checkpoint.
            AudioCmd::Extend(until) => {
                if let Some(stop) = &stop_at {
                    *stop.lock().unwrap() = until;
                }
            }
            AudioCmd::SetVolume(v) => *volume.lock().unwrap() = v.clamp(0.0, 1.0),
            AudioCmd::SetPaused(p) => paused.store(p, Ordering::SeqCst),
            // Bumping the generation makes the current clip stop itself.
            AudioCmd::Stop => {
                generation.fetch_add(1, Ordering::SeqCst);
                stop_at = None;
                paused.store(false, Ordering::SeqCst);
            }
            AudioCmd::Quit => break,
        }
    }
}

/// Add one clip to the mixer, wired to stop when superseded, to track volume, and
/// to pause/resume in place via the shared `paused` flag.
fn add_clip<S>(
    mixer: &Mixer,
    source: S,
    generation: &Arc<AtomicU64>,
    my_gen: u64,
    volume: &Arc<Mutex<f32>>,
    paused: &Arc<AtomicBool>,
) where
    S: Source + Send + 'static,
{
    let generation = generation.clone();
    let volume = volume.clone();
    let paused = paused.clone();
    let start_volume = *volume.lock().unwrap();
    let clip = source
        .amplify(start_volume)
        .pausable(false)
        .stoppable()
        .periodic_access(ACCESS, move |s| {
            if generation.load(Ordering::SeqCst) != my_gen {
                s.stop();
            } else {
                let p = s.inner_mut(); // Pausable<Amplify<_>>
                p.set_paused(paused.load(Ordering::SeqCst));
                p.inner_mut().set_factor(*volume.lock().unwrap());
            }
        });
    mixer.add(clip);
}

/// Like `add_clip`, but self-stops once playback passes a *movable* `stop_at`
/// (or a newer clip supersedes it). `Extend` mutates the shared `stop_at`, so the
/// same uninterrupted clip can be carried past its checkpoint. `played` counts up
/// one `ACCESS` per callback, i.e. real playback time (these effects don't warp
/// the timeline), so it stays in step with `stop_at`.
fn add_clip_until<S>(
    mixer: &Mixer,
    source: S,
    generation: &Arc<AtomicU64>,
    my_gen: u64,
    volume: &Arc<Mutex<f32>>,
    stop_at: Arc<Mutex<Duration>>,
) where
    S: Source + Send + 'static,
{
    let generation = generation.clone();
    let volume = volume.clone();
    let start_volume = *volume.lock().unwrap();
    let mut played = Duration::ZERO;
    let clip = source
        .amplify(start_volume)
        .stoppable()
        .periodic_access(ACCESS, move |s| {
            played += ACCESS;
            if generation.load(Ordering::SeqCst) != my_gen || played >= *stop_at.lock().unwrap() {
                s.stop();
            } else {
                s.inner_mut().set_factor(*volume.lock().unwrap());
            }
        });
    mixer.add(clip);
}
