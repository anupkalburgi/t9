#!/bin/sh
# Forwards a Claude Code hook payload (stdin) to `t9 hook claude <event>`.
# Never fails the session: if t9 is not installed, it says so on stderr and exits 0.
T9="${T9_BIN:-$(command -v t9 2>/dev/null)}"
[ -z "$T9" ] && [ -x "$HOME/.cargo/bin/t9" ] && T9="$HOME/.cargo/bin/t9"
[ -z "$T9" ] && [ -x "$HOME/.local/bin/t9" ] && T9="$HOME/.local/bin/t9"
if [ -z "$T9" ]; then
  echo "t9: t9 not found on PATH; install it (cargo install --path crates/t9) or set T9_BIN" >&2
  exit 0
fi
exec "$T9" hook claude "$1"
