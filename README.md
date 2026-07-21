# hitair 🎵

A music-guessing game in the spirit of *Songless* / *Heardle*, shipped as both a
**desktop app** and a **terminal app**. A hidden song is revealed in growing
snippets — first **0.5s**, then 1s, 2s, 4s, 7s, 11s, 15s — and you race to name
it. Each wrong guess or skip unlocks a longer clip. Play **solo**, or host a live
online **lobby** and race friends round after round.

Songs and 30-second previews come from the public **Deezer API** (no account or
API key required). Guesses use a live autocomplete: start typing and pick the
exact track from a dropdown, just like the real thing.

Two frontends, one core: **`hitair-gui`** (a native egui desktop app) and
**`hitair`** (the terminal UI). Both are thin frontends over the same engine, so
they play identically.

```
  ♪ hitair — guess the song                    Score 4  ·  Streak 2  ·  Round 3
 ──────────────────────────────────────────────────────────────────────────────
  Guess 3/7   ·   Clip 2.0s
  ▶  0.8s ▕████████░░░░░░░░░░░░░░░░▏ 2.0s
  ✗ Love — SDM   ·   ⏭ skipped

  ╭ Type the title/artist ───────────────────────────────────────────────────╮
  │ bohem▏                                                                     │
  ╰───────────────────────────────────────────────────────────────────────────╯
  ╭ Suggestions (↑↓ to pick, Enter to guess) ─────────────────────────────────╮
  │ › Bohemian Rhapsody  — Queen                                               │
  │   Bohemian Like You  — The Dandy Warhols                                   │
  ╰───────────────────────────────────────────────────────────────────────────╯
 Type to search   ↑↓ pick   Enter guess   Ctrl+R replay   Tab skip   Esc menu
```

The **desktop app** wraps the same game in a designed "after-hours" look: a
**Home** landing, a signature **reveal meter** laid out on the song's real
timeline, album art on the reveal, a **player profile**, and a **Settings**
screen. Both apps open each round with a short **"get ready" countdown** before
the clip plays.

## Install

