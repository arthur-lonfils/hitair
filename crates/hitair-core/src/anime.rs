//! Anime-round sourcing for the "Anime (Japan)" category.
//!
//! Pipeline: **AniList** ranks popular anime and gives their titles/aliases →
//! **AnimeThemes** provides the authoritative song↔anime↔theme link (which OP/ED
//! a song is) → **Deezer** supplies the actual audio preview. The popular list is
//! fetched once per process; each round picks a random anime, a random one of its
//! themes, and resolves the song on Deezer (retrying a few anime if a song has no
//! preview). Only this category uses the module — everything else is pure Deezer.

use anyhow::{Result, bail};
use serde::Deserialize;
use std::sync::OnceLock;
use tokio::sync::OnceCell;

use crate::deezer::{DeezerClient, Track};

const ANILIST_URL: &str = "https://graphql.anilist.co";
const ANIMETHEMES_URL: &str = "https://api.animethemes.moe/anime";
/// How many popular anime to pull for the pool.
const POOL_SIZE: u32 = 80;
/// How many anime to try before giving up on finding a playable song.
const RESOLVE_TRIES: usize = 8;

/// Anime metadata attached to a round: what the song is from. Powers accepting
/// the anime as a correct guess and the "Opening 1 · Demon Slayer" reveal.
#[derive(Debug, Clone)]
pub struct AnimeTag {
    /// Preferred display name (English if known, else romaji).
    pub anime: String,
    /// Every acceptable title/alias, for matching a typed guess.
    pub titles: Vec<String>,
    /// Human theme label, e.g. "Opening 1".
    pub theme: String,
}

/// One popular anime: how to look it up on AnimeThemes + its accepted titles.
#[derive(Debug, Clone)]
struct AnimeEntry {
    /// AnimeThemes lookup key (the romaji name).
    lookup: String,
    display: String,
    titles: Vec<String>,
}

static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
static POOL: OnceCell<Vec<AnimeEntry>> = OnceCell::const_new();

fn http() -> &'static reqwest::Client {
    HTTP.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("hitair/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// Resolve one anime round: a Deezer track + its preview bytes + the anime tag.
/// `avoid` holds recently-played song keys to skip (see `game::song_key`).
pub async fn resolve_anime_round(
    deezer: &DeezerClient,
    avoid: &[String],
) -> Result<(Track, Vec<u8>, AnimeTag)> {
    let pool = pool().await?;
    for _ in 0..RESOLVE_TRIES {
        let entry = &pool[rand_index(pool.len())];
        let themes = match themes_for(&entry.lookup).await {
            Ok(t) if !t.is_empty() => t,
            _ => continue,
        };
        let theme = &themes[rand_index(themes.len())];
        let query = format!("{} {}", theme.song, theme.artist);
        let Ok(results) = deezer.search(query.trim()).await else {
            continue;
        };
        let mut with_preview = results.into_iter().filter(|t| t.has_preview());
        // Require the artist to match when we know it, so we never play an
        // unrelated song that just happens to share the title (scripts differ
        // between AnimeThemes/Deezer, so we verify by artist, not title).
        let track = if theme.artist.trim().is_empty() {
            with_preview.next()
        } else {
            with_preview.find(|t| artist_matches(t.artist_name(), &theme.artist))
        };
        let Some(track) = track else {
            continue;
        };
        // Skip a song we've played recently.
        if avoid.contains(&crate::game::song_key(&track)) {
            continue;
        }
        let Ok(preview) = deezer.download_preview(&track.preview).await else {
            continue;
        };
        return Ok((
            track,
            preview,
            AnimeTag {
                anime: entry.display.clone(),
                titles: entry.titles.clone(),
                theme: theme_label(&theme.slug),
            },
        ));
    }
    bail!("couldn't find a playable anime song")
}

/// The process-wide popular-anime pool (fetched once).
async fn pool() -> Result<&'static Vec<AnimeEntry>> {
    POOL.get_or_try_init(fetch_popular).await
}

