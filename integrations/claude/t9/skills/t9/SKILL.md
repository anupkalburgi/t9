---
name: t9
description: Shared, persistent project memory through the `t9` CLI: the project's About, decision log, experiments, resources (datasets, models, services …) and history. Use throughout any coding session where a t9 brief appeared at session start, whenever the user makes or rejects a choice (architecture, testing, tooling, product, process), before proposing an approach or asking the user to choose between options, and whenever the work involves the plan, phases, features, tasks, bugs, regressions, things to revisit, ideas, open questions, progress notes, or handing off to a later session. Record these with t9 instead of only in chat; run `t9 context` when resuming work.
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
- If the brief says the folder is **not linked to a t9 project**, say so to the user in your **first reply**,
  before other work: offer the suggested `t9 project init --name "…" --key …` from the brief (they may pick another
  name or key), or `t9 project link KEY` for a similar existing project it lists. Run it once they agree. Do not
  create or link a project without asking, and do not stay silent about it.

## Project context: About and resources

- If the brief says **About (not written yet)**, draft it from the repository (README, AGENTS.md, manifests such
  as Cargo.toml, package.json or pyproject.toml), show the user, and record it once they agree:
  `t9 about set what "…"`, then `t9 about add "…" --section stack|run|architecture|data|terms|constraints|links`.
- When you learn a durable fact about the project (how to run something, a constraint, a term), add it:
  `t9 about add "integration tests need Docker running" --section run`.
- When you work with a **resource**, record it once: datasets, tables, pipelines, dashboards, notebooks, models,
  endpoints, prompts, evals, indexes, features, services, APIs, databases, queues, buckets, environments.
  `t9 add resource "Invoices table" --type table --at analytics.billing.invoices --owner data-eng -b "one row per
  invoice; refreshed hourly; amounts in cents"`. Check `t9 resources` first.
- **Never record credentials.** Record where they are kept ("in 1Password: billing/prod-db", "env var
  DB_PASSWORD"). t9 refuses text that looks like a secret.

## History: consult it before planning

Before planning any non-trivial approach, and whenever the user asks "should we…", "why don't we…", "what about…":

```sh
t9 history "webhook batching"      # everything on the topic, oldest first
t9 timeline --days 14              # what happened recently, and in what order
```

Build the plan from what comes back, and cite ids:
- follow accepted decisions; don't re-propose rejected options without new information;
- don't repeat a **failed experiment** unless what made it fail has changed (its `retry when` condition, or a
  passed retry date flagged as "worth retrying"); if it has, say so and propose the retry;
- look for **ideas** that can be combined with what is known now;
- mention dead ends the user may have forgotten.

## Experiments

When trying something whose outcome is uncertain (a performance change, a model, a prompt, a library, a schema):

```sh
t9 add experiment "Batch webhooks in groups of 50" --hypothesis "fewer 429 errors" --focus
t9 log -k finding "provider rejects batches above 10" -r runs/2026-11-18-batch/
t9 done BIL-12 -n "429s down 80%"                                       # it worked
t9 status BIL-12 failed -n "batch cap is 10" --retry-when "provider raises the cap" --retry-after 90d
t9 status BIL-12 inconclusive -n "sandbox too small to tell" --retry-when "staging data available"
```

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
| Starting a unit of work | `t9 focus BIL-12` (prints the decisions, experiments and resources that apply), or `t9 add task "…" --parent BIL-3 --focus` |
| A meaningful step, result or finding | `t9 log "…"` (attaches to the focus; `-r path` for files, commits, run folders) |
| You find a bug | `t9 add bug "…" [-l regression-of:BIL-9] [-p 1]` |
| Something worked before and broke | `t9 add regression "…" -l regression-of:BIL-9` |
| "We should come back to this" / a loose end | `t9 add revisit "…"` |
| A question only the human can answer | `t9 add question "Short question?" -b -` with the options, their trade-offs and your recommendation in the body (not in the title), plus where the evidence is (`-r` or a path) |
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
- `t9 history "…"` — everything on a topic; `t9 timeline` — what happened when; `t9 about`; `t9 resources`
- `t9 show BIL-12` — an item with its children, links and journal
- `t9 journal -n 20` — recent entries; `t9 search "…"` — full-text search (`-A` for all projects)
- `t9 triage` — entries not filed under an item; `t9 attach ENTRY BIL-12` files one
- Add `--json` to any command for machine-readable output.
