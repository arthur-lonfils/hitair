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
  `crossterm::EventStream` (keys), an mpsc channel of async `Msg`s, and a 100ms tick.
- `ui.rs` — all `ratatui` rendering. Pure function of `&App`; mutates nothing.
- `game.rs` — round state, clip schedule, guess matching (id + normalized fuzzy).
- `deezer.rs` — Deezer API client (search/charts/playlists/track/preview).
- `audio.rs` — rodio playback on a **dedicated OS thread** (rodio is `!Send`, so it
  never crosses `.await`); driven by an mpsc command channel.
- `config.rs` — clip schedule + category catalog (+ optional `~/.config/hitair/config.toml`).
- `update.rs` — self-update/uninstall (reqwest + `self-replace`; extract via flate2+tar / zip).
- `supa.rs` — Supabase REST client for Challenge mode (parties + scores).

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

## Challenge mode (Supabase)

- Project `hitair-backend` (ref `wcaduezxyxawehfsxcci`). The **publishable** key is
  embedded in `supa.rs` — that's fine; security is Row-Level Security.
- Schema: `supabase/schema.sql` (also a migration in `supabase/migrations/`). Apply
  with `supabase link --project-ref <ref>` then `supabase db push` (token-only, no
  DB password needed).
- Score inserts must **omit** `created_at` (it's `NOT NULL default now()`), hence
  `#[serde(skip_serializing_if)]`.
- **Solo play must never require the network.** Challenge is opt-in; a `None` Supabase
  client just disables it.

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
