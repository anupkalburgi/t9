# Changelog

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
