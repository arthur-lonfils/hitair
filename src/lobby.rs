//! Live multi-round Challenge lobby: the broadcast game protocol and the
//! cumulative scoring every client runs over the same broadcast stream (so all
//! clients agree on the standings without a central authority — the host only
//! drives round progression).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// Broadcast event names on the Realtime channel.
pub const EV_ROUND_START: &str = "round_start";
pub const EV_RESULT: &str = "result";
pub const EV_ROUND_OVER: &str = "round_over";
pub const EV_GAME_OVER: &str = "game_over";
pub const EV_NEW_GAME: &str = "new_game";

/// Host → all: begin round `round` with this song.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RoundStart {
    pub round: u32,
    pub track_id: i64,
}

/// A player → all: how they did this round.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RoundResult {
    pub round: u32,
    pub name: String,
    pub solved: bool,
    /// Clips revealed before the correct guess (1 = got it on the first clip).
    pub clips: u32,
    pub time_ms: u32,
    pub mistakes: u32,
}

/// Host → all: start a fresh game in the same lobby.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NewGame {
    pub rounds: u32,
    pub mode: String,
    pub category: String,
}

/// Points for solving on clip `clips` of `max_clips` (fewer clips ⇒ more points).
pub fn round_points(solved: bool, clips: u32, max_clips: u32) -> u32 {
    if !solved {
        return 0;
    }
    max_clips.saturating_sub(clips.saturating_sub(1)).max(1)
}

#[derive(Default, Clone)]
struct Tally {
    points: u32,
    time_ms: u64,
    solves: u32,
}

/// One row of the leaderboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Standing {
    pub name: String,
    pub points: u32,
    pub time_ms: u64,
    pub solves: u32,
}

/// Cumulative standings aggregated from every `RoundResult` seen.
#[derive(Default)]
pub struct Standings {
    entries: HashMap<String, Tally>,
}

impl Standings {
    fn record(&mut self, r: &RoundResult, max_clips: u32) {
        let e = self.entries.entry(r.name.clone()).or_default();
        e.points += round_points(r.solved, r.clips, max_clips);
        e.time_ms += r.time_ms as u64;
        if r.solved {
            e.solves += 1;
        }
    }

    /// Best first: most points, then least total time, then name.
    pub fn ranked(&self) -> Vec<Standing> {
        let mut v: Vec<Standing> = self
            .entries
            .iter()
            .map(|(name, t)| Standing {
                name: name.clone(),
                points: t.points,
                time_ms: t.time_ms,
                solves: t.solves,
            })
            .collect();
        v.sort_by(|a, b| {
            b.points
                .cmp(&a.points)
                .then(a.time_ms.cmp(&b.time_ms))
                .then_with(|| a.name.cmp(&b.name))
        });
        v
    }
}

/// The shared game state each client maintains from the broadcast stream.
pub struct Game {
    pub rounds: u32,
    pub max_clips: u32,
    /// Current round (1-based); 0 before the first `round_start`.
    pub round: u32,
    pub standings: Standings,
    submitted: Vec<String>,
}

impl Game {
    pub fn new(rounds: u32, max_clips: u32) -> Self {
        Self {
            rounds: rounds.max(1),
            max_clips: max_clips.max(1),
            round: 0,
            standings: Standings::default(),
            submitted: Vec::new(),
        }
    }

    pub fn start_round(&mut self, round: u32) {
        self.round = round;
        self.submitted.clear();
    }

    /// Record a result for the current round. Returns true if it was new.
    pub fn on_result(&mut self, r: &RoundResult) -> bool {
        if r.round != self.round || self.submitted.iter().any(|n| n == &r.name) {
            return false;
        }
        self.submitted.push(r.name.clone());
        self.standings.record(r, self.max_clips);
        true
    }

    /// Have all `player_count` players submitted for the current round?
    pub fn round_complete(&self, player_count: usize) -> bool {
        self.submitted.len() >= player_count
    }

    /// How many players have submitted a result for the current round.
    pub fn submitted_count(&self) -> usize {
        self.submitted.len()
    }

    pub fn is_final_round(&self) -> bool {
        self.round >= self.rounds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(name: &str, round: u32, solved: bool, clips: u32, time_ms: u32) -> RoundResult {
        RoundResult {
            round,
            name: name.into(),
            solved,
            clips,
            time_ms,
            mistakes: clips.saturating_sub(1),
        }
    }

    #[test]
    fn points_reward_fewer_clips() {
        assert_eq!(round_points(true, 1, 7), 7); // first clip
        assert_eq!(round_points(true, 7, 7), 1); // last clip
        assert_eq!(round_points(false, 7, 7), 0); // unsolved
    }

    #[test]
    fn standings_accumulate_and_rank() {
        let mut g = Game::new(2, 7);
        g.start_round(1);
        assert!(g.on_result(&result("Alice", 1, true, 2, 3000))); // 6 pts
        assert!(g.on_result(&result("Bob", 1, true, 3, 5000))); //   5 pts
        assert!(!g.on_result(&result("Alice", 1, true, 1, 100))); // dup ignored
        assert!(g.round_complete(2));

        g.start_round(2);
        assert!(g.on_result(&result("Bob", 2, true, 1, 2000))); //   7 pts -> Bob 12
        assert!(g.on_result(&result("Alice", 2, false, 7, 9000))); // 0 pts -> Alice 6
        assert!(g.is_final_round());

        let board = g.standings.ranked();
        assert_eq!(board[0].name, "Bob");
        assert_eq!(board[0].points, 12);
        assert_eq!(board[1].name, "Alice");
        assert_eq!(board[1].points, 6);
    }

    #[test]
    fn result_round_trips_json() {
        let r = result("Zoe", 3, true, 4, 4200);
        let v = serde_json::to_value(&r).unwrap();
        let back: RoundResult = serde_json::from_value(v).unwrap();
        assert_eq!(r, back);
    }
}
