# Term Organize MCP server

Unofficial Northeastern course planner. Not affiliated with, endorsed by, or sponsored by Northeastern University or Sandbox.

SHARED CONTRACT

- Transport: stdio, newline-delimited JSON-RPC 2.0, one message per line, per the MCP stdio spec (protocol 2025-06-18). No Content-Length framing.
- Term objects: {"term":"202730","name":"Spring 2027"}. The key is always "term", never "code".
- Revisions: integer, starting at 1 and incremented on every change to that plan. A plan with no `planRevisions` entry, or a stored 0, is revision 1 on read and on the `expected_revision` check. The next edit is revision 2.
- Errors: a tool result with isError:true and structuredContent {"error":{"code":C,"message":M}}, where C is one of: invalid_argument, not_found, revision_conflict, confirm_required, read_only, storage_error, unknown_tool. The message is never empty. Arguments that do not match the tool schema (a missing field, a string where a number is required, an object where a string is required) are `invalid_argument` in that same object, not a plain-text tool error. A well-formed term, course, or CRN that is not in the bundled catalogs is `not_found`. A write tool called with --read-only on returns read_only, not unknown_tool. Write tools stay hidden from tools/list.
- Instructors in get_course: [{"name":..., "email":... (optional)}]. The email is present only when its domain is EXACTLY northeastern.edu or neu.edu, compared case-insensitively. Subdomains are not allowed (husky.neu.edu, ccs.neu.edu, mail.neu.edu are all dropped).
- create_plan {name, terms?: ["202710","202730"]}, a list of term strings.
- reorganize_plan accepts either {plan_id} or {plan:{terms:[{term, courses:[{subject, number}]}]}}. It returns {moves:[{subject, number, from_term, to_term, reason}], unresolved:[{subject, number, reason}], plan:{...}}.
- Reorganize rule, the same everywhere: a course moves only into a term the plan already has. The target term's bundled catalog must offer the course, prerequisite order within the plan must be kept, and it never adds a semester. A course is offered in a term when the bundled catalog for that term has at least one section of it.
- Shared fixture: tests/fixtures/mcp-reorganize-fixture.json. It is a plan with CS 4805, CS 2535, CS 2700, CS 4130 and CS 2701 in Fall 2026, plus an empty Spring 2027. Each server must have a test that checks its output against the expected moves. Work out the expected result from the bundled 202730 catalog and the prereqs, and write it into the fixture. Include in your report the per-course reasoning: offered in 202730 yes/no, and prereq status.
- Storage failures: if the plan file exists but can't be read (permission denied, sandbox, malformed JSON, unknown newer version), every plan tool returns storage_error with the path and reason. NEVER treat a read failure as an empty plan list, and NEVER write over a file you couldn't read. A missing file is the only case that counts as empty.

