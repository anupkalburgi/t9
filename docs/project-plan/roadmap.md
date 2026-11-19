# Roadmap

Every milestone builds on the same core (`t9-core::Store`) and keeps the storage format.

| Milestone | Theme | Status |
|---|---|---|
| M0 | Core and CLI | Done |
| M1 | Agent integration: Claude Code plugin, `AGENTS.md` | Done |
| M1+ | Decision log, git history, terminal UI, release binaries and installer | Done |
| M2 | Local server (`t9 serve`) and a live TUI | Next |
| M3 | MCP server, importing plans, decision mining, hygiene | Planned |
| M4 | Semantic search and a web UI | Planned |
| M5 | Packaging and reach | Planned |

## M2: local server

- `t9 serve` on `127.0.0.1` and a Unix socket, with a bearer token in `~/.t9/.state/token`.
- A REST API mirroring the CLI's JSON shapes: projects, context, items (`If-Match: rev`), entries, sessions,
  decisions, search.
- Server-sent events for item, entry and session changes.
- A file watcher (FSEvents/inotify), so edits made in an editor appear at once.
- Start at login (launchd).
- The TUI switches its data layer to the server: instant updates instead of 2-second polling.
- TUI additions: decision form with alternatives, areas and scope; an item field editor; a sessions view.

## M3: MCP, import, hygiene

- `t9 mcp`: a stdio MCP server exposing context, add, log, status, decide, reject, check, search and show.
- `t9 import FILE`: turn an existing plan document (headings, checklists, "to revisit" lists) into items, with a
  preview.
- Decision mining: at session end, read the transcript and propose unrecorded decisions as `proposed` items for
  the human to accept in the TUI.
- `t9 move`, `t9 rm` (to a trash folder), `t9 doctor --fix`, interactive triage.

## M4: semantic search and web UI

- Optional local embeddings in `sqlite-vec` with hybrid BM25 + vector ranking, behind a cargo feature.
- Better decision recall when different words are used for the same topic.
- A web UI on the M2 API.

## M5: packaging and reach

- A Homebrew tap and signed macOS binaries (release binaries, a curl installer and Linux CI are done).
- More harnesses (Codex hooks when available, Cursor, Gemini CLI, …).
- Optional automatic push of the vault to a git remote, to sync between machines.
