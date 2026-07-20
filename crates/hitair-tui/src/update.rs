//! Self-update and uninstall for an installed `hitair` binary.
//!
//! Reuses the existing `reqwest` (rustls) stack: it checks the latest GitHub
//! release, downloads the asset for this platform, extracts the binary, and
//! swaps it in via the `self-replace` crate (which handles the running-executable
//! dance on Windows too). On update it also drops the desktop app (`hitair-gui`)
//! next to itself, best-effort, so a terminal-only install picks up the GUI.

use std::path::Path;

use anyhow::{Context, Result};

const REPO: &str = "arthur-lonfils/hitair";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub enum Outcome {
    /// Already on the latest terminal app, and the desktop app is present.
    UpToDate,
    /// Updated the terminal app. `gui` is true if the desktop app (`hitair-gui`)
    /// was (re)installed alongside it to match.
    Updated { version: String, gui: bool },
    /// The terminal app was already current; the missing desktop app was added.
    GuiInstalled,
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

/// Update the terminal app to the latest release and keep the desktop app in
/// sync beside it: refreshed when the TUI updates, installed if it's missing.
pub async fn perform_update() -> Result<Outcome> {
    let slug = asset_slug().context("no prebuilt binary for this platform")?;
    let client = http()?;
    let latest = fetch_latest_version(&client).await?;
    let updated = is_newer(CURRENT_VERSION, &latest);

    if updated {
        let (asset, bin_in_archive) = if cfg!(windows) {
            (format!("hitair-{slug}.zip"), "hitair.exe")
        } else {
            (format!("hitair-{slug}.tar.gz"), "hitair")
        };
        let url = format!("https://github.com/{REPO}/releases/latest/download/{asset}");
        let bytes = client
            .get(url)
            .send()
            .await?
            .error_for_status()
            .context("downloading release asset")?
            .bytes()
            .await?;
        let binary = extract(&bytes, bin_in_archive)?;
        replace_running_binary(&binary)?;
    }

    // Keep the desktop app alongside us. On an update, refresh it to match; when
    // already current, add it only if missing. Best-effort — never fails.
    let gui = ensure_gui_sibling(&client, slug, updated)
        .await
        .unwrap_or(false);

    Ok(match (updated, gui) {
        (true, gui) => Outcome::Updated {
            version: latest,
            gui,
        },
        (false, true) => Outcome::GuiInstalled,
        (false, false) => Outcome::UpToDate,
    })
}

/// Ensure `hitair-gui` sits next to the running binary. With `force`, always
/// (re)downloads it; otherwise only when it's absent. Returns whether it wrote
/// the file. `Ok(false)` if this release/platform has no desktop build.
async fn ensure_gui_sibling(client: &reqwest::Client, slug: &str, force: bool) -> Result<bool> {
    let (asset, bin_in_archive, out_name) = if cfg!(windows) {
        (
            format!("hitair-gui-{slug}.zip"),
            "hitair-gui.exe",
            "hitair-gui.exe",
        )
    } else {
        (
            format!("hitair-gui-{slug}.tar.gz"),
            "hitair-gui",
            "hitair-gui",
        )
    };
    let exe = std::env::current_exe()?;
    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    let dest = dir.join(out_name);
    if !force && dest.exists() {
        return Ok(false); // already installed and TUI unchanged
    }

    let url = format!("https://github.com/{REPO}/releases/latest/download/{asset}");
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Ok(false); // no GUI build for this platform/release
    }
    let bytes = resp.bytes().await?;
    let binary = extract(&bytes, bin_in_archive)?;
    std::fs::write(&dest, &binary).with_context(|| format!("writing {}", dest.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(true)
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
