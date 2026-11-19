# t9

**Project memory for coding agents.** t9 keeps the plan, open work, decisions and progress of a long-running
project outside the chat, so every new session (after compaction, another agent, or you the next morning) starts
from facts instead of a reconstruction.

- **Works with any agent.** It's a CLI, so anything that can run a shell command can use it. A Claude Code plugin
  adds a brief at session start and recalls past decisions as you chat; Codex and others get an `AGENTS.md`
  section.
- **A decision log that comes back when it matters.** Choices, rejected options and changes of mind are recorded
  with their reasons, and resurface in later sessions when their topic does.
- **Plain Markdown you own.** Everything lives in `~/.t9` as Markdown with YAML front matter, committed to git
  automatically. Readable in any editor or Obsidian.
- **Fast and small.** One Rust binary, no daemon. Most commands take a few milliseconds and peak memory stays
  under 30 MB on a project with 50,000 journal entries.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/anupkalburgi/t9/main/install.sh | sh
```

This downloads the binary for your machine (macOS or Linux, arm64 or x86_64) from the
[latest release](https://github.com/anupkalburgi/t9/releases/latest), verifies its checksum, installs it to
`~/.local/bin`, and adds that folder to your `PATH` in your shell's startup file if it isn't there yet. Open a new
terminal afterwards. Options: `T9_INSTALL_DIR` to install elsewhere, `T9_VERSION=v0.1.1` to pin a version,
`T9_NO_MODIFY_PATH=1` to leave your shell files alone. To build from
source instead: `cargo install --git https://github.com/anupkalburgi/t9 t9` (Rust 1.89+).

Connect your agents:

```sh
t9 install claude      # Claude Code plugin; prints the two `claude plugin …` commands to run
t9 install agents-md   # adds the t9 section to ~/.codex/AGENTS.md (or --file PATH)
```

## Use

```sh
cd ~/code/my-project
t9 project init --name "My Project" --key MP   # once; worktrees and clones are found by git remote
t9 tui                                         # see everything: overview, plan tree, decisions, journal
```

Agents do most of the writing. You can too:

```sh
t9 add plan "Ship v2"                                    # MP-1
t9 add feature "Search" --parent MP-1                    # MP-2
t9 add task "Index journal entries" --parent MP-2 --focus
t9 log "50k entries index in 4 s" -r bench/results.txt
t9 decide "Use SQLite FTS5 for search" --because "no extra process" -r "tantivy: separate index to sync"
t9 reject "Run a daemon for all writes" --because "the CLI must work with nothing running"
t9 handoff "Indexing done; next: ranking"
t9 context        # the brief an agent reads at session start
```

## Docs

- [Concepts](docs/getting-started/concepts.md) · [For humans](docs/getting-started/humans.md) ·
  [For agents](docs/getting-started/agents.md) · [Decision log](docs/getting-started/decisions.md) ·
  [TUI](docs/getting-started/tui.md) · [CLI reference](docs/getting-started/cli-reference.md)
- [Architecture](docs/design/architecture.md) · [Design rationale](docs/design/design.md) ·
  [Performance](docs/design/performance.md)
- [Status](docs/project-plan/status.md) · [Roadmap](docs/project-plan/roadmap.md) ·
  [Backlog](docs/project-plan/backlog.md) · [Changelog](CHANGELOG.md)

## Status

Early but complete enough for daily use. Developed on macOS; Linux is built and tested in CI. See the
[roadmap](docs/project-plan/roadmap.md) for what's next: a local server for instant UI updates, an MCP server,
and importing existing plan documents.

## License

[MIT](LICENSE)
