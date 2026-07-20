# CLAUDE.md

Guidance for working in this repo. hitair is a terminal *Songless* — a TUI music
guessing game in Rust: guess a song from growing preview snippets (0.5s → 15s) via
a live Deezer autocomplete, with an optional online "Challenge" mode.

## Commands

```sh
cargo run                     # play (TUI)
cargo run -- --smoke          # non-interactive: Deezer fetch + decode + audio
cargo run -- --challenge-smoke# non-interactive: Supabase party round-trip
cargo run -- --update|--uninstall|--version|--help

# The CI gate — run all three before committing; ci.yml enforces them:
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

**Linux build prerequisite:** ALSA headers — `sudo apt install libasound2-dev pkg-config`.

## Architecture

Single binary; modules in `src/`:

- `main.rs` — CLI arg dispatch + terminal lifecycle (`ratatui::init`/`restore`).
- `app.rs` — the `App` state machine and the async loop: `tokio::select!` over
  `crossterm::EventStream` (keys), an mpsc channel of async `Msg`s, an optional
  lobby `RtEvent` receiver (handed in via `pending_lobby_rx` so the arm borrows a
  loop-local, not `self`), and a 100ms tick.
- `ui.rs` — all `ratatui` rendering. Pure function of `&App`; mutates nothing.
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
- **pty tests are unreliable for asserting rendered text** (ratatui diffs frames);
  verify via exit code, "no panic", or DB side effects instead.

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
- Realtime transport verified by `--realtime-smoke`; the full multi-round game +
  restart by `--lobby-smoke` (two live clients that must converge). The App host
  path is checked by a sized-pty harness asserting no panic + a `parties` row lands.
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

## Conventions

- **Attribution:** commits/PRs use an `Assisted by: Claude Opus 4.8` footer — never a
  `Co-Authored-By: Claude` trailer.
- Match the surrounding code's style; every change must pass fmt + clippy + tests.
