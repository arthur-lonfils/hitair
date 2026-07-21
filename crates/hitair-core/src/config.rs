//! Game configuration: the clip-length schedule and the catalog of categories a
//! player can pick from.
//!
//! Sensible defaults are baked in. If `~/.config/hitair/config.toml` exists it
//! can override the schedule and append extra playlist categories, e.g.:
//!
//! ```toml
//! schedule = [0.5, 1, 2, 3, 5, 8, 13]
//!
//! [[playlists]]
//! name = "My Mix"
//! id = 908622995
//! ```

use std::time::Duration;

use serde::Deserialize;

/// Where a category draws its songs from.
#[derive(Debug, Clone, Copy)]
pub enum CategorySource {
    /// Deezer genre chart (`/chart/{id}/tracks`); id 0 == overall top.
    Chart(i64),
    /// Deezer playlist (`/playlist/{id}/tracks`).
    Playlist(i64),
}

#[derive(Debug, Clone)]
pub struct Category {
    pub name: String,
    pub source: CategorySource,
}

impl Category {
    pub fn chart(name: &str, id: i64) -> Self {
        Self {
            name: name.into(),
            source: CategorySource::Chart(id),
        }
    }
    pub fn playlist(name: &str, id: i64) -> Self {
        Self {
            name: name.into(),
            source: CategorySource::Playlist(id),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Clip length (seconds) revealed at each level. Length == number of guesses.
    pub schedule: Vec<f32>,
    /// Fallback genre charts, shown until Deezer's live `/genre` list loads.
    pub genres: Vec<Category>,
    /// Playlist categories (decade mixes + any from config.toml); always shown.
    pub playlists: Vec<Category>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // Songless-style doubling-ish reveal, capped at 15s.
            schedule: vec![0.5, 1.0, 2.0, 4.0, 7.0, 11.0, 15.0],
            genres: vec![
                // Genre charts (verified to return data).
                Category::chart("Top Charts (All)", 0),
                Category::chart("Pop", 132),
                Category::chart("Rock", 152),
                Category::chart("Rap / Hip-Hop", 116),
                Category::chart("Dance", 113),
                Category::chart("R&B", 165),
                Category::chart("Jazz", 129),
                Category::chart("Classical", 98),
                Category::chart("Metal", 464),
            ],
            playlists: vec![
                // Editorial decade playlists (real Deezer playlist ids).
                Category::playlist("70s Hits", 8877326262),
                Category::playlist("80s Hits", 867825522),
                Category::playlist("90s Hits", 878989033),
                Category::playlist("2000s Pop-Rock", 8074584322),
                Category::playlist("2010s Pop-Rock", 8074581462),
                // Japanese anime openings/OST (Deezer Japan editorial).
                Category::playlist("Anime (Japan)", 5206929684),
                Category::playlist("Blind Test (70s–20s)", 7089916404),
            ],
        }
    }
}

impl Config {
    /// Load defaults, then apply an optional TOML override if present.
    pub fn load() -> Self {
        let mut cfg = Config::default();
        if let Some(file) = read_file_config() {
            if let Some(schedule) = file.schedule
                && !schedule.is_empty()
            {
                cfg.schedule = schedule;
            }
            for p in file.playlists {
                cfg.playlists.push(Category::playlist(&p.name, p.id));
            }
        }
        cfg
    }

    /// The full menu used before live genres load: fallback genres then playlists.
    pub fn default_categories(&self) -> Vec<Category> {
        self.genres
            .iter()
            .chain(self.playlists.iter())
            .cloned()
            .collect()
    }

    pub fn schedule_durations(&self) -> Vec<Duration> {
        self.schedule
            .iter()
            .map(|s| Duration::from_secs_f32(*s))
            .collect()
    }
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    schedule: Option<Vec<f32>>,
    #[serde(default)]
    playlists: Vec<PlaylistEntry>,
}

#[derive(Debug, Deserialize)]
struct PlaylistEntry {
    name: String,
    id: i64,
}

fn read_file_config() -> Option<FileConfig> {
    let path = dirs::config_dir()?.join("hitair").join("config.toml");
    let text = std::fs::read_to_string(path).ok()?;
    toml::from_str(&text).ok()
}
