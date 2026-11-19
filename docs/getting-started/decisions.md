# The decision log

Over many sessions you make choices, reject ideas and change your mind. If those live only in chat, the next
session re-asks the same questions or re-proposes something you already turned down. t9 keeps a decision log for
each project, plus a global one, and brings the relevant entries back when they matter.

## What gets recorded

A **decision** is an item of kind `decision`. Its status is one of:

| Status | Meaning |
|---|---|
| `accepted` | Chosen, and in force. This is the default for `t9 decide`. |
| `rejected` | Considered and turned down (`t9 reject`). Shown as "Ruled out" so it isn't proposed again. |
| `proposed` | Recommended but not yet confirmed (`t9 decide --proposed`). |
| `superseded` | Replaced by a later decision (`t9 decide … -s OLD`). Kept for the record. |

Its body has the shape of an architecture decision record (ADR):

```markdown
## Why

no extra process; 29 ms worst case is fast enough

## Rejected alternatives

- tantivy: separate index to keep in sync, +4 MB binary

## Notes

Benchmarks: docs/design/performance.md
```

Each decision can also have:

- **Areas:** topics such as `architecture`, `testing`, `tooling`, `storage`, `api`, `ui`, `product` or `process`.
  Pick a small set per project and use it consistently.
- **Scope:** `--for BIL-3` links the decision to an item with an `applies-to` link. It then applies to that item
  and everything under it in the plan tree.
- **Global:** `--global` puts it in the global log. It then applies to every project, which suits personal rules
  such as "always use uv" or "never deploy without asking".

An **idea** (`t9 add idea "…"`) is something that came up and is parked. Accept it later with
`t9 status ID accepted` (or turn it into a task), or reject it with `t9 status ID rejected`.

## Recording

```sh
t9 decide "Use FTS5 for search" --because "no extra process" \
   -r "tantivy: separate index to keep in sync" --area search
t9 reject "Use a daemon for all writes" --because "the CLI must work with nothing running" --area architecture
t9 decide "Integration tests use real temp vaults, no mocks" --area testing --for BIL-2
t9 decide "Use tantivy after all" -s BIL-4 --because "we need fuzzy search"      # change of mind
t9 decide "Always use uv for Python" --global --because "one tool for environments"
t9 decide "Cache the brief per session" --proposed --because "saves 15 ms"      # for the human to confirm
t9 add idea "Trigram tokenizer for identifiers" --area search
```

`-r` and `--area` can be repeated. `--because`, `--body` and `-b -` read long text from stdin.

## Recall: when the log is used

| Moment | What happens |
|---|---|
| Session start | The brief shows "Decisions that apply to your focus", "Rules for every project (global)", "Current decisions", "Ruled out" and "Parked ideas". |
| `t9 focus BIL-12` | Prints the decisions that apply to BIL-12: scoped to it or an item above it, sharing an area or tag with them, or matching its text. |
| Every message you send (Claude Code) | A hook searches the log for the topic of your message. On a strong match (two or more content words, or a distinctive word in the title), it adds "t9: earlier decisions that may bear on this message" with the ids to the agent's context. If your message reads like a decision ("let's go with", "instead of", "never", "switch to", …), it adds a one-line reminder to record it. Otherwise it adds nothing. It costs about 3–6 ms per message. |
| Before proposing or asking | The agent runs `t9 check "topic"`, which searches decisions, rejections and ideas of every status, here and in the global log. |
| Before stopping | The agent sweeps the session for choices it hasn't recorded, then writes the handoff. |

A decision from three weeks ago that has dropped out of the brief still comes back as soon as you mention its
topic. That includes a word that appears only among its rejected alternatives.

## Reviewing the log

```sh
t9 decisions                     # accepted, proposed and rejected, newest first
t9 decisions --area testing      # one area
t9 decisions --all               # include superseded ones
t9 decisions --global            # the global log
t9 decisions --md > DECISIONS.md # an ADR-style document: a table plus each decision in full
t9 check "search engine"         # has this come up before?
t9 show BIL-4                     # one decision with its links (supersedes, superseded-by, applies-to)
```

Decisions are ordinary item files under `~/.t9/projects/<slug>/items/`, committed to git with everything else, so
`t9 vault git log -p -- projects/<slug>/items/BIL-4-*.md` shows how a decision evolved.

## For agents

The rules are in the t9 skill and the AGENTS.md section:

1. Before proposing a design, picking a tool, or asking the user to choose, run `t9 check "topic"`. Follow what was
   decided and cite the id.
2. When the user decides or rejects something, record it immediately with `t9 decide` or `t9 reject`, including
   `--because` and any rejected alternatives, and mention the id in the reply.
3. Don't reopen accepted or rejected decisions unless the user does or there is new information. Then say which
   decision is being questioned, and record the change with `-s`.
4. Before stopping, record any choices from the session that aren't in the log yet.
