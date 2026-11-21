# CLI reference

`t9 <command> --help` shows the same information for each command.

## Global options

| Option | Env | Meaning |
|---|---|---|
| `--json` | | Print JSON instead of text |
| `--cwd DIR` | | Act as if run from `DIR` |
| `--agent NAME` | `T9_AGENT` | Who is writing. Default: `human` at a terminal, else `unknown`. |
| `--session ID` | `T9_SESSION` | Session id or its last characters. Default: the open session for this directory. |
| `-P, --project KEY` | `T9_PROJECT` | Project key or slug. Default: resolved from the directory. |

Other environment variables: `T9_VAULT` (vault, default `~/.t9`), `T9_HOME` (state dir, default `~/.t9/.state`),
`T9_GIT=0` (no automatic commit), `T9_GIT_DELAY_MS`, `T9_TRACE=1` (timings on stderr). Settings file:
`~/.t9/config.toml` with `git` and `git_delay_ms`. Wherever a command takes text, `-` reads it from stdin.

Values:

- **Kinds:** `plan`, `phase`, `feature`, `task`, `bug`, `regression`, `revisit`, `decision`, `question`, `idea`,
  `experiment`, `resource`, `note`.
  Aliases: `milestone`, `feat`, `todo`, `reg`.
- **Statuses:** `open`, `active`, `blocked`, `done`, `dropped`, `proposed`, `accepted`, `rejected`, `superseded`,
  `failed`, `inconclusive`. `succeeded` is an alias of `done`.
  Aliases: `wip`, `doing`, `in-progress`, `closed`, `fixed`, `wontfix`, `cancelled`.
- **Entry kinds:** `progress`, `note`, `blocker`, `finding`, `handoff`, `decision`, `status-change`, `checkin`
  (written by `t9 still`).
- **Dates** (`--due`, `--retry-after`): `2026-12-01`, `today`, `tomorrow`, `mon`…`sun` (the next one, never
  today), or a delay: `3d`, `2w`, `1m`; `none` clears.
- **Links** are written `rel:ID`, where rel is one of `blocks`, `blocked-by`, `relates`, `regression-of`,
  `supersedes`, `superseded-by`, `duplicate-of`, `applies-to`.

## Reading

