//! hitair-core — the UI-agnostic core of hitair.
//!
//! Everything a frontend needs and nothing it doesn't: the Deezer client, the
//! audio actor, the round/guess logic, config, and the online Challenge lobby
//! (Supabase REST + Realtime). Both the terminal (`hitair-tui`) and the desktop
//! GUI (`hitair-gui`) are thin frontends over these modules.

pub mod audio;
pub mod config;
pub mod deezer;
pub mod game;
pub mod lobby;
pub mod profile;
pub mod realtime;
pub mod session;
pub mod supa;
pub mod update;

/// Pin **aws-lc-rs** as the process-wide rustls crypto provider.
///
/// The desktop build links a *second* rustls provider transitively — `egui_extras`'
/// http image loader pulls in `ring` alongside our `aws-lc-rs` (via reqwest). With
/// two providers compiled in, rustls can't pick one from crate features and panics
/// on the first TLS handshake. Install our chosen stack before any HTTPS runs so
/// every path (reqwest, the websocket, the album-art loader) uses aws-lc-rs.
///
/// Idempotent and best-effort — a no-op where only one provider exists (the TUI).
/// Both binaries call this once at startup.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}
