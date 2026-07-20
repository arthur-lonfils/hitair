//! Supabase-backed online "Challenge" parties. Entirely opt-in — Solo play never
//! touches this module.
//!
//! Talks to PostgREST over HTTPS with the project's *publishable* key; access is
//! governed by Row-Level Security (see `supabase/schema.sql`), so the key is safe
//! to embed in the client.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub(crate) const SUPABASE_URL: &str = "https://wcaduezxyxawehfsxcci.supabase.co";
pub(crate) const SUPABASE_KEY: &str = "sb_publishable_1QaBMI1l76j-5ccK1jxVcQ_XEqjqMhk";

/// Unambiguous alphabet for party codes (no 0/O/1/I/L).
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
const CODE_LEN: usize = 6;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Party {
    pub code: String,
    pub visibility: String, // "public" | "private"
    pub max_players: i32,
    pub track_id: i64,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub album: Option<String>,
    pub schedule: Vec<i32>, // clip lengths in ms
    pub host_name: String,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// Fields a host supplies to open a party (the code is allocated for them).
#[derive(Debug, Clone, Serialize)]
pub struct NewParty {
    pub code: String,
    pub visibility: String,
    pub max_players: i32,
    pub track_id: i64,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub schedule: Vec<i32>,
    pub host_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Score {
    pub party_code: String,
    pub player_name: String,
    pub solved: bool,
    pub clips_used: i32,
    pub time_ms: i32,
    pub mistakes: i32,
    // Omit on insert so Postgres fills the `default now()`; read back on select.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

#[derive(Clone)]
pub struct SupaClient {
    http: reqwest::Client,
}

impl SupaClient {
    pub fn new() -> Result<Self> {
        use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
        let mut headers = HeaderMap::new();
        headers.insert("apikey", HeaderValue::from_static(SUPABASE_KEY));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {SUPABASE_KEY}"))?,
        );
        let http = reqwest::Client::builder()
            .user_agent(concat!("hitair/", env!("CARGO_PKG_VERSION")))
            .default_headers(headers)
            .build()
            .context("building Supabase client")?;
        Ok(Self { http })
    }

    fn table(&self, name: &str) -> String {
        format!("{SUPABASE_URL}/rest/v1/{name}")
    }

    /// Insert a party. Returns `Ok(None)` if the code was already taken.
    async fn try_create_party(&self, party: &NewParty) -> Result<Option<Party>> {
        let resp = self
            .http
            .post(self.table("parties"))
            .header("Prefer", "return=representation")
            .json(party)
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::CONFLICT {
            return Ok(None);
        }
        let rows: Vec<Party> = resp
            .error_for_status()
            .context("creating party")?
            .json()
            .await?;
        Ok(rows.into_iter().next())
    }

    /// Allocate a unique code and create the party.
    pub async fn create_party(&self, mut party: NewParty) -> Result<Party> {
        for _ in 0..6 {
            party.code = generate_code();
            if let Some(created) = self.try_create_party(&party).await? {
                return Ok(created);
            }
        }
        bail!("couldn't allocate a free party code")
    }

    pub async fn get_party(&self, code: &str) -> Result<Option<Party>> {
        let rows: Vec<Party> = self
            .http
            .get(self.table("parties"))
            .query(&[
                ("code", format!("eq.{}", code.to_uppercase())),
                ("limit", "1".into()),
            ])
            .send()
            .await?
            .error_for_status()
            .context("looking up party")?
            .json()
            .await?;
        Ok(rows.into_iter().next())
    }

    pub async fn list_public_parties(&self, limit: u32) -> Result<Vec<Party>> {
        let rows: Vec<Party> = self
            .http
            .get(self.table("parties"))
            .query(&[
                ("visibility", "eq.public".to_string()),
                ("order", "created_at.desc".into()),
                ("limit", limit.to_string()),
            ])
            .send()
            .await?
            .error_for_status()
            .context("listing public parties")?
            .json()
            .await?;
        Ok(rows)
    }

    pub async fn submit_score(&self, score: &Score) -> Result<()> {
        self.http
            .post(self.table("scores"))
            .json(score)
            .send()
            .await?
            .error_for_status()
            .context("submitting score")?;
        Ok(())
    }

    /// Leaderboard for a party: solved first, then fewest clips, then fastest.
    pub async fn leaderboard(&self, code: &str, limit: u32) -> Result<Vec<Score>> {
        let rows: Vec<Score> = self
            .http
            .get(self.table("scores"))
            .query(&[
                ("party_code", format!("eq.{}", code.to_uppercase())),
                ("order", "solved.desc,clips_used.asc,time_ms.asc".into()),
                ("limit", limit.to_string()),
            ])
            .send()
            .await?
            .error_for_status()
            .context("loading leaderboard")?
            .json()
            .await?;
        Ok(rows)
    }

    /// Number of results submitted to a party (used to enforce `max_players`).
    pub async fn player_count(&self, code: &str) -> Result<usize> {
        let rows: Vec<serde_json::Value> = self
            .http
            .get(self.table("scores"))
            .query(&[
                ("party_code", format!("eq.{}", code.to_uppercase())),
                ("select", "id".into()),
            ])
            .send()
            .await?
            .error_for_status()
            .context("counting players")?
            .json()
            .await?;
        Ok(rows.len())
    }
}

/// A random, unambiguous party code like `7Q2F9K`.
pub fn generate_code() -> String {
    (0..CODE_LEN)
        .map(|_| {
            let idx = (rand::random::<u32>() as usize) % CODE_ALPHABET.len();
            CODE_ALPHABET[idx] as char
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{CODE_ALPHABET, CODE_LEN, generate_code};

    #[test]
    fn codes_are_well_formed() {
        for _ in 0..50 {
            let code = generate_code();
            assert_eq!(code.len(), CODE_LEN);
            assert!(code.bytes().all(|b| CODE_ALPHABET.contains(&b)));
        }
    }
}
