#!/usr/bin/env bash
# Point the Flatpak manifest at a release. With no argument it follows
# whatever GitHub currently calls latest.
#
# Flathub runs flatpak-external-data-checker against the manifest's
# x-checker-data block and opens this same bump as a pull request by itself
# once the app is published there. This script is what does the job before
# that, and when a release needs to go out faster than the checker's cycle.
set -euo pipefail

cd "$(dirname "$0")"

# GNU sed only. Below, `sed -i -e` edits in place and `0,/re/` addresses the
# first match, both GNU behaviour. BSD sed (macOS) reads `-i -e` as "keep a
# backup with the suffix -e", which is how `io.extremelabs.omacal.yml-e` once
# got committed, and has no `0,/re/` at all. Refuse rather than half-work.
if ! sed --version >/dev/null 2>&1; then
  echo "bump.sh needs GNU sed (on macOS: brew install gnu-sed, then put gsed's gnubin first on PATH)" >&2
  exit 1
fi

repo=x3me/omacal
manifest=io.extremelabs.omacal.yml
metainfo=io.extremelabs.omacal.metainfo.xml

version=${1:-}
if [ -z "$version" ]; then
  version=$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest" \
    | sed -n 's/.*"tag_name": *"v\{0,1\}\([^"]*\)".*/\1/p' | head -1)
fi
[ -n "$version" ] || { echo "could not determine a version" >&2; exit 1; }

url="https://github.com/$repo/releases/download/v$version/omacal_${version}_amd64.deb"
echo "fetching $url"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fL --progress-bar "$url" -o "$tmp/omacal.deb"
sha=$(sha256sum "$tmp/omacal.deb" | cut -d' ' -f1)
date=$(curl -fsSL "https://api.github.com/repos/$repo/releases/tags/v$version" \
  | sed -n 's/.*"published_at": *"\([0-9-]*\)T.*/\1/p' | head -1)

# The manifest carries several checksums now — the Ayatana tray stack pins
# its own sources — so the .deb's is addressed by adjacency, not pattern:
# the sha256 rewritten is the line directly under the one release URL. The
# guard checks that URL appears exactly once, so a second .deb appearing in
# the manifest fails loudly instead of half-updating.
count=$(grep -c "^ *url: https://github.com/$repo/releases/download/.*\.deb$" "$manifest")
[ "$count" = 1 ] || { echo "expected 1 omacal .deb url in $manifest, found $count" >&2; exit 1; }

sed -i \
  -e "s|^\( *url: \)https://github.com/$repo/releases/download/.*\.deb$|\1$url|" \
  "$manifest"
sed -i \
  -e "\|^ *url: https://github.com/$repo/releases/download/.*[.]deb\$|{n;s|^\( *sha256: \)[0-9a-f]\{64\}\$|\1$sha|}" \
  "$manifest"

# One new entry at the top of the list, not a rewrite: the metainfo keeps
# every release, and the substitution that used to live here turned all of
# them into the new version (bit v0.20.0, 2026-09-02). Idempotent, so a
# rerun for a version already listed changes nothing.
entry="<release version=\"$version\" date=\"${date:-$(date -u +%F)}\"/>"
if grep -q "<release version=\"$version\"" "$metainfo"; then
  echo "metainfo already lists $version"
else
  sed -i -e "0,/^\( *\)<release /s||\1$entry\n\1<release |" "$metainfo"
fi

echo "manifest now at $version"
grep -n "url: https://github.com/$repo/releases\|sha256:" "$manifest"
grep -n "<release " "$metainfo"
