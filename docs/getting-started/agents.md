# Getting started: for agents

You are a coding agent working on a project that uses t9. t9 is the project's memory outside your
conversation: the plan, open items, decisions and a journal of progress. It is how you learn where things stand
when a session starts, and how the next session (yours after compaction, another agent's, or the human's review)
learns what you did. You use it through the `t9` CLI. Terms are defined in [concepts.md](concepts.md).

## The protocol

### 1. At the start of a session

The Claude Code plugin prints the brief automatically. In other harnesses, run:

```sh
t9 context
```

Read it and continue from the **Last handoff** and **Current focus**. Tell the user which item ids you are
picking up.

If the brief says **this folder is not linked to a t9 project**, tell the user in your first reply, before other
work. The brief gives a ready-made command with a suggested name and key; offer it, or link a similar existing
project it lists. Run it once they agree; don't create or link one silently:

```sh
t9 project link BIL                                   # existing project
t9 project init --name "Billing Service" --key BIL    # new project
```

If the brief says **About (not written yet)**, draft it from the README and manifests, show the user, and record
it once they agree (`t9 about set what "…"`, `t9 about add "…" --section stack`). See
[context-and-history.md](context-and-history.md).

### 2. When you start a unit of work

Before planning anything non-trivial, consult the history and plan from it, citing ids:

```sh
t9 history "webhook batching"   # decisions, rejected options, experiments and outcomes, ideas, resources
t9 timeline --days 14
```

Don't repeat a failed experiment unless what made it fail has changed; retry what has become worth retrying;
combine old ideas with what is known now.

Look for an existing item before creating one:

```sh
t9 ls --tree
t9 search "decimal temp table"
```

Then focus it, or create it and focus it. Focusing moves an `open` item to `active`, and later logs attach to it:

```sh
t9 focus BIL-12
t9 add task "Pass temp-table schemas to the binder" --parent BIL-3 --focus
```

### 3. While you work

Record a few entries per hour of work, not one per tool call:

| When | Command |
|---|---|
| A meaningful step or result | `t9 log "Webhook handler is idempotent; replayed 500 events, no duplicates"` |
| Something you learned | `t9 log -k finding "The provider rounds each line item, not the invoice total"` |
| Evidence exists in files | `t9 log "…" -r runs/2026-11-18-replay/ -r abc1234` |
| You found a bug | `t9 add bug "Partial refunds fail with 422 from the provider" --parent BIL-3 -p 1` |
| Something that worked broke | `t9 add regression "…" -l regression-of:BIL-31` |
| A loose end or shortcut | `t9 add revisit "Currency conversion runs twice on retried invoices"` |
| Only the human can answer | `t9 add question "Keep supporting the v1 invoice format?"` |
| Before proposing an approach or asking the user to choose | `t9 check "topic"`. Follow what was decided and cite the id. |
| The user chooses something | `t9 decide "Use FTS5" --because "no extra process" -r "tantivy: extra index" --area search` |
| The user rules something out | `t9 reject "Daemon for all writes" --because "CLI must work alone" --area architecture` |
| The user changes direction | `t9 decide "Drop the legacy XML export" -s BIL-9 --because "no customer has used it in a year"` |
| The user states a rule for all projects | `t9 decide "Always use uv for Python" --global` |
| An idea for later | `t9 add idea "Trigram tokenizer for identifiers" --area search` |
| You try something uncertain | `t9 add experiment "…" --hypothesis "…" --focus`; then `t9 done ID` or `t9 status ID failed -n "why" --retry-when "…"` |
| You use a dataset, model, endpoint, service … | `t9 add resource "…" --type table --at analytics.billing.invoices -b "schema, freshness, quirks"` (never credentials) |
| You learn a durable fact about the project | `t9 about add "…" --section run` |
| You're blocked | `t9 status BIL-12 blocked -n "waiting for provider sandbox access"` |
| You're done | `t9 done BIL-12 -n "verified: 200/200 sample invoices match"` |

### 4. Before you stop

Also do this whenever the user says to wrap up, or before a long, risky operation. First, record any choices or
rejections from this session that aren't in the decision log yet. Then:

```sh
t9 handoff "Stopped after the webhook replay. Next: BIL-14 (partial refunds). Risk: the provider sandbox allows 100 requests/min."
```

A good handoff says where you stopped, what comes next (with ids), and any open risks.

## Decisions

Decisions made in chat are lost unless recorded, and then get argued again in the next session. The full guide is
[decisions.md](decisions.md). In short:

- The brief's "Decisions that apply to your focus", "Rules for every project" and "Ruled out" sections are binding
  unless the user reopens them.
- In Claude Code, a user message may arrive with "t9: earlier decisions that may bear on this message". Treat it
  the same way.
- Record a decision the moment it is made, not at the end of the session.

## Writing good entries

Be specific. Use ids and reference evidence by path:

- Good: `t9 log "Duplicate charges come from retried webhooks without an idempotency key; fix in the handler (BIL-14)" -r runs/2026-11-18-replay/`
- Too vague: `t9 log "made progress on the refund stuff"`
- Too much: pasting 200 lines of test output. Reference the file instead.

Other conventions:

- One idea per item. A bug found while doing a task is its own item, linked or parented, not a sentence in the
  task's journal.
- Quote ids (BIL-12) in your replies to the user so they can look them up.
- For long text, pass `-` and pipe it in:
  ```sh
  t9 add bug "Snippets break on CJK text" -b - <<'EOF'
  Steps: search for 漢字 in any project.
  Expected: highlighted snippet. Actual: empty snippet.
  EOF
  ```
- Don't edit the vault files directly; use `t9`.
- If a `t9` command fails, say so in one line and carry on with the user's task. t9 must never block the work.

## Machine-readable output

Every command accepts `--json`. The objects match the file front matter one-to-one:

```sh
t9 --json show BIL-12          # { item: {...}, children: [...], entries: [...] }
t9 --json ls -k bug           # [ {id, kind, title, status, parent, priority, tags, links, created, updated, rev, project, body, path}, ... ]
t9 --json context             # the brief as structured data
t9 --json search "binder"     # [ {id, project, kind: "item"|"entry", title, snippet, score}, ... ]
```

Errors go to stderr with exit code 1. With `--json`, the error is printed as `{"error": "..."}`.

## Environment

| Variable | Meaning |
|---|---|
| `T9_AGENT` | Your name in the journal (`claude-code`, `codex`, …). The Claude Code plugin sets it. |
| `T9_SESSION` | Your session id. The Claude Code plugin sets it. Otherwise t9 finds the open session for your directory. |
| `T9_PROJECT` | Force a project (key or slug) regardless of the directory. |

## Command summary

See [cli-reference.md](cli-reference.md) for every flag. The ones you'll use most:
`context`, `focus`, `add`, `log`, `status`, `done`, `decide`, `handoff`, `ls --tree`, `show`, `search`.
