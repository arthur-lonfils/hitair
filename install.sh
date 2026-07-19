#!/bin/sh
# hitair installer for Linux and macOS.
#   curl -fsSL https://raw.githubusercontent.com/arthur-lonfils/hitair/main/install.sh | sh
#
# Override the install directory with HITAIR_INSTALL_DIR=/path sh install.sh
set -eu

REPO="arthur-lonfils/hitair"
BIN="hitair"
INSTALL_DIR="${HITAIR_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '%s\n' "$*"; }
err() { printf 'error: %s\n' "$*" >&2; exit 1; }

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

asset="${BIN}-${plat}-${cpu}.tar.gz"
url="https://github.com/${REPO}/releases/latest/download/${asset}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

say "Downloading ${asset}…"
if command -v curl >/dev/null 2>&1; then
  curl -fSL "$url" -o "$tmp/$asset" || err "download failed: $url"
elif command -v wget >/dev/null 2>&1; then
  wget -qO "$tmp/$asset" "$url" || err "download failed: $url"
else
  err "need curl or wget to download"
fi

tar -xzf "$tmp/$asset" -C "$tmp"
mkdir -p "$INSTALL_DIR"
mv "$tmp/$BIN" "$INSTALL_DIR/$BIN"
chmod +x "$INSTALL_DIR/$BIN"
if version="$("$INSTALL_DIR/$BIN" --version 2>/dev/null)"; then
  say "Installed $version to $INSTALL_DIR"
else
  say "Installed $BIN to $INSTALL_DIR"
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

say "Done. Run: $BIN"
