# Parity with the Mac app

Linux build of Term Organize. The spec is the Swift app in [Northeastern-Term-Organize](https://github.com/AlexmChadwick/Northeastern-Term-Organize), especially the README section “On the Mac”. Shortcuts use Ctrl where the Mac app uses Command.

Status words: **ported** (same behavior, covered by tests or the window), **substituted** (a Linux stand-in for an Apple-only API), **partial** (the core is there and the window does not expose all of it), **gap** (not in this build).

## Window

| Mac | Linux | Status |
| --- | --- | --- |
| Sidebar of Search, Subjects, Saved, Professors, Free Rooms, schedules, degree plans, watchlist | `crates/app/src/ui.rs` areas, `adw::NavigationSplitView`. The split and each page expand so the page fills the window (`--smoke` checks the height) | ported |
| Term menu in the toolbar | Header `DropDown`. Bundled and cached terms open immediately. The menu also lists Banner’s terms (`classSearch/getTerms`) and downloads a missing term with `CatalogBuilder` (`catalog.rs`), then caches `term-{code}.catalog` | ported |
| Resizable panes; small window floats the sidebar | Subjects, Schedules, and Degree Plans use `gtk::Paned` with both children resizing, so the divider keeps a share of the width (0.28, 0.26, 0.24). Dragging it under 80px hides that pane. Double-click restores the share. The sidebar collapses under 860px | ported |
| Course and professor windows side by side | Clicking a course or professor opens an `adw::Window`. Search stays put, so several courses can be open at once | ported |
| Tables, context menus, Return to open, Delete to remove | Right-click menus on courses, professors, sections, schedule courses, degree-plan courses, the watchlist, and plan lists. Return opens a course or professor (a section row copies its CRN). Delete removes a saved course, a schedule or degree-plan course, a watched section, or a plan. Column buttons sort by code, title, and open seats | ported |
| Drag a degree-audit PDF onto the window | `gtk::DropTarget` on the window, plus File → Import and Ctrl+O | ported |
| Drag courses between semesters, or a code onto a schedule | Semester rows drag by course id (`editing::move_course_at`). A search row or a degree-plan course can be dropped onto a semester or onto the schedule’s course list (`attach_schedule_drop`) | ported |
| Menus and shortcuts (New Schedule, New Degree Plan, Import, Export, Print, Reload, areas 1–8) | Menubar and accelerators in `install_actions`. Print is Ctrl+P | ported |
| Settings: General, Data, iCloud, Notifications, About | Preferences window. Folder sync stands in for iCloud. About uses `adw::AboutWindow` and `crates/core/src/legal.rs` | substituted |
| Dark mode | `adw::StyleManager` Follow system / Light / Dark | ported |
| First-run unofficial notice | Alert dialog with the same sentence as the Mac app | ported |

## Search

| Swift | Linux | Status |
| --- | --- | --- |
| `Services/SearchEngine.swift` — codes over titles, prefixes, typos, aliases, CRN, every token | `crates/core/src/search.rs` | ported |
| Filters: subjects, NUpath (all must match), level, credits, campus, class type, honors, open seats, instruction, allowed days, starts after, ends before | Same struct. The window exposes each of those, including a class-type dropdown (one type at a time) | ported |
| Empty query lists courses that pass the filters | Same | ported |
| Sort relevance, number, title, open seats | Header dropdown | ported |

## Courses and Banner

| Swift | Linux | Status |
| --- | --- | --- |
| Course page, credits, NUpath, description | Course page | ported |
| Prerequisite and corequisite trees | `outline.rs` `RequisiteLayout`, drawn as chips (done, grey if the term does not list the course, click opens that course). Spoken description underneath | ported |
| Live sections, seats, waitlists | Snapshot sections always. “Refresh seats” calls `BannerClient::live_seats` (`term/search` then `searchResults/searchResults`) and overlays CRN counts. Banner’s other detail endpoints (description HTML, faculty meetings, cross-lists) are parsed when HTML is in hand (`banner.rs`) but the window does not fetch each one | partial |
| `BannerClient.swift` full session pool and term scrape | `fetch_all_sections` probes `totalCount`, then uses up to four search sessions. Each session fetches the pages `page % sessions`, one request at a time. `perform` retries HTTP 429, 5xx, and transport errors four times: delay starts at 1s, doubles, and stops at 20s, plus 0–0.5s of jitter. Sections merge through `catalog::merge`. The app still ships `data/catalog/term-202710.catalog` | ported |
| Save, recents, watch a section, copy CRN | Those actions | ported |

## Schedules

| Swift | Linux | Status |
| --- | --- | --- |
| Plans, primary plan, add and remove courses | `SchedulePlan` and the Schedules pane | ported |
| Week calendar | Cairo week drawing. Saturday and Sunday appear when Settings → show weekends is on, or when a meeting falls on them. The hour range follows the calendar start and end hours and stretches to fit the meetings | ported |
| Conflict detection, 5-minute masks, half-term date overlap | `schedule.rs` `ScheduleSlotMask`, `Meeting::overlaps` | ported |
| Generator: DFS, forward checking, optional courses, sampling, best-N | `schedule::generate` | ported |
| Preferences survey | Time-of-day bands, days off and strength, earliest and latest, day layout, gaps, breaks, minimum break, lunch, blocked times (add, enable, strength, remove), format and strength, campus and strength, seats, instructors, credit target, plus the natural-language line. The window uses `AdwClamp`, `AdwPreferencesGroup`, `ComboRow`, `SwitchRow`, and `EntryRow`, with day toggles that wrap | ported |
| Ranking with reasons | List shows CRNs and reason text | ported |
| No-results suggestions | `suggest_relaxations` titles when Generate finds nothing | ported |
| Export .ics | `schedule::ics`, same VTIMEZONE and UID shape | ported |
| Export an image | `schedule::week_png` | ported |
| Copy CRNs | `schedule::copy_crns` and Ctrl+C | ported |
| Autofill from the degree plan | `planlogic::schedule_suggestions` | ported |
| Apple Intelligence description | `nl.rs` `interpret`, deterministic | substituted |
| EventKit | An `.ics` file you import yourself | substituted |

## Degree plans and the audit

| Swift | Linux | Status |
| --- | --- | --- |
| PDF import of a View Plan export | `pdftotext -layout`, then `degree_format::parse` | ported |
| Edit semesters and courses, checks, totals | Add semester, add a code, drag between semesters, duplicate and prerequisite checks, credit totals, offering line on each course | ported |
| Export PDF in the same layout | `degree_format::pdf` (A4, Helvetica, 115-dash rule, hour labels) | ported |
| Autofill missing catalog courses | Audit leftovers that exist in the snapshot | ported |
| Requirements from catalog.northeastern.edu | “Load requirements” fetches the path and runs `program_parser`. Saved next to user data | ported |
| `DegreeAudit.swift` greedy fill, substitutions, petitions, waivers, rebalance, NUpath, minimums | `degree_audit.rs` and `nupath.rs`. Petition questions (`petition.rs`) are a section on the degree plan: same-as-requirement, any number, or a different minimum, plus near misses | ported |
| Equivalences, renumbered courses, title match | `equivalences.rs` groups, `titles_alike`, `cross_list_equivalent`. Degree Plans asks “Does CS 2510 count as CS 2100?” for a plan course the term does not list (`suggestions.rs`). Yes declares an equivalence. No is remembered in `dismissed-equivalences.txt` | ported |
| Older-term offering history and catalog entries | `offering.rs` and `course_info.rs`. Degree Plans → Check older terms asks Banner, caches the answers, shows offering summaries, and suggests semester moves (`reorder.rs`) | ported |
| Student name and ID | Kept only when present in an import. Left out of exports and folder sync unless Settings includes them | ported |

## Seats, rooms, professors, Up Next

| Swift | Linux | Status |
| --- | --- | --- |
| Seat alerts every 15 minutes while open | In-app timer (`seat_check_minutes`, default 15) and `notify-send`. Compares against the snapshot, plus any seats refreshed earlier in the session | ported |
| Background refresh | Portal `org.freedesktop.portal.Background.RequestBackground` on startup, and `packaging/termorganize-seat-watch.timer` running `termorganize --seat-watch`. Not a direct BackgroundTasks clone | substituted |
| Free rooms timeline | Cairo bars from 7:00 AM to 10:00 PM (`rooms::Moment::FIRST_CLASS` through `LAST_CLASS`). Green is free, dimmer before the current time when the date is today. Gray blocks are classes, labeled with the course. Day, date, and building search stay | ported |
| Professors | `professors.rs` and a professor page | ported |
| Menu-bar Up Next | Header subtitle via `upnext.rs` when a generated or selected schedule has meetings today | substituted |
| Spotlight | `termorganize --search` and `search_provider.rs` (`org.gnome.Shell.SearchProvider2`) | substituted |

## Links, settings, data

| Swift | Linux | Status |
| --- | --- | --- |
| `termorganize://` and `neuclasssearch://` | `links.rs`, opened from the command line and the `.desktop` MimeType | ported |
| PDF Open With | `MimeType=application/pdf` and a path argument | ported |
| User data JSON, export and import | `store.rs`. Export omits the student unless the setting is on | ported |
| iCloud / CloudKit merge | `sync.rs` last-write-wins folder sync. Device-only fields (color scheme, seat interval) stay local. No CloudKit | substituted |
| Legal text and disclaimer | `legal.rs`, first-run dialog, About | ported |
| Tests | All 373 Swift `@Test`s are ported under `crates/core/tests/` (`prerequisite_data.rs` 99, `schedule_suites.rs` 120, `degree_suites.rs` 66, `support_suites.rs` 88), plus `parity.rs`. CloudKit container, entitlement, and code-signature checks assert the Linux stand-in: merge does not read a container id or a signature. The live Banner call in `banner.rs` stays ignored | ported |

## Not carried over

- The iPhone and iPad apps, widgets, and App Intents.
- CloudKit entitlements and code-signature checks.
- A split pane that floats over the other view when the window is too narrow for both. The sidebar collapses under 860px instead.
- Every Mac table column. Header buttons sort by code, title, and open seats.
