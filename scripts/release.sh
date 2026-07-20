#!/bin/sh
# Cut a release in one command: bump the version, commit, tag, and push.
# The `release` GitHub Actions workflow then builds and publishes the binaries.
#
#   scripts/release.sh 0.1.1
set -eu

[ $# -eq 1 ] || { echo "usage: $0 X.Y.Z" >&2; exit 1; }
version="$1"
echo "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' \
  || { echo "error: version must be X.Y.Z (semver)" >&2; exit 1; }

cd "$(dirname "$0")/.."

[ -z "$(git status --porcelain)" ] || { echo "error: working tree not clean" >&2; exit 1; }
branch="$(git rev-parse --abbrev-ref HEAD)"
[ "$branch" = main ] || { echo "error: not on main (on '$branch')" >&2; exit 1; }

# Bump the workspace version (first `version = ` under [workspace.package]); each
# crate inherits it via `version.workspace = true`.
awk -v v="$version" '
  /^\[/ { pkg = ($0 == "[workspace.package]") }
  pkg && /^version = / && !done { print "version = \"" v "\""; done = 1; next }
  { print }
' Cargo.toml > Cargo.toml.tmp && mv Cargo.toml.tmp Cargo.toml

cargo build --quiet   # refresh Cargo.lock with the new version

# Roll CHANGELOG.md's [Unreleased] notes into a dated version section.
if [ -f CHANGELOG.md ]; then
  today="$(date -u +%Y-%m-%d)"
  awk -v v="$version" -v d="$today" '
    !done && /^## \[Unreleased\]/ {
      print "## [Unreleased]"; print ""; print "## [" v "] - " d
      done = 1; next
    }
    { print }
  ' CHANGELOG.md > CHANGELOG.tmp && mv CHANGELOG.tmp CHANGELOG.md
fi

git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "Release v$version"
git tag -a "v$version" -m "hitair v$version"
git push origin main
git push origin "v$version"

echo "Pushed v$version. Watch the build:"
echo "  https://github.com/arthur-lonfils/hitair/actions"
