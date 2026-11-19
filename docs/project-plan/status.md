# Status

Version 0.1.0. Usable day to day on macOS; Linux is expected to work but is not tested yet.

## What exists

| Area | What you get |
|---|---|
| Storage | Markdown vault in `~/.t9` (projects, items, daily journals, sessions, inbox) with YAML front matter; SQLite index with FTS5, rebuildable from the files; every change committed to git in the background |
| Projects | Found from the directory (`.t9` pin file, registered roots, git remote), so worktrees and clones resolve automatically; suggestions in unlinked directories |
| Items | plan, phase, feature, task, bug, regression, revisit, idea, question, decision, note; a tree via `parent`, typed links, priorities, tags, areas, optimistic concurrency |
| Decision log | Decisions with reasons and rejected alternatives, rejections, proposals, supersede chains, scope (`--for`), areas, a global log; recall in the brief, on focus, with `t9 check`, and on every message in Claude Code |
| Journal and sessions | Progress, findings, blockers, notes, handoffs, automatic status-change entries; sessions with focus; triage of unfiled and inbox entries |
| Brief | `t9 context`: a token-budgeted summary for agents at session start |
| TUI | `t9 tui`: an overview for humans (waiting on you, latest change, handoff, in progress, plans, issues, decisions), plan tree, decision log, journal, triage, search; records decisions, items, notes and status; live refresh |
| Integrations | Claude Code plugin (SessionStart, UserPromptSubmit and SessionEnd hooks, plus a skill); an `AGENTS.md` section for Codex and others |
| Quality | 37 tests: unit, TUI rendering and key handling, and end-to-end CLI tests covering resolution, inbox adoption, hand edits, lossless reindex, concurrent writers, the Claude hooks, background commits and the decision log |

Performance on a 2,000-item, 50,000-entry project: `t9 log` takes about 3 ms in-process, `t9 context` 17 ms, and
the message hook 3 to 6 ms. Peak memory is 16 MB for the CLI and 27 MB for the TUI. Details are in
[performance.md](../design/performance.md).
