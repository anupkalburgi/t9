# Design rationale

Why t9 is built the way it is. For how it works, see [architecture.md](architecture.md).

## The problem

Long projects span days and many agent sessions. Context is lost at compaction and between sessions, and the
project's real state ends up scattered: plan documents with hand-maintained "to revisit" lists, notes in an
`AGENTS.md`, log files, and the human's memory. Nothing ties a note to the plan item it belongs to, records that
a decision replaced an earlier one, or tells a fresh session where things stand. Decisions made in chat are
forgotten and get argued again; rejected ideas get proposed again.

## Goals

- **Any harness.** Claude Code, Codex, and anything that can run a shell command.
- **Fast and small.** Writes in a few milliseconds, small memory, scales to large projects.
- **Plain files you own.** Readable and editable without t9, diffable, versioned.
- **Low friction.** The agent is never blocked; unfiled work is surfaced for triage instead of rejected.
- **The right context at the right time.** A short brief at session start, and the relevant past decisions
  when their topic comes up.

Non-goals for now: multiple users, cloud sync (git covers backup), replacing an issue tracker, orchestrating
agents.

## Key decisions

Each entry gives the choice, the reason, and the main alternatives that were rejected.

**1. A CLI that works without a daemon.** Every harness can run shell commands; nothing has to be running; a
crashed server can never lose an agent's update. The CLI writes the vault and index directly.
*Rejected:* an HTTP API as the primary interface (needs a running process and a client in every harness); MCP
only (not every harness supports it).

**2. Markdown files are the source of truth; SQLite is a cache.** Files are readable, editable in any editor or
Obsidian, diffable and versioned, and outlive the tool. The index (with FTS5 search) is rebuilt from them at any
time. *Rejected:* SQLite as the primary store (opaque to humans and editors); JSON files (not pleasant to read).

**3. One item type with a kind, arranged in a tree.** Plans, phases, features, tasks, bugs, regressions,
revisits, ideas, questions, decisions and notes share one shape; the tree (`parent`) gives the hierarchy and
typed links give the rest. Projects and agents vary too much for a fixed Plan → Feature → Task schema.

**4. The project comes from the directory.** A `.t9` pin file, registered roots, then the git remote, so
worktrees and clones resolve to the same project with no setup. Unlinked directories still accept writes (they
go to an inbox) and the brief suggests the likely project.

**5. Soft enforcement.** Writes never fail because something isn't set up. Instead, unfiled entries, inbox
entries and stale items show up in the brief, `t9 triage` and `t9 doctor`.

**6. A token-budgeted brief.** `t9 context` puts the most valuable information first (handoff, focus, plan, open
issues, decisions) and cuts to a budget, so it is safe to inject into every session.

**7. A decision log with recall.** Decisions carry their reasons and rejected alternatives; rejections are
first-class; changes of mind supersede and link. Recall is by scope (the item or its ancestors), area and text,
in the brief, on `t9 focus`, through `t9 check`, and on every user message in Claude Code. Agents may create
plans and accept decisions on their own; the human reviews and can reverse them.
*Rejected:* decisions as free-form notes (not recallable); asking the human to approve every decision (too much
friction).

**8. Git history of the vault, committed in the background.** Every change is recoverable and reviewable, with no
cost to write latency: a detached committer waits briefly, so bursts become one commit.

**9. Cheap sync instead of scanning.** Filesystem calls can be slow (about 30 µs per `stat` on a machine with
endpoint security), so reads check directory mtimes, recent journals and the item being read, plus a full check
every 5 minutes. *Rejected:* scanning every file on every command (80 ms on 2,000 files).

**10. The TUI runs in-process.** It reads the store directly and refreshes every 2 seconds, so it works now without
a server. All its data access is in one module, ready to move to a server later.

**11. Times are stored in UTC and shown in local time** on a 12-hour clock with the zone (`8:11 PM EST`).

## Open questions

- Which harnesses beyond Claude Code and Codex need first-class support?
- Should the brief include anything from other projects beyond the global decision log?
- When does semantic search (embeddings) become worth its size and complexity over lexical recall?
