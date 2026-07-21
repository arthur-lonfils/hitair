# CLAUDE.md

Guidance for working in this repo. hitair is a *Songless* music guessing game in
Rust — guess a song from growing preview snippets (0.5s → 15s) via a live Deezer
autocomplete, with an optional online "Challenge" lobby. It ships as a **desktop
app** (`hitair-gui`, egui) and a **terminal app** (`hitair`, ratatui) over a
shared core.

## Commands

```sh
cargo run                         # play the GUI (default-member is hitair-gui)
cargo run -p hitair-tui           # play the TUI
cargo run -p hitair-tui -- --smoke           # Deezer fetch + decode + audio
cargo run -p hitair-tui -- --lobby-smoke     # 2-client realtime lobby game
cargo run -p hitair-tui -- --realtime-smoke | --challenge-smoke
# (the smokes live in the TUI binary; the GUI takes no args)

# The CI gate — run all before committing; ci.yml enforces them (--workspace!):
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

**Linux build prerequisites:** ALSA + the egui/eframe libs —
`sudo apt install libasound2-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev pkg-config`.

**GUI dev loop (this machine has a display):** build, then launch with
`WAYLAND_DISPLAY= ` (force X11 so ImageMagick can grab it) and screenshot by
window name: `import -window "$(xwininfo -name hitair -int | awk '/Window id/{print $4}')" out.png`.
`HITAIR_GUI_PREVIEW=playing|result|challenge|host|browse|join|lobby` seeds a screen
with fake data so any layout can be screenshotted without the network/audio.

## Architecture

A cargo **workspace** (`crates/`) so the core is shared by multiple frontends:

- **`hitair-core`** (lib) — the UI-agnostic core: `audio`, `config`, `deezer`,
  `game`, `lobby`, `realtime`, `supa`, `update` (self-update/uninstall for either
  binary — updates whichever is running + keeps its sibling in sync).
- **`hitair-tui`** (bin **`hitair`**) — the ratatui frontend: `main`, `app`, `ui`.
  The binary keeps the name `hitair` so install/self-update are unchanged.
- **`hitair-gui`** (bin **`hitair-gui`**) — the egui/eframe desktop frontend over
  the same core: `main` (runtime + eframe app + preview seeding), `theme` (palette
  + embedded fonts), `input` (egui events → `Key`), `ui` (per-screen rendering).
  A tokio runtime is entered for the process so the session's `tokio::spawn` works
  from the winit main thread; each frame pumps `Msg`/`RtEvent` into the session
  and renders. Album art on the reveal loads via `egui_extras` image loaders.

`cargo run`/`fmt`/`clippy`/`test` at the workspace root operate on all members.
The version lives once under `[workspace.package]`; each crate inherits it via
`version.workspace = true` (that's what `scripts/release.sh` bumps). Modules:

- `session.rs` (**core**) — the `Session`: all app state + every transition (round
  lifecycle, search, the online lobby + spectator flow, audio, `Msg`/`RtEvent`
  handling). UI-agnostic — frontends feed it a frontend-neutral `Key`
  (`handle_key`/`list_click`) + the async pumps (`handle_msg`/`handle_rt_event`/
  `on_tick`) and render from its public state. The shared brain both frontends drive.
- `main.rs` (tui) — CLI arg dispatch + terminal lifecycle (`ratatui::init`/`restore`).
- `app.rs` (tui) — thin adapter over `Session`: the `tokio::select!` loop over
  `crossterm::EventStream`, the `Msg` channel, the lobby `RtEvent` receiver (taken
  from the session via `take_pending_lobby_rx` so the arm borrows a loop-local, not
  `self`), and a 100ms tick; maps crossterm → `Key` and clicks → intents. Also owns
  the self-update check (needs `update.rs`).
- `ui.rs` (tui) — all `ratatui` rendering. Pure function of `&Session`; mutates nothing.
- `game.rs` — round state, clip schedule, guess matching (id + normalized fuzzy).
- `deezer.rs` — Deezer API client (search/charts/playlists/track/preview).
- `audio.rs` — rodio playback on a **dedicated OS thread** (rodio is `!Send`, so it
  never crosses `.await`); driven by an mpsc command channel.
- `config.rs` — clip schedule + category catalog (+ optional `~/.config/hitair/config.toml`).
- `update.rs` — self-update/uninstall (reqwest + `self-replace`; extract via flate2+tar / zip).
- `supa.rs` — Supabase REST client (parties table = lobby discovery for Browse).
- `realtime.rs` — Supabase **Realtime** (Phoenix channels over WebSocket) client
  for the live lobby: presence + broadcast on a tokio task, over mpsc channels.
- `lobby.rs` — the broadcast game protocol (`round_start/result/round_over/
  game_over/new_game`) + the cumulative `Standings`/`Game` every client runs.

**Async model:** the UI loop is on the main thread; HTTP runs as tokio tasks that
send results back as `Msg`; audio is its own thread. Nothing `!Send` is held here.

## Gotchas (things that already bit us)

- **Edition 2024:** `gen` is a reserved keyword.
- **clippy is `-D warnings` in CI:** prefer let-chains, `u.is_multiple_of(n)`,
  test modules at the *end* of the file, no `#[allow]` without reason.
- **Deezer previews are ID3v2.4-tagged MP3** — symphonia fails with
  `IoError("out of bounds")`. `audio::strip_id3v2` removes the tag before decoding.
- **rodio 0.22 API** is new: `DeviceSinkBuilder::open_default_sink()` +
  `Player::connect_new(sink.mixer())` (not the old `OutputStream`/`Sink`).
