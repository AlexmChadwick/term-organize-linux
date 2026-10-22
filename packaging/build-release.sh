#!/usr/bin/env bash
# Build the release bundles: a single-file Flatpak and an x86_64 AppImage.
# Vendors crates so flatpak-builder can compile without network inside the sandbox.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

version=$(awk -F'"' '/^version = / { print $2; exit }' Cargo.toml)
if [[ -z "$version" ]]; then
  echo "could not read the workspace version from Cargo.toml" >&2
  exit 1
fi
tag=${RELEASE_TAG:-v$version}

if ! grep -q "TERMORGANIZE_VERSION: \"${version}\"" packaging/org.termorganize.TermOrganize.yml; then
  echo "manifest TERMORGANIZE_VERSION is not ${version}" >&2
  exit 1
fi
if ! grep -q "<release version=\"${version}\"" packaging/org.termorganize.TermOrganize.metainfo.xml; then
  echo "metainfo has no release ${version}" >&2
  exit 1
fi
python3 - "$version" << 'PY'
import sys
version = sys.argv[1]
text = open("Cargo.lock", encoding="utf-8").read().splitlines()
wanted = {"termorganize", "termorganize-core"}
seen = set()
i = 0
while i < len(text):
    line = text[i]
    if line.startswith("name = "):
        name = line.split('"')[1]
        if name in wanted:
            nxt = text[i + 1] if i + 1 < len(text) else ""
            if nxt != f'version = "{version}"':
                sys.exit(f"Cargo.lock {name} is not {version}")
            seen.add(name)
    i += 1
missing = wanted - seen
if missing:
    sys.exit(f"Cargo.lock is missing {sorted(missing)}")
PY

echo "Fetching crates for ${tag}"
cargo fetch --locked

src=$(mktemp -d)
repo=$(mktemp -d)
build=$(mktemp -d)
trap 'rm -rf "$src" "$repo" "$build"' EXIT

echo "Vendoring crates into a clean tree"
rsync -a \
  --exclude /target \
  --exclude /.git \
  --exclude /dist \
  --exclude /vendor \
  --exclude /.flatpak-builder \
  ./ "$src/"
test -f "$src/crates/core/vendor/lzfse/src/lzfse_decode.c"
(
  cd "$src"
  cargo vendor --locked vendor/cargo >/dev/null
  mkdir -p .cargo
  cp packaging/cargo-vendor.config.toml .cargo/config.toml
)

mkdir -p dist
flatpak_out="dist/TermOrganize-${tag}-x86_64.flatpak"
echo "Building ${flatpak_out}"
flatpak-builder --user --disable-rofiles-fuse --force-clean --delete-build-dirs \
  --repo="$repo" --arch=x86_64 \
  "$build" "$src/packaging/org.termorganize.TermOrganize.yml"
flatpak build-bundle --arch=x86_64 "$repo" "$flatpak_out" org.termorganize.TermOrganize

echo "Building the host release binary"
cargo build --release --locked --offline

app_out="dist/TermOrganize-${tag}-x86_64.AppImage"
echo "Building ${app_out}"
tools="dist/.tools"
mkdir -p "$tools"
linuxdeploy="$tools/linuxdeploy-x86_64.AppImage"
gtk_plugin="$tools/linuxdeploy-plugin-gtk.sh"
if [[ ! -x "$linuxdeploy" ]]; then
  wget -q -O "$linuxdeploy" \
    https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage
  chmod +x "$linuxdeploy"
fi
if [[ ! -x "$gtk_plugin" ]]; then
  wget -q -O "$gtk_plugin" \
    https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh
  chmod +x "$gtk_plugin"
fi

appdir="dist/AppDir"
rm -rf "$appdir"
mkdir -p \
  "$appdir/usr/bin" \
  "$appdir/usr/share/termorganize/catalog" \
  "$appdir/usr/share/icons/hicolor/scalable/apps" \
  "$appdir/usr/share/applications" \
  "$appdir/usr/share/metainfo"
install -Dm755 target/release/termorganize "$appdir/usr/bin/termorganize"
install -Dm644 data/catalog/term-*.catalog -t "$appdir/usr/share/termorganize/catalog/"
install -Dm644 data/legal/LICENSE.txt "$appdir/usr/share/termorganize/LICENSE.txt"
install -Dm644 packaging/org.termorganize.TermOrganize.metainfo.xml \
  "$appdir/usr/share/metainfo/org.termorganize.TermOrganize.metainfo.xml"
install -Dm644 packaging/org.termorganize.TermOrganize.svg \
  "$appdir/usr/share/icons/hicolor/scalable/apps/org.termorganize.TermOrganize.svg"

# linuxdeploy expects the plugin name on PATH.
export PATH="$tools:$PATH"
export ARCH=x86_64
export VERSION="$tag"
export DEPLOY_GTK_VERSION=4
export APPIMAGE_EXTRACT_AND_RUN=1
export LDAI_OUTPUT="$root/$app_out"
rm -f "$app_out" TermOrganize*.AppImage Term_Organize*.AppImage ./*.AppImage
(
  cd "$root"
  "$linuxdeploy" --appimage-extract-and-run \
    --appdir "$root/$appdir" \
    --executable "$root/$appdir/usr/bin/termorganize" \
    --desktop-file "$root/packaging/org.termorganize.TermOrganize.desktop" \
    --icon-file "$root/packaging/org.termorganize.TermOrganize.svg" \
    --plugin gtk \
    --output appimage
)
if [[ ! -f "$app_out" ]]; then
  found=$(find "$root" -maxdepth 2 -name '*.AppImage' ! -path '*/.tools/*' -print -quit)
  if [[ -z "$found" ]]; then
    echo "linuxdeploy did not write an AppImage" >&2
    exit 1
  fi
  mv "$found" "$app_out"
fi
chmod +x "$app_out"

echo "Built:"
ls -lh "$flatpak_out" "$app_out"
