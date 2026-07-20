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
pub mod realtime;
pub mod session;
pub mod supa;
pub mod update;