- **TLS:** reqwest uses the `rustls` feature (aws-lc-rs) to avoid OpenSSL — keep it
  that way so Linux/cross builds stay clean. Do **not** add a second crypto stack
  (that's why we hand-rolled the updater instead of the `self_update` crate).
- **ratatui 0.30:** use `Frame::area()`; `crossterm_0_29` feature is pinned so there
  is one crossterm.
- **egui/eframe 0.35:** the `App` entry point is `fn ui(&mut self, ui, frame)` (it
  wraps a CentralPanel for you) — not `update(ctx, ..)`; style via
  `ctx.all_styles_mut(..)` (there's no `ctx.style()/set_style`); `RichText` has no
  `letter_spacing`. Glyphs missing from Inter (search/arrows/+−) are **drawn** with
  the painter, not typed, so no icon font is needed. `default-members = hitair-gui`
  means bare `cargo build/test/clippy` only touch the GUI — always pass `--workspace`.
- **pty tests are unreliable for asserting rendered text** (ratatui diffs frames);
  verify via exit code, "no panic", or DB side effects instead. (Screen-transition
  text like a new screen's title *does* land in the pty and can be grepped.)
- **Phoenix presence updates append a meta.** Re-`track`ing (an `update_presence`)
  does not replace the entry's meta — it adds one and/or emits a leave-then-join.
  So `realtime::meta_entry` reads the **last** meta (newest state), and the
  `presence_diff` handler applies **leaves before joins**. Reading `metas[0]`
  returns the stale original state.

## Challenge mode — live lobby (Supabase Realtime)

- Project `hitair-backend` (ref `wcaduezxyxawehfsxcci`). The **publishable** key is
  embedded in `supa.rs`/`realtime.rs` — fine; security is Row-Level Security.
- **The lobby is Realtime-driven, not table-driven.** The party code is the
  Realtime topic (`realtime:lobby-<code>`); presence = the live roster, broadcasts
  = the game events. The `parties` table row is only a **discovery ad** for Browse
  — hosting inserts a row with a **placeholder `track_id = 0`** (songs are chosen
  per round and pushed over broadcast, never stored). So **no schema migration was
  needed** for the lobby; `supabase/schema.sql` is unchanged.
- **Scoring has no server authority.** Every client aggregates the same
  `RoundResult` broadcasts (`lobby::Standings`), so boards converge; the host only
  decides round order (`round_start` / `game_over` / `new_game`). Broadcasts echo
  to the sender (`config.broadcast.self = true`), and `Game::on_result` dedups by
  name so recording locally + receiving the echo is safe.
- **Late joiners spectate.** A client only *plays* a game it received the
  `new_game` for (the host always does); joining mid-game leaves `playing_this_game`
  false, so `round_start` makes it a spectator (watches the board, no audio) until
  the next `new_game`. The host re-broadcasts `EV_GAME_STATE` on any roster change
  during a running game so a fresh joiner flips to spectating immediately. Presence
  carries a `spectating` flag so everyone sees a players vs. waiting split.
- Realtime transport verified by `--realtime-smoke` (incl. the presence `spectating`
  round-trip); the full multi-round game + restart by `--lobby-smoke` (two live
  clients that must converge). The App host path + the mid-game spectator path are
  checked by sized-pty harnesses asserting no panic + the expected screen/DB side
  effects.
- The old one-shot `scores`-table flow is gone from the app; `submit_score`/
  `leaderboard`/`player_count` remain in `supa.rs` and are still exercised by
  `--challenge-smoke` (schema/RLS round-trip). Inserts still **omit** `created_at`.
- **Solo play must never require the network.** Challenge is opt-in; a `None`
  Supabase client just disables it.

## Releasing

Tag-driven. Jot changes under `## [Unreleased]` in `CHANGELOG.md`, then:

```sh
scripts/release.sh 0.5.0   # bumps Cargo.toml, rolls the changelog, tags v0.5.0, pushes
```

Pushing the `v*` tag triggers `.github/workflows/release.yml`, which builds all 5
targets (macOS Intel is **cross-compiled** on the arm runner; Windows needs NASM)
and publishes the Release with the tagged version's changelog section as the notes.

**Desktop-app packaging (per OS — so it's a *real* app, not a bare binary):**
- **Windows:** the GUI is a `windows`-subsystem app in release builds
  (`#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` in
  `hitair-gui/src/main.rs`) so no console opens. `hitair-gui/build.rs` embeds
  `assets/icon.ico` via `winresource` (a `cfg(windows)` build-dep — not fetched on
  other hosts) for the Explorer/taskbar icon.
- **macOS:** `release.yml` wraps the GUI in a `hitair-gui.app` bundle via
  `scripts/make-macos-app.sh` (Info.plist + `sips`/`iconutil` → `.icns`); the tarball
  contains the `.app`. `install.sh` drops it in `~/Applications` + a PATH shim;
  `itch.yml` points the `osx-*` channels' `.itch.toml` at `hitair-gui.app`. In-app
  Restart relaunches the bundle via `open -n` (bare-binary relaunch never surfaces
  a window on macOS). The app is **unsigned** — Gatekeeper blocks a browser-quarantined
  copy; the `curl | sh` install clears the quarantine.
- **Linux:** first-run installs an XDG `.desktop` launcher (`desktop.rs`).
- **Icon source:** `hitair-gui --emit-icon <path> [size]` renders the procedural
  disc to PNG; `assets/icon.png` (1024) + `assets/icon.ico` (ImageMagick
  `-define icon:auto-resize=…`) are committed and regenerated from it when the art
  changes.

## Conventions

- **Attribution:** commits/PRs use an `Assisted by: Claude Opus 4.8` footer — never a
  `Co-Authored-By: Claude` trailer.
- Match the surrounding code's style; every change must pass fmt + clippy + tests.
