# hitair 🎵

A terminal music-guessing game in the spirit of *Songless* / *Heardle*. A hidden
song is revealed in growing snippets — first **0.5s**, then 1s, 2s, 4s, 7s, 11s,
15s — and you race to name it. Each wrong guess or skip unlocks a longer clip.

Songs and 30-second previews come from the public **Deezer API** (no account or
API key required). Guesses use a live autocomplete: start typing and pick the
exact track from a dropdown, just like the real thing.

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

On Linux the binary needs the ALSA runtime library at play time
(`sudo apt install libasound2` — already present on most desktops).

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
| Menu    | type to filter · `↑`/`↓` move · `Enter` play · `Esc` clear filter / quit |
| Playing | type to search · `↑`/`↓` pick suggestion · `Enter` guess · `Ctrl+R` replay clip · `Tab` skip · `Esc` back to menu |
| Result  | `Enter` next song · `m` menu · `q` quit |

Guessing is autocomplete-based: a guess is correct when the track you pick matches
the answer (by Deezer id, or by a normalized title + artist so remasters still
count).

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

## How it works

- **`deezer.rs`** — async client (`reqwest` + `serde`) for search / charts /
  playlists / preview download.
- **`audio.rs`** — a dedicated audio thread owns rodio's `!Send` output device and
  plays exact-length clips via `take_duration`. Deezer previews carry an ID3v2.4
  tag that trips symphonia, so it's stripped before decoding.
- **`app.rs`** — the state machine and a `tokio::select!` loop multiplexing key
  events, async HTTP results, and a tick used to debounce autocomplete search.
- **`game.rs`** — round state, the clip schedule, and guess evaluation.
- **`ui.rs`** — `ratatui` rendering.

## License

MIT
