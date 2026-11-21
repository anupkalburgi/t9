# t9 for Claude Code

t9 is a shared, persistent project memory for coding agents and the people running them. This plugin
connects Claude Code to it:

- **SessionStart hook**: prints the t9 brief for the current directory's project (last handoff, current
  focus, plan outline, open bugs/regressions/revisits, current decisions, recent activity) at startup, resume,
  `/clear` and after compaction, and registers a t9 session for this conversation.
- **UserPromptSubmit hook**: on each message, recalls earlier decisions that bear on it, and adds a short
  reminder when something new needs attention (a question the user asked the agents, due, overdue, ripe, waiting
  on the user), once per item per day.
- **SessionEnd hook**: closes that session.
- **t9 skill**: tells Claude when to record work with the `t9` CLI: focus, progress, bugs, revisits,
  decisions and handoffs.

Requires the `t9` binary on `PATH` (or `T9_BIN`). Install it with `cargo install --path crates/t9`, then run
`t9 install claude` to write this plugin and print the commands that add it.
