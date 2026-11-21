# The web UI

```sh
t9 web            # opens this folder's project in your browser
t9 web BIL-119    # opens one item
t9 web --no-open  # just print the link
```

`t9 web` starts a small server inside the `t9` binary and opens your browser. There is nothing to install and
nothing leaves your machine. It refreshes on its own when agents record something (it checks every 3 seconds,
and waits if you are typing). Stop it with Ctrl-C.

## Pages

| Page | What you see |
|---|---|
| **Overview** | Waiting on you (questions, proposals, blocked work), Needs attention (overdue, due soon, ripe, stale, going in circles, closed without an outcome), the latest change, the last handoff, what the agent is working on, each plan with its progress, in progress, experiments, open issues, next tasks, decisions in force, resources, ruled-out options, ideas, recent activity. |
| **Map** | The project at the centre, plans and features branching out, leaves grouped by outcome (in progress, open issues, questions, experiments, decisions, ideas, dead ends, done). Click a node and it opens in a panel beside the map, with its path back to the project highlighted: its description, what is inside it with progress, what applies from history, its latest entries, and buttons to change its status, add a note or answer a question. Ids in the panel select on the map, so you can explore without leaving it; double-click (or "open full page") for the full page, and "show on the map" on any item page brings you back to it. Click a group to open it; drag to move; ⌘-scroll to zoom; Esc deselects. Dead ends have dashed red edges; ↻ marks a spinning branch. With nothing selected the panel shows **Are we going in circles?** (see [`t9 insights`](context-and-history.md#going-in-circles)) and the busiest branches. |
| **Plan** | The whole tree (plan → phase → feature → task, with bugs, revisits, questions, ideas, experiments) with progress per branch, expand and collapse, filters by kind and text, and closed items on request. |
| **Questions** | Open and answered questions, with when and by whom each was asked. |
| **Decisions** | Every decision, rejection and change of mind, newest first. |
| **Experiments** | Running, failed or inconclusive (with retry conditions), and succeeded. |
| **Resources** | Datasets, tables, models, endpoints, services, with where they live and who owns them. |
| **Timeline** | **Lanes**: one lane per feature (or plan, or group of loose items such as decisions and questions), most recently active first, with a sessions row and an activity histogram. Each lane shows what happened as marks (● new, ✓ done, ⇢ combined or superseded, ✗ dropped or failed, ◆ decision, ★ finding, ⚑ blocker, ✉ handoff, ↺ reopened, ? question); bursts collapse into one pill (`●45`, `✗5`), and quiet hours are squeezed into a marked break (`≈5h`) so busy stretches get the room. Open a lane to see each item as a bar from created to closed, coloured by outcome, with due dates (◇) and idle work hatched. Click anything to see its story in the side panel; double-click opens its page. **Story**: each day in a sentence ("Closed 17 (14 ✓, 3 ✗), 12 of them in one batch at 12:50 AM. Opened 6. Decided QR-130, QR-131…"), then its key events, with runs folded. Ranges: 24 hours to all. |
| **Journal** | Every entry. |
| **About** | The project's About. |
| **Search** | Full-text matches plus the topic's history (`t9 history`), oldest first. |

## Item pages

Every item ID anywhere in the UI is a link. Items that need attention carry a badge (`overdue 3 days`, `due
today`, `ripe`, `idle 6d`, `waiting`, `loop`). An item's page, and the Map panel, start with its **story**:

- **Goal**: why it exists (stated, or inferred from its parent, labelled so), with a button to state it;
- **Outcome**: what came out of it, read from the closing note (✓ done, ⇢ combined or superseded, ✗ dropped or
  failed, ~ inconclusive; batch closures are flagged), or for open work the latest progress;
- **Led to**: links both ways, and the items the closing note names ("combined into QR-19");
- **Inside**: for a feature or plan, how its children turned out, with an example note per group;
- **Now**: why it needs attention, and its due date.

Below the story, an item's page shows:

- where it sits in the plan (its parent chain) and what is inside it, with progress;
- its fields, its description, and its whole history (every journal entry about it);
- **where it came from**: when it was created, by which agent, in which session; the entries just before it
  (for a question, why it was asked); and what the agent was working on at the time;
- **what happened since**: how much was recorded afterwards, which items got the most attention, and the latest
  entries, so you can tell whether the agent moved on;
- decisions, experiments and resources that apply to it, and links in both directions;
- a form to change its status (closing asks what came out of it), set a due date, say it is still relevant, or
  add a note.

File paths mentioned in items and notes (for example `local/reports/…/options.md`) open as pages too, rendered if
they are Markdown. Only files inside the project's folder are shown, read-only, never hidden or credential-like
files.

## Answering questions

An open question has an **Answer** box. Your answer is recorded on the question, the question is closed, and (by
default) the answer is also recorded as a decision. Agents see it in their next brief and in recall, and follow
it.

## Security

The server listens on `127.0.0.1` only. The link `t9 web` prints carries a token for this run; the browser trades
it for an HttpOnly, SameSite=Strict cookie, and every request without it is refused. Requests addressed to any
other host name are refused (DNS-rebinding defense), and changes require a header that other web pages cannot
send. A strict Content-Security-Policy allows only t9's own script.
