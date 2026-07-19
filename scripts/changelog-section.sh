#!/bin/sh
# Print the CHANGELOG.md notes for a version (X.Y.Z), for use as release notes.
# Stops at the next version header or the link-reference block.
#
#   scripts/changelog-section.sh 0.4.0
set -eu

[ $# -eq 1 ] || { echo "usage: $0 X.Y.Z" >&2; exit 1; }
version="$1"
cd "$(dirname "$0")/.."

awk -v v="$version" '
  $0 ~ "^## \\[" v "\\]" { grab = 1; next }        # start after this versions header
  grab && (/^## \[/ || /^\[[^]]+\]:[[:space:]]/) { exit }  # stop at next version or links
  grab { print }
' CHANGELOG.md | sed '/./,$!d'   # drop leading blank lines
