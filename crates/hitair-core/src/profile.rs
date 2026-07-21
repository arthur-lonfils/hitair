//! Local player profile: identity plus lifetime and per-category stats and a
//! recent-games log, persisted to the config dir. Entirely offline — no network.
//!
//! Stored at `~/.config/hitair/profile.json` (platform config dir). Missing or
//! unparseable files fall back to a fresh default, and every field is
//! `serde(default)` so older/newer files still load.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// How many recent rounds to keep in the history log.
const MAX_RECENT: usize = 40;
/// Minimum rounds in a category before it's eligible as "best category".
const BEST_CATEGORY_MIN: u32 = 3;

/// One finished round handed to the profile to fold into the stats + history.
pub struct RoundRecord {
    pub title: String,
    pub artist: String,
    pub category: String,
    pub won: bool,
    pub clips: u32,
    pub points: u32,
}

/// A single played round, newest kept first in `Stats::recent`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RecentGame {
    pub title: String,
    pub artist: String,
    pub category: String,
    pub won: bool,
    pub clips: u32,
    pub points: u32,
    /// Unix seconds when it was played (0 if the clock was unavailable).
    pub at: u64,
}

/// Aggregate record for one category.
#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq, Eq)]
pub struct CategoryStat {
    pub rounds: u32,
    pub wins: u32,
    pub points: u64,
}

impl CategoryStat {
    pub fn win_rate(&self) -> f32 {
        if self.rounds == 0 {
            0.0
        } else {
            self.wins as f32 / self.rounds as f32
        }
    }
}

/// Lifetime stats across every round the player has finished.
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct Stats {
    pub rounds: u32,
    pub wins: u32,
    pub total_points: u64,
    pub best_streak: u32,
    pub current_streak: u32,
    #[serde(default)]
    pub by_category: HashMap<String, CategoryStat>,
    #[serde(default)]
    pub recent: Vec<RecentGame>,
}

impl Stats {
    pub fn win_rate(&self) -> f32 {
        if self.rounds == 0 {
            0.0
        } else {
            self.wins as f32 / self.rounds as f32
        }
    }

    /// The category the player does best at — highest win rate, then most
    /// played — among those with enough rounds to be meaningful.
    pub fn best_category(&self) -> Option<(&str, &CategoryStat)> {
        self.by_category
            .iter()
            .filter(|(_, c)| c.rounds >= BEST_CATEGORY_MIN)
            .max_by(|a, b| {
                a.1.win_rate()
                    .partial_cmp(&b.1.win_rate())
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.1.rounds.cmp(&b.1.rounds))
            })
            .map(|(k, v)| (k.as_str(), v))
    }
}

/// The player's local profile.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Profile {
    pub name: String,
    /// Accent-colour key the frontend maps to a colour (see the theme). The
    /// player's chosen identity colour, used for their avatar/name.
    #[serde(default = "default_accent")]
    pub accent: String,
    #[serde(default)]
    pub stats: Stats,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: default_name(),
            accent: default_accent(),
            stats: Stats::default(),
        }
    }
}

impl Profile {
    /// Load the saved profile, or a fresh default if none/unreadable.
    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Persist to disk (best-effort; a write failure is silently ignored).
    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(p, json);
        }
    }

    /// Fold a finished round into the lifetime + per-category stats and history.
    pub fn record_round(&mut self, r: RoundRecord) {
        let s = &mut self.stats;
        s.rounds += 1;
        s.total_points += r.points as u64;
        if r.won {
            s.wins += 1;
            s.current_streak += 1;
        } else {
            s.current_streak = 0;
        }
        s.best_streak = s.best_streak.max(s.current_streak);

        let cat = s.by_category.entry(r.category.clone()).or_default();
        cat.rounds += 1;
        cat.points += r.points as u64;
        if r.won {
            cat.wins += 1;
        }

        s.recent.insert(
            0,
            RecentGame {
                title: r.title,
                artist: r.artist,
                category: r.category,
                won: r.won,
                clips: r.clips,
                points: r.points,
                at: now_secs(),
            },
        );
        s.recent.truncate(MAX_RECENT);
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("hitair").join("profile.json"))
}

fn default_accent() -> String {
    "coral".into()
}

/// A friendly default name from the OS user, falling back to "player".
pub fn default_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "player".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(category: &str, won: bool, points: u32) -> RoundRecord {
        RoundRecord {
            title: "Song".into(),
            artist: "Artist".into(),
            category: category.into(),
            won,
            clips: 2,
            points,
        }
    }

    #[test]
    fn records_totals_streaks_and_categories() {
        let mut p = Profile::default();
        p.record_round(rec("Pop", true, 6));
        p.record_round(rec("Pop", true, 5));
        p.record_round(rec("Rock", false, 0));
        p.record_round(rec("Pop", true, 7));

        assert_eq!(p.stats.rounds, 4);
        assert_eq!(p.stats.wins, 3);
        assert_eq!(p.stats.total_points, 18);
        assert_eq!(p.stats.best_streak, 2); // Pop,Pop then broken by Rock
        assert_eq!(p.stats.current_streak, 1); // the final Pop win
        assert_eq!(p.stats.by_category["Pop"].rounds, 3);
        assert_eq!(p.stats.by_category["Pop"].wins, 3);
        assert_eq!(p.stats.by_category["Rock"].wins, 0);
        // Newest first.
        assert_eq!(p.stats.recent.first().unwrap().category, "Pop");
        assert_eq!(p.stats.recent.len(), 4);
    }

    #[test]
    fn best_category_needs_enough_rounds() {
        let mut p = Profile::default();
        // Rock is 1/1 but too few rounds; Pop is 2/3 and qualifies.
        p.record_round(rec("Rock", true, 7));
        for won in [true, true, false] {
            p.record_round(rec("Pop", won, if won { 5 } else { 0 }));
        }
        assert_eq!(p.stats.best_category().map(|(n, _)| n), Some("Pop"));
    }

    #[test]
    fn recent_is_capped() {
        let mut p = Profile::default();
        for _ in 0..(MAX_RECENT + 10) {
            p.record_round(rec("Pop", true, 1));
        }
        assert_eq!(p.stats.recent.len(), MAX_RECENT);
    }
}
