# Term Organize for Linux

Unofficial Northeastern course search, schedule builder, and degree-plan checker for GNOME.

**Term Organize is unofficial. It is not affiliated with, endorsed by, or sponsored by Northeastern University or Sandbox.**

Course information is copyright Northeastern University. This program is free software under the GNU Affero General Public License, version 3 only (`AGPL-3.0-only`). The license text is [`LICENSE`](LICENSE). Attribution and the SearchNEU derivation are in [`NOTICE.md`](NOTICE.md).

The notices in the app call this an independent Linux adaptation of [SearchNEU](https://github.com/sandboxnu/searchneu). The source offer in the app points at <https://github.com/AlexmChadwick/Northeastern-Term-Organize>.

## Privacy

An instructor address is kept only when its domain is exactly `northeastern.edu` or `neu.edu`. The comparison is ASCII case-insensitive and does not trim the domain. Any other parsed address is dropped. That includes another domain, a subdomain such as `husky.neu.edu`, `ccs.neu.edu`, or `mail.neu.edu`, and a lookalike: an extra label (`northeastern.edu.evil.com`), a trailing character, or a Unicode confusable in the domain. Obfuscated spellings (bracketed "at" or "dot", or an HTML or percent-encoded `@` that is not an allowed address) fail the check the bundled catalogs have to pass, so those spellings are not kept in the snapshots. Catalog files are scrubbed on read and on write.

The app does not create an account and does not collect analytics or tracking. Schedules, degree plans, and the watchlist stay on this computer. A degree-audit PDF you choose to import can contain a student name and ID. Those two fields are kept in the local plan file. They are left out of the sync folder, left out of exports unless you turn on "Include student name and ID in exports", and the local MCP server does not return them.

Course, section, seat, and instructor data comes from Northeastern's public Banner class search at `nubanner.neu.edu` (`StudentRegistrationSsb`). Loading degree requirements reads a page on `catalog.northeastern.edu`.

## What it does

One window. The sidebar is Search, Subjects, Saved, Professors, Free Rooms, Schedules, Degree Plans, and Watchlist. The header term menu lists bundled snapshots and any catalogs already cached. On a normal launch it also asks Banner for the term list and adds whatever comes back. A fresh launch opens the newest bundled term unless a saved term code is still available.

This repository ships `data/catalog/term-202710.catalog` and `data/catalog/term-202730.catalog` (Fall 2026 and Spring 2027). Those files are enough to search, build schedules, and read sections offline.

**Search.** The box takes a course, a professor, a CRN, or a short name such as "orgo". A linked control filters **All**, **Undergrad**, or **Grad**. Undergrad is a course number below 5000. Grad is 5000 and up. That band combines with the other filters: open seats, honors, no prerequisites, no corequisites, subject, campus, in person or online, NUpath, level, credits, weekdays, and a start or end time. Sort is relevance, course number, title, or open seats.

Course lists and section lists are single-selection list views. Arrow keys move one row while that list has focus, and the selected row scrolls into view. Activating a course opens it. Activating a section copies its CRN. On the saved list, Delete or Backspace removes the selected course.

**Seats and waitlist.** A section row shows seats remaining and waitlist remaining, for example `4 seats · waitlist 2`. The detail under the list shows enrollment and waitlist against capacity: `Enrollment 28 / 40 · Waitlist 2 / 10`. Search rows show how many sections are open.

**CPS restriction.** When loaded restrictions limit every known section to College of Professional Studies students, the course says "Restricted to CPS (College of Professional Studies) students". A mix says "Some sections are CPS only". A CPS-only section repeats the longer note. Restrictions that have not been loaded stay blank.

**Variable credits.** A course whose low and high credit values differ is variable credit. Search shows the range. On a schedule row and on a degree-plan semester, that course has a whole-number stepper labeled like `Credits: 4 (1 to 8)`. The value stays inside the catalog range.

**Schedules.** Add courses, mark which ones are required, and generate. The page draws the week. **Export Calendar** writes an `.ics` file for the chosen result, including weekly meetings and final exams. The calendar name is the plan name, a hyphen, and the term (`My Plan - Fall 2026`). **Export Week Image** writes a PNG of the week. **Copy CRNs** copies the selected result.

**Degree plans.** Import a degree-audit "View Plan" PDF, or start an empty plan. You can add semesters, type a course into the last semester, drag a course onto another semester, and set variable credits. **Load requirements** fetches the catalog path you enter and the page runs an audit, listing unmet and partial rules. **Autofill** and **Export PDF** are on the same page.

**Watchlist.** **Watch** on a section adds it. **Check seats** asks Banner for current counts. A **Waitlist** checkbox on a watched row opts in to a waitlist alert. `termorganize --seat-watch` does one pass and calls `notify-send` for each alert. While the app is open it also asks the desktop portal for permission to autostart that command. `packaging/termorganize-seat-watch.timer` runs the same check every 15 minutes if you install it.

**Other pages.** Subjects lists courses in one subject. Professors lists people teaching this term. Free rooms draws each room from 7:00 AM to 10:00 PM (green is free, gray is a class). A course that is not in the open term is looked up from Banner. If Banner does not list it, the page says so and offers a catalog link.

Shortcuts:

| Shortcut | Action |
| --- | --- |
| Ctrl+F | Focus search |
| Ctrl+1 through Ctrl+8 | Switch sidebar areas, in sidebar order |
| Ctrl+N | New schedule |
| Shift+Ctrl+N | New degree plan |
| Ctrl+O | Import a degree-plan PDF |
| Ctrl+E | Export the local user file as JSON |
| Ctrl+P | Print the current view |
| Ctrl+R | Reload |
| Ctrl+, | Settings |
| Ctrl+C | Copy |
| Ctrl+Q | Quit |
| F9 or Ctrl+Alt+0 | Show or hide the sidebar |

## Build, test, and run

You need a C compiler (the catalog decoder compiles the vendored LZFSE sources), pkg-config, GTK 4.14, and libadwaita 1.5. The app crate enables those library versions with the `v4_14` and `v1_5` features. Rust must be 1.85 or newer. A dependency uses edition 2024, so Cargo 1.83 cannot parse the tree. Continuous integration uses the stable toolchain.

On Ubuntu 24.04 these packages provide GTK 4.14 and libadwaita 1.5:

```bash
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev poppler-utils
cargo test --workspace
cargo run --release -p termorganize
```

`pdftotext`, from `poppler-utils`, is what imports a degree-audit PDF. It is not required to compile. Seat alerts call `notify-send` when that program is on `PATH`.

`cargo test --workspace` is the test step in CI. CI also runs `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --release`, and `termorganize --smoke` under Xvfb.

Set `TERMORGANIZE_DATA` to choose a data directory. If `<that directory>/catalog` already contains `term-*.catalog` files, those snapshots replace the ones shipped in this repository. With the variable set, `user.json` is stored in that directory. With it unset, the app uses `$XDG_DATA_HOME/termorganize`, or `~/.local/share/termorganize`. If a Flatpak copy has also written `~/.var/app/org.termorganize.TermOrganize/data/termorganize/user.json`, the newer of the two files is the one that is opened. The app and `term-organize-mcp` share this file.

## Commands

```text
termorganize                      open the window
termorganize file.pdf             import a degree-plan PDF
termorganize 'termorganize://search?q=orgo'
termorganize --search "fundies"
termorganize --search-provider    GNOME Shell search provider
termorganize --seat-watch         one seat-alert pass
termorganize --smoke              open at 1440x900, require the page to fill the window, then quit
termorganize --version
```

Passing an `.html` file loads it as catalog requirements for the degree plan that is already open. A PDF imports a new plan.

## Desktop files in this repository

`packaging/` has a `.desktop` file, a metainfo file, an icon, a GNOME Shell search-provider service, a seat-watch systemd user timer, and a Flatpak manifest. `packaging/build-release.sh` vendors crates and builds a local Flatpak and an x86_64 AppImage on this machine. Installing the desktop files for your user:

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

The `.desktop` file registers `termorganize://`, `neuclasssearch://`, and opening a PDF with Term Organize.

## Local MCP server

`term-organize-mcp` speaks MCP over stdio (protocol `2025-06-18`) for the bundled Fall 2026 and Spring 2027 catalogs and for degree plans in the local user file. It does not use the network, and it does not return a student name or student ID. Tool names and JSON shapes are in [`docs/MCP.md`](docs/MCP.md).

```bash
cargo run -p term-organize-mcp
```

`--read-only`, or `TERM_ORGANIZE_MCP_READONLY=1`, hides the plan write tools. `--data-dir`, or `TERM_ORGANIZE_MCP_DATA`, picks the user-file directory.

A client can launch the binary directly:

```json
{
  "mcpServers": {
    "term-organize": {
      "command": "/path/to/term-organize-mcp"
    }
  }
}
```

## Layout

`crates/core` is the catalog, search, schedule generator, degree audit, plan file, and email filter, and it is what `cargo test` covers along with the other crates. `crates/app` is the GTK 4 / libadwaita window. `crates/mcp` is the local MCP server. [`PARITY.md`](PARITY.md) maps each Swift area to the Rust file that corresponds to it.

## License

GNU Affero General Public License v3.0 only. See [`LICENSE`](LICENSE). Course data from Northeastern is not covered by that grant. See [`NOTICE.md`](NOTICE.md).
