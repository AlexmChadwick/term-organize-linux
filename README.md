# Term Organize for Linux

Unofficial Northeastern course search, schedule builder, and degree-plan checker for GNOME.

**Not affiliated with, endorsed by, or sponsored by Northeastern University or Sandbox.**

Course information is © Northeastern University. This program is free software under the GNU Affero General Public License v3.0 only. It is an independent Linux adaptation of [Term Organize](https://github.com/AlexmChadwick/Northeastern-Term-Organize), which is itself based on [SearchNEU](https://github.com/sandboxnu/searchneu) (AGPL-3.0). See `LICENSE` and `NOTICE.md`.

## Install

Term Organize is an early beta. It is unofficial and not affiliated with Northeastern University.

- **Download:** when available, the [Releases page](https://github.com/AlexmChadwick/term-organize-linux/releases) has a signed Flatpak and AppImage. See "Downloads" below to verify and install them.
- **Build from source:** install Rust (cargo) and the GTK 4 and libadwaita development packages, then follow "Build" below.

## What it does

One window, with a sidebar of Search, Subjects, Saved, Professors, Free Rooms, Schedules, Degree Plans, and Watchlist, and the term in the header. The bundled Fall 2026 and Spring 2027 snapshots (`data/catalog/term-202710.catalog` and `data/catalog/term-202730.catalog`) are enough to search, build schedules, and read sections offline. A fresh launch opens the newest bundled term; the header lists every bundled term. It keeps instructor addresses on `northeastern.edu` and `neu.edu` (and their subdomains) and omits any other address from the Banner field. Refresh seats asks Banner for current counts. Load requirements reads a catalog.northeastern.edu program page.

## Build

Rust stable, GTK 4.14, and libadwaita 1.5.

```bash
sudo apt install libgtk-4-dev libadwaita-1-dev poppler-utils pkg-config
cargo run --release
```

`pdftotext` (from poppler-utils) imports a degree-audit “View Plan” PDF. Desktop notifications use `notify-send`.

Set `TERMORGANIZE_DATA` to a directory whose `catalog/` folder contains the bundled `term-*.catalog` files if you are not running from this repository. When that folder has snapshots, they are the terms the app ships. User data is JSON under `$XDG_DATA_HOME/termorganize` (or `~/.local/share/termorganize`). The Flatpak build stores that same folder at `~/.var/app/org.termorganize.TermOrganize/data/termorganize`. The app and `term-organize-mcp` share this file.

## Commands

```text
termorganize                      open the window
termorganize file.pdf             import a degree-plan PDF
termorganize 'termorganize://search?q=orgo'
termorganize --search "fundies"
termorganize --search-provider    GNOME Shell search provider
termorganize --seat-watch         one seat-alert pass
termorganize --smoke              open at 1440×900, require the page to fill the window, then quit
```

Keyboard shortcuts follow the Mac app with Ctrl in place of Command: Ctrl+N new schedule, Shift+Ctrl+N new degree plan, Ctrl+O import, Ctrl+E export, Ctrl+P print, Ctrl+R reload, Ctrl+1 through Ctrl+8 switch areas, Ctrl+, settings.

## Install the desktop pieces

```bash
install -Dm755 target/release/termorganize ~/.local/bin/termorganize
install -Dm644 packaging/org.termorganize.TermOrganize.desktop ~/.local/share/applications/org.termorganize.TermOrganize.desktop
install -Dm644 packaging/org.termorganize.TermOrganize.metainfo.xml ~/.local/share/metainfo/org.termorganize.TermOrganize.metainfo.xml
install -Dm644 packaging/org.termorganize.TermOrganize.svg ~/.local/share/icons/hicolor/scalable/apps/org.termorganize.TermOrganize.svg
install -Dm644 packaging/org.termorganize.TermOrganize.search-provider.ini ~/.local/share/gnome-shell/search-providers/org.termorganize.TermOrganize.search-provider.ini
install -Dm644 packaging/org.termorganize.TermOrganize.SearchProvider.service ~/.local/share/dbus-1/services/org.termorganize.TermOrganize.SearchProvider.service
mkdir -p ~/.config/systemd/user
sed 's|/usr/bin/termorganize|'"$HOME"'/.local/bin/termorganize|' packaging/termorganize-seat-watch.service > ~/.config/systemd/user/termorganize-seat-watch.service
cp packaging/termorganize-seat-watch.timer ~/.config/systemd/user/
systemctl --user enable --now termorganize-seat-watch.timer
update-desktop-database ~/.local/share/applications || true
```

The `.desktop` file registers `termorganize://`, `neuclasssearch://`, and PDF “Open with”. While the app is open it also asks the portal Background API for permission to autostart `termorganize --seat-watch`. The systemd timer is the reliable 15-minute check when the portal request is denied.

A Flatpak manifest is `packaging/org.termorganize.TermOrganize.yml`. `packaging/build-release.sh` vendors crates and builds the release Flatpak and AppImage.

## Downloads

Pushing a tag that matches `v*` (for example `v1.1.0`) runs `.github/workflows/release.yml`. The workflow does not run on pull requests or branch pushes. It uploads a **draft** GitHub release, which is not published. A tag that contains `-rc` is also marked as a prerelease. The draft contains `TermOrganize-<tag>-x86_64.flatpak`, `TermOrganize-<tag>-x86_64.AppImage`, `SHA256SUMS`, `SHA256SUMS.asc`, and the public key `term-organize-release-key.asc`.

Releases are signed by **Term Organize Releases (unofficial)**, fingerprint `5276 3E58 9A37 573E 2C75 5AE2 A120 52B3 ED28 4C89`.

### Verify

```bash
gpg --import term-organize-release-key.asc
gpg --fingerprint 52763E589A37573E2C755AE2A12052B3ED284C89
gpg --verify SHA256SUMS.asc SHA256SUMS
sha256sum -c SHA256SUMS
```

Trust the key only when that fingerprint matches.

### Install

Flatpak, using the GNOME 47 runtime:

```bash
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak install --user ./TermOrganize-v1.1.0-x86_64.flatpak
flatpak run org.termorganize.TermOrganize
```

AppImage. GTK 4 and libadwaita are bundled. Degree-plan PDF import still needs `pdftotext` on the host.

```bash
chmod +x TermOrganize-v1.1.0-x86_64.AppImage
./TermOrganize-v1.1.0-x86_64.AppImage
```

## Local MCP server

`term-organize-mcp` speaks MCP over stdio (protocol `2025-06-18`) for the bundled Fall 2026 and Spring 2027 catalogs and for the degree plans in the local user file. It does not use the network. Tool names, parameters, and JSON shapes are documented in `docs/MCP.md`.

```bash
cargo run -p term-organize-mcp
```

Claude Desktop (`claude_desktop_config.json`) and Cursor (`.cursor/mcp.json`) can both launch it:

```json
{
  "mcpServers": {
    "term-organize": {
      "command": "/path/to/term-organize-mcp"
    }
  }
}
```

`--read-only` or `TERM_ORGANIZE_MCP_READONLY=1` hides the plan write tools. `--data-dir` or `TERM_ORGANIZE_MCP_DATA` picks the user-file directory when you do not want the shared Flatpak or XDG location.

## Layout of the code

`crates/core` is the catalog, search, schedule generator, degree audit, plan file, and file formats, with `cargo test`. `crates/app` is the GTK 4 / libadwaita window. `crates/mcp` is the local MCP server. `PARITY.md` maps each Swift area to the Rust file and says what is still missing.
