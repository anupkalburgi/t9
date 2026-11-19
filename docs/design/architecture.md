# t9 architecture

How t9 is built. Why it is built this way is in [design.md](design.md); latency and memory measurements are
in [performance.md](performance.md).

## 1. Shape of the system

```
   Claude Code            Codex / other agents          human at a terminal
 ┌──────────────┐        ┌────────────────────┐        ┌───────────────────┐
 │ plugin:      │        │ AGENTS.md section  │        │ t9 …              │
 │  SessionStart│        │ ("run t9 context   │        │ $EDITOR / Obsidian│
 │  SessionEnd  │        │  first, then …")   │        │  on the vault     │
 │  skill       │        └─────────┬──────────┘        └───┬───────────┬───┘
 └──────┬───────┘                  │ shell                 │ shell     │ edits files
        │ t9 hook claude …         │ t9 …                  │ t9 …      │
        ▼                          ▼                       ▼           │
 ┌───────────────────────────────────────────────────────────────┐    │
 │ t9 (one binary, crates/t9): clap CLI · hook · install · print │    │
 └──────────────────────────────┬────────────────────────────────┘    │
                                ▼                                     │
 ┌───────────────────────────────────────────────────────────────┐    │
 │ t9-core (crates/t9-core): Store                                │    │
 │  resolve · sessions · items · journal · context · search · sync│    │
 └──────────┬──────────────────────────────────────┬─────────────┘    │
            │ write first, atomically, under lock  │ derived          │
            ▼                                      ▼                  │
 ┌────────────────────────────┐        ┌──────────────────────────┐   │
 │ Vault  ~/.t9            │◀───────│ Index  ~/.t9/.state/index.db│   │
 │ Markdown + YAML front      │ sync   │ SQLite WAL + FTS5        │   │
 │ matter. SOURCE OF TRUTH    │───────▶│ rebuildable at any time  │   │
 └────────────────────────────┘        └──────────────────────────┘   │
            ▲                                                         │
            └─────────────────────────────────────────────────────────┘
```

There is no daemon. Every `t9` invocation opens the index, brings it up to date with the vault cheaply,
does its work and exits. A daemon (`t9 serve`) is planned for the UIs, not for correctness (§12).

## 2. Crates and modules

| Path | Responsibility |
|---|---|
| `crates/t9-core/src/model.rs` | Types: `Kind`, `Status`, `EntryKind`, `Link`, `ProjectMeta`, `ItemMeta`, `EntryMeta`, `SessionMeta`; id/key validation, slugs. The `*Meta` structs are exactly the front matter, the JSON API shape and the index's `meta` column. |
| `frontmatter.rs` | Split/parse/render `---\nyaml---\n\nbody`. Rewriting front matter leaves the body byte-for-byte unchanged. |
| `journal.rs` | Journal block format: render, parse, rewrite one block's header, remove one block. |
| `git.rs` | Read-only git discovery by reading `.git` files directly (worktrees, packed refs, remote normalization). No `git` process. |
| `env.rs` | Locations and settings: vault (`T9_VAULT`, `~/.t9`), state dir (`T9_HOME`, `<vault>/.state`), `config.toml` in the vault (`git`, `git_delay_ms`). |
| `vcs.rs` | Automatic git history of the vault: background committer, lock, commit message from changed paths, `git.log`. |
| `index.rs` | SQLite schema, migrations-by-rebuild, row mapping, filters, FTS search. Free functions over `&Connection`, so they work inside transactions. |
| `store.rs` | `Store`: the API every front end uses. Locking, atomic writes, sync, resolution, sessions, items, journal, attach/adopt. |
| `context.rs` | Builds the brief (`Brief`) and renders it to a token budget. |
| `about.rs` | The project's About: sections in the body of `project.md`, parse/render, `about_add`, `about_set`, `about_confirm`. |
| `history.rs` | `history(topic)` (everything on a topic, oldest first) and `timeline(project, since, topic)`, both as `Event`s. |
| `unlinked.rs` | Folders where agents worked without a project: suggestions, the registry in `.state/unlinked.json`, once-per-session hints. |
| `decisions.rs` | The decision log: `decide`/`reject`, ADR-shaped bodies, the GLOBAL project, recall by scope, area and text (`applicable_decisions`, `check`, `prompt_context`), the Markdown export. |
| `crates/t9-tui` | `t9 tui` (ratatui): `data.rs` is the only module that touches the store (the seam for a future REST backend), `app.rs` holds state and keys, `ui.rs` renders. Refreshes every 2 s; writes as `human` in a session whose cwd is the vault, so agents never pick it up. |
| `crates/t9/src/main.rs` | clap definitions and dispatch. Builds a `Caller` (cwd, agent, session, project) from flags and env. |
| `print.rs` | Human-readable output. `--json` prints the same structs serialized. |
| `hook.rs` | `t9 hook claude session-start|prompt|session-end`: reads the hook payload from stdin, never fails the session. |
| `install.rs` | `t9 install claude|agents-md`: writes integrations embedded in the binary (`include_str!`). |
| `integrations/` | Sources of the Claude Code plugin (local marketplace) and the AGENTS.md section. |

