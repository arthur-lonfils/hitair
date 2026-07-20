# hitair 🎵

A music-guessing game in the spirit of *Songless* / *Heardle*, as both a
**desktop app** and a **terminal app**. A hidden song is revealed in growing
snippets — first **0.5s**, then 1s, 2s, 4s, 7s, 11s, 15s — and you race to name
it. Each wrong guess or skip unlocks a longer clip. Play solo, or host a live
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

A live **playback bar** shows the current clip playing. The menu is
**type-to-filter** — start typing to narrow the categories (pulled live from
Deezer's genres, plus decade playlists), or paste a **Deezer playlist id/URL**
to play a custom list.

## Install

Prebuilt binaries are attached to each [release](https://github.com/arthur-lonfils/hitair/releases).
One-line installers:

**Linux / macOS**
```sh
curl -fsSL https://raw.githubusercontent.com/arthur-lonfils/hitair/main/install.sh | sh
```

**Windows (PowerShell)**
```powershell
irm https://raw.githubusercontent.com/arthur-lonfils/hitair/main/install.ps1 | iex
```

Both installers put **`hitair-gui`** (desktop) and **`hitair`** (terminal) on your
PATH. On Linux the binaries need the ALSA runtime at play time
(`sudo apt install libasound2`); the desktop app also needs the usual GL +
windowing libraries (`libxkbcommon`, Mesa GL, X11/Wayland) — already present on a
normal desktop. To **build** the desktop app from source on Debian/Ubuntu:
`sudo apt install libasound2-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev pkg-config`.

## Updating & uninstalling

hitair updates itself in place from the GitHub releases:

```sh
hitair --update      # download + install the latest release
hitair --uninstall   # remove the installed binary
hitair --version     # print the version
```

It also checks for a newer release on startup (in the background). When one is
available the menu shows an **⬆ Update available** banner — press **Ctrl+U** to
update or **Ctrl+X** to uninstall. Set `HITAIR_NO_UPDATE_CHECK=1` to disable the
startup check.

> **Upgrading from a version before 0.3.0?** Those builds predate `--update`, so
> just re-run the install one-liner above once — it overwrites your binary with
> the latest release. From 0.3.0 on, `hitair --update` handles it for you.

## Build from source

Requirements: Rust (2024 edition; built with 1.95), a network connection, and on
Linux the ALSA development headers:

```sh
sudo apt install libasound2-dev pkg-config
cargo run --release
```

Validate networking + audio without the TUI:

```sh
cargo run -- --smoke
```

## Controls

| Screen  | Keys |
|---------|------|
| Menu    | type to filter · `↑`/`↓` move · `Enter` play · `Ctrl+O` online challenge · `Esc` clear filter / quit |
| Playing | type to search · `↑`/`↓` pick suggestion · `Enter` guess · `Ctrl+R` replay clip · `Tab` skip · `Esc` back to menu |
| Result  | `Enter` next song · `m` menu · `q` quit |

**Volume:** `Ctrl+↑` / `Ctrl+↓` at any time (shown in the header). **Mouse:** click
the on-screen **Replay / Skip / Vol** buttons, click a category / suggestion /
lobby row, click the host's **Start / Next / Leave** buttons, and scroll to move
the selection.

**Game modes:** on the menu, `←` / `→` cycles the audio effect — **2× Nightcore**,
**0.5× Slowed**, **Reversed**, **Muffled**, or Normal. The reveal always plays the
real song.

Guessing is autocomplete-based: a guess is correct when the track you pick matches
the answer (by Deezer id, or by a normalized title + artist so remasters still
count). The full preview plays once the round ends.

## Categories & config

The menu lists genre charts (fetched live from Deezer's `/genre`, with a baked-in
fallback) and decade playlists (70s–2010s + a mixed "Blind Test"). Type to filter
the list, or paste a Deezer playlist id/URL to play any list. Override the clip
schedule or add your own playlists via `~/.config/hitair/config.toml`:

```toml
schedule = [0.5, 1, 2, 3, 5, 8, 13]   # seconds per level (length = number of guesses)

[[playlists]]
name = "My Mix"
id = 908622995                         # a Deezer playlist id
```

## Challenge mode — live lobby (online)

Optional real-time multiplayer — Solo never needs the network. Press **`Ctrl+O`**
on the menu to open Challenge:

- **Host a lobby** — pick a **song pool** (a category), the **number of rounds**,
  and the **game mode** (the audio effect applied to every round). Choose
  **public** (anyone can browse & join) or **private** (code only). You get a short
  code to share; friends join and gather in a live waiting room.
- **Browse public lobbies** / **Join by code** — jump into an open lobby.

The host **launches each round** — everyone plays the *same* song at the same time,
then sees the reveal and a **running leaderboard** (fewer clips ⇒ more points). At
the end the host can start a **fresh game in the same lobby** without re-inviting
anyone; players just stay put. Set your name with `n` in the Challenge menu.

Join while a game is already running and you become a **spectator** — you watch the
live board from a "waiting to join" list and are folded in as a player when the host
starts the next game (you can't drop into a game mid-way).

The live lobby is powered by **Supabase Realtime** (Phoenix channels over
WebSocket): presence gives the live roster, and broadcasts carry the game events.
Scoring is computed identically on every client from the same broadcast stream, so
everyone agrees on the board with no central authority — the host only drives the
round order. Discovery (the public list) uses a Supabase Postgres table via a
public *publishable* key, governed by Row-Level Security
([`supabase/schema.sql`](supabase/schema.sql)). If the network is unavailable, Solo
play is unaffected.

## How it works

- **`deezer.rs`** — async client (`reqwest` + `serde`) for search / charts /
  playlists / preview download.
- **`audio.rs`** — a dedicated audio thread owns rodio's `!Send` output device and
  plays exact-length clips via `take_duration`. Deezer previews carry an ID3v2.4
  tag that trips symphonia, so it's stripped before decoding.
- **`app.rs`** — the state machine and a `tokio::select!` loop multiplexing key
  events, async HTTP results, live-lobby Realtime events, and a debounce tick.
- **`game.rs`** — round state, the clip schedule, and guess evaluation.
- **`ui.rs`** — `ratatui` rendering.
- **`realtime.rs`** — the Supabase Realtime (Phoenix channel) client for the live
  lobby: presence + broadcast over a WebSocket, on its own tokio task.
- **`lobby.rs`** — the broadcast game protocol and the cumulative scoring every
  client runs over the shared stream.

## License

MIT.

The desktop GUI embeds two fonts under the SIL Open Font License 1.1 —
[Inter](https://github.com/rsms/inter) and
[Space Grotesk](https://github.com/floriankarsten/space-grotesk); their license
text ships in `crates/hitair-gui/assets/`.
