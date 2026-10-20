# Term Organize MCP server

Unofficial Northeastern course planner. Not affiliated with, endorsed by, or sponsored by Northeastern University or Sandbox.

`term-organize-mcp` is a local [Model Context Protocol](https://modelcontextprotocol.io) server. It speaks JSON-RPC 2.0 over stdin and stdout, protocol version `2025-06-18`. It reads the bundled Fall 2026 (`202710`) and Spring 2027 (`202730`) catalogs only. It can also read and update degree plans in the same `user.json` the Term Organize app loads.

The server does not use the network, does not log in, and does not return student name or student ID. Those fields can sit in the file after a PDF import. They stay there for the app, and they are left out of every MCP response.

Instructor addresses are passed through the same filter as the app (`crates/core/src/email.rs`). A response only includes `northeastern.edu`, `neu.edu`, and their subdomains. Any other address in a Banner field is omitted.

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

`--read-only` hides every write tool. `TERM_ORGANIZE_MCP_READONLY=1` does the same thing. Clients then see only the catalog tools plus `list_plans` and `get_plan`.

## Where plans are stored

The app and the server share one file, `user.json`, through `crates/core`. A native build and an AppImage use `$XDG_DATA_HOME/termorganize`, or `~/.local/share/termorganize` when `XDG_DATA_HOME` is unset. The Flatpak build writes under `~/.var/app/org.termorganize.TermOrganize/data/termorganize`, which is `$XDG_DATA_HOME/termorganize` inside the sandbox.

When `TERMORGANIZE_DATA` is unset, the server picks whichever of those directories already contains `user.json`. If both do, the newer file wins. If neither does, it uses the native path.

Override that choice in this order:

1. `--data-dir PATH`
2. `TERM_ORGANIZE_MCP_DATA` (this does not redirect catalog loading)
3. `TERMORGANIZE_DATA` (the app override; also the directory the server uses when the two flags above are unset)
4. The Flatpak versus XDG choice above

Writes take `expected_revision`. The revision lives in `planRevisions` on the user file, next to the degree plan, and increases when the plan body changes. A save from the open app bumps it too, so a stale `expected_revision` fails with `revision_conflict` and the file is left as it was. Each save copies the previous `user.json` to `user.json.bak` (one rolling backup), writes `user.json.tmp`, and renames that into place.

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
{ "error": "revision_conflict", "revision": 2, "message": "The plan changed. Reload it and try again." }
```

`error` is `revision_conflict`, `not_found`, `rejected`, `confirm_required`, or `io`.

Times use `to` between the start and the end (`9:00 AM to 10:05 AM`). Credits use `3 to 4` when the course has a range.

### list_terms

No parameters.

```json
{ "terms": [{ "code": "202730", "name": "Spring 2027" }] }
```

Newest term first.

### search_courses

`term` (name or Banner code), optional `query`, `subject`, `number`, `attribute`, `limit` (default 25, at most 100).

```json
{
  "term": { "code": "202710", "name": "Fall 2026" },
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

`email` is omitted when the Banner field has no Northeastern address. Day letters are `U M T W R F S` (Tuesday is `T`, Thursday is `R`).

### check_schedule

`term`, `crns` (array of CRN strings).

```json
{
  "term": { "code": "202710", "name": "Fall 2026" },
  "clashes": [{ "first": "11111", "second": "22222", "reason": "The meetings overlap." }],
  "unknown_crns": ["00000"]
}
```

### suggest_schedule

`term`, `courses` (each `{ "subject", "number", "crn"? }`), optional `constraints`:

```json
{ "no_classes_before": "9:30 AM", "no_classes_after": "17:00", "days_off": ["Friday"] }
```

Times accept `9:30`, `09:30`, `9:30 AM`, and `17:00`. Days accept full names and the letters above. Constraints are required: sections outside them are dropped. Up to 8 schedules come back, best first.

```json
{
  "term": { "code": "202710", "name": "Fall 2026" },
  "schedules": [{ "crns": ["12345"], "sections": [] }]
}
```

Each section includes the get_course fields plus `subject` and `number`.

### reorganize_plan

`plan.terms[]` with `term` and `courses` (`subject`, `number`, optional `crn`).

The core library's existing reorder only moves blocking courses out of one semester, and it can add a semester. This tool is a separate whole-plan pass in `crates/core/src/reorganize.rs`. It moves a course into another term the plan already has when the bundled catalogs show the current term does not teach it, or when chosen CRNs overlap. It does not append a term after the last one, so the plan does not grow. Offering history is the two bundled snapshots only. When a term has no chosen CRNs and at most six courses, the schedule generator looks for a clash and the pass tries to move one side.

```json
{
  "plan": { "terms": [{ "term": "Fall 2026", "courses": [] }] },
  "moves": [{ "subject": "CS", "number": "2510", "from": "Fall 2026", "to": "Spring 2027", "reason": "CS 2510 has no sections in Fall 2026." }],
  "notes": []
}
```

A CRN that belongs to the old term is cleared when the destination catalog does not have that section.

### list_plans

```json
{
  "plans": [{
    "id": "uuid",
    "name": "My plan",
    "terms": [{ "name": "Fall 2026", "code": "202710" }],
    "updated_at": "2026-10-20T00:00:00Z",
    "revision": 1
  }]
}
```

`code` is null when that season and year are not one of the bundled catalogs.

### get_plan

`plan_id`.

```json
{
  "id": "uuid",
  "name": "My plan",
  "revision": 1,
  "updated_at": "2026-10-20T00:00:00Z",
  "terms": [{
    "name": "Fall 2026",
    "code": "202710",
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
    "unoffered": [{ "term": "Fall 2026", "subject": "CS", "number": "2510" }]
  }
}
```

`section` is the get_course section object, or null when the stored CRN is not in that term. A course the user typed that is not a catalog course has `label` and `offered: null`.

### create_plan

`name`, optional `terms` (array of term names or Banner codes, such as `"Fall 2026"` or `"202710"`). Returns the get_plan object. The first saved degree plan is the primary plan, matching the app.

### rename_plan

`plan_id`, `name`, `expected_revision`. Returns the get_plan object.

### add_course

`plan_id`, `term`, `subject`, `number`, optional `crn`, `expected_revision`. The term must already be on the plan. The course and CRN must exist in the bundled catalog for that term. Returns the get_plan object.

### remove_course

`plan_id`, `term`, `subject`, `number`, `expected_revision`. Returns the get_plan object.

### move_course

`plan_id`, `subject`, `number`, `from_term`, `to_term`, `expected_revision`. The destination must be on the plan and must offer the course in a bundled catalog. A CRN that is not a section there is cleared. Returns the get_plan object.

### set_section

`plan_id`, `term`, `subject`, `number`, `crn`, `expected_revision`. The CRN must be a section of that course in the bundled catalog. Returns the get_plan object.

### apply_reorganize

`plan_id`, `expected_revision`. Runs the same pass as `reorganize_plan` and, when something moved, saves it. The response is the get_plan object plus `moves` and `notes`. When nothing moved, the file and the revision stay as they were.

### delete_plan

`plan_id`, `expected_revision`, `confirm`. `confirm` must be true. Otherwise the tool returns `confirm_required` and does not write. Success:

```json
{ "deleted": true, "plan_id": "uuid" }
```
