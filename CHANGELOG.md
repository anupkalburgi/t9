# Changelog

## 0.1.7

- **Map:** explore without leaving it. Clicking a node opens it in a panel beside the map (description, what is
  inside it with progress, what applies from history, latest entries) and highlights its path back to the
  project and everything under it. You can change its status, add a note or answer a question there. Ids in the
  panel select on the map and open their group. Double-click for the full page; "show on the map" on every item
  page brings you back to the same spot. Drag to move, ⌘-scroll to zoom, Esc to deselect; the map keeps its
  position and selection when you come back.

## 0.1.6

- **Map:** a new web page with the project at the centre and its plans, features and work branching out. Leaves
  are grouped by outcome (in progress, open issues, questions, experiments, decisions, ideas, dead ends, done);
  click a group to open it, a node for its page. Dead ends are drawn with dashed red edges; ↻ marks a spinning
  branch. Zoom, fit, expand all and collapse all.
- **Are we going in circles?** `t9 insights` (alias `loops`) and the Map and Overview pages flag decisions that
  changed two or more times, work that was closed and reopened, new work that resembles an earlier dead end
  (failed experiment, rejected option, dropped item, superseded decision), branches with lots of activity but
  nothing finished or issues opening faster than they close, and work active for days without finishing.
- The agent brief has a "Watch out: signs of going in circles" section, so agents notice too.

## 0.1.5

- **Web UI:** `t9 web [ID]` opens a local browser UI: an overview, the full plan as a tree with progress and
  filters, and a page for every item with its place in the plan, its whole history, where it came from (when, by
  which agent and session, what was happening just before) and what happened since. Questions can be answered
  there; the answer closes the question and is recorded as a decision. Also decisions, experiments, resources,
  timeline, journal, About and search, and referenced files rendered in place. Refreshes as agents work.
  Local only: 127.0.0.1, a per-run token in an HttpOnly cookie, Host checks, a strict CSP.
- Agents are told to keep question titles short and put the options, trade-offs and a recommendation in the body.

## 0.1.4

- TUI: every row shows how recently it changed: ● bright green (15 min), ● green (1 hour), • yellow (today),
  · grey (this week). The most recently touched item is tagged **NEW**. A legend sits at the top of the
  Overview. Journal rows use a compact time and fit the pane.

## 0.1.3

- **Project context:** each project has an About (what it is, goals, architecture, stack, how to run it, key
  terms, constraints, links) in `project.md`, shown at the top of every brief. `t9 about`, `about add`,
  `about set`, `about edit`, `about confirm`. Agents draft it from the README for new projects.
- **Resources:** `t9 add resource "…" --type dataset|table|model|endpoint|service|… --at LOCATION --owner O`
  and `t9 resources`. t9 refuses text that looks like a credential.
- **Experiments:** `t9 add experiment "…" --hypothesis "…"`; outcomes `done`, `failed`, `inconclusive` with
  `--retry-when` and `--retry-after`; "Worth retrying" in the brief and the TUI.
- **History and timeline:** `t9 history "topic"` (everything on a topic, oldest first) and `t9 timeline`
  (what happened when); a Timeline tab in the TUI. Recall on focus and on each message now includes experiments
  and resources.
- The skill and the AGENTS.md section tell agents to consult history before planning.

## 0.1.2

- Folders where an agent works without a t9 project are no longer silent: the brief tells the agent to raise it
  in its first reply with a ready-made `t9 project init --name … --key …`, the Claude Code message hook reminds
  once per session, and the folder shows as "not set up" in `t9 tui` (press `i` to create the project there),
  `t9 project ls` and `t9 doctor`.
- Fix: entries written in the same millisecond could sort out of order (seen as the wrong LATEST change).
  Ids now come from a monotonic ULID generator.
- The installer adds `~/.local/bin` to your `PATH` (zsh, bash, fish, or `~/.profile`) when it isn't there
  already; `T9_NO_MODIFY_PATH=1` skips this.

## 0.1.1

- Prebuilt binaries for macOS (Apple Silicon, Intel) and Linux (x86_64, ARM64), and a one-line installer:
  `curl -fsSL https://raw.githubusercontent.com/anupkalburgi/t9/main/install.sh | sh`.
- Fix: concurrent writers could fail with a duplicate-id error while indexing the same journal entry. Index
  writes are now atomic (the index is rebuilt automatically on first run).
- Fix: `t9 … | head` no longer reports a broken pipe.
- CI now also runs on Linux.

## 0.1.0

First release.

- **CLI** (`t9`): projects resolved from the directory (including git worktrees), items in a tree with typed
  links, journal entries, sessions with focus, handoffs, triage, full-text search, `doctor`, `--json`
  everywhere.
- **Vault** in `~/.t9`: Markdown with YAML front matter as the source of truth, a rebuildable SQLite index, and
  automatic git commits in the background.
- **Brief** (`t9 context`): a token-budgeted summary for agents at session start.
- **Decision log**: decisions with reasons and rejected alternatives, rejections, proposals, supersede chains,
  scope and areas, a global log; recalled in the brief, on focus, with `t9 check`, and on every user message in
  Claude Code.
- **TUI** (`t9 tui`): an overview for humans, plan tree, decision log, journal, triage and search, with live
  refresh.
- **Integrations**: a Claude Code plugin (hooks and skill) and an `AGENTS.md` section for Codex and others.