| Command | Options |
|---|---|
| `t9 context` | `--budget N`: approximate token budget (default 1500) |
| `t9 pending` | What needs attention: overdue, due soon, ripe, waiting on the user, stale, going in circles, closed without an outcome · `-A` all projects · `-n N` (alias: `outstanding`) |
| `t9 whereami` | |
| `t9 ls` | `-k KIND`… · `-s STATUS`… · `-a/--all` include closed · `-t/--tree` · `--parent ID` · `-A/--all-projects` · `-n N` |
| `t9 show ID` | `-n N` journal entries (default 15) |
| `t9 journal` | `-i ID` · `--of-session ID` · `-k KIND`… · `--days N` · `-n N` (default 30) · `-A` (alias: `timeline`) |
| `t9 search WORDS…` | `-A` · `-n N` (default 20). Every word must match, as a prefix. |
| `t9 triage` | `--days N` (default 14) |
| `t9 about` | The project's About. `about add TEXT --section S`, `about set S TEXT` (`-` reads stdin), `about confirm`, `about edit`. Sections: what, goals, non-goals, architecture, stack, run, data, terms, constraints, links |
| `t9 history TOPIC…` | Everything recorded about a topic, oldest first · `-A` all projects |
| `t9 insights` | Signs of going in circles: flip-flopping decisions, reopened work, repeats of dead ends, spinning branches, stuck work (alias: `loops`) |
| `t9 timeline` | Key events, newest first, by day · `--days N` (default 30) · `--topic T` · `-n N` |
| `t9 resources` | Resources of this project · `--type T` · `-a` include dropped |
| `t9 check WORDS…` | `-n N` (default 10). Decisions, rejections and ideas of any status matching any word, in this project and the global log |
| `t9 decisions` | `--area A`… · `-a/--all` (include superseded) · `-g/--global` · `--md` (ADR-style document) |
| `t9 web [ID]` | Browser UI (opens this folder's project, or one item) · `--port N` · `--no-open`; see [web.md](web.md) |
| `t9 tui` | Terminal UI (opens on this directory's project); see [tui.md](tui.md) |
| `t9 project ls` / `t9 project show [KEY]` | |
| `t9 session ls` | `--open` · `-A` · `-n N` |
| `t9 focus` | With no id: prints the current focus |

## Writing

| Command | Options |
|---|---|
| `t9 add KIND TITLE…` | `--parent ID` · `--goal WHY` · `--due WHEN` · `--after ID`… (waits on it) · `--area A`… · experiments: `--hypothesis H` `--retry-when C` `--retry-after DATE` · resources: `--type T` `--at LOCATION` `--owner O` · `-b/--body TEXT` · `-t/--tag T`… · `-p/--priority 0-9` · `-s/--status S` · `-l/--link rel:ID`… · `-f/--focus` |
| `t9 log MESSAGE…` | `-k/--kind K` (default progress) · `-i/--item ID` (default: the focus) · `-r/--ref PATH`… |
| `t9 focus ID` | `--goal WHY` (recorded if it has none) · `--no-activate` (don't move `open` to `active`) · `--clear` |
| `t9 status ID STATUS` | `-n/--note TEXT` (when closing: the outcome) · `--due` `--goal` `--after` · `--retry-when C` · `--retry-after DATE` (a date, `30d`/`6w`/`3m`, or `none`) |
| `t9 done ID` | `-n/--note TEXT` (alias `--outcome`): what came out of it. Without one, t9 suggests recording it; items that waited on this one are listed as ripe |
| `t9 still ID -n WHY` | Still relevant: a check-in that resets the idle clock without counting as progress |
| `t9 update ID` | `--goal WHY` · `--due WHEN\|none` · `--after ID`… · `--area A`… · `--hypothesis` `--retry-when` `--retry-after` `--type` `--at` `--owner` · `--title` · `--status` · `--parent ID\|none` · `-p 0-9\|none` · `-t TAG`… · `--untag TAG`… · `-l rel:ID`… · `-b BODY` · `--rev N` (fail on conflict) · `-n NOTE` |
| `t9 decide TITLE…` | `-w/--because WHY` · `-r/--rejected "option: why not"`… · `--area A`… · `--for ID`… (applies to that item and below) · `-s/--supersedes ID`… · `--proposed` · `-g/--global` · `-b/--body NOTES` · `--parent ID` |
| `t9 reject TITLE…` | The same options as `decide` (except `-s`, `--proposed`); records a `rejected` decision |
| `t9 handoff MESSAGE…` | |
| `t9 attach ENTRY ID` | ENTRY is an entry id or its last 6 characters |
| `t9 edit ID` | Opens `$VISUAL` / `$EDITOR` |

## Projects and sessions

| Command | Options |
|---|---|
| `t9 project init` | `-n/--name` (default: directory name) · `-k/--key` (default: derived from the name) · `-d/--description` · `--no-root` |
| `t9 project link KEY` | Adds this directory and its git remote to the project |
| `t9 session start` | `--harness-id ID` (resume the session with this id) · `--context` (also print the brief) · `--budget N` |
| `t9 session end` | `--handoff TEXT` |

## Maintenance and integrations

| Command | Meaning |
|---|---|
| `t9 reindex` | Rebuild the index from the vault |
| `t9 vault commit` | Commit vault changes now (t9 also does this automatically in the background after every write) |
| `t9 vault path` | Print the vault path |
| `t9 vault git ARGS…` | Run git in the vault, e.g. `t9 vault git log --oneline -10` |
| `t9 doctor` | Rebuild, then report unparseable files, missing parents or link targets, stale active items, sessions that never ended, unfiled entries |
| `t9 install claude [--dir DIR]` | Write the Claude Code plugin (default `~/.t9/integrations/claude`) and print the commands that add it |
| `t9 install agents-md [--file F] [--print]` | Add or update the t9 section of `~/.codex/AGENTS.md` (or `F`) |
| `t9 hook claude session-start\|prompt\|session-end` | Used by the Claude Code plugin. Reads the hook payload from stdin and always exits 0. `prompt` prints earlier decisions matching the user's message, or nothing. |