async fn fetch_popular() -> Result<Vec<AnimeEntry>> {
    #[derive(Deserialize)]
    struct Resp {
        data: Data,
    }
    #[derive(Deserialize)]
    struct Data {
        #[serde(rename = "Page")]
        page: Page,
    }
    #[derive(Deserialize)]
    struct Page {
        media: Vec<Media>,
    }
    #[derive(Deserialize)]
    struct Media {
        title: Title,
        #[serde(default)]
        synonyms: Vec<String>,
    }
    #[derive(Deserialize)]
    struct Title {
        romaji: Option<String>,
        english: Option<String>,
        native: Option<String>,
    }

    let query = format!(
        "query{{Page(page:1,perPage:{POOL_SIZE}){{media(type:ANIME,sort:POPULARITY_DESC,format_in:[TV]){{title{{romaji english native}} synonyms}}}}}}"
    );
    let resp: Resp = http()
        .post(ANILIST_URL)
        .json(&serde_json::json!({ "query": query }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let mut out = Vec::new();
    for m in resp.data.page.media {
        let Some(romaji) = m.title.romaji.clone() else {
            continue;
        };
        let mut titles: Vec<String> = [m.title.romaji, m.title.english.clone(), m.title.native]
            .into_iter()
            .flatten()
            .collect();
        titles.extend(m.synonyms);
        let display = m.title.english.unwrap_or_else(|| romaji.clone());
        out.push(AnimeEntry {
            lookup: romaji,
            display,
            titles,
        });
    }
    anyhow::ensure!(!out.is_empty(), "AniList returned no anime");
    Ok(out)
}

/// One theme of an anime: its slug (OP1/ED2…) + song title + first artist.
struct ThemeSong {
    slug: String,
    song: String,
    artist: String,
}

async fn themes_for(name: &str) -> Result<Vec<ThemeSong>> {
    #[derive(Deserialize)]
    struct Resp {
        anime: Vec<AnimeRow>,
    }
    #[derive(Deserialize)]
    struct AnimeRow {
        #[serde(default)]
        animethemes: Vec<ThemeRow>,
    }
    #[derive(Deserialize)]
    struct ThemeRow {
        slug: String,
        song: Option<SongRow>,
    }
    #[derive(Deserialize)]
    struct SongRow {
        title: Option<String>,
        #[serde(default)]
        artists: Vec<ArtistRow>,
    }
    #[derive(Deserialize)]
    struct ArtistRow {
        name: String,
    }

    let resp: Resp = http()
        .get(ANIMETHEMES_URL)
        .query(&[
            ("filter[name]", name),
            ("include", "animethemes.song.artists"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let mut out = Vec::new();
    for a in resp.anime {
        for t in a.animethemes {
            if let Some(song) = t.song
                && let Some(title) = song.title
            {
                let artist = song
                    .artists
                    .first()
                    .map(|x| x.name.clone())
                    .unwrap_or_default();
                out.push(ThemeSong {
                    slug: t.slug,
                    song: title,
                    artist,
                });
            }
        }
    }
    Ok(out)
}

/// "OP1" → "Opening 1", "ED2" → "Ending 2" (ignoring any `-Suffix`).
fn theme_label(slug: &str) -> String {
    let base = slug.split('-').next().unwrap_or(slug);
    let (kind, num) = if let Some(n) = base.strip_prefix("OP") {
        ("Opening", n)
    } else if let Some(n) = base.strip_prefix("ED") {
        ("Ending", n)
    } else {
        return base.to_string();
    };
    if num.is_empty() {
        kind.to_string()
    } else {
        format!("{kind} {num}")
    }
}

fn rand_index(len: usize) -> usize {
    (rand::random::<u64>() % len.max(1) as u64) as usize
}

/// Do two artist names refer to the same act? Alphanumerics-only, case-folded,
/// accepting containment (feat./collab spellings vary). Empty ⇒ can't tell ⇒ no.
fn artist_matches(a: &str, b: &str) -> bool {
    let (a, b) = (norm(a), norm(b));
    !a.is_empty() && !b.is_empty() && (a == b || a.contains(&b) || b.contains(&a))
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::theme_label;

    #[test]
    fn theme_labels() {
        assert_eq!(theme_label("OP1"), "Opening 1");
        assert_eq!(theme_label("ED2"), "Ending 2");
        assert_eq!(theme_label("OP1-AdventPremium"), "Opening 1");
        assert_eq!(theme_label("OP"), "Opening");
    }
}
