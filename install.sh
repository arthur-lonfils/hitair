#!/bin/sh
# hitair installer for Linux and macOS.
#   curl -fsSL https://raw.githubusercontent.com/arthur-lonfils/hitair/main/install.sh | sh
#
# Installs both the desktop GUI (`hitair-gui`) and the terminal app (`hitair`).
# Override the install directory with HITAIR_INSTALL_DIR=/path sh install.sh
set -eu

REPO="arthur-lonfils/hitair"
INSTALL_DIR="${HITAIR_INSTALL_DIR:-$HOME/.local/bin}"

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

# The terminal app must be present; the GUI is best-effort per platform.
install_bin hitair || err "could not download hitair for ${plat}-${cpu}"
gui=0
install_bin hitair-gui && gui=1 || say "note: no desktop build for ${plat}-${cpu} — installed the terminal app only"

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

if [ "$gui" = 1 ]; then
  say "Done. Run: hitair-gui (desktop) — or hitair (terminal)"
else
  say "Done. Run: hitair"
fi
