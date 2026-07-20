//! Round state and guess evaluation.
//!
//! Because guesses are chosen from a Deezer autocomplete list, a correct guess
//! is normally an exact track-id match. We keep a normalized title+artist
//! fallback so that a remaster/re-release (same song, different id) still counts.

use std::sync::Arc;
use std::time::Duration;

use crate::deezer::Track;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Playing,
    Won,
    Lost,
}

/// How the clip is transformed while you guess. The reveal always plays normally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GameMode {
    #[default]
    Normal,
    /// 2× speed (higher pitch).
    Fast,
    /// 0.5× speed (lower pitch).
    Slow,
    /// Clip played backward.
    Reverse,
    /// Muffled, as if underwater (low-pass).
    Muffled,
}

impl GameMode {
    pub const ALL: [GameMode; 5] = [
        GameMode::Normal,
        GameMode::Fast,
        GameMode::Slow,
        GameMode::Reverse,
        GameMode::Muffled,
    ];

    pub fn label(self) -> &'static str {
        match self {
            GameMode::Normal => "Normal",
            GameMode::Fast => "2× Nightcore",
            GameMode::Slow => "0.5× Slowed",
            GameMode::Reverse => "Reversed",
            GameMode::Muffled => "Muffled",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|&m| m == self).unwrap_or(0)
    }

    pub fn next(self) -> GameMode {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> GameMode {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// One song to guess: the hidden answer, its preview audio, and progress.
pub struct Round {
    pub answer: Track,
    pub preview: Arc<Vec<u8>>,
    /// Clip length per level, in seconds ascending.
    schedule: Vec<Duration>,
    /// Current 0-based level (clip index). Higher == more audio revealed.
    pub level: usize,
    /// One entry per wrong guess or skip, for display.
    pub guesses: Vec<GuessLog>,
    pub outcome: Outcome,
}

#[derive(Debug, Clone)]
pub enum GuessLog {
    Wrong(String),
    Skipped,
}

impl Round {
    pub fn new(answer: Track, preview: Vec<u8>, schedule: Vec<Duration>) -> Self {
        debug_assert!(
            !schedule.is_empty(),
            "schedule must have at least one level"
        );
        Self {
            answer,
            preview: Arc::new(preview),
            schedule,
            level: 0,
            guesses: Vec::new(),
            outcome: Outcome::Playing,
        }
    }

    /// Clip length currently unlocked.
    pub fn current_clip(&self) -> Duration {
        self.schedule[self.level.min(self.schedule.len() - 1)]
    }

    pub fn total_levels(&self) -> usize {
        self.schedule.len()
    }

    /// Human-friendly current clip length, e.g. "0.5s" or "15s".
    pub fn current_clip_label(&self) -> String {
        let secs = self.current_clip().as_secs_f32();
        if secs.fract() == 0.0 {
            format!("{secs:.0}s")
        } else {
            format!("{secs:.1}s")
        }
    }

    /// 1-based guess number shown to the player.
    pub fn guess_number(&self) -> usize {
        self.level + 1
    }

    /// Score awarded for winning at the current level: more for fewer clips.
    pub fn score_value(&self) -> u32 {
        (self.schedule.len() - self.level) as u32
    }

    /// Reveal the next clip; if none remain, the round is lost.
    fn advance(&mut self) {
        if self.level + 1 >= self.schedule.len() {
            self.outcome = Outcome::Lost;
        } else {
            self.level += 1;
        }
    }

    /// Submit a selected track as a guess. Returns `true` if it was correct.
    pub fn submit_guess(&mut self, guess: &Track) -> bool {
        if is_correct(guess, &self.answer) {
            self.outcome = Outcome::Won;
            true
        } else {
            self.guesses.push(GuessLog::Wrong(guess.display()));
            self.advance();
            false
        }
    }

    /// Give up this clip and reveal the next one.
    pub fn skip(&mut self) {
        self.guesses.push(GuessLog::Skipped);
        self.advance();
    }
}

/// Is `guess` the same song as `answer`?
pub fn is_correct(guess: &Track, answer: &Track) -> bool {
    if guess.id == answer.id {
        return true;
    }
    title_matches(&guess.title, &answer.title)
        && artist_matches(guess.artist_name(), answer.artist_name())
}

fn title_matches(a: &str, b: &str) -> bool {
    similar(&normalize_title(a), &normalize_title(b), 0.9)
}

fn artist_matches(a: &str, b: &str) -> bool {
    let (a, b) = (normalize(a), normalize(b));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    // Artists collaborate under many spellings; accept containment or a close match.
    a == b || a.contains(&b) || b.contains(&a) || similar(&a, &b, 0.85)
}

fn similar(a: &str, b: &str, threshold: f64) -> bool {
    if a.is_empty() || b.is_empty() {
        return a == b;
    }
    strsim::normalized_levenshtein(a, b) >= threshold
}

/// Lowercase, drop accents-insensitively-ish (ascii only), keep alphanumerics,
/// collapse runs of separators to single spaces.
fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.chars() {
        if ch.is_alphanumeric() {
            for lc in ch.to_lowercase() {
                out.push(lc);
            }
            prev_space = false;
        } else if !prev_space {
            out.push(' ');
            prev_space = true;
        }
    }
    out.trim().to_string()
}

/// Normalize a title, first stripping version noise: parenthetical/bracketed
/// groups (`(feat. X)`, `(Live)`, `[Remix]`), a trailing `" - ..."` version tag
/// (`- Remastered 2011`), and `feat.`/`ft.` collaborator suffixes.
fn normalize_title(s: &str) -> String {
    // Drop bracketed groups.
    let mut cleaned = String::with_capacity(s.len());
    let mut depth = 0i32;
    for ch in s.chars() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = (depth - 1).max(0),
            _ if depth == 0 => cleaned.push(ch),
            _ => {}
        }
    }
    // Drop a trailing " - ..." version tag.
    if let Some(idx) = cleaned.find(" - ") {
        cleaned.truncate(idx);
    }
    // Drop "feat"/"ft" collaborator suffixes.
    let lower = cleaned.to_lowercase();
    for marker in [" feat.", " feat ", " ft.", " ft ", " featuring "] {
        if let Some(idx) = lower.find(marker) {
            cleaned.truncate(idx);
            break;
        }
    }
    normalize(&cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: i64, title: &str, artist: &str) -> Track {
        Track {
            id,
            title: title.into(),
            preview: "x".into(),
            artist: crate::deezer::Artist {
                id: 0,
                name: artist.into(),
            },
            album: None,
        }
    }

    #[test]
    fn exact_id_wins_regardless_of_text() {
        let answer = track(42, "Some Song", "Some Artist");
        let guess = track(42, "totally different text", "whoever");
        assert!(is_correct(&guess, &answer));
    }

    #[test]
    fn remaster_with_different_id_still_matches() {
        let answer = track(1, "Bohemian Rhapsody", "Queen");
        let guess = track(2, "Bohemian Rhapsody - Remastered 2011", "Queen");
        assert!(is_correct(&guess, &answer));
    }

    #[test]
    fn parenthetical_and_feat_are_ignored() {
        assert_eq!(normalize_title("Song (feat. Someone) [Live]"), "song");
        assert_eq!(normalize_title("Song - 2019 Remaster"), "song");
        assert_eq!(normalize_title("Song feat. Someone"), "song");
    }

    #[test]
    fn different_song_does_not_match() {
        let answer = track(1, "Bohemian Rhapsody", "Queen");
        let guess = track(2, "We Will Rock You", "Queen");
        assert!(!is_correct(&guess, &answer));
    }

    #[test]
    fn schedule_progression_and_loss() {
        let schedule = vec![Duration::from_secs(1), Duration::from_secs(2)];
        let answer = track(1, "Answer", "Artist");
        let wrong = track(2, "Wrong", "Artist");
        let mut round = Round::new(answer, vec![0u8; 4], schedule);

        assert_eq!(round.level, 0);
        assert_eq!(round.guess_number(), 1);
        assert_eq!(round.score_value(), 2); // full points at level 0 of 2

        assert!(!round.submit_guess(&wrong));
        assert_eq!(round.level, 1); // advanced, not yet lost
        assert_eq!(round.outcome, Outcome::Playing);

        assert!(!round.submit_guess(&wrong));
        assert_eq!(round.outcome, Outcome::Lost); // out of levels
        assert_eq!(round.guesses.len(), 2);
    }

    #[test]
    fn correct_guess_wins() {
        let answer = track(7, "Answer", "Artist");
        let mut round = Round::new(answer.clone(), vec![0u8; 4], vec![Duration::from_secs(1)]);
        assert!(round.submit_guess(&answer));
        assert_eq!(round.outcome, Outcome::Won);
    }
}
