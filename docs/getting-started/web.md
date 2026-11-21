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
| **Overview** | Waiting on you (questions, proposals, blocked work), the latest change, the last handoff, what the agent is working on, each plan with its progress, in progress, experiments, open issues, next tasks, decisions in force, resources, ruled-out options, ideas, recent activity. |
| **Map** | The project at the centre, plans and features branching out, leaves grouped by outcome (in progress, open issues, questions, experiments, decisions, ideas, dead ends, done). Click a group to open it and a node for its page. Dead ends have dashed red edges; ↻ marks a spinning branch. Above it, **Are we going in circles?** (see [`t9 insights`](context-and-history.md#going-in-circles)). |
| **Plan** | The whole tree (plan → phase → feature → task, with bugs, revisits, questions, ideas, experiments) with progress per branch, expand and collapse, filters by kind and text, and closed items on request. |
| **Questions** | Open and answered questions, with when and by whom each was asked. |
| **Decisions** | Every decision, rejection and change of mind, newest first. |
| **Experiments** | Running, failed or inconclusive (with retry conditions), and succeeded. |
| **Resources** | Datasets, tables, models, endpoints, services, with where they live and who owns them. |
| **Timeline**, **Journal** | What happened when, by day; every entry. |
| **About** | The project's About. |
| **Search** | Full-text matches plus the topic's history (`t9 history`), oldest first. |

## Item pages

Every item ID anywhere in the UI is a link. An item's page shows:

- where it sits in the plan (its parent chain) and what is inside it, with progress;
- its fields, its description, and its whole history (every journal entry about it);
- **where it came from**: when it was created, by which agent, in which session; the entries just before it
  (for a question, why it was asked); and what the agent was working on at the time;
- **what happened since**: how much was recorded afterwards, which items got the most attention, and the latest
  entries, so you can tell whether the agent moved on;
- decisions, experiments and resources that apply to it, and links in both directions;
- a form to change its status or add a note.

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
