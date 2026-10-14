# Notice

**Term Organize** — an unofficial iPhone, iPad and Mac app for searching Northeastern
University courses, building class schedules, planning degrees, and getting seat alerts.

Copyright (C) 2026 Term Organize contributors. Source code:
<https://github.com/AlexmChadwick/Northeastern-Term-Organize>.

This program is free software: you can redistribute it and/or modify it under the terms
of the GNU Affero General Public License as published by the Free Software Foundation,
version 3 (SPDX: `AGPL-3.0-only`).

This program is distributed in the hope that it will be useful, but **WITHOUT ANY
WARRANTY**; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
PARTICULAR PURPOSE. See the GNU Affero General Public License for more details. The full
license text is in [`LICENSE`](LICENSE); the same text ships inside the app and can be read
under **Settings › Acknowledgements & Licenses**.

If you distribute this app (for example through TestFlight or the App Store) you must make
its complete corresponding source code available to recipients under the same license
(AGPL-3.0 §6). Before distributing, set `LegalText.sourceCodeURL`
(`NEUClassSearch/Logic/Settings/LegalText.swift`) to where the source can be downloaded;
the app's About and Licenses screens then show that link in their source-code offer. Note
that app-store terms may impose restrictions that conflict with the AGPL; only the
copyright holders (including SearchNEU's) can grant an exception for their code.

## Based on SearchNEU

This app is a modified work based on **SearchNEU** — <https://github.com/sandboxnu/searchneu>
— Copyright (C) Sandbox at Northeastern University and the SearchNEU contributors, licensed
under the GNU Affero General Public License v3. Upstream revision consulted:
`3ffa0f2402054ffe1115178b92bb3b99da1bdcba` (2026-09-30).

The following parts were ported from SearchNEU's TypeScript sources to Swift and modified
on 2026-10-02 to 2026-10-05. Each file carries a notice naming its upstream source files, the upstream
copyright holder, the license, and the date and nature of the modification.

| This project | Derived from (SearchNEU) | Modifications |
| --- | --- | --- |
| `NEUClassSearch/Core/Banner/BannerParsers.swift` | `packages/scraper/src/generate/steps/reqs.ts` (`parsePrereqs`, `parseCoreqs`), `packages/scraper/src/generate/steps/courseNames.ts` | Ported to Swift; tolerant of unknown subjects and score-less tests; college/department parsing added |
| `NEUClassSearch/Core/Banner/CatalogBuilder.swift` | `packages/scraper/src/generate/marshall.ts` (`arrangeCourses`) | Ported to Swift; merge logic for on-device refreshes added |
| `NEUClassSearch/Core/Banner/BannerClient.swift` | `packages/scraper/src/generate/endpoints.ts`, `packages/scraper/src/generate/fetch.ts`, `packages/scraper/src/generate/steps/sections.ts`, `packages/scraper/src/generate/steps/terms.ts`, `packages/scraper/src/generate/steps/campuses.ts` | Endpoint usage, cookie-pool and retry approach re-implemented with URLSession (smaller session pool); per-course live refresh, attribute lookups and course-detail merging added |
| `NEUClassSearch/Core/Banner/BannerModels.swift` | `packages/scraper/src/schemas/banner/*.ts`, `packages/scraper/src/generate/marshall.ts` (`parseMeetingTimes`) | Lenient Decodable models; meeting times converted to minutes after midnight and dates to ISO format |
| `NEUClassSearch/Core/Models/Requisite.swift` | `packages/scraper/src/types.ts`, `packages/scraper/src/generate/steps/reqs.ts` (`getAllPrereqsForSingleCourse`, `mergeSameConditionTypes`) | Requisite tree as a Swift enum; satisfaction checks, summary and compact Codable encoding added |
| `NEUClassSearch/Core/Models/TermCatalog.swift` | `packages/scraper/src/generate/steps/reqs.ts` (`populatePostReqs`) | `recomputePostreqs()` ported to Swift; postrequisites stored as a sorted course list |
| `Tools/CatalogSnapshot/main.swift` | `packages/scraper/src/generate/main.ts` | Same scrape flow, lower request concurrency, writes the app's catalog format |
| `NEUClassSearchiOS/Features/Course/RequisiteTreeView.swift` | `apps/searchneu/components/catalog/Requisites.tsx` | Reimplemented natively in SwiftUI |
| `NEUClassSearchMac/Catalog/MacRequisiteTreeView.swift` | `apps/searchneu/components/catalog/Requisites.tsx` | Reimplemented natively in SwiftUI for the Mac: courses are buttons that open course windows, with hover states and context menus |
| `NEUClassSearch/Logic/Schedule/ScheduleScoring.swift` | `apps/searchneu/lib/scheduler/binaryMeetingTime.ts`, `apps/searchneu/lib/scheduler/filters.ts` | Interval and measurement code ported to Swift (split out of the generator); scores weighted by the student's preferences added |
| `NEUClassSearch/Logic/Schedule/ScheduleGenerator.swift` | `apps/searchneu/lib/scheduler/generateSchedules.ts`, `apps/searchneu/lib/scheduler/binaryMeetingTime.ts`, `apps/searchneu/lib/scheduler/filters.ts` | Ported to Swift; odometer replaced by a depth-first search with forward checking, maximal optional-course combinations, scoring and sort options, sampling of very large search spaces |
| `NEUClassSearch/Logic/Watchlist/WatchlistComparison.swift` | `packages/scraper/src/update/main.ts` (seat-change rules) | Reimplemented as a pure function feeding on-device notifications instead of SMS |

The rest of the app (SwiftUI interface, widgets, storage, notifications, rooms, calendar
export) is original work released under the same license. Some of it was designed after
SearchNEU's features (notably the on-device search in
`NEUClassSearch/Services/SearchEngine.swift` — cf.
`apps/searchneu/lib/catalog/use-catalog-search.ts` — the course page and the rooms page),
but these are independent implementations, not translations of SearchNEU's code.

