---
name: t9
description: Shared, persistent project memory through the `t9` CLI, including the project's decision log. Use throughout any coding session where a t9 brief appeared at session start, whenever the user makes or rejects a choice (architecture, testing, tooling, product, process), before proposing an approach or asking the user to choose between options, and whenever the work involves the plan, phases, features, tasks, bugs, regressions, things to revisit, ideas, open questions, progress notes, or handing off to a later session. Record these with t9 instead of only in chat; run `t9 context` when resuming work.
---

# t9: project memory that outlives the conversation

t9 keeps a project's plan, open items, **decision log** and progress journal outside the chat, so the next
session (yours after compaction, another agent's, or the human's review) starts from facts instead of a
reconstruction. Everything is plain Markdown in `~/.t9`, committed to git automatically; `t9` is the only
interface you use.

## At the start of a session

- The brief usually arrives automatically (SessionStart hook). If it did not, run `t9 context`.
- Continue from **Last handoff** and **Current focus**. Mention the item ids you are picking up.
- Respect **Decisions that apply to your focus**, **Rules for every project** and **Ruled out**. Do not re-open
  them unless the user does or you have new information; if you do, say which decision you are questioning.
- If the brief says the directory is **not linked to a project**: show the user the suggested projects and ask
  which one this work belongs to. Then run `t9 project link KEY`, or `t9 project init --name "Name" [--key XY]`
  for a new project. Do not pick silently.

## Decisions: the most important part

Choices made in chat are lost unless recorded, and then get re-litigated in the next session.

**Before** you propose a design, pick a library or tool, or ask the user to choose between options:

```sh
t9 check "search engine tantivy"     # decided? rejected? parked?
```

If something comes back, follow it and cite its id ("per BIL-12 we use FTS5"). Messages from the user may
also arrive with "t9: earlier decisions that may bear on this message" added by a hook: treat those the same way.

**When** the user decides something, or says no to something, record it **immediately**, then mention the id:

| The user… | Command |
|---|---|
| Chooses an option | `t9 decide "Use FTS5 for search" --because "no extra process" -r "tantivy: separate index, +4 MB" --area search` |
| Rules an option out | `t9 reject "Use a daemon for all writes" --because "CLI must work with nothing running" --area architecture` |
| Changes an earlier decision | `t9 decide "Use tantivy after all" -s BIL-12 --because "need fuzzy search"` |
| Decides for one part of the plan | add `--for BIL-3` (applies to BIL-3 and everything under it) |
| States a rule for all projects ("always use uv", "never deploy without asking") | add `--global` |
| Floats an idea to look at later | `t9 add idea "Trigram tokenizer for identifiers" --area search` |
| Has not decided yet, but you recommend something | `t9 decide "…" --proposed --because "…"` |

Signals: "let's go with", "use X instead", "no", "don't", "never", "always", "from now on", "rather than",
picking one of the options you offered. Areas are short topics: `architecture`, `testing`, `tooling`,
`storage`, `api`, `ui`, `product`, `process`, or whatever the project uses consistently.

## While working

Write a few entries per hour of work, not one per tool call. One or two sentences each, specific.

| When | Command |
|---|---|
| Starting a unit of work | `t9 focus BIL-12` (prints the decisions that apply), or `t9 add task "…" --parent BIL-3 --focus` |
| A meaningful step, result or finding | `t9 log "…"` (attaches to the focus; `-r path` for files, commits, run folders) |
| You find a bug | `t9 add bug "…" [-l regression-of:BIL-9] [-p 1]` |
| Something worked before and broke | `t9 add regression "…" -l regression-of:BIL-9` |
| "We should come back to this" / a loose end | `t9 add revisit "…"` |
| A question only the human can answer | `t9 add question "…"` |
| Blocked | `t9 status BIL-12 blocked -n "waiting on …"` |
| Finished | `t9 done BIL-12 -n "how it was verified"` |

Structure big work as a tree with `--parent`: plan → phase → feature → task. Give features and tasks an
`--area` so decisions in that area are recalled when they are focused.

## Before you stop

1. Sweep: were any choices made or options rejected in this session that are not recorded yet? Record them.
2. `t9 handoff "where I stopped; what is next (ids); open risks"`.

## Rules

- Look before adding: `t9 ls --tree`, `t9 search "…"`, `t9 check "…"`. Reuse existing ids.
- Quote ids (BIL-12) in your replies so the human can look them up.
- Reference large outputs by path (`-r local/runs/…`); never paste them into entries.
- For long text pass `-` and pipe it in: `t9 log - <<'EOF' … EOF`, `t9 decide "…" -b - <<'EOF' … EOF`.
- Do not edit vault files directly; use `t9` (the human may edit them by hand; t9 picks that up).
- If a `t9` command fails, say so briefly and carry on with the user's task. t9 must never block the work.

## Reading

- `t9 context` — the brief; `t9 ls --tree` — open items as a tree (`--all` includes closed)
- `t9 decisions [--area A] [--global] [--md]` — the decision log; `t9 check "…"` — decided before?
- `t9 show BIL-12` — an item with its children, links and journal
- `t9 journal -n 20` — recent entries; `t9 search "…"` — full-text search (`-A` for all projects)
- `t9 triage` — entries not filed under an item; `t9 attach ENTRY BIL-12` files one
- Add `--json` to any command for machine-readable output.
