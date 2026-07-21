//! First-run "set hitair up as a real app" actions, one path per OS.
//!
//! Best-effort and self-contained — the app runs fine without any of it. The
//! desktop wizard (the `Setup` screen) calls [`run`] when the user opts in.

use std::path::Path;

use anyhow::Result;

/// Set hitair up as a launchable desktop app for the current OS, returning a
/// short human summary of what happened.
pub fn run(exe: &Path, icon_png: &[u8]) -> Result<String> {
    platform(exe, icon_png)
}

#[cfg(target_os = "linux")]
fn platform(exe: &Path, icon_png: &[u8]) -> Result<String> {
    use std::os::unix::fs::PermissionsExt;

    // Keep a stable copy on PATH so a launcher pointing at ~/Downloads doesn't
    // break when the download is moved or cleared.
    let bin_dir = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("no home directory"))?
        .join(".local/bin");
    let installed = bin_dir.join("hitair-gui");
    let mut extra = String::new();
    let target = if exe == installed {
        exe.to_path_buf()
    } else {
        std::fs::create_dir_all(&bin_dir)?;
        std::fs::copy(exe, &installed)?;
        std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o755))?;
        extra = format!(" (installed to {})", bin_dir.display());
        installed
    };
    crate::desktop::install(&target, icon_png)?;
    Ok(format!(
        "Added hitair to your applications menu{extra} — search for it like any app."
    ))
}

#[cfg(target_os = "windows")]
fn platform(exe: &Path, _icon_png: &[u8]) -> Result<String> {
    use std::process::Command;
    let appdata = std::env::var_os("APPDATA").ok_or_else(|| anyhow::anyhow!("no %APPDATA%"))?;
    let lnk = Path::new(&appdata).join(r"Microsoft\Windows\Start Menu\Programs\hitair.lnk");
    // Create a Start Menu shortcut via the WScript.Shell COM object.
    let script = format!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');\
         $s.TargetPath='{}';$s.WorkingDirectory='{}';$s.Save()",
        lnk.display(),
        exe.display(),
        exe.parent().unwrap_or(Path::new(".")).display(),
    );
    let status = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()?;
    anyhow::ensure!(status.success(), "creating the Start Menu shortcut failed");
    Ok("Added hitair to your Start Menu — search for it in the Start menu.".into())
}

#[cfg(target_os = "macos")]
fn platform(exe: &Path, _icon_png: &[u8]) -> Result<String> {
    // Already running from inside a bundle → nothing to do.
    if exe
        .components()
        .any(|c| c.as_os_str().to_string_lossy().ends_with(".app"))
    {
        return Ok("hitair is already set up as an app.".into());
    }
    let app = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("no home directory"))?
        .join("Applications/hitair-gui.app");
    let macos = app.join("Contents/MacOS");
    std::fs::create_dir_all(&macos)?;
    std::fs::copy(exe, macos.join("hitair-gui"))?;
    std::fs::write(
        app.join("Contents/Info.plist"),
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" ",
            "\"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
            "<plist version=\"1.0\"><dict>\n",
            "  <key>CFBundleName</key><string>hitair</string>\n",
            "  <key>CFBundleIdentifier</key><string>be.londer.hitair</string>\n",
            "  <key>CFBundleExecutable</key><string>hitair-gui</string>\n",
            "  <key>CFBundlePackageType</key><string>APPL</string>\n",
            "  <key>NSHighResolutionCapable</key><true/>\n",
            "</dict></plist>\n",
        ),
    )?;
    Ok(format!("Installed hitair to {}.", app.display()))
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn platform(_exe: &Path, _icon_png: &[u8]) -> Result<String> {
    Ok("hitair is ready to play.".into())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    // Touches $HOME + the filesystem, so it's opt-in: `cargo test -- --ignored`.
    #[test]
    #[ignore = "mutates $HOME; run explicitly"]
    fn linux_setup_installs_binary_and_launcher() {
        let home = std::env::temp_dir().join(format!("hitair-setup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        // SAFETY: single-threaded, isolated ignored test.
        unsafe { std::env::set_var("HOME", &home) };

        let download = home.join("Downloads").join("hitair-gui");
        std::fs::create_dir_all(download.parent().unwrap()).unwrap();
        std::fs::write(&download, b"binary").unwrap();

        let msg = super::run(&download, b"\x89PNG-fake").unwrap();

        assert!(home.join(".local/bin/hitair-gui").exists(), "binary copied");
        assert!(
            home.join(".local/share/applications/hitair.desktop")
                .exists(),
            "launcher written"
        );
        assert!(msg.contains("applications menu"), "summary: {msg}");
        std::fs::remove_dir_all(&home).ok();
    }
}
