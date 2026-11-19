# Concepts

Read this once. It explains the nouns that every other page and every `t9` command use.

## Project

A long-running piece of work, usually one repository. A project has:

- a **key**, 2 to 10 capital letters or digits, such as `BIL`. Item ids start with it: `BIL-1`, `BIL-2`, …
- a **name** and a **slug**, such as `Billing Service` and `billing-service`. The slug is its folder in the vault.
- **roots**: the directories that belong to it.
- **remotes**: the normalized git remote URLs, such as `github.com/acme/billing-service`.

`t9` works out the project from the directory you run it in. It checks a `.t9` file first, then the roots,
then the git remote. Matching on the git remote means worktrees and clones of the same repository find the
project without extra setup.

## Item

Anything you want to track. Every item has a **kind**, a **status**, a title, and an optional Markdown body.

| Kind | Use it for |
|---|---|
| `plan` | The big picture, such as "Move invoicing to the new payments API" |
| `phase` | A stage of a plan |
| `feature` | A capability being built |
| `task` | A concrete unit of work |
| `bug` | Something broken |
| `regression` | Something that used to work; link it with `regression-of:ID` |
| `revisit` | "We should come back to this": a loose end, a shortcut taken, a doubt |
| `question` | Something only a human can answer |
| `decision` | A choice that was made or an option that was rejected, with the reasons (see [decisions.md](decisions.md)) |
| `idea` | Something that came up and is parked for later |
| `note` | Reference material that is none of the above |

Status is `open`, `active`, `blocked`, `done` or `dropped`. Decisions use `proposed`, `accepted`, `rejected` and
`superseded`. `done`, `dropped`, `rejected` and `superseded` count as closed, so `t9 ls` hides them by default.

Items can carry **areas**: topics such as `architecture`, `testing` or `storage`. When an item is focused, decisions
in the same area are recalled.

Items form a **tree** through `parent`: plan → phase → feature → task. Bugs and revisits hang under whatever they
relate to. Items can also have **links** of these types: `blocks`, `blocked-by`, `relates`, `regression-of`,
`supersedes`, `superseded-by`, `duplicate-of`.

Items also have an optional priority from 0 (highest) to 9, and tags.

## Decisions and the decision log

Each project has a decision log, and there is a global one for rules that apply everywhere.

- `t9 decide "…" --because "…" -r "alternative: why not" --area A` records a choice.
- `t9 reject "…"` records an option that was turned down.
- `-s BIL-17` records a change of mind: BIL-17 becomes `superseded`, and the two are linked both ways.
- `--for ID` scopes a decision to part of the plan.
- `--global` makes it apply to every project.

Decisions come back when they matter: in the brief, when you focus an item, before an agent proposes something
(`t9 check`), and in Claude Code on every message whose topic matches. See [decisions.md](decisions.md).

## Journal entry

A timestamped note, appended to the project's journal for the day. Entry kinds:

| Entry kind | Use it for |
|---|---|
| `progress` | The default: what was done |
| `finding` | Something learned |
| `blocker` | Something stopping the work |
| `note` | Anything else |
| `handoff` | Where you stopped and what comes next, for the next session |
| `status-change` | Written automatically when an item's status changes |
| `decision` | Written automatically by `t9 decide` |

An entry can be **filed** under an item. One that is not filed shows up in `t9 triage`.

## Session

One run of an agent, or of a person: which agent, in which directory, on which branch and commit, and when.
t9 creates and finds sessions automatically. In Claude Code, the plugin starts the session and resumes it after
compaction.

A session can have a **focus**, the item being worked on. `t9 log` without `--item` attaches the entry to the
focus.

## Inbox

Running `t9 log` in a directory that is not linked to a project still works. The entry goes to the **inbox**,
along with the directory it came from. When you later run `t9 project init` or `t9 project link` there, those
entries move into the project.

## The brief

`t9 context` gives a short summary of a project that fits a token budget. It has these sections:

- last handoff
- current focus
- items in progress
- plan outline with progress
- open bugs, regressions, revisits and questions
- next tasks
- decisions that apply to the focus, global rules, current decisions, options ruled out, parked ideas
- recent activity
- what needs attention

It is the first thing an agent reads in a session.

## Vault and index

The **vault** (`~/.t9`) holds the real data: plain Markdown files with YAML front matter. You can read them,
edit them, `grep` them, or open them in Obsidian. It is a git repository, and t9 commits every change
automatically, so the history of the plan, its decisions and the journal is never lost.

The **index** (`~/.t9/.state/index.db`) is a SQLite cache with full-text search. t9 keeps it in step with the vault
and can rebuild it at any time with `t9 reindex`.
