# Contributing

```sh
cargo build
cargo test                          # unit, TUI and end-to-end CLI tests
cargo fmt && cargo clippy --all-targets
python3 scripts/bench.py            # latency benchmark (after cargo build --release)
```

Layout:

- `crates/t9-core`: model, vault, index, sync, resolution, brief, decisions, git history.
- `crates/t9`: the CLI, harness hooks and `t9 install`.
- `crates/t9-tui`: the terminal UI.
- `integrations/`: the Claude Code plugin and the `AGENTS.md` section (embedded in the binary).
- `docs/`: user guides, design and the project plan.

Keep the vault format backwards compatible: files written by an older t9 must still read. Tests never touch
`~/.t9`; they set `T9_VAULT` and `T9_HOME` to temporary directories.
