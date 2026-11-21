# Project context and history

t9 keeps four kinds of long-lived context for a project, so a new session (or a new person) can plan with the
full story instead of rediscovering it:

| What | Question it answers | Commands |
|---|---|---|
| **About** | What is this project, how is it built, how do I run it? | `t9 about`, `about set`, `about add`, `about edit` |
| **Resources** | What data, models and services does it use, and where are they? | `t9 add resource`, `t9 resources` |
| **Experiments** | What did we try, did it work, and is it worth trying again? | `t9 add experiment`, `t9 status … failed --retry-when` |
| **History and timeline** | Everything we know about this topic; what happened when? | `t9 history "topic"`, `t9 timeline` |

Together with the [decision log](decisions.md), these are what agents are told to consult before planning.

## About

The project's About lives in the body of `~/.t9/projects/<slug>/project.md`, one `## Section` per topic:

| Section | For | Aliases |
|---|---|---|
| `what` | What it is, who uses it | summary, overview, purpose |
| `goals`, `non-goals` | What it is and isn't for | |
| `architecture` | Main parts and how they fit | arch, design |
| `stack` | Languages, frameworks, services | tech, tools |
| `run` | Build, test and run | build, test, setup, dev |
| `data` | Data, models and services in broad strokes (details are resources) | datasets, models |
| `terms` | Key terms | glossary |
| `constraints` | Rules and limits | limits |
| `links` | Docs, dashboards, tickets | docs, references |

```sh
t9 about                                                   # show it
t9 about set what "Charges customers and sends invoices."  # replace a section
t9 about add "Rust, Postgres, the provider's REST API" --section stack
t9 about add "integration tests need Docker running" --section run
t9 about edit                                              # open project.md in $EDITOR
t9 about confirm                                           # still accurate: mark it reviewed
```

A compact form (one line per section) is at the top of every brief, so agents start with it. When a project is
new and its About is empty, the brief asks the agent to draft it from the repository's README and manifests and
to confirm it with you. Agents add durable facts as they learn them.

## Resources

A resource is anything the project works with that lives somewhere else. Pick the type that fits:

| Discipline | Types |
|---|---|
| Data | `dataset`, `table`, `pipeline`, `dashboard`, `notebook` |
| ML | `model`, `dataset`, `feature`, `notebook`, `eval` |
| AI | `model`, `endpoint`, `prompt`, `eval`, `index` |
| Backend | `service`, `api`, `database`, `queue`, `bucket`, `environment` |
| Everyone | `repo`, `doc`, `other` |

```sh
t9 add resource "Invoices table" --type table --at analytics.billing.invoices --owner data-eng \
   -b "One row per invoice; refreshed hourly; amounts in cents"
t9 add resource "Churn model v3" --type model --at "registry: models:/churn/3" -b "XGBoost; AUC 0.87 on Oct data"
t9 add resource "Payments provider API" --type api --at https://api.provider.example/v2 --owner payments-team
t9 resources                    # all of them; --type model for one type
```

Resources appear in the brief, in `t9 history`, and in the recall on focus and on each message, so mentioning
"the invoices table" brings back what is known about it.

**t9 never stores credentials.** It refuses text that looks like a password, key or token, and asks you to record
where the secret is kept instead (`"in 1Password: billing/prod-db"`, `"env var DB_PASSWORD"`).

## Experiments

```sh
t9 add experiment "Batch webhooks in groups of 50" --hypothesis "fewer 429 errors" --focus
t9 log -k finding "provider rejects batches above 10" -r runs/2026-11-18-batch/
```

Then record the outcome:

| Outcome | Command |
|---|---|
| It worked | `t9 done BIL-12 -n "429s down 80%"` (status `done`; `succeeded` is accepted too) |
| It failed | `t9 status BIL-12 failed -n "batch cap is 10" --retry-when "provider raises the cap" --retry-after 90d` |
| Can't tell yet | `t9 status BIL-12 inconclusive -n "sandbox too small" --retry-when "staging data available"` |

`--retry-after` takes a date (`2026-12-01`) or a delay (`30d`, `6w`, `3m`); `--retry-after none` clears it. A
failed or inconclusive experiment whose retry date has passed is listed as **ripe** under **Needs attention** in
the brief, `t9 pending` and the TUI, until it is retried or checked in (`t9 still`). Experiments are part of recall, so an agent about to try the same thing again is reminded of what
happened last time and why.

## History

```sh
t9 history "webhook batching"
```

