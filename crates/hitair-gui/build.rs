//! Build script: on Windows, embed the app icon into the .exe so it carries the
//! hitair icon in Explorer and when pinned to the taskbar. A no-op everywhere
//! else (the winresource dependency is only pulled in on a Windows host).

fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/icon.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        // Best-effort: a missing RC toolchain shouldn't fail the whole build.
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon embed skipped: {e}");
        }
    }
}