`term-organize-mcp` is a local [Model Context Protocol](https://modelcontextprotocol.io) server. It speaks JSON-RPC 2.0 over stdin and stdout, protocol version `2025-06-18`, one JSON object per line. It does not use Content-Length framing. It reads the bundled Fall 2026 (`202710`) and Spring 2027 (`202730`) catalogs only. It can also read and update degree plans in the same `user.json` the Term Organize app loads.

The server does not use the network, does not log in, and does not return student name or student ID. Those fields can sit in the file after a PDF import. They stay there for the app, and they are left out of every MCP response.

Instructor addresses are passed through the same filter as the app (`crates/core/src/email.rs`). A response includes an email only when the domain is exactly `northeastern.edu` or `neu.edu`. Subdomains are dropped.

## Run

```bash
cargo run -p term-organize-mcp
```

The binary is `target/release/term-organize-mcp` after `cargo build -p term-organize-mcp --release`.

```text
term-organize-mcp --help
term-organize-mcp --read-only
term-organize-mcp --data-dir /path/to/termorganize
```

`--read-only` hides every write tool from `tools/list`. `TERM_ORGANIZE_MCP_READONLY=1` does the same thing. A client that calls a write tool anyway gets `read_only`, not `unknown_tool`. Clients then see only the catalog tools plus `list_plans`, `get_plan`, and `reorganize_plan`.

## Where plans are stored

The app and the server share one file, `user.json`, through `crates/core`. A native build and an AppImage use `$XDG_DATA_HOME/termorganize`, or `~/.local/share/termorganize` when `XDG_DATA_HOME` is unset. The Flatpak build writes under `~/.var/app/org.termorganize.TermOrganize/data/termorganize`, which is `$XDG_DATA_HOME/termorganize` inside the sandbox.

When `TERMORGANIZE_DATA` is unset, the server picks whichever of those directories already contains `user.json`. If both do, the newer file wins. If neither does, it uses the native path.

Override that choice in this order:

1. `--data-dir PATH`
2. `TERM_ORGANIZE_MCP_DATA` (this does not redirect catalog loading)
3. `TERMORGANIZE_DATA` (the app override; also the directory the server uses when the two flags above are unset)
4. The Flatpak versus XDG choice above

Writes take `expected_revision`. The revision lives in `planRevisions` on the user file, next to the degree plan, and increases when the plan body changes. A plan the app saved before that map existed has no entry. That plan is revision 1: `list_plans` and `get_plan` report 1, and a write with `expected_revision` 1 is accepted. The write then stores revision 2. A save from the open app bumps it too, so a stale `expected_revision` fails with `revision_conflict` and the file is left as it was. Each save copies the previous `user.json` to `user.json.bak` (one rolling backup), writes `user.json.tmp`, and renames that into place.

The desktop app also watches this file. Before it saves, it compares `planRevisions` with the copy it loaded, and the file's modification time plus a SHA-256 of the bytes. Plans only one side edited are merged. A plan both sides edited keeps the file already on disk, and the app's copy is saved beside it as `<name> (edited in app)`, so an MCP write is not replaced. A change while the window is open is reloaded through a debounced file monitor.

MCP plans are the app's degree plans (`degree_plans` in `user.json`). A chosen section is stored as `crn` on the course. The app loader reads that field back.

## Client config

Claude Desktop (`claude_desktop_config.json`) and Cursor (`.cursor/mcp.json`):

```json
{
  "mcpServers": {
    "term-organize": {
      "command": "/path/to/term-organize-mcp"
    }
  }
}
```

Read only, with an explicit plan directory:

```json
{
  "mcpServers": {
    "term-organize": {
      "command": "/path/to/term-organize-mcp",
      "args": ["--read-only", "--data-dir", "/path/to/termorganize"]
    }
  }
}
```

## Tools

Parameter names are snake_case. The same names and JSON shapes are used on macOS, Windows, and Linux. Read tools set `readOnlyHint`. `delete_plan`, `remove_course`, and `apply_reorganize` set `destructiveHint`. Every tool sets `openWorldHint` to false.

A tool failure the user should see (a stale revision, an unknown course, a missing `confirm`) comes back as a tool result with `isError` and this JSON:

```json
{ "error": { "code": "revision_conflict", "message": "The plan changed (revision 2). Reload it and try again." } }
```

`code` is `invalid_argument`, `not_found`, `revision_conflict`, `confirm_required`, `read_only`, `storage_error`, or `unknown_tool`. `message` is never empty.

`invalid_argument` is a malformed value: an unrecognized term name, a `plan_id` that is not a UUID, a time that is not `8am` or `8:00 AM`, or arguments that fail to deserialize (`plan_id` omitted, `expected_revision` sent as `"1"`, `terms` sent as `[{"term":"Fall 2026"}]` instead of `["Fall 2026"]`). `not_found` is a term, course, or CRN that is well formed and still absent: no bundled catalog for that term, no such course in the catalog, or a CRN that is not a section of that course. An unknown plan id is also `not_found`.

Times use `to` between the start and the end (`9:00 AM to 10:05 AM`). Credits use `3 to 4` when the course has a range.

### list_terms

No parameters.

```json
{ "terms": [{ "term": "202730", "name": "Spring 2027" }] }
```

Newest term first.

### search_courses

`term` (name or Banner code), optional `query`, `subject`, `number`, `attribute`, `limit` (default 25, at most 100), `no_prerequisites`, and `no_corequisites`.

`no_prerequisites` and `no_corequisites` default to false. When one is true, a course is kept only when its details were loaded and that requisite was omitted. A course whose details were not loaded is unknown, so it stays out. An empty requisite group counts as none once the details are loaded.

```json
{
  "term": { "term": "202710", "name": "Fall 2026" },
  "courses": [{
    "subject": "CS",
    "number": "2100",
    "title": "Program Design and Implementation 1",
    "credits": "4",
    "min_credits": 4,
    "max_credits": 4,
    "attributes": []
  }]
}
```

### get_course

`term`, `subject`, `number`. Same course fields as search, plus `sections`:

```json
{
  "crn": "12345",
  "section": "01",
  "days": "MWF",
  "times": "9:00 AM to 10:05 AM",
  "location": "SN 108",
  "instructors": [{ "name": "Ada Lovelace", "email": "lovelace@northeastern.edu" }],
  "seats_remaining": 3,
  "seat_capacity": 30
}
```

`email` is omitted unless the domain is exactly `northeastern.edu` or `neu.edu`. Day letters are `U M T W R F S` (Tuesday is `T`, Thursday is `R`).

### check_schedule

`term`, `crns` (array of CRN strings).

```json
{
  "term": { "term": "202710", "name": "Fall 2026" },
  "clashes": [{ "first": "11111", "second": "22222", "reason": "The meetings overlap." }],
  "unknown_crns": ["00000"]
}
```

### suggest_schedule

`term`, `courses` (each `{ "subject", "number", "crn"? }`), optional `constraints`:

```json
{ "no_classes_before": "9:30 AM", "no_classes_after": "17:00", "days_off": ["Friday"] }
```

Times are `HH:MM` on a 24-hour clock (`08:00`, `17:00`) or an explicit 12-hour time (`8am`, `8:00 AM`, `9:00 PM`). Anything else, including a bare `8`, is `invalid_argument`. Days accept full names and the letters above. Constraints are required: sections outside them are dropped. Up to 8 schedules come back, best first.

A course listed with `crn` uses only that section. The same subject and number may appear more than once only when every copy has its own CRN and the CRNs differ. A second copy with no CRN, or the same CRN twice, is `invalid_argument`. Generation does not merge those copies into one course.

```json
{
  "term": { "term": "202710", "name": "Fall 2026" },
  "schedules": [{ "crns": ["12345"], "sections": [] }]
}
```

Each section includes the get_course fields plus `subject` and `number`.

### reorganize_plan

Either `{ "plan_id": "uuid" }` or `{ "plan": { "terms": [{ "term": "202710", "courses": [{ "subject": "CS", "number": "2510" }] }] } }`.

The core library's existing reorder only moves blocking courses out of one semester, and it can add a semester. This tool is a separate whole-plan pass in `crates/core/src/reorganize.rs`. A course moves only into a term the plan already has. The target term's bundled catalog must offer the course (at least one section), and the pass never adds a semester. Prerequisite order is kept only between courses that are on the plan. A prerequisite that is not on the plan does not block a move: it may already be completed, transferred, or AP credit, and moving the course does not make that situation worse. A move is refused when it would place a course ahead of a prerequisite that is also on the plan, or leave another planned course ahead of this one. Corequisites are not part of the check. A refused course stays put and is listed in `unresolved`. Offering checks use the two bundled snapshots.

```json
{
  "moves": [{ "subject": "CS", "number": "4805", "from_term": "202710", "to_term": "202730", "reason": "CS 4805 has no sections in 202710." }],
  "unresolved": [],
  "plan": { "terms": [{ "term": "202730", "courses": [{ "subject": "CS", "number": "4805" }] }] }
}
```

A chosen CRN stays on the course. A pinned course moves only when the destination catalog contains that same CRN. Otherwise the course stays in its term and is listed in `unresolved`. An unpinned clash can still move, and that move does not invent a CRN. Two rows that share a subject and number stay separate when their CRNs differ. The shared fixture is `tests/fixtures/mcp-reorganize-fixture.json`.

### list_plans

```json
{
  "plans": [{
    "id": "uuid",
    "name": "My plan",
    "terms": [{ "term": "202710", "name": "Fall 2026" }],
    "updated_at": "2026-10-20T00:00:00Z",
    "revision": 1
  }]
}
```

`term` is the Banner code when that season and year are a bundled catalog, and the term name otherwise.

### get_plan

`plan_id`.

```json
{
  "id": "uuid",
  "name": "My plan",
  "revision": 1,
  "updated_at": "2026-10-20T00:00:00Z",
  "terms": [{
    "term": "202710",
    "name": "Fall 2026",
    "courses": [{
      "subject": "CS",
      "number": "2100",
      "title": "Program Design and Implementation 1",
      "credits": 4,
      "crn": "12345",
      "section": {},
      "offered": true
    }]
  }],
  "validation": {
    "clashes": [],
    "not_offered": [{ "term": "Fall 2026", "subject": "CS", "number": "2510" }],
    "duplicates": ["CS 4973 appears more than once. Each copy still counts."]
  }
}
```

`section` is the get_course section object, or null when the stored CRN is not in that term. A course the user typed that is not a catalog course has `label` and `offered: null`.

### create_plan

`name`, optional `terms` (array of term names or Banner codes, such as `"Fall 2026"` or `"202710"`). Returns the get_plan object. The first saved degree plan is the primary plan, matching the app.

### rename_plan

`plan_id`, `name`, `expected_revision`. Returns the get_plan object.

### add_course

`plan_id`, `term`, `subject`, `number`, optional `crn`, `expected_revision`. The term must already be on the plan. The course and CRN must exist in the bundled catalog for that term. A second copy is always accepted, with or without a CRN, including the same CRN twice. `validation.duplicates` lists each repeated course as `<subject> <number> appears more than once. Each copy still counts.` A repeat is two copies with no distinct sections, or the same CRN twice. Two different CRNs, or a title containing "topic", is not a duplicate. Returns the get_plan object.

### remove_course

`plan_id`, `term`, `subject`, `number`, `expected_revision`. Returns the get_plan object.

### move_course

`plan_id`, `subject`, `number`, `from_term`, `to_term`, `expected_revision`. The destination must be on the plan and must offer the course in a bundled catalog. The chosen CRN is not cleared or replaced. Returns the get_plan object.

### set_section

`plan_id`, `term`, `subject`, `number`, `crn`, `expected_revision`. The CRN must be a section of that course in the bundled catalog. This is the only tool that changes a chosen CRN. One row of that course is updated. When several rows share the number, the row that does not yet have a CRN is the one that changes. When every row already has a different CRN, the call is `invalid_argument`. Setting a CRN the plan already uses is `invalid_argument`, and setting the CRN a row already has leaves the plan unchanged. Returns the get_plan object.

### apply_reorganize

`plan_id`, `expected_revision`. Runs the same pass as `reorganize_plan` and, when something moved, saves it. The response is the get_plan object plus `moves` and `unresolved`. When nothing moved, the file and the revision stay as they were.

### delete_plan

`plan_id`, `expected_revision`, `confirm`. `confirm` must be true. Otherwise the tool returns `confirm_required` and does not write. Success:

```json
{ "deleted": true, "plan_id": "uuid" }
```