**Desktop (itch.io)** — the easiest way to keep it updated:
[**rbtech.itch.io/hitair**](https://rbtech.itch.io/hitair). Install through the
**itch app** and it delta-patches new versions for you (the built-in updater
stands down when it detects the itch app).

**Prebuilt binaries** are attached to each
[release](https://github.com/arthur-lonfils/hitair/releases). One-line installers:

**Linux / macOS**
```sh
curl -fsSL https://raw.githubusercontent.com/arthur-lonfils/hitair/main/install.sh | sh
```

**Windows (PowerShell)**
```powershell
irm https://raw.githubusercontent.com/arthur-lonfils/hitair/main/install.ps1 | iex
```

Both installers put **`hitair`** (terminal) on your PATH, and the **desktop app**
as a real application per platform:

- **Linux** — a `hitair-gui` binary on PATH; on first launch it installs an
  **application-menu launcher + icon** so you can start it like any other app
  (removable in **Settings → Desktop app**; skipped under the itch app). Needs the
  ALSA runtime (`sudo apt install libasound2`) plus the usual GL + windowing
  libraries (`libxkbcommon`, Mesa GL, X11/Wayland) — already present on a normal
  desktop.
- **macOS** — a proper **`hitair-gui.app`** bundle installed to `~/Applications`
  (Dock icon, Launchpad/Spotlight, no Terminal window), plus a `hitair-gui` shim
  on PATH. The app is unsigned, so if you download the tarball manually from a
  browser, right-click → **Open** the first time (the `curl … | sh` installer
  clears the quarantine for you).
- **Windows** — a `hitair-gui.exe` desktop app (no console window), carrying the
  hitair icon in Explorer.

## Playing

Launch the desktop app (`hitair-gui`, or `cargo run`) or the terminal app
(`hitair`, or `cargo run -p hitair-tui`). Both open on **Home**: pick **Play solo**,
**Play online**, **Profile**, or **Settings**.

**Play solo** → choose a **song pool** (a genre chart, a decade playlist, the
Anime pool, or a pasted Deezer playlist). Each round opens with a **countdown**
(5 seconds for your first round of the session, 3 after that), then the clip
plays. Start typing to search Deezer and pick the track; a wrong guess or **Skip**
unlocks the next, longer clip. Solve it in as few clips as possible — fewer clips
mean more points, and consecutive solves build a **streak**. The full preview and
album art play on the reveal.

**Game modes** apply an audio effect to the clips — **2× Nightcore**, **0.5×
Slowed**, **Reversed**, **Muffled**, or **Normal**. Cycle it with `←`/`→` on the
category menu (or set a default in Settings). The reveal always plays the real
song.

**Guess the anime, not just the song.** The **Anime (Japan)** pool sources its
rounds from AnimeThemes (ranked by AniList popularity), so naming the *anime* an
opening/ending is from counts as a solve — type "Attack on Titan" (or "AoT",
"Shingeki no Kyojin") as well as the song title. The reveal shows which theme it
was ("Opening 1 · Attack on Titan").

Songs don't repeat in quick succession: solo avoids the last ~20 you played, and
an online game never plays the same song twice.

## Profile & stats

Open your **Profile** from Home (or the header avatar in the desktop app,
`Ctrl+P` in the terminal) to set a **name** and **accent colour**, and to see
lifetime stats (rounds, solve rate, best streak, points), a **per-category**
win-rate breakdown, and a **recent-games** history. It's saved locally to
`~/.config/hitair/` and updated after every round — solo and online.

## Settings, maintenance & "What's new"

**Settings** (from Home) holds your default game mode and volume (remembered
between sessions), the **Desktop app** launcher toggle, and — on the desktop
build — a **Maintenance** block: current version, **Update now** when a release is
available, **Restart**, and **Uninstall**. A **What's new** screen shows the
release notes for this and past versions, and pops up once automatically after an
update. (Maintenance and the built-in updater are hidden under the itch app.)

## Challenge mode — live lobby (online)

Optional real-time multiplayer — solo never needs the network. Choose **Play
online** on Home (or `Ctrl+O` on the menu):

- **Host a lobby** — pick a **song pool**, the **number of rounds**, and the
  **game mode** (applied to every round). Choose **public** (anyone can browse &
  join) or **private** (code only). You get a short code to share; friends join
  and gather in a live waiting room. From the waiting room you can reopen
  **Settings** to change the lobby without re-hosting.
- **Browse public lobbies** / **Join by code** — jump into an open lobby.

The host **launches each round** — everyone plays the *same* song at the same
time, then sees the reveal and a **running leaderboard** (fewer clips ⇒ more
points; guess the right artist but the wrong song for a half-value consolation).
While you're still guessing, a live line shows who's already finished the round.
At the end the host can start a **fresh game in the same lobby** without
re-inviting anyone. Set your name with `n` in the Challenge menu.

Join while a game is already running and you become a **spectator** — you watch
the live board from a "waiting to join" list and are folded in as a player when
the host starts the next game (you can't drop into a game mid-way).

The live lobby is powered by **Supabase Realtime** (Phoenix channels over
WebSocket): presence gives the live roster, and broadcasts carry the game events.
Scoring is computed identically on every client from the same broadcast stream, so
everyone agrees on the board with no central authority — the host only drives the
round order. Discovery (the public list) uses a Supabase Postgres table via a
public *publishable* key, governed by Row-Level Security
([`supabase/schema.sql`](supabase/schema.sql)). If the network is unavailable,
solo play is unaffected.

## Controls

| Screen  | Keys |
|---------|------|
| Home    | `↑`/`↓` move · `Enter` select · `Esc` quit |
| Menu    | type to filter · `↑`/`↓` move · `Enter` play · `←`/`→` game mode · `Ctrl+O` play online · `Ctrl+P` profile · `Esc` back to Home |
| Playing | type to search · `↑`/`↓` pick suggestion · `Enter` guess · `Ctrl+R` replay clip · `Tab` skip · `Esc` back |
| Result  | `Enter` next song · `m` menu · `q` quit |

**Volume:** `Ctrl+↑` / `Ctrl+↓` at any time (shown in the header). **Mouse:** click
the on-screen **Replay / Skip / Vol** buttons, click a category / suggestion /
lobby row, click the host's **Start / Next / Leave** buttons, and scroll to move
the selection. The desktop app's text fields (search, filter, name, join code) are
focused inputs — just start typing.

Guessing is autocomplete-based: a guess is correct when the track you pick matches
the answer (by Deezer id, or by a normalized title + artist so remasters still
count), or — on an anime round — when you name the anime.

## Categories & config

The menu lists genre charts (fetched live from Deezer's `/genre`, with a baked-in
fallback), decade playlists (70s–2010s + a mixed "Blind Test"), and the **Anime
(Japan)** pool. Type to filter the list, or paste a Deezer playlist id/URL to play
any list. Override the clip schedule or add your own playlists via
`~/.config/hitair/config.toml`:

```toml
schedule = [0.5, 1, 2, 3, 5, 8, 13]   # seconds per level (length = number of guesses)

[[playlists]]
name = "My Mix"
id = 908622995                         # a Deezer playlist id
```

## Updating & uninstalling

When installed from GitHub (the one-line installers), hitair updates itself in
place from the GitHub releases:

```sh
hitair --update      # download + install the latest release
hitair --uninstall   # remove the installed binary
hitair --version     # print the version
```

The updater refreshes whichever binary you ran **and** keeps its sibling in step,
so `hitair` and `hitair-gui` stay on the same version. Both apps also check for a
newer release on startup (in the background) and surface it — the desktop app in
**Settings → Maintenance**, the terminal app as an **⬆ Update available** banner
(`Ctrl+U` update, `Ctrl+X` uninstall). Set `HITAIR_NO_UPDATE_CHECK=1` to disable
the startup check.

When launched from the **itch app**, the built-in updater stands down and itch
delta-patches updates instead (detected via the `ITCHIO_API_KEY` the app injects).

> **Upgrading from a version before 0.3.0?** Those builds predate `--update`, so
> just re-run the install one-liner above once. From 0.3.0 on, `hitair --update`
> handles it for you.

## Build from source

A cargo **workspace**. Requirements: Rust (2024 edition), a network connection,
and on Linux the ALSA + windowing/GL development headers:

```sh
sudo apt install libasound2-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev pkg-config

cargo run                    # play the desktop GUI (the default member)
cargo run -p hitair-tui      # play the terminal UI
```

Validate networking + audio decode without a UI (the smokes live in the TUI
binary):

```sh
cargo run -p hitair-tui -- --smoke            # Deezer fetch + decode + audio
cargo run -p hitair-tui -- --lobby-smoke      # 2-client realtime lobby game
```

The CI gate (run all before committing; note `--workspace`):

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Architecture

A cargo workspace under `crates/` so the core is shared by both frontends:

- **`hitair-core`** — the UI-agnostic engine, and a shared **`Session`**
  controller holding all app state and every transition (round lifecycle, search,
  the online lobby + spectator flow, audio). Frontends feed it a frontend-neutral
  `Key` and pump async results; both render from its public state.
  - **`deezer.rs`** — async client (`reqwest` + `serde`) for search / charts /
    playlists / preview download.
  - **`audio.rs`** — a dedicated audio thread owns rodio's `!Send` output device
    and plays exact-length clips via `take_duration`. Deezer previews carry an
    ID3v2.4 tag that trips symphonia, so it's stripped before decoding.
  - **`game.rs`** — round state, the clip schedule, and guess evaluation.
  - **`anime.rs`** — the AniList → AnimeThemes → Deezer pipeline for the anime pool.
  - **`realtime.rs`** / **`lobby.rs`** — the Supabase Realtime (Phoenix channel)
    client and the broadcast game protocol every client scores over.
  - **`supa.rs`**, **`config.rs`**, **`profile.rs`**, **`update.rs`**,
    **`changelog.rs`**, **`desktop.rs`** — lobby discovery, config/catalog, the
    saved profile, self-update/uninstall, the embedded changelog, and the Linux
    desktop launcher.
- **`hitair-tui`** (binary **`hitair`**) — the `ratatui` frontend: a
  `tokio::select!` loop over key events, async HTTP results, lobby Realtime events,
  and a tick, mapping input to `Key` and rendering `&Session`.
- **`hitair-gui`** (binary **`hitair-gui`**) — the `egui`/`eframe` desktop
  frontend over the same core: a tokio runtime is entered for the process, and each
  frame pumps async results into the session and renders.

## License

MIT.

The desktop GUI embeds two fonts under the SIL Open Font License 1.1 —
[Inter](https://github.com/rsms/inter) and
[Space Grotesk](https://github.com/floriankarsten/space-grotesk); their license
text ships in `crates/hitair-gui/assets/`.
