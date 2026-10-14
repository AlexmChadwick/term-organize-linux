# Term Organize for Linux

Unofficial Northeastern course search, schedule builder, and degree-plan checker for GNOME.

**Not affiliated with, endorsed by, or sponsored by Northeastern University or Sandbox.**

Course information is © Northeastern University. This program is free software under the GNU Affero General Public License v3.0 only. It is an independent Linux adaptation of [Term Organize](https://github.com/AlexmChadwick/Northeastern-Term-Organize), which is itself based on [SearchNEU](https://github.com/sandboxnu/searchneu) (AGPL-3.0). See `LICENSE` and `NOTICE.md`.

## What it does

One window, with a sidebar of Search, Subjects, Saved, Professors, Free Rooms, Schedules, Degree Plans, and Watchlist, and the term in the header. The bundled Fall 2026 snapshot (`data/catalog/term-202710.catalog`) is enough to search, build schedules, and read sections offline. It keeps instructor addresses on `northeastern.edu` and `neu.edu` (and their subdomains) and omits any other address from the Banner field. Refresh seats asks Banner for current counts. Load requirements reads a catalog.northeastern.edu program page.

## Build

Rust stable, GTK 4.14, and libadwaita 1.5.

```bash
sudo apt install libgtk-4-dev libadwaita-1-dev poppler-utils pkg-config
cargo run --release
```

`pdftotext` (from poppler-utils) imports a degree-audit “View Plan” PDF. Desktop notifications use `notify-send`.

Set `TERMORGANIZE_DATA` to a directory that contains `catalog/term-202710.catalog` if you are not running from this repository. User data is JSON under `$XDG_DATA_HOME/termorganize` (or `~/.local/share/termorganize`).

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

A Flatpak manifest is `packaging/org.termorganize.TermOrganize.yml`.

## Layout of the code

`crates/core` is the catalog, search, schedule generator, degree audit, and file formats, with `cargo test`. `crates/app` is the GTK 4 / libadwaita window. `PARITY.md` maps each Swift area to the Rust file and says what is still missing.
