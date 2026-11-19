# Changelog

## Unreleased

- The installer adds `~/.local/bin` to your `PATH` (zsh, bash, fish, or `~/.profile`) when it isn't there
  already; `T9_NO_MODIFY_PATH=1` skips this.

## 0.1.1

- Prebuilt binaries for macOS (Apple Silicon, Intel) and Linux (x86_64, ARM64), and a one-line installer:
  `curl -fsSL https://raw.githubusercontent.com/anupkalburgi/t9/main/install.sh | sh`.
- Fix: concurrent writers could fail with a duplicate-id error while indexing the same journal entry. Index
  writes are now atomic (the index is rebuilt automatically on first run).
- Fix: `t9 … | head` no longer reports a broken pipe.
- CI now also runs on Linux.

## 0.1.0

First release.

- **CLI** (`t9`): projects resolved from the directory (including git worktrees), items in a tree with typed
  links, journal entries, sessions with focus, handoffs, triage, full-text search, `doctor`, `--json`
  everywhere.
- **Vault** in `~/.t9`: Markdown with YAML front matter as the source of truth, a rebuildable SQLite index, and
  automatic git commits in the background.
- **Brief** (`t9 context`): a token-budgeted summary for agents at session start.
- **Decision log**: decisions with reasons and rejected alternatives, rejections, proposals, supersede chains,
  scope and areas, a global log; recalled in the brief, on focus, with `t9 check`, and on every user message in
  Claude Code.
- **TUI** (`t9 tui`): an overview for humans, plan tree, decision log, journal, triage and search, with live
  refresh.
- **Integrations**: a Claude Code plugin (hooks and skill) and an `AGENTS.md` section for Codex and others.
