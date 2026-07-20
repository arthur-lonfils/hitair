# Changelog

All notable changes to hitair are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Jot changes under **[Unreleased]** as you work; `scripts/release.sh X.Y.Z` moves
them under the new version, and the release workflow publishes that section as the
GitHub Release notes.

## [Unreleased]

## [0.9.0] - 2026-07-20
### Added
- **Desktop app (`hitair-gui`).** A native egui/eframe GUI with the full game —
  solo (Menu → Playing → Result) and the online Challenge lobby — in a designed
  "after-hours" look with embedded Inter + Space Grotesk, a signature reveal
  meter, and album art on the reveal. Ships alongside the terminal `hitair`
  binary; both installers now put both on your PATH.

### Changed
- The project is now a **cargo workspace**: `hitair-core` (the UI-agnostic engine
  + a shared `Session` controller) with two thin frontends, `hitair-tui` and
  `hitair-gui`. `cargo run` launches the GUI; `cargo run -p hitair-tui` the TUI.
  No gameplay change — both frontends drive the same core.

## [0.8.0] - 2026-07-20
### Added
- **Spectators** — joining a lobby while a game is running no longer drops you
  into the current rounds. You wait in a **"waiting to join" list** and watch the
  live leaderboard; you become a player automatically when the host starts the
  next game (after all rounds finish). Everyone sees who's playing vs. waiting.

## [0.7.0] - 2026-07-20
### Added
- **Live Challenge lobby** — a real, multi-round online mode. The host opens a
  **lobby** (public & browsable, or private by code), friends join and stay in a
  live waiting room (powered by Supabase Realtime presence), and the host
  **launches the rounds** — everyone plays the same song at the same time. A
  running **leaderboard** builds across rounds (fewer clips ⇒ more points), and
  when the game ends the host can start a **fresh game in the same lobby** without
  re-inviting anyone.
- **Online game modes** — the host picks the audio effect (Normal, 2× Nightcore,
  0.5× Slowed, Reversed, Muffled) and the number of rounds when opening the lobby;
  it applies to every round for all players.

### Changed
- Challenge mode is now the live lobby: **Host a lobby / Browse public lobbies /
  Join by code**. Scoring is computed identically on every client from the
  broadcast stream (no central authority — the host only drives round order).

## [0.6.0] - 2026-07-20
### Added
- **Game modes** (Solo) — cycle the audio effect with `←` / `→` on the menu:
  **2× Nightcore**, **0.5× Slowed**, **Reversed**, and **Muffled**, alongside
  Normal. The clip window stays the same real-time length; the end-of-round
  reveal always plays the real song. Online game modes will arrive with the
  party round/lobby system.

## [0.5.0] - 2026-07-20
### Added
- **Volume control** — `Ctrl+↑` / `Ctrl+↓` adjust the output volume (shown in the
  header), applied live to whatever is playing.
- **Mouse support** — clickable **Replay / Skip / Vol** buttons on the Playing
  screen, clickable category / suggestion / party rows, **Next song / Menu**
  buttons on the result screen, and scroll-wheel list navigation.
- On the reveal (round end and party leaderboard), the **whole preview** now
  plays instead of just the last clip.

### Fixed
- Skipping now reliably auto-plays the next, longer clip. The audio actor adds
  clips straight to the mixer with a generation stamp instead of rodio's
  `Player::clear()`, whose `to_clear` counter could leak onto a fresh clip and
  silence it.

## [0.4.0] - 2026-07-19
### Added
- **Challenge mode (online, opt-in).** Press `Ctrl+O` on the menu to play head to
  head. Host a **party** (public and browsable, or private by code) with a
  **max-player** cap; browse public parties; or join by code. Everyone races the
  same song and lands on a **shared leaderboard** that refreshes live, ranked by
  fewest clips then fastest time. Set your name with `n`.
- Backed by [Supabase](https://supabase.com) (hosted Postgres + REST) using a
  public publishable key with Row-Level Security; schema in `supabase/schema.sql`.

Solo play remains fully offline and unaffected.

## [0.3.0] - 2026-07-19
### Added
- **Self-update and uninstall.** `hitair --update` replaces the running binary
  with the latest release; `hitair --uninstall` removes it; `--version`/`--help`.
- The TUI checks for a newer release on startup and shows an **⬆ Update available**
  banner — `Ctrl+U` to update, `Ctrl+X` to uninstall. Disable with
  `HITAIR_NO_UPDATE_CHECK=1`.

## [0.2.1] - 2026-07-19
### Fixed
- Build the Intel macOS binary by cross-compiling on the Apple-Silicon runner; the
  hosted `macos-13` runners queued indefinitely. Releases now ship all 5 targets.

## [0.2.0] - 2026-07-19
### Added
- Animated **playback bar** showing the current clip's position.
- **Type-to-filter** category menu, plus pasting a Deezer playlist id/URL to play
  any list.
- **Live genres** fetched from Deezer (with a baked-in fallback).
- A **scoring animation** on the result screen and a header score flash.

## [0.1.0] - 2026-07-19
### Added
- Initial release: a terminal *Songless* — guess a song from growing preview
  snippets (0.5s → 15s) via a live Deezer autocomplete.
- Precise clip playback on a dedicated rodio audio thread.
- Cross-platform release binaries (Linux x86_64/aarch64, macOS arm64/x86_64,
  Windows x86_64) and one-line install scripts.

[Unreleased]: https://github.com/arthur-lonfils/hitair/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/arthur-lonfils/hitair/releases/tag/v0.4.0
[0.3.0]: https://github.com/arthur-lonfils/hitair/releases/tag/v0.3.0
[0.2.1]: https://github.com/arthur-lonfils/hitair/releases/tag/v0.2.1
[0.2.0]: https://github.com/arthur-lonfils/hitair/releases/tag/v0.2.0
[0.1.0]: https://github.com/arthur-lonfils/hitair/releases/tag/v0.1.0