## 3. Storage

### 3.1 Vault (source of truth)

```
~/.t9/                               a git repository
  config.toml                        settings (committed)
  .gitignore                         .state/, integrations/, temp files
  .state/                            index.db, locks/, git.log (T9_HOME; disposable, ignored)
  integrations/claude/               generated by `t9 install claude` (ignored)
  projects/<slug>/
    project.md                       front matter: key, name, status, roots[], remotes[], created
    items/<KEY>-<n>-<title-slug>.md  one item per file; front matter = ItemMeta, body = description
    journal/<YYYY-MM-DD>.md          local-date file of appended entry blocks
    sessions/<YYYYMMDDTHHMMZ>-<agent>-<suffix>.md   front matter = SessionMeta
  inbox/
    journal/…  sessions/…           pseudo-project INBOX: writes from unlinked directories
```

Item file:

```markdown
---
id: BIL-7
kind: decision
title: Keep the legacy XML export
status: superseded
links:
- rel: superseded-by
  to: BIL-8
created: 2026-11-18T02:33:05Z
updated: 2026-11-18T02:33:05Z
created_by: claude-code
created_in: 01M7P80BJDS7QG806Q1FDG8QAW
rev: 2
---

some enterprise customers may still need it
```

Unknown front-matter keys added by a human are preserved (`extra`, flattened). `rev` increments on every t9 write.

Journal block (the comment line is machine-readable JSON; the `###` heading is regenerated from it; everything up
to the next header is the body, so bodies can be edited by hand):

```markdown
<!-- t9 {"id":"01M7P80BMMX0994NH67BJQ2QK9","kind":"progress","at":"2026-11-18T02:33:05Z","item":"BIL-5","session":"01M7…","agent":"claude-code","refs":["runs/2026-11-18-replay"]} -->
### 21:33 · progress · BIL-5 · claude-code
Replayed 500 webhook events: no duplicate charges
```

Ids: items are `<KEY>-<n>` (n = max on disk or in the index, + 1, under the project lock); entries and sessions
are ULIDs, shown by their last 6 characters.

### 3.2 Index (derived)

`~/.t9/.state/index.db`, SQLite in WAL mode, `synchronous=NORMAL`, 10 s busy timeout. Schema version in
`PRAGMA user_version`; any mismatch drops and rebuilds everything from the vault (there are no migrations,
because nothing in the index is primary data except `sessions.last_seen`).

