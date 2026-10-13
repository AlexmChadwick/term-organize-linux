# Parity with the Mac app

Linux build of Term Organize. The spec is the Swift app in [Northeastern-Term-Organize](https://github.com/AlexmChadwick/Northeastern-Term-Organize), especially the README section “On the Mac”. Shortcuts use Ctrl where the Mac app uses Command.

Status words: **ported** (same behavior, covered by tests or the window), **substituted** (a Linux stand-in for an Apple-only API), **partial** (the core is there and the window does not expose all of it), **gap** (not in this build).

## Window

| Mac | Linux | Status |
| --- | --- | --- |
| Sidebar of Search, Subjects, Saved, Professors, Free Rooms, schedules, degree plans, watchlist | `crates/app/src/ui.rs` areas, `adw::NavigationSplitView` | ported |
| Term menu in the toolbar | Header `DropDown`. Bundled and cached terms open immediately. The menu also lists Banner’s terms (`classSearch/getTerms`) and downloads a missing term with `CatalogBuilder` (`catalog.rs`), then caches `term-{code}.catalog` | ported |
| Resizable panes; small window floats the sidebar | `gtk::Paned` inside Subjects, Schedules, and Degree Plans. Sidebar collapses under 860px. Double-click does not reset a pane | partial |
| Course and professor windows side by side | Clicking a course or professor opens an `adw::Window`. Search stays put, so several courses can be open at once | ported |
| Tables, context menus, Return to open, Delete to remove | List rows, click to open, Remove buttons. No column sort UI | partial |
| Drag a degree-audit PDF onto the window | `gtk::DropTarget` on the window, plus File → Import and Ctrl+O | ported |
| Drag courses between semesters, or a code onto a schedule | Semester rows drag by course id (`editing::move_course_at`). A search row can be dropped onto a semester. Dropping onto the schedule builder is still add-by-code | partial |
| Menus and shortcuts (New Schedule, New Degree Plan, Import, Export, Print, Reload, areas 1–8) | Menubar and accelerators in `install_actions`. Print is Ctrl+P | ported |
| Settings: General, Data, iCloud, Notifications, About | Preferences window. Folder sync stands in for iCloud. About uses `adw::AboutWindow` and `crates/core/src/legal.rs` | substituted |
| Dark mode | `adw::StyleManager` Follow system / Light / Dark | ported |
| First-run unofficial notice | Alert dialog with the same sentence as the Mac app | ported |

## Search

| Swift | Linux | Status |
| --- | --- | --- |
| `Services/SearchEngine.swift` — codes over titles, prefixes, typos, aliases, CRN, every token | `crates/core/src/search.rs` | ported |
| Filters: subjects, NUpath (all must match), level, credits, campus, class type, honors, open seats, instruction, allowed days, starts after, ends before | Same struct. The window exposes subject, honors, open seats, campus, instruction, NUpath, level, credits, days, and time bounds. Class type is in the filter struct and not a separate control | partial |
| Empty query lists courses that pass the filters | Same | ported |
| Sort relevance, number, title, open seats | Header dropdown | ported |

## Courses and Banner

| Swift | Linux | Status |
| --- | --- | --- |
| Course page, credits, NUpath, description | Course page | ported |
| Prerequisite and corequisite trees | `outline.rs` `RequisiteLayout`, drawn as chips (done, grey if the term does not list the course, click opens that course). Spoken description underneath | ported |
| Live sections, seats, waitlists | Snapshot sections always. “Refresh seats” calls `BannerClient::live_seats` (`term/search` then `searchResults/searchResults`) and overlays CRN counts. Banner’s other detail endpoints (description HTML, faculty meetings, cross-lists) are parsed when HTML is in hand (`banner.rs`) but the window does not fetch each one | partial |
| `BannerClient.swift` full session pool and term scrape | `BannerClient` opens one search session, pages sections, and merges them with `catalog::merge` (credits, attributes, borrowed descriptions, postreqs). Retries with jitter are not a separate pool. The app still ships `data/catalog/term-202710.catalog` | partial |
| Save, recents, watch a section, copy CRN | Those actions | ported |

## Schedules

| Swift | Linux | Status |
| --- | --- | --- |
| Plans, primary plan, add and remove courses | `SchedulePlan` and the Schedules pane | ported |
| Week calendar | Cairo week drawing, Mon–Fri 8–21. The “show weekends” setting is stored and not yet drawn | partial |
| Conflict detection, 5-minute masks, half-term date overlap | `schedule.rs` `ScheduleSlotMask`, `Meeting::overlaps` | ported |
| Generator: DFS, forward checking, optional courses, sampling, best-N | `schedule::generate` | ported |
| Preferences survey | Time-of-day bands, days off and strength, earliest and latest, day layout, gaps, breaks, minimum break, lunch, blocked times (add, enable, strength, remove), format and strength, campus and strength, seats, instructors, credit target, plus the natural-language line | ported |
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
| Equivalences, renumbered courses, title match | `equivalences.rs` groups, `titles_alike`, `cross_list_equivalent`. The “did CS 2510 count as CS 2100?” prompt is not in the window | partial |
| Older-term offering history and catalog entries | `offering.rs` and `course_info.rs`. Degree Plans → Check older terms asks Banner, caches the answers, shows offering summaries, and suggests semester moves (`reorder.rs`) | ported |
| Student name and ID | Kept only when present in an import. Left out of exports and folder sync unless Settings includes them | ported |

## Seats, rooms, professors, Up Next

| Swift | Linux | Status |
| --- | --- | --- |
| Seat alerts every 15 minutes while open | In-app timer (`seat_check_minutes`, default 15) and `notify-send`. Compares against the snapshot, plus any seats refreshed earlier in the session | ported |
| Background refresh | Portal `org.freedesktop.portal.Background.RequestBackground` on startup, and `packaging/termorganize-seat-watch.timer` running `termorganize --seat-watch`. Not a direct BackgroundTasks clone | substituted |
| Free rooms timeline | `rooms.rs` plus a day, date, and building search | ported |
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
| Tests | `crates/core/tests/parity.rs` covers search, the schedule generator, degree-plan text, catalog audit, substitutions, and watchlist sync. `banner.rs` tests the live-seat JSON parse (the live Banner call stays ignored). PrerequisiteDataTests, the rest of the degree-audit suites, and the schedule scoring suites are not ported assertion by assertion | partial |

## Not carried over

- The iPhone and iPad apps, widgets, and App Intents.
- CloudKit entitlements and code-signature checks.
- Banner’s retry-with-jitter session pool. Term download and older-term lookups are one request at a time, then cached.
- Double-click to reset a split pane, column sorting, and dropping a course onto the schedule builder (semester drag-and-drop is in).
- Drawing Saturday and Sunday on the week calendar. The “show weekends” setting is stored.
- A class-type filter control. The filter struct still has the field.
- The “did this renumbering count?” prompt. Equivalence suggestions exist in `suggestions.rs`; the degree plan does not ask that question in its own screen.
