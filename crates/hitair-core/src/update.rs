//! Self-update and uninstall for the desktop app (`hitair-gui`).
//!
//! Reuses the existing `reqwest` (rustls) stack: it checks the latest GitHub
//! release, downloads the asset for this platform, extracts it, and swaps it in
//! via the `self-replace` crate (which handles the running-executable dance on
//! Windows too). On macOS the binary lives inside the `.app` bundle and is
//! replaced in place, keeping the bundle intact.

use std::path::Path;

use anyhow::{Context, Result};

const REPO: &str = "arthur-lonfils/hitair";
/// The single published desktop binary name.
const BINARY: &str = "hitair-gui";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub enum Outcome {
    /// Already on the latest build.
    UpToDate,
    /// Updated the running binary to `version`.
    Updated { version: String },
}

/// Release-asset slug for this platform, matching the published file names
/// (e.g. `hitair-linux-x86_64.tar.gz`). `None` if we don't ship this platform.
pub fn asset_slug() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("macos", "aarch64") => "macos-arm64",
        ("macos", "x86_64") => "macos-x86_64",
        ("windows", "x86_64") => "windows-x86_64",
        _ => return None,
    })
}

/// Whether the itch.io app is managing this install (and therefore its updates).
///
/// The itch app injects `ITCHIO_API_KEY` when it launches a game whose
/// `.itch.toml` requests an API scope (see `.github/workflows/itch.yml`). When it
/// does, itch delta-patches updates itself, so our self-updater must stand down to
/// avoid fighting it. A copy downloaded from the itch *web* page and run directly
/// has no such variable, so it keeps the built-in GitHub updater.
pub fn is_itch_managed() -> bool {
    std::env::var_os("ITCHIO_API_KEY").is_some()
}

/// `(release asset, binary name inside the archive)` for `name`.
fn asset_for(name: &str, slug: &str) -> (String, String) {
    if cfg!(windows) {
        (format!("{name}-{slug}.zip"), format!("{name}.exe"))
    } else {
        (format!("{name}-{slug}.tar.gz"), name.into())
    }
}

fn http() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!("hitair/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("building HTTP client")
}

async fn fetch_latest_version(client: &reqwest::Client) -> Result<String> {
    #[derive(serde::Deserialize)]
    struct Rel {
        tag_name: String,
    }
    let rel: Rel = client
        .get(format!(
            "https://api.github.com/repos/{REPO}/releases/latest"
        ))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()
        .context("querying GitHub releases")?
        .json()
        .await?;
    Ok(rel.tag_name.trim_start_matches('v').to_string())
}

/// Returns the latest version if it is newer than the running build.
pub async fn latest_if_newer() -> Result<Option<String>> {
    let latest = fetch_latest_version(&http()?).await?;
    Ok(is_newer(CURRENT_VERSION, &latest).then_some(latest))
}

/// Update the running binary to the latest release.
pub async fn perform_update() -> Result<Outcome> {
    let slug = asset_slug().context("no prebuilt binary for this platform")?;
    let client = http()?;
    let latest = fetch_latest_version(&client).await?;
    if !is_newer(CURRENT_VERSION, &latest) {
        return Ok(Outcome::UpToDate);
    }
    let (asset, bin_in_archive) = asset_for(BINARY, slug);
    let url = format!("https://github.com/{REPO}/releases/latest/download/{asset}");
    let bytes = client
        .get(url)
        .send()
        .await?
        .error_for_status()
        .context("downloading release asset")?
        .bytes()
        .await?;
    let binary = extract(&bytes, &bin_in_archive)?;
    replace_running_binary(&binary)?;
    Ok(Outcome::Updated { version: latest })
}

/// Remove the running binary from disk (config under the user config dir stays).
pub fn uninstall() -> Result<std::path::PathBuf> {
    let exe = std::env::current_exe()?;
    self_replace::self_delete().context("removing the hitair binary")?;
    Ok(exe)
}

fn replace_running_binary(new_bytes: &[u8]) -> Result<()> {
    let exe = std::env::current_exe()?;
    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(".hitair-update.tmp");
    std::fs::write(&tmp, new_bytes).context("writing the new binary")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    self_replace::self_replace(&tmp).context("replacing the running binary")?;
    let _ = std::fs::remove_file(&tmp);
    Ok(())
}

fn is_newer(current: &str, other: &str) -> bool {
    semver_tuple(other) > semver_tuple(current)
}

fn semver_tuple(v: &str) -> (u64, u64, u64) {
    let mut parts = v.split('.').map(|p| p.trim().parse::<u64>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

#[cfg(not(windows))]
fn extract(bytes: &[u8], name: &str) -> Result<Vec<u8>> {
    use std::io::Read;
    let gz = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(gz);
    for entry in archive.entries().context("reading tar archive")? {
        let mut entry = entry?;
        let is_match = entry
            .path()?
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n == name)
            .unwrap_or(false);
        if is_match {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            return Ok(buf);
        }
    }
    anyhow::bail!("{name} not found in the downloaded archive")
}

#[cfg(windows)]
fn extract(bytes: &[u8], name: &str) -> Result<Vec<u8>> {
    use std::io::{Cursor, Read};
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).context("reading zip archive")?;
    let mut file = archive
        .by_name(name)
        .with_context(|| format!("{name} not in the archive"))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::{asset_slug, is_newer};

    #[test]
    fn version_comparison() {
        assert!(is_newer("0.2.1", "0.3.0"));
        assert!(is_newer("0.2.1", "0.2.2"));
        assert!(is_newer("0.9.9", "1.0.0"));
        assert!(!is_newer("0.2.1", "0.2.1"));
        assert!(!is_newer("0.3.0", "0.2.9"));
    }

    #[test]
    fn slug_is_one_we_publish() {
        if let Some(s) = asset_slug() {
            assert!(matches!(
                s,
                "linux-x86_64"
                    | "linux-aarch64"
                    | "macos-arm64"
                    | "macos-x86_64"
                    | "windows-x86_64"
            ));
        }
    }
}