```
2026-10-02  idea          BIL-10   [open] Batch webhooks to cut API calls
2026-10-09  experiment    BIL-12   [failed] Batch webhooks in groups of 50 — hypothesis: fewer 429 errors · retry when: provider raises the cap   ← retry date 2026-11-01 has passed
2026-10-09  finding       (BIL-12) provider rejects batches above 10
2026-10-09  status-change (BIL-12) BIL-12: active → failed — batch cap is 10
2026-10-10  decision      BIL-13   [accepted] Send webhooks one at a time — batches above 10 are rejected
2026-11-14  finding       —        provider changelog: batch limit raised to 100
```

It searches every kind of item and the journal, keeps what matches at least half of the topic's words, adds each
matched decision's supersede chain and each matched item's status changes, and sorts it oldest first. `-A`
searches every project; `--json` returns the events for agents.

Agents are told to run it before planning anything non-trivial, and to plan from it: follow decisions, don't
repeat failures unless what made them fail has changed, retry what has become worth retrying, combine old ideas
with what is known now, and cite the ids.

## Timeline

```sh
t9 timeline                      # last 30 days, newest first, grouped by day
t9 timeline --days 90 --topic "invoices"
```

It lists what changed the project's course: work starting and finishing, decisions and rejections, experiment
outcomes, blockers, findings, handoffs, and new plans, features, experiments, resources, bugs and questions. The
TUI shows the last 60 days in tab **6 Timeline**.

## Going in circles

```sh
t9 insights        # or: t9 loops
```

t9 looks at the project's history for signs that work is going round and round:

| Signal | What it means |
|---|---|
| **flip-flop** | A decision changed two or more times (a supersede chain of three or more). |
| **reopened** | Something was closed and then opened again. |
| **repeat** | Open work (or a new decision) that resembles an earlier dead end: a failed or inconclusive experiment, a rejected option, a dropped item, a superseded decision. |
| **spinning** | A branch of the plan with many entries in the last 7 days but nothing under it finished, or new issues opening faster than they close. |
| **stuck** | Work active for 5 days or more, with many entries, and not finished. |

The strongest signals appear in the agent's brief and `t9 pending` under "Needs attention" (as `loop`), and in
the web **Map** panel, which marks spinning branches with ↻ and dead ends with dashed red edges.

## Goal and outcome

Every item answers two questions:

- **Goal**: why it exists, what it serves. Set it with `t9 add … --goal "…"`, `t9 focus ID --goal "…"` (recorded
  if it has none) or `t9 update ID --goal "…"`. When an item has none, t9 shows its nearest ancestor's goal, or
  its parent's title, labelled as inferred.
- **Outcome**: what came out of it: the note you close it with, `t9 done ID -n "result; commits; what it led
  to"`. t9 reads it: ✓ done, ⇢ combined into, duplicate of or superseded by another item, ✗ dropped, obsolete or
  failed, ~ inconclusive; ids it names become "led to" links. Closing without a note works, but t9 suggests
  recording one, and the item is listed as closed without an outcome for a week.

The web UI shows both at the top of every item (panel and page), with what it led to and, for a feature or plan,
how its children turned out.

## What needs attention, and reminders

`t9 pending` (and the brief's **Needs attention** section) is one ranked list:

| Reason | When |
|---|---|
| `overdue`, `due-soon` | a `--due` date has passed, or is within 2 days |
| `ripe` | what it waits on (`--after ID`, a `blocked-by` link) has closed since it was last touched; or an experiment's retry date has passed |
| `loop` | the going-in-circles signals (`t9 insights`) |
| `waiting` | a question open 2+ days; a proposed decision not accepted after 3+ days |
| `stale` | active and untouched 3+ days; blocked and untouched 5+ days |
| `unexplained` | closed in the last week without an outcome |

Old open backlog (no priority, untouched 30+ days) is only counted. Ideas, revisits, resources and accepted
decisions are never flagged for age. `t9 still ID -n "waiting on …"` says an item is still needed (it resets the
idle clock without counting as progress); drop what is no longer needed.

Set `--due` only when the user gives a date; use `--after ID` for "once X is done".

**Reminders reach the user through the agent.** At session start the brief lists what needs attention and tells
the agent to mention what concerns the user. On each message, a hook adds a short "t9 reminder" when something
*new* comes up (it became due or overdue, it became ripe because another session closed what it waited on, a
question has waited too long); each item and reason is shown once per session per day. `t9 done` also says which
items are ripe now. In the web UI and TUI the same list is a "Needs attention" card, with badges on items.

