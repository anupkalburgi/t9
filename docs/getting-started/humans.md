# Getting started: for humans

This guide installs t9, links your first project, connects your coding agents, and shows how to review what
they record. It takes about 10 minutes. The vocabulary is explained in [concepts.md](concepts.md).

## 1. Install

```sh
curl -fsSL https://raw.githubusercontent.com/anupkalburgi/t9/main/install.sh | sh
t9 --version
```

The script picks the right binary (macOS or Linux, arm64 or x86_64), checks it against the release's
`SHA256SUMS`, and installs it to `~/.local/bin` (`T9_INSTALL_DIR` to change, `T9_VERSION` to pin). Run it again
to upgrade.

From source, with Rust 1.89 or newer:

```sh
cargo install --git https://github.com/anupkalburgi/t9 t9
```

## 2. The vault and its history

Everything t9 records lives in `~/.t9`:

```
~/.t9/
  projects/ inbox/     your data: Markdown files, the source of truth
  config.toml          settings (optional)
  .state/              index, locks, logs; disposable and git-ignored
  integrations/        generated plugins (`t9 install`); git-ignored
```

`~/.t9` is a git repository. After every command that changes it, t9 commits in the background. Commits use
your git identity (or `t9 <t9@localhost>` if you have none) and run your global git hooks. Browse the history
with:

```sh
t9 vault git log --oneline -20        # or: git -C ~/.t9 log
t9 vault git show HEAD
```

To back it up or sync it between machines, add a remote and push: `t9 vault git remote add origin …`.

Settings in `~/.t9/config.toml`:

```toml
git = true            # automatic commits (default true); T9_GIT=0 turns them off for one command
git_delay_ms = 1500   # how long the background committer waits, so a burst of writes becomes one commit
```

To use a different vault for a while, for example for experiments, set `T9_VAULT=/some/dir`.

## 3. Link your first project

```sh
cd ~/code/billing-service
t9 context
```

In a directory that isn't linked yet, `t9 context` says so and lists existing projects that look similar. To
create a project:

```sh
t9 project init --name "Billing Service" --key BIL
```

To add another checkout, worktree or clone to an existing project:

```sh
cd ~/worktrees/billing-service/feature-x
t9 project link BIL
```

Worktrees and clones with the same git remote usually resolve without linking; check with `t9 whereami`. To pin a
directory to a project explicitly, write a `.t9` file in it that contains the key.

## 4. Seed the plan

Agents add items as they work, but it helps to give them the big picture first:

```sh
t9 add plan "Move invoicing to the new payments API" -p 1
t9 add phase "Invoices and refunds" --parent BIL-1
t9 add feature "Exact tax rounding" --parent BIL-2
t9 add revisit "Totals differ by one cent on multi-currency invoices" --parent BIL-3 -b - <<'EOF'
3 of 200 sample invoices differ from the old system by one cent.
Evidence: runs/2026-11-05-invoice-diff/
EOF
t9 decide "Drop the legacy XML export" --because "no customer has used it in a year"
t9 ls --tree
```

## 5. Connect your agents

**Claude Code** uses a plugin that shows the brief at session start and after every compaction, and teaches
Claude when to record things:

```sh
t9 install claude
claude plugin marketplace add ~/.t9/integrations/claude
claude plugin install t9@t9
```

Start a new Claude Code session in the project and the brief appears at the top. Re-run `t9 install claude`
after upgrading t9.

**Codex and other agents** that read an `AGENTS.md` file:

```sh
t9 install agents-md                          # adds a section to ~/.codex/AGENTS.md
t9 install agents-md --file ./AGENTS.md       # or into one repository's AGENTS.md
t9 install agents-md --print                  # or just print the section
```

Running it again updates the section in place.

**Any other agent** that can run shell commands: tell it to run `t9 context` first and follow
[agents.md](agents.md).

## 6. Daily use

The quickest way to review a project is the terminal UI: `t9 tui` (see [tui.md](tui.md)). It shows the brief,
the plan tree, the decision log, the journal and triage, refreshes as agents work, and lets you record
decisions, notes and status changes. You can also open `~/.t9` in Obsidian or any editor.

From the shell:

| You want to… | Run |
|---|---|
| Browse everything interactively | `t9 tui` |
| See where a project stands | `t9 context` |
| See the open work as a tree | `t9 ls --tree` (add `--all` to include closed items) |
| See only bugs and revisits | `t9 ls -k bug -k revisit` |
| Look at one item and its history | `t9 show BIL-12` |
| See what happened recently | `t9 journal -n 30`, or `t9 journal --days 2 -A` across all projects |
| See what one agent session did | `t9 session ls`, then `t9 journal --of-session ABC123` |
| Find something | `t9 search "temp table decimal"` (`-A` searches all projects) |
| Record a decision | `t9 decide "…" --because "…" -r "alternative: why not" --area testing` |
| Rule something out | `t9 reject "…" --because "…"` |
| Change your mind | `t9 decide "…" -s BIL-17 --because "…"` |
| Set a rule for every project | `t9 decide "Always use uv" --global` |
| Review the decision log | `t9 decisions [--area A] [--md]`; `t9 check "topic"` |
| Park an idea | `t9 add idea "…"` |
| Add a thought about existing work | `t9 add revisit "…"` |
| Tidy up unfiled notes | `t9 triage`, then `t9 attach ENTRY BIL-12` |
| Fix a typo or rewrite a description | `t9 edit BIL-12` (opens `$EDITOR`) |
| Check for problems | `t9 doctor` |

Every command accepts `--json`.

## 7. Editing the vault by hand

Item files are ordinary Markdown, for example `~/.t9/projects/billing-service/items/BIL-12-….md`. You can
change front-matter fields such as `status`, `priority`, `tags` and `parent`, or the body, in any editor. Things
to know:

- Leave `id` alone. Ids are permanent.
- Journal bodies can be edited too. Don't touch the `<!-- t9 {...} -->` line above each entry.
- t9 notices your change immediately when it involves the item you then look at (`t9 show`, `t9 update`), a new
  or renamed file, or a journal from the last 7 days. Other in-place edits show up within 5 minutes. Run
  `t9 reindex` to force it.
- Keys you add to front matter are kept when t9 rewrites the file.

## 8. Troubleshooting

| Symptom | Fix |
|---|---|
| Changes are not being committed | `t9 doctor` reports the last failed commit; details are in `~/.t9/.state/git.log`. `t9 vault commit` commits now and shows the error. |
| "this directory is not linked to a project" | `t9 project link KEY` or `t9 project init` |
| An agent's entries land in the inbox | The agent ran in an unlinked directory. Link it; the entries move over automatically. |
| The brief didn't appear in Claude Code | `claude plugin list` should list t9. Check `t9` is on `PATH`, or set `T9_BIN`. |
| Something looks stale | `t9 reindex` |
| You want to know what t9 sees | `t9 whereami` shows the project, how it was found, the git branch, the session and the vault |
| It feels slow | `T9_TRACE=1 t9 context` prints timings; see [../design/performance.md](../design/performance.md) |
