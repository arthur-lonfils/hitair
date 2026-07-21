# Changelog

All notable changes to hitair are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Jot changes under **[Unreleased]** as you work; `scripts/release.sh X.Y.Z` moves
them under the new version, and the release workflow publishes that section as the
GitHub Release notes.

## [Unreleased]

## [0.14.0] - 2026-07-21
### Added
- **A "get ready" countdown before each round.** Rounds now open with a short
  lead-in — a big number in a depleting ring (in the desktop app) — so the clip
  never catches you mid-scroll. The run's first round gives you 5 seconds to
  settle in; every round after is a quick 3. Solo and online; in a lobby the
  scoring timer only starts when the clip does, so the countdown never counts
  against you.

### Changed
- **The reveal meter is a real song timeline.** Its checkpoint ticks were evenly
  spaced, but the checkpoints are 0.5 / 1 / 2 / 4 / 7 / 11 / 15 seconds — very
  different amounts of song. The ticks now sit at each checkpoint's true time
  (bunched near the start, spread toward the end), and the unlocked fill and
  playhead track real seconds.

## [0.13.1] - 2026-07-21
### Changed
- **No more quick song repeats.** A song won't come back around for a while —
  solo remembers the last ~20 played and won't redraw them, and an online game
  never plays the same song twice (falling back only if the pool is too small).
- **Cleaner, fuller guess search.** The autocomplete fetches more results and
  collapses duplicates of the same song — album cut, single, remaster, "(Live)"
  — so you see distinct options instead of the same track five times, and the
  one you want is less likely to be crowded out.

## [0.13.0] - 2026-07-21
### Added
- **What's new, in the app.** A changelog screen (Settings → What's new — press
  `w` in the terminal) shows the release notes for this and past versions — and
  pops up once automatically after an update, so you always see what changed.
- **hitair is a proper desktop app on Linux now.** The desktop build adds an
  application-menu launcher + icon on first run (removable in Settings → Desktop
  app), so you can search for and start hitair like any other app. Skipped under
  the itch app, which handles its own shortcuts.

### Changed
- **Update, uninstall, and restart moved into Settings → Maintenance.** The
  desktop app now has a tidy Maintenance block — current version, an **Update
  now** button when a release is available, **Restart**, and **Uninstall** —
  instead of the Home-screen banner. After updating it shows "Update ready —
  restart to apply", and Restart relaunches the new build. (All hidden under the
  itch app, which owns updates.)

## [0.12.1] - 2026-07-21
### Changed
- **Updates defer to the itch.io app.** When hitair is launched from the itch app,
  the built-in self-updater stands down — itch delta-patches updates itself, so the
  two no longer fight over the same install. Copies from GitHub or the itch *web*
  download keep the built-in GitHub updater. Detected via `ITCHIO_API_KEY`, which
  the app injects thanks to an `.itch.toml` manifest now shipped with the itch
  build; the in-app update/uninstall controls hide under the itch app too.

## [0.12.0] - 2026-07-21
### Added
- **A proper Home screen + Settings.** The app now opens on a **Home** landing —
  a welcome-back line with your stats and big entries for Play solo / Play online /
  Profile / Settings. A new **Settings** screen holds your default effect and
  volume (now **remembered** between sessions), with a pointer to edit your
  identity on the profile.
- **Guess the anime, not just the song.** The **Anime (Japan)** category now sources
  its rounds from AnimeThemes (ranked by AniList popularity), so naming the *anime*
  an opening/ending is from counts as a correct guess — type "Attack on Titan"
  (or "AoT", "Shingeki no Kyojin") as well as the song title. The reveal shows the
  theme it was — "Opening 1 · Attack on Titan". Songs still play from Deezer, and
  the round falls back to the Deezer anime playlist if the pipeline is unavailable.
- **Player profile.** A profile screen — avatar + editable name + a chosen accent
  colour — with lifetime stats (rounds, solve rate, best streak, points), a
  per-category win-rate breakdown, and a recent-games history. Saved locally to
  `~/.config/hitair/`, updated after every round (solo and online). Open it from
  the header avatar in the desktop app, or `Ctrl+P` in the terminal.

## [0.11.0] - 2026-07-21
### Added
- **Anime category.** A new "Anime (Japan)" song pool of Japanese anime
  openings/OST, alongside the genres and decade mixes.
- **Right-artist credit.** Guess the right artist but the wrong song and the guess
  is flagged (an amber "right artist" chip, cyan in the terminal) and banks a
  consolation worth **half** the clip's value — once per round, and only if it
  beats what you'd otherwise score. Works solo and on the online leaderboard.
- **See who's finished a round while you're still guessing.** In a lobby, the
  Playing screen shows a live "N/M finished — names" line as other players land
  their guesses.

### Changed
- **Skipping to the next round asks first if players are still guessing.** The
  host can still move on, but now confirms ("N still guessing — skip anyway?")
  instead of cutting people off mid-round by accident.

## [0.10.1] - 2026-07-20
### Fixed
- **Skip now keeps the song playing instead of restarting.** If you skip while the
  clip is still playing, it carries on past the old checkpoint to the new one —
  uninterrupted — instead of replaying from zero. (Skipping *after* the clip has
  reached its checkpoint, and Replay, still start from the beginning. Applies to
  Normal and Muffled; the speed/reverse effects restart.)
- **The reveal meter's playhead now sweeps from the start.** On replay the playhead
  began at the previous checkpoint tick, making it look like the song restarted
  mid-way. It now sweeps from the very beginning up to your playback position, over
  a faint bar marking how much is unlocked — matching the audio, which always
  plays from the start.
- **Desktop app crashed on launch (v0.10.0).** The GUI links two rustls crypto
  providers — `egui_extras`' image loader pulls in `ring` alongside our
  `aws-lc-rs` — so rustls couldn't choose one and panicked on the first HTTPS
  call (the startup update check). hitair now pins aws-lc-rs at startup, so every
  TLS path (update check, lobby websocket, album art) uses the one provider.

## [0.10.0] - 2026-07-20
### Added
- **Host can change lobby settings without re-hosting.** From the waiting room the
  host opens **Settings** (or `s`) to change rounds, game mode, song pool,
  visibility, and max players; changes apply to the lobby and the next game.
- **Empty lobbies clean themselves up.** When the host or the last member leaves,
  the lobby's Browse ad is deleted, so dead lobbies no longer linger.
- **GUI ↔ TUI feature parity.** The desktop app now has everything the terminal
  app does: in-app **self-update** (an "update available" banner + button, and
  `Ctrl+U`) and **uninstall** (with a confirm dialog, and `Ctrl+X`), a round
  counter and score-flash in the header, and `--version` / `--help`.

### Fixed
- **GUI usability.** Text fields (category filter, guess search, join code, name)
  are now real focused inputs that auto-focus, so you can just start typing — the
  old painted fields never received keystrokes. Added a visible **Back** button in
  the header, a **Play online** button on the menu (online play was keyboard-only
  before), and a status toast so errors/notices are shown.

### Changed
- **Self-update works from either binary.** The update engine moved to
  `hitair-core` and updates whichever binary is running (`hitair` or `hitair-gui`)
  *and* keeps its sibling in sync — refreshed on an update, installed if missing.
  Best-effort and never fails the update. So updating from either app keeps both
  in step.

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
