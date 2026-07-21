//! Linux desktop integration: install/remove an XDG `.desktop` launcher + icon so
//! the desktop app shows up in the application menu and is searchable. A no-op on
//! macOS/Windows (they use app bundles / installers instead).

/// Whether desktop-launcher integration applies on this platform.
pub const SUPPORTED: bool = cfg!(target_os = "linux");

#[cfg(target_os = "linux")]
mod imp {
    use anyhow::{Context, Result};
    use std::path::{Path, PathBuf};

    const APP_ID: &str = "hitair";

    fn desktop_path() -> Option<PathBuf> {
        Some(
            dirs::data_dir()?
                .join("applications")
                .join("hitair.desktop"),
        )
    }

    fn icon_path() -> Option<PathBuf> {
        Some(dirs::data_dir()?.join("icons/hicolor/256x256/apps/hitair.png"))
    }

    pub fn is_installed() -> bool {
        desktop_path().is_some_and(|p| p.exists())
    }

    /// Write the launcher + icon, pointing at `exec` (the running GUI binary).
    pub fn install(exec: &Path, icon_png: &[u8]) -> Result<()> {
        let icon = icon_path().context("no data dir")?;
        if let Some(parent) = icon.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&icon, icon_png)?;

        let desktop = desktop_path().context("no data dir")?;
        if let Some(parent) = desktop.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Quote Exec so a path with spaces still launches.
        let entry = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Version=1.0\n\
             Name=hitair\n\
             GenericName=Music guessing game\n\
             Comment=Guess the song from a growing preview snippet\n\
             Exec=\"{exec}\"\n\
             Icon={APP_ID}\n\
             Terminal=false\n\
             Categories=Game;\n\
             Keywords=music;song;guess;quiz;\n\
             StartupWMClass={APP_ID}\n",
            exec = exec.display(),
        );
        std::fs::write(&desktop, entry)?;

        // Best-effort: nudge the menu to pick it up now (ignored if absent).
        if let Some(dir) = desktop.parent() {
            let _ = std::process::Command::new("update-desktop-database")
                .arg(dir)
                .status();
        }
        Ok(())
    }

    pub fn remove() -> Result<()> {
        if let Some(p) = desktop_path() {
            let _ = std::fs::remove_file(p);
        }
        if let Some(p) = icon_path() {
            let _ = std::fs::remove_file(p);
        }
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use anyhow::Result;
    use std::path::Path;

    pub fn is_installed() -> bool {
        false
    }
    pub fn install(_exec: &Path, _icon_png: &[u8]) -> Result<()> {
        Ok(())
    }
    pub fn remove() -> Result<()> {
        Ok(())
    }
}

pub use imp::{install, is_installed, remove};
