# Status

Version 0.1.5. Usable day to day. Prebuilt binaries for macOS and Linux (arm64, x86_64) install with one line;
CI tests both systems.

## What exists

| Area | What you get |
|---|---|
| Storage | Markdown vault in `~/.t9` (projects, items, daily journals, sessions, inbox) with YAML front matter; SQLite index with FTS5, rebuildable from the files; every change committed to git in the background |
| Projects | Found from the directory (`.t9` pin file, registered roots, git remote), so worktrees and clones resolve automatically; suggestions in unlinked directories |
| Items | plan, phase, feature, task, bug, regression, revisit, idea, question, decision, note; a tree via `parent`, typed links, priorities, tags, areas, optimistic concurrency |
| Decision log | Decisions with reasons and rejected alternatives, rejections, proposals, supersede chains, scope (`--for`), areas, a global log; recall in the brief, on focus, with `t9 check`, and on every message in Claude Code |
| Context and history | A per-project About (what it is, stack, how to run it, …) at the top of every brief; resources (datasets, tables, models, endpoints, services …); experiments with outcomes and retry conditions; `t9 history "topic"` and `t9 timeline`; a Timeline tab |
| Journal and sessions | Progress, findings, blockers, notes, handoffs, automatic status-change entries; sessions with focus; triage of unfiled and inbox entries |
| Brief | `t9 context`: a token-budgeted summary for agents at session start |
| TUI | `t9 tui`: an overview for humans (waiting on you, latest change, handoff, in progress, plans, issues, decisions), plan tree, decision log, journal, triage, search; records decisions, items, notes and status; live refresh |
| Web UI | `t9 web`: overview, the full plan tree, item pages with where each item came from and what happened since, answering questions (recorded as decisions), decisions, experiments, resources, timeline, search, referenced files; local only with a per-run token |
| Integrations | Claude Code plugin (SessionStart, UserPromptSubmit and SessionEnd hooks, plus a skill); an `AGENTS.md` section for Codex and others |
| Quality | 37 tests: unit, TUI rendering and key handling, and end-to-end CLI tests covering resolution, inbox adoption, hand edits, lossless reindex, concurrent writers, the Claude hooks, background commits and the decision log |

Performance on a 2,000-item, 50,000-entry project: `t9 log` takes about 3 ms in-process, `t9 context` 17 ms, and
the message hook 3 to 6 ms. Peak memory is 16 MB for the CLI and 27 MB for the TUI. Details are in
[performance.md](../design/performance.md).
