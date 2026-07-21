#!/bin/sh
# hitair installer for Linux and macOS.
#   curl -fsSL https://raw.githubusercontent.com/arthur-lonfils/hitair/main/install.sh | sh
#
# Installs the desktop app (`hitair-gui`). Override the install directory with
# HITAIR_INSTALL_DIR=/path sh install.sh
set -eu

REPO="arthur-lonfils/hitair"
INSTALL_DIR="${HITAIR_INSTALL_DIR:-$HOME/.local/bin}"
# macOS ships the GUI as a real .app bundle; install it here (per-user, no sudo).
APP_DIR="${HITAIR_APP_DIR:-$HOME/Applications}"

say() { printf '%s\n' "$*"; }
err() { printf 'error: %s\n' "$*" >&2; exit 1; }

fetch() { # <url> <out> → 0 on success
  if command -v curl >/dev/null 2>&1; then curl -fSL "$1" -o "$2" 2>/dev/null
  elif command -v wget >/dev/null 2>&1; then wget -qO "$2" "$1" 2>/dev/null
  else err "need curl or wget to download"; fi
}

os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Linux)  plat="linux" ;;
  Darwin) plat="macos" ;;
  *) err "unsupported OS: $os — on Windows use install.ps1" ;;
esac
case "$arch" in
  x86_64 | amd64) cpu="x86_64" ;;
  aarch64 | arm64) [ "$plat" = macos ] && cpu="arm64" || cpu="aarch64" ;;
  *) err "unsupported architecture: $arch" ;;
esac

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$INSTALL_DIR"
base="https://github.com/${REPO}/releases/latest/download"

# Install one binary from its release archive. Returns non-zero if unavailable.
install_bin() { # <binary>
  bin="$1"
  asset="${bin}-${plat}-${cpu}.tar.gz"
  say "Downloading ${asset}…"
  fetch "${base}/${asset}" "$tmp/$asset" || return 1
  tar -xzf "$tmp/$asset" -C "$tmp"
  mv "$tmp/$bin" "$INSTALL_DIR/$bin"
  chmod +x "$INSTALL_DIR/$bin"
  say "Installed $bin to $INSTALL_DIR"
}

# macOS: install the GUI as a proper .app bundle in ~/Applications (Finder- and
# Spotlight-launchable, no Terminal), plus a CLI shim on PATH.
install_macos_app() {
  asset="hitair-gui-macos-${cpu}.tar.gz"
  say "Downloading ${asset}…"
  fetch "${base}/${asset}" "$tmp/$asset" || return 1
  tar -xzf "$tmp/$asset" -C "$tmp" || return 1
  [ -d "$tmp/hitair-gui.app" ] || return 1
  mkdir -p "$APP_DIR"
  rm -rf "$APP_DIR/hitair-gui.app"
  mv "$tmp/hitair-gui.app" "$APP_DIR/hitair-gui.app"
  # Best-effort: clear any quarantine so the unsigned app opens without a prompt.
  xattr -dr com.apple.quarantine "$APP_DIR/hitair-gui.app" 2>/dev/null || true
  ln -sf "$APP_DIR/hitair-gui.app/Contents/MacOS/hitair-gui" "$INSTALL_DIR/hitair-gui"
  say "Installed hitair-gui.app to $APP_DIR"
}

# Install the desktop app (macOS as a .app bundle; Linux as a binary on PATH).
if [ "$plat" = macos ]; then
  install_macos_app || err "could not download hitair-gui for macos-${cpu}"
else
  install_bin hitair-gui || err "could not download hitair-gui for ${plat}-${cpu}"
fi

# Audio needs the ALSA runtime library on Linux.
if [ "$plat" = linux ] && command -v ldconfig >/dev/null 2>&1; then
  if ! ldconfig -p 2>/dev/null | grep -q 'libasound\.so\.2'; then
    say "note: audio needs libasound2 — e.g. 'sudo apt install libasound2'"
  fi
fi

# Remind about PATH if needed.
case ":$PATH:" in
  *":$INSTALL_DIR:"*) : ;;
  *) say "note: add it to PATH — echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.profile" ;;
esac

if [ "$plat" = macos ]; then
  say "Done. Open hitair from Launchpad/Spotlight — or run hitair-gui in a terminal."
else
  say "Done. Run: hitair-gui  (it adds an app-menu launcher on first start)"
fi
