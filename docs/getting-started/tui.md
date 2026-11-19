# The terminal UI and other ways to view the vault

## `t9 tui`

```sh
cd ~/code/billing-service && t9 tui     # opens on this directory's project
t9 tui                                  # anywhere else: the project list
```

The TUI reads the vault directly (no daemon) and refreshes every 2 seconds, so work that agents record shows up
while it is open. Anything you record from it is attributed to `human` and committed to git in the background.

| Screen | Shows |
|---|---|
| Projects | Every project with open items, open issues, accepted decisions and last activity. `Enter` opens one. Folders where agents worked without a project are listed as **⚠ not set up**; `i` creates the project there (name and key pre-filled) and moves the notes waiting in the inbox into it. Opening `t9 tui` inside such a folder selects it. |
| 1 Overview | The project at a glance, for a human: the project's **about** line, **waiting on you** (open questions, proposed decisions, blocked work), the last handoff, what the agent is focused on, work in progress, each plan with a progress bar, open issues, next tasks, decisions in force, options ruled out, global rules, parked ideas and recent activity ("2h ago"), running experiments, experiments **worth retrying**, and resources. Times use a 12-hour clock in the machine's local time zone (`8:11 PM EST`). One line per item, cut to the width of the pane; two columns on wide terminals. `b` switches to the agent's brief, exactly as an agent receives it. |
| 2 Items | The plan as a tree (plan → phase → feature → task, with bugs, revisits and ideas). The detail pane shows the selected item: fields, body, the decisions that apply to it, children and journal. `c` shows closed items. |
| 3 Decisions | The decision log, newest first, including rejected and superseded decisions, with the Why and rejected alternatives in the detail pane. |
| 4 Journal | All entries, newest first, with agent, item and refs. |
| 5 Triage | Entries not filed under an item, plus inbox entries from unlinked directories. `f` files one. |
| 6 Timeline | The last 60 days, newest first, by day: work started and finished, decisions, experiment outcomes, blockers, findings, handoffs, new plans, experiments and resources. |

**What changed recently?** Every row starts with a dot for how recently it changed: ● bright green within 15
minutes, ● green within the hour, • yellow today, · grey this week. The most recently touched item is tagged
**NEW**. A legend is at the top of the Overview.

**How current is it?** The status line at the bottom always shows `● live · refreshed 8:42:07 PM EST · last
update 3m ago by claude-code`; the dot turns yellow (paused) while a prompt or dialog is open. The Overview starts with
**LATEST**, the newest change (who, which item, what). Rows under In progress, Plan and Next up show when each
item was last touched, and items touched in the last 15 minutes get a green ● next to their id.

Keys (`?` shows them in the app):

| Key | Action |
|---|---|
| `1`–`6`, `Tab`, `Shift-Tab` | Switch tab |
| `↑` `↓` / `j` `k`, `PgUp` `PgDn`, `g` `G` | Move in the list (scroll the brief) |
| `J` `K` | Scroll the detail pane |
| `/` | Search; `Tab` switches between this project and all projects; `Enter` opens the hit |
| `d` | Record a decision: `Title \| why` |
| `x` | Record a rejected option: `Title \| why` |
| `a` / `A` | Add an item: `bug Crash on save` / add a child of the selected item |
| `n` | Add a note to the journal (on the selected item, if there is one) |
| `s` | Change status: start, block or close a task; accept or reject a proposed decision; supersede |
| `e` | Open the selected item's Markdown file in `$EDITOR` (the TUI pauses and resumes) |
| `r` | Refresh now |
| `Esc` / `←` | Back to the project list |
| `q`, `Ctrl-C` | Quit |

For reasons, alternatives, areas or scope, record the decision with `t9 decide` in a shell, or press `e` on it
afterwards and edit the file.

## Other ways to view the vault

Everything is Markdown with YAML front matter in `~/.t9`, so any tool that reads Markdown works:

- **Obsidian.** Open `~/.t9` as a vault ("Open folder as vault"). Items, journals and sessions are ordinary
  notes; the front matter appears as properties, so you can filter and sort by `status`, `kind` and `areas`
  (for example with the Dataview plugin). Edits are picked up by t9.
- **VS Code, or any editor.** Open `~/.t9/projects/<project>`. `items/` has one file per item, `journal/` one
  file per day.
- **The shell:** `t9 context`, `t9 ls --tree`, `t9 decisions`, `t9 journal`, `t9 show ID`, `t9 search "…"`.
  Add `--md` to `t9 decisions` for a document you can paste or share.
- **History:** `t9 vault git log --oneline`, or `git -C ~/.t9 log -p -- projects/<slug>/items/BIL-4-*.md` to see
  how one decision or task changed over time.