"SearchNEU" and "Sandbox" are names of their respective owners. This project is **not
affiliated with, endorsed by, or sponsored by Sandbox or Northeastern University**.

## Course data

Course, section, seat and instructor information is retrieved from Northeastern
University's public class search (<https://nubanner.neu.edu/StudentRegistrationSsb>) — the
same source SearchNEU uses. Course titles, descriptions and other catalog content are
© Northeastern University. The bundled snapshot
(`NEUClassSearch/Resources/Catalog/term-202710.catalog`) was generated on 2026-10-03 with
`Tools/build-catalog-snapshot.sh`. It includes the instructor names and the
`northeastern.edu` and `neu.edu` addresses (including their subdomains) that
Northeastern publishes in its class search. Any other address Banner had stored
in those fields was removed from the bundled snapshot. This data belongs to Northeastern
University and is **not** covered by this project's AGPL license grant. "Northeastern
University" and "NUpath" are used only descriptively; no Northeastern logos or other marks
are included.

SearchNEU's separate cache repository (`sandboxnu/searchneu-cache`) was **not** used or
redistributed, because it does not carry a license.

Degree requirements are read on the device from Northeastern's public academic catalog
(<https://catalog.northeastern.edu>) for the catalog year and program the user picks, and kept
on the device. Like the class search data, catalog content is © Northeastern University; none
of it is bundled with the app, and it is shown only to the user who looked it up.

### Degree audit plans

The degree plan importer reads, and the exporter reproduces, the text layout of the PDF that
Northeastern's degree audit planner creates with *View Plan* (header fields, “Completed
Courses”, semester headings and course lines, on A4 pages in Helvetica). The layout was
reimplemented from a sample export; no code, templates or other material from the degree
audit system are included, and the PDF library that system uses is not used here (exports
are drawn with Apple's UIKit). Imported plans stay on the device.

## Third-party software

No third-party libraries or assets are used; the only third-party code is the
SearchNEU-derived code listed above. The app uses only Apple system frameworks (such as
SwiftUI, WidgetKit, EventKit, UserNotifications, BackgroundTasks, AppIntents, CoreSpotlight,
CloudKit, FoundationModels, NaturalLanguage, PDFKit, UniformTypeIdentifiers and Foundation).
Apple Intelligence (FoundationModels) runs on the device; nothing typed into the schedule
description is sent anywhere. SF Symbols are used in the interface under Apple's terms; the
app icon is an original vector drawing and does not use SF Symbols.

## Linux port

The GNOME build in this tree (`crates/core`, `crates/app`) is a further modification of the
Swift sources above, released under the same AGPL-3.0-only terms. Search, schedule generation,
degree audit, NUpath matching, requisite parsing, and seat comparison follow those Swift
files, which themselves follow SearchNEU where the table says so. The window uses GTK 4 and
libadwaita. iCloud is a folder of JSON. Spotlight is a GNOME Shell search provider. Schedule
descriptions are parsed on the device by `crates/core/src/nl.rs` and are not sent to a model.

The catalog snapshot is LZFSE-compressed. The decoder is the vendored C library in
`crates/core/vendor/lzfse`, which is Apple's LZFSE under its own BSD-style license
(`crates/core/vendor/lzfse/LICENSE`). HTTP uses the `ureq` crate. Neither is covered by the
SearchNEU grant; both are separate third-party components.

The unofficial notice is unchanged: this program is not affiliated with, endorsed by, or
sponsored by Northeastern University or Sandbox.
