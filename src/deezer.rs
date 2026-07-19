//! Minimal async client for the public Deezer API.
//!
//! No authentication is required for the `search`, `genre`, `chart` and
//! `playlist` endpoints we use. Track `preview` fields are signed 30s MP3 URLs
//! that expire after a few hours, so callers should download the bytes at round
//! start rather than holding the URL.

use anyhow::{Context, Result};
use serde::Deserialize;
use serde::de::DeserializeOwned;

const BASE: &str = "https://api.deezer.com";

/// A generic `{ "data": [ ... ] }` envelope used by most Deezer list endpoints.
#[derive(Debug, Deserialize)]
struct DataResponse<T> {
    data: Vec<T>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Artist {
    #[allow(dead_code)] // part of the API model; kept for completeness
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Album {
    #[allow(dead_code)] // part of the API model; kept for completeness
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub title: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Track {
    pub id: i64,
    pub title: String,
    /// 30s preview MP3 URL. Deezer returns an empty string when unavailable.
    #[serde(default)]
    pub preview: String,
    pub artist: Artist,
    #[serde(default)]
    pub album: Option<Album>,
}

impl Track {
    pub fn has_preview(&self) -> bool {
        !self.preview.is_empty()
    }

    pub fn artist_name(&self) -> &str {
        &self.artist.name
    }

    pub fn album_title(&self) -> Option<&str> {
        self.album
            .as_ref()
            .map(|a| a.title.as_str())
            .filter(|t| !t.is_empty())
    }

    /// "Title — Artist", used in the autocomplete list and reveal screen.
    pub fn display(&self) -> String {
        format!("{} — {}", self.title, self.artist.name)
    }
}

/// Deezer genre. Retained for a future dynamic category menu; the shipped menu
/// uses curated genre ids in `config.rs`.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct Genre {
    pub id: i64,
    pub name: String,
}

/// Cheap-to-clone handle around a shared `reqwest` client.
#[derive(Clone)]
pub struct DeezerClient {
    http: reqwest::Client,
}

impl DeezerClient {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("hitair/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("building HTTP client")?;
        Ok(Self { http })
    }

    async fn get_list<T: DeserializeOwned>(
        &self,
        url: &str,
        query: &[(&str, &str)],
    ) -> Result<Vec<T>> {
        let resp = self
            .http
            .get(url)
            .query(query)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?
            .error_for_status()
            .with_context(|| format!("bad status from {url}"))?;
        let parsed: DataResponse<T> = resp.json().await.context("decoding Deezer JSON")?;
        Ok(parsed.data)
    }

    /// Full-text search over tracks — used for guess autocomplete. Results are
    /// returned as-is (we do *not* require a preview here, since the player is
    /// only naming a track, not listening to it).
    pub async fn search(&self, query: &str) -> Result<Vec<Track>> {
        self.get_list(&format!("{BASE}/search"), &[("q", query), ("limit", "12")])
            .await
    }

    #[allow(dead_code)]
    pub async fn genres(&self) -> Result<Vec<Genre>> {
        self.get_list(&format!("{BASE}/genre"), &[]).await
    }

    /// Top tracks for a genre chart (`genre_id` 0 == overall top).
    pub async fn chart_tracks(&self, genre_id: i64) -> Result<Vec<Track>> {
        self.get_list(
            &format!("{BASE}/chart/{genre_id}/tracks"),
            &[("limit", "100")],
        )
        .await
    }

    pub async fn playlist_tracks(&self, playlist_id: i64) -> Result<Vec<Track>> {
        self.get_list(
            &format!("{BASE}/playlist/{playlist_id}/tracks"),
            &[("limit", "300")],
        )
        .await
    }

    /// Look up a single track by id (its `preview` URL is freshly signed).
    /// Used by Challenge mode to fetch the exact song a party is playing.
    pub async fn track(&self, id: i64) -> Result<Track> {
        let track: Track = self
            .http
            .get(format!("{BASE}/track/{id}"))
            .send()
            .await
            .context("requesting track")?
            .error_for_status()
            .context("bad status for track")?
            .json()
            .await
            .context("decoding track JSON")?;
        Ok(track)
    }

    /// Download the raw bytes of a preview MP3.
    pub async fn download_preview(&self, url: &str) -> Result<Vec<u8>> {
        let bytes = self
            .http
            .get(url)
            .send()
            .await
            .context("requesting preview")?
            .error_for_status()
            .context("bad status downloading preview")?
            .bytes()
            .await
            .context("reading preview bytes")?;
        Ok(bytes.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A trimmed real Deezer /search response — verifies our structs match the
    // wire format without hitting the network.
    const SEARCH_FIXTURE: &str = r#"{
        "data": [
            {
                "id": 4091937401,
                "title": "Bohemian Rhapsody",
                "preview": "https://cdnt-preview.dzcdn.net/x.mp3",
                "artist": { "id": 412, "name": "Queen" },
                "album": { "id": 1007321681, "title": "A Night At The Opera" }
            },
            {
                "id": 7,
                "title": "No Preview Song",
                "preview": "",
                "artist": { "id": 9, "name": "Nobody" }
            }
        ],
        "total": 2
    }"#;

    #[test]
    fn parses_search_response() {
        let parsed: DataResponse<Track> = serde_json::from_str(SEARCH_FIXTURE).unwrap();
        assert_eq!(parsed.data.len(), 2);

        let first = &parsed.data[0];
        assert_eq!(first.id, 4091937401);
        assert_eq!(first.title, "Bohemian Rhapsody");
        assert_eq!(first.artist_name(), "Queen");
        assert_eq!(first.album_title(), Some("A Night At The Opera"));
        assert!(first.has_preview());
        assert_eq!(first.display(), "Bohemian Rhapsody — Queen");

        // Missing album + empty preview must degrade gracefully.
        let second = &parsed.data[1];
        assert!(!second.has_preview());
        assert_eq!(second.album_title(), None);
    }
}