| Table | Content |
|---|---|
| `projects`, `project_roots`, `project_remotes` | project rows; roots and normalized remotes for resolution |
| `items` | `rid INTEGER PK`, `id UNIQUE`, filter columns (project, kind, status, parent, priority, updated), `meta` JSON, `body`, `path` |
| `entries` | `rid`, `id UNIQUE`, project, item, session, kind, at, `meta` JSON, body, file |
| `sessions` | id, project, agent, harness_id, cwd, started, ended, `last_seen` (index-only), meta, path |
| `files` | `(path, project, kind, mtime_ns, size)` for every vault file and directory read, plus a per-project `#full-check` marker |
| `items_fts`, `entries_fts` | FTS5 external-content tables on `rid`, porter + unicode61 tokenizer, maintained by AFTER INSERT/DELETE triggers; title weighted 4× body |

Rows are never `REPLACE`d (REPLACE does not fire delete triggers); they are deleted and re-inserted.

## 4. Write path

Example: `t9 add bug "…" --parent BIL-3`.

```
resolve project (parent's project wins)          ── read-only
sync_project(key)                                ── read-only unless something changed
validate parent and link targets exist
session_for_write(caller)                        ── find or start the caller's session
acquire flock ~/.t9/.state/locks/BIL.lock            ── serializes writers of one project, across processes
  n = max(item numbers on disk, in index) + 1
  render front matter + body
  dir_clean = (items/ mtime == recorded mtime)
  write <file>.tmpPID, rename over target        ── atomic: readers see old or new, never half
  upsert index row (triggers update FTS)
  record file sig; if dir_clean, record new dir sig  ── own writes never force a rescan
release lock
```

Journal writes append a block to today's file under the same lock (append, not rename). If the file's
recorded signature did not match before the append (someone edited it), the whole file is re-parsed.

Status changes write a `status-change` entry; `decide` writes the decision item, marks superseded items, links
both ways and writes a `decision` entry.

## 5. Read path and sync

Filesystem calls are expensive on the target machine (~30 µs per `stat`, ~40 ms to list 2,000 files), so a read
cannot rescan the vault. `sync_project(key)`:

1. Load the project's recorded file signatures in one query.
2. For each of `items/`, `journal/`, `sessions/`: `stat` the directory. If its mtime/size is unchanged and no
   full check is due, skip it. Otherwise list it and `stat` each file. (Directory mtime changes on create,
   delete and rename, which covers atomic writes and most editors.)
3. Always `stat` journal files from the last 7 days (they are appended to in place, which does not change the
   directory).
4. A full check of all three directories runs if the last one was more than 5 minutes ago; it catches in-place
   edits of older files.
5. If nothing differs, return without writing. Otherwise open `BEGIN IMMEDIATE`, re-check and re-index each
   changed file, forget removed files, record directory signatures and the full-check time, commit.

Reads of a single item (`show`, `update`, `status`, `focus`, …) additionally `stat` that item's file, so an
in-place edit of the item being acted on is always seen. `sync_projects` (all `project.md` files) runs on every
open; it is one `stat` per project. `t9 reindex` drops the index and re-reads everything.

Consequence: an in-place edit (not rename) of an item file older than the fast-path checks can take up to 5
minutes to appear in listings and the brief. The planned daemon's file watcher removes this delay (§12).

## 6. Project resolution

`Store::resolve(caller)`, first match wins:

1. `--project` / `T9_PROJECT` (key or slug).
2. A `.t9` file in the cwd or any parent (first non-comment line: key or slug).
3. Registered root that contains the cwd (longest match). Paths are canonicalized (`/tmp` → `/private/tmp`).
4. Git remote: walk up to `.git`; if it is a file (worktree), follow `gitdir:` and `commondir`; read `origin`
   (else the first remote) from the common `config`; normalize (`git@github.com:A/B.git` →
   `github.com/a/b`); look it up in `project_remotes`.
