//! Audio playback lives on its own OS thread.
//!
//! rodio's device handle (`MixerDeviceSink`) and `Player` are `!Send`, so they
//! must never cross an `.await`. Instead of fighting the async runtime, we run
//! an "actor": a dedicated thread owns the audio device and receives commands
//! over a plain `std::sync::mpsc` channel. Playback is non-blocking, so it never
//! stalls the TUI event loop.

use std::io::Cursor;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use rodio::{Decoder, DeviceSinkBuilder, Player, Source};

enum AudioCmd {
    /// Play the first `duration` of the given MP3 bytes, replacing anything
    /// currently playing.
    Play {
        bytes: Arc<Vec<u8>>,
        duration: Duration,
    },
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
    /// Play the first `duration` of `bytes`. No-op if no audio device is present.
    pub fn play(&self, bytes: Arc<Vec<u8>>, duration: Duration) {
        let _ = self.tx.send(AudioCmd::Play { bytes, duration });
    }

    pub fn stop(&self) {
        let _ = self.tx.send(AudioCmd::Stop);
    }

    /// Whether a real output device was opened. When false, the game still runs
    /// (visual only) but nothing is audible.
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
    // The device handle must stay alive for the whole loop, otherwise playback
    // stops the moment it is dropped.
    let mut sink = match DeviceSinkBuilder::open_default_sink() {
        Ok(sink) => {
            let _ = ready_tx.send(true);
            sink
        }
        Err(_) => {
            // No device (e.g. headless / SSH). Report unavailable and exit; the
            // std channel is unbounded so senders never block on the dead loop.
            let _ = ready_tx.send(false);
            return;
        }
    };
    // Don't print rodio's "Dropping DeviceSink" notice on shutdown.
    sink.log_on_drop(false);

    let player = Player::connect_new(sink.mixer());

    while let Ok(cmd) = rx.recv() {
        match cmd {
            AudioCmd::Play { bytes, duration } => {
                // Drop whatever is playing, then decode a fresh clip. `Cursor`
                // needs an owned buffer; we hand it the raw MP3 frames (ID3v2
                // tag stripped) so decoding never trips on the tag.
                player.clear();
                let cursor = Cursor::new(strip_id3v2(bytes.as_ref()).to_vec());
                if let Ok(decoder) = Decoder::new_mp3(cursor) {
                    // `take_duration` truncates the sample stream at the exact
                    // sample count, so sub-second clips (0.5s) are precise.
                    player.append(decoder.take_duration(duration));
                    player.play();
                }
            }
            AudioCmd::Stop => player.clear(),
            AudioCmd::Quit => break,
        }
    }
}
