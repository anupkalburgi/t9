# Plan: goal and outcome, freshness and reminders, the Timeline

Agreed with the user after two rounds of design review (capture, UX, timeline, freshness, and a critic each
round). Decisions taken:

- **One new field for intent: `goal`** (why an item exists, what it serves). The **outcome stays the closing
  note** (`t9 done ID -n "…"`), which agents already write; it is shown prominently and classified.
- **A `due` date** on any item, set only when the human gives one.
- **Reminders reach the user through the coding agent**, at the right moments; no OS notifications.
- **The Timeline becomes a lane chart.**

## What the real data showed

- Outcomes are already captured: every closing status change in the sample vault carried a note. They were
  hidden in the panel (a two-line "status-change" entry near the bottom).
- Goals are missing: bodies say when or where, not why; the why lives in the parent's title.
- Reminders are mostly *event-triggered* ("after QR-126 merges, re-measure …"), written into titles because
  there was nowhere else for them. "Ripe" (what it waited on has closed) matters more than "overdue".
- Raw age is noise (about 45 new items a day): freshness must come from commitments: active or blocked work,
  priorities, questions, proposals, dates the human set.
- The brief's "Needs attention" section was rendered last, so the token budget cut it first.

## One "needs attention" model (`t9-core/src/attention.rs`)

Ranked once; the brief, `t9 pending`, the prompt hook, the web Overview, map badges and the timeline take the top
of it. It replaces "Watch out", "Worth retrying", the stale lines of "Needs attention" and the planned "closed
without outcome" as separate lists.

| Reason | When |
|---|---|
| `overdue` | open, active or blocked, and `due` is before today |
| `due-soon` | `due` is today or within 2 days |
| `ripe` | something it waits on (`blocked-by`, set with `--after ID`) has closed since it was last touched; or a failed or inconclusive experiment's retry date has passed |
| `loop` | the "going in circles" signals (flip-flop, reopened, repeat, spinning, stuck), severity 2 or more |
| `stale` | active and untouched for 3+ days; blocked and untouched for 5+ days |
| `waiting` | a question open for 2+ days, a proposed decision not accepted after 3+ days (waiting on the human) |
| `unexplained` | closed in the last 7 days without a note (no outcome) |
| backlog | open, no priority, untouched for 30+ days: a count only, never a list |

Ideas, revisits, resources and accepted decisions are never flagged for age. "Touched" is the later of the item's
own update and its newest journal entry.

## Capture

- `t9 add … --goal "…"`, `t9 update ID --goal "…" --due fri`, `t9 focus ID --goal "…"` (only if it has none).
- `--due` accepts `2026-12-01`, `today`, `tomorrow`, `mon`…`sun`, `3d`, `2w`, `1m`; `none` clears.
- `--after ID` on `add` and `update`: a `blocked-by` link; the item becomes ripe when ID closes.
- `t9 done ID -n "…"`: the note is the outcome: "what came out of it: result, commits, what it led to".
  Closing without a note still works, and prints one line suggesting an outcome. Closing an item that others wait
  on prints which ones are now ripe.
- `t9 still ID -n "waiting on …"`: a check-in: still relevant, resets the idle clock, does not count as progress
  (and is ignored by the going-in-circles signals). The reason is required.
- `t9 pending [--json] [-A]` (alias `outstanding`): everything that needs attention, by reason, with age, idle
  time and time in status.

## Reminders, at the right times, in the agent

Agents only run while a session is open, so "timely" means these moments:

1. **Session start and after compaction**: the brief's "Needs attention" section, moved up (after the focus) so
   the budget never cuts it, with an instruction to mention to the user what concerns them (overdue, due today,
   ripe, waiting on them).
2. **On the user's messages**: one short reminder line when something *new* needs attention: became due or
   overdue (for example on a new day), became ripe because another session closed what it waited on, a question
   has waited too long. Each (item, reason) is shown once per session per day, so it never nags; nothing is
   printed otherwise.
3. **When the agent acts**: `t9 done` prints what became ripe; `t9 focus` shows the item's due date and goal.
4. **When the human looks**: badges (`overdue 3d`, `due 2d`, `idle 6d`) and a "Needs attention" card in the web
   UI and TUI.

## Display

- Panel story header: **Goal** (stated, or "inferred from QR-3"), **Outcome** (the closing note in full,
  classified ✓ done, ✗ obsolete or dropped or failed, ⇢ combined or superseded; batch closures flagged), **Led
  to** (links both ways, and ids named in the closing note with their verb), a breadcrumb with titles, a roll-up
  of children by outcome. The derivation lives in Rust (`story.rs`) with unit tests; the browser only renders it.
- Closing from the panel asks for a reason (it used to send none).
- Map: outcome marks on nodes, quiet done and dead-end nodes, groups that say why, hover cards.
- Timeline: a lane chart: one lane per top-level plan or feature, bars from created to closed coloured by
  outcome, outcome and event marks, bursts collapsed, long quiet gaps compressed, agent-session band, due marks, a
  "now" line, a story line per day, and the same side panel as the Map.

## Releases

| Release | Scope |
|---|---|
| **0.1.8** | `goal`, `due`, `--after`; attention model; `t9 pending`, `t9 still`; brief reordered; reminders in the prompt hook and on `done`/`focus`; skill wording; panel story header and the closing-reason fix; web badges and the Needs attention card |
| **0.1.9** | The lane-chart Timeline with Story mode and the shared panel (done; the timeline is its own asset file). Still to do: split `app.js` into modules, map hover cards |
| **0.2.0** (done) | "Ask the agents": questions from the web panel addressed to agents, shown in the brief, answered with `t9 answer`; map semantic zoom |

## Not doing

New `outcome` or `result` fields, a blocking Stop hook, transcript mining, OS notifications or background jobs,
recurring items, numeric scores, flagging every old open item, LLM summaries.