5. Unresolved: the cwd maps to the `INBOX` pseudo-project, and suggestions are computed by comparing the
   directory name (and its parent's) with project slugs, names and root names.

`project init` registers the git work-tree top (or the cwd) and the remote; `project link` adds a root and
remote to an existing project and removes that root from any other project. Both then call `adopt_inbox`, which
moves inbox entries whose recorded `cwd` now resolves to the project into its journal.

## 7. Sessions and focus

A session records who (agent, harness session id), where (cwd, branch, head), when, and the current focus item.
It is a vault file, so a reindex keeps it; `last_seen` is updated in the index on every write.

Which session a command belongs to:

1. `--session` / `T9_SESSION` (full id or suffix).
2. Otherwise the most recently active open session in the same project whose cwd is the caller's cwd or a parent
   of it, active within 12 hours, matching `--agent`/`T9_AGENT` if given.
3. Otherwise a write starts one; a write from an unlinked directory does not (inbox entries have no session).

Claude Code: the SessionStart hook passes Claude's `session_id` as `harness_id`. The same id after `/clear`,
resume or compaction resumes the same t9 session. The hook writes `T9_SESSION` and `T9_AGENT` to
`CLAUDE_ENV_FILE`, so the agent's later shell commands carry them.

Focus: `t9 focus ID` stores the item on the session (and moves an `open` item to `active`). `t9 log` without
`--item` attaches to the focus unless the focus has since been closed.

Agent name: `--agent`/`T9_AGENT`; when unset, `human` if stdin is a terminal, else `unknown`.

## 8. The context brief

`context::build` gathers, for the resolved project:

| Section | Source | Cap |
|---|---|---|
| header | project, cwd, branch, session | — |
| Last handoff | newest `handoff` entry | 1,200 chars |
| Current focus | session focus, if not closed | body: 6 lines |
| In progress | `active`/`blocked` non-structural items | 10 |
| Plan … x/y done | each open plan: done/total over descendants; open phases/features to depth 2 | 12 lines |
| Open bugs, regressions, revisits, questions | not closed, not already shown, by priority | 10 |
| Next up | open tasks not shown, by priority | 8 |
| About | the project's About, one line per section | 8 lines |
| From history: applies to your focus | `applicable_decisions(focus)` (§9): decisions, experiments, resources | 5 |
| Rules for every project (global) | accepted GLOBAL decisions | 6 |
| Current decisions | `accepted` decisions not shown above, newest first | 6 |
| Ruled out | `rejected` decisions and ideas | 5 |
| Parked ideas | open `idea` items | 5 |
| Experiments running / Worth retrying | open or active experiments; failed or inconclusive ones whose retry date passed | 5 each |
| Resources | resources not dropped, by type | 10 |
| Recent activity | newest entries except handoffs | 8 |
| Needs attention | unfiled entries (14 days), inbox count, active items idle 3+ days | — |

`context::render` writes sections in that order into a character budget of `budget × 4`. A section header is
only written if at least one line fits; dropped lines become "… N more"; a truncated brief says how to get more.
In an unlinked directory the brief lists matching projects and the two commands to link or create one.

## 9. Decision log and recall

Decisions are items (`kind: decision`) with status `accepted`, `proposed`, `rejected` or `superseded`, `areas`,
and links: `supersedes`/`superseded-by` for changes of mind, and `applies-to` for scope. The body is ADR-shaped
(`## Why`, `## Rejected alternatives`, `## Notes`). A reserved project `GLOBAL` (folder `_global`, no roots, never
resolved from a directory, never suggested) holds decisions for every project. It is created on the first
`--global` decision.

Recall uses three signals:

| Signal | Rule | Weight |
|---|---|---|
| Scope | the decision has `applies-to` the item or an ancestor (100), or is a child of one (80) | strongest |
| Area | the decision's areas intersect the areas and tags of the item and its ancestors | 50 |
| Text | content words of the item or message (stop words removed, 2+ characters, up to 16) retrieved with FTS5 (any word, prefix) over decisions and ideas, then re-scored by distinct word hits in title, areas and body; words of 4+ characters match by prefix | 2 per hit, 3 per title hit |

- **Strict text matching** (used by focus, the brief and the message hook) needs two distinct hits, or one hit of
  5+ characters in the title. It skips superseded decisions and parked ideas.
- **`t9 check`** accepts any hit and every status.
- **Projects searched:** the current project and GLOBAL.

The message hook (`t9 hook claude prompt`, Claude Code `UserPromptSubmit`) runs strict matching on the user's
message and prints at most 3 decisions (id, status, date, title, the first line of "Why"). If a phrase list
matches ("let's go with", "instead of", "never", "switch to", …) it also prints a one-line reminder to record the
decision. It prints nothing otherwise, so most messages cost about 3 ms and add no tokens.

## 10. Git history

Every vault change is committed, without making writes wait for git:

1. `Store` marks itself dirty whenever it writes a vault file (all writes go through `Store::put` or the
   journal append).
2. When the CLI's `Store` is dropped (on success or error) and it wrote, it starts
   `t9 vault commit --background` as a detached process (own process group, no stdio) and exits.
3. The committer sleeps `git_delay_ms` (default 1.5 s), takes `.state/locks/git.lock`, writes `.gitignore` and runs
   `git init` if needed, then `git add -A`. If nothing is staged it exits; otherwise it commits with a subject
   derived from the staged paths (`t9: BIL-12, BIL-3, journal billing-service`) and a body listing them.
4. Several committers in a burst queue on the lock; the first commits everything, the rest find nothing staged.
   Ten writes in quick succession produce one or two commits.
5. Commits use the user's git identity if configured (otherwise `t9 <t9@localhost>`) and run global hooks.
   Success and failure lines go to `.state/git.log`; `t9 doctor` reports the last failure.

Reads never commit. `t9 vault commit` commits synchronously and prints the result. Disabled by `git = false` or
`T9_GIT=0`.

## 11. Concurrency and failure behavior

- **Writers** of one project are serialized by `flock` on `~/.t9/.state/locks/<KEY>.lock`; git committers on `locks/git.lock`. Locks are never nested
  within a process (a second `flock` on the same file from the same process would block).
- **Index transactions** that write use `BEGIN IMMEDIATE` and are only opened after a read-only check found
  changes, so concurrent readers never fail with `SQLITE_BUSY` on a lock upgrade. Single-statement writes rely
  on autocommit plus the busy timeout. Schema creation re-checks the version after taking the write lock.
- **Crash safety**: item, session and rewritten journal files are written to a temp file and renamed. Appends are
  a single `write` of a whole block. If the process dies between the file write and the index write, the next
  sync sees an unrecorded signature and re-indexes the file.
- **Bad files** (unparseable front matter, malformed headers, duplicate ids) are skipped with a warning (shown by
  `t9 doctor`) and recorded, so they are not re-read until they change.
- **Hooks never fail the agent's session**: `t9 hook` prints errors to stderr and exits 0; the shell wrapper exits
  0 when `t9` is not installed.
- **Optimistic concurrency** for item edits: `t9 update --rev N` fails with `conflict` if the file moved on.

Tested end to end (`crates/t9/tests/cli.rs`): 16 parallel `log` and 8 parallel `add` processes produce exactly 16
entries and 8 unique ids, and a rebuild from files gives the same result.

## 12. Planned components

| Component | Plan |
|---|---|
| `t9 serve` | axum on 127.0.0.1 + Unix socket, bearer token from `~/.t9/.state/token`; REST mirroring the JSON shapes; SSE event stream; FSEvents watcher calling the same per-file indexing as sync, which makes in-place edits visible immediately. |
| `t9 tui` | **Built** (in-process, see §2). Moves to `t9 serve` once the daemon exists, by swapping `data.rs`. |
| `t9 mcp` | stdio MCP server exposing the same `Store` operations as tools. |
| import | `t9 import` of existing plan documents into items, agent-assisted. |
| semantic search | optional embeddings in `sqlite-vec`, hybrid with BM25. |

All of them are thin layers over `Store`; nothing in the storage or sync model changes.

## 13. Security and privacy

Local only. No network access; nothing leaves the machine. The vault and index are created with the user's
default permissions. The planned daemon binds to localhost and requires a token so that web pages cannot write to
it. Agents are told to reference large outputs by path instead of pasting them, which also keeps secrets that may
appear in logs out of the journal.
