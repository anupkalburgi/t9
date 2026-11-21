//! Rendering. Pure function of [`App`] state.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap};
use t9_core::context::{first_line, local_time};
use t9_core::decisions::why;
use t9_core::env::display_path;
use t9_core::*;

use crate::app::{App, Detail, Mode, Screen, Tab};

fn status_style(s: Status) -> Style {
    match s {
        Status::Active => Style::new().fg(Color::Green),
        Status::Blocked => Style::new().fg(Color::Red),
        Status::Done | Status::Dropped | Status::Superseded => Style::new().fg(Color::DarkGray),
        Status::Rejected => Style::new().fg(Color::Red).add_modifier(Modifier::DIM),
        Status::Failed => Style::new().fg(Color::Red),
        Status::Inconclusive => Style::new().fg(Color::Yellow).add_modifier(Modifier::DIM),
        Status::Accepted => Style::new().fg(Color::Cyan),
        Status::Proposed => Style::new().fg(Color::Yellow),
        Status::Open => Style::new(),
    }
}

fn kind_style(k: Kind) -> Style {
    match k {
        Kind::Bug | Kind::Regression => Style::new().fg(Color::Red),
        Kind::Revisit | Kind::Question => Style::new().fg(Color::Yellow),
        Kind::Decision => Style::new().fg(Color::Cyan),
        Kind::Idea => Style::new().fg(Color::Magenta),
        Kind::Experiment => Style::new().fg(Color::Green),
        Kind::Resource => Style::new().fg(Color::LightBlue),
        Kind::Plan | Kind::Phase | Kind::Feature => Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD),
        Kind::Task | Kind::Note => Style::new(),
    }
}

fn panel(title: &str, focused: bool) -> Block<'static> {
    let b = Block::bordered().title(format!(" {title} "));
    if focused {
        b.border_style(Style::new().fg(Color::Cyan))
    } else {
        b.border_style(Style::new().fg(Color::DarkGray))
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let [header, body, footer] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)]).areas(f.area());
    draw_header(f, app, header);
    match app.screen {
        Screen::Projects => draw_projects(f, app, body),
        Screen::Project => match app.tab {
            Tab::Brief => draw_brief(f, app, body),
            Tab::Items => draw_items(f, app, body),
            Tab::Decisions => draw_decisions(f, app, body),
            Tab::Journal => draw_entries(f, app, body, false),
            Tab::Triage => draw_entries(f, app, body, true),
            Tab::Timeline => draw_timeline(f, app, body),
        },
    }
    draw_footer(f, app, footer);
    match &app.mode {
        Mode::Help => draw_help(f),
        Mode::Input(i) => {
            let area = centered(f.area(), 80, 7);
            f.render_widget(Clear, area);
            let text = Text::from(vec![
                Line::from(Span::styled(i.hint.clone(), Style::new().fg(Color::DarkGray))),
                Line::from(""),
                Line::from(vec![
                    Span::raw("> "),
                    Span::raw(i.buffer.clone()),
                    Span::styled("█", Style::new().fg(Color::Cyan)),
                ]),
                Line::from(""),
                Line::from(Span::styled("Enter to save · Esc to cancel", Style::new().fg(Color::DarkGray))),
            ]);
            f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }).block(panel(&i.prompt, true)), area);
        }
        Mode::Pick(p) => {
            let area = centered(f.area(), 34, p.options.len() as u16 + 2);
            f.render_widget(Clear, area);
            let items: Vec<ListItem> =
                p.options.iter().map(|s| ListItem::new(Span::styled(s.as_str(), status_style(*s)))).collect();
            let mut st = ListState::default().with_selected(Some(p.sel));
            f.render_stateful_widget(
                List::new(items)
                    .block(panel(&format!("Status of {}", p.item), true))
                    .highlight_symbol("▶ ")
                    .highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
                area,
                &mut st,
            );
        }
        Mode::Search(s) => {
            let area = centered(f.area(), 110, f.area().height.saturating_sub(4).max(10));
            f.render_widget(Clear, area);
            let scope = if s.all_projects { "all projects" } else { "this project" };
            let [q, list] = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).areas(area);
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::raw("/ "),
                    Span::raw(s.query.clone()),
                    Span::styled("█", Style::new().fg(Color::Cyan)),
                ]))
                .block(panel(&format!("Search {scope} · Tab: switch scope · Enter: open · Esc: close"), true)),
                q,
            );
            let items: Vec<ListItem> = s
                .hits
                .iter()
                .map(|h| {
                    let id = if h.kind == "entry" { format!("entry {}", short_id(&h.id)) } else { h.id.clone() };
                    let snippet = h.snippet.replace('\n', " ");
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("{:<7}", h.project), Style::new().fg(Color::DarkGray)),
                        Span::styled(format!("{id:<13}"), Style::new().fg(Color::Cyan)),
                        Span::raw(if h.title.is_empty() { snippet } else { format!("{} — {snippet}", h.title) }),
                    ]))
                })
                .collect();
            let mut st = ListState::default().with_selected(if s.hits.is_empty() { None } else { Some(s.sel) });
            let title = format!("{} results", s.hits.len());
            f.render_stateful_widget(
                List::new(items)
                    .block(panel(&title, false))
                    .highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
                list,
                &mut st,
            );
        }
        Mode::Normal => {}
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let mut spans = vec![Span::styled(" t9 ", Style::new().bg(Color::Cyan).fg(Color::Black).bold()), Span::raw(" ")];
    match app.screen {
        Screen::Projects => {
            spans.push(Span::styled("Projects", Style::new().bold()));
            f.render_widget(Line::from(spans), area);
        }
        Screen::Project => {
            let name = app.project_name();
            spans.push(Span::styled(name.clone(), Style::new().bold()));
            spans.push(Span::raw("  "));
            let used = (name.chars().count() + 7) as u16;
            let [left, right] =
                Layout::horizontal([Constraint::Length(used.min(area.width)), Constraint::Min(0)]).areas(area);
            f.render_widget(Line::from(spans), left);
            let titles: Vec<&str> = Tab::ALL.iter().map(|t| t.title()).collect();
            let idx = Tab::ALL.iter().position(|t| *t == app.tab).unwrap_or(0);
            f.render_widget(
                Tabs::new(titles)
                    .select(idx)
                    .highlight_style(Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD | Modifier::UNDERLINED))
                    .divider(" "),
                right,
            );
        }
    }
}

/// `● live · refreshed 18:42:07 · last update 3m ago by claude-code`.
fn freshness(app: &App) -> Vec<Span<'static>> {
    let live = (chrono::Local::now() - app.refreshed_at).num_seconds() < 6;
    let mut spans = vec![
        Span::styled(" ● ", Style::new().fg(if live { Color::Green } else { Color::Yellow })),
        Span::styled(
            format!(
                "{} {}",
                if live { "live · refreshed" } else { "paused · refreshed" },
                t9_core::clock::time_secs(&app.refreshed_at.with_timezone(&chrono::Utc))
            ),
            Style::new().fg(Color::DarkGray),
        ),
    ];
    if app.screen == Screen::Project
        && let Some(l) = &app.latest
    {
        spans.push(Span::styled(" · last update ", Style::new().fg(Color::DarkGray)));
        spans.push(Span::styled(ago(&l.at), Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(format!(" by {}", l.who), Style::new().fg(Color::DarkGray)));
    }
    spans
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let mut spans = freshness(app);
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let room = (area.width as usize).saturating_sub(used + 3);
    if let Some(m) = &app.message {
        let style = if m.starts_with("error") { Style::new().fg(Color::Red) } else { Style::new().fg(Color::Yellow) };
        spans.push(Span::styled(format!(" │ {}", fit(m, room)), style));
        f.render_widget(Line::from(spans), area);
        return;
    }
    let hints = match (app.screen, app.tab) {
        (Screen::Projects, _) => "↑↓ select · Enter open · i set up a folder · / search all · ? help · q quit",
        (_, Tab::Brief) => {
            "? help · ↑↓ scroll · b agent brief · d decide · x reject · a add · n note · / search · 1-5 tabs"
        }
        (_, Tab::Items) => {
            "? help · ↑↓ select · s status · e edit · a add · A child · n note · c closed · J/K scroll detail"
        }
        (_, Tab::Decisions) => {
            "? help · ↑↓ select · s accept/reject · e edit · d decide · x reject · n note · / search"
        }
        (_, Tab::Journal) => "? help · ↑↓ select · J/K scroll · n note · / search · Esc projects",
        (_, Tab::Triage) => "? help · ↑↓ select · f file under an item · n note · / search",
        (_, Tab::Timeline) => "? help · ↑↓ scroll · last 60 days, newest first · t9 history \"topic\" for one subject",
    };
    if room > 8 {
        spans.push(Span::styled(format!(" │ {}", fit(hints, room)), Style::new().fg(Color::DarkGray)));
    }
    f.render_widget(Line::from(spans), area);
}

fn draw_projects(f: &mut Frame, app: &mut App, area: Rect) {
    let [left, right] = Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)]).areas(area);
    let rows: Vec<ListItem> = app
        .projects
        .iter()
        .map(|r| {
            let last = r.last.as_ref().map(local_time).unwrap_or_else(|| "—".into());
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:<8}", r.project.meta.key), Style::new().fg(Color::Cyan)),
                Span::styled(format!("{:<26}", first_line(&r.project.meta.name, 24)), Style::new().bold()),
                Span::raw(format!("{:>4} open ", r.open)),
                Span::styled(
                    format!("{:>3} issues ", r.issues),
                    if r.issues > 0 { Style::new().fg(Color::Yellow) } else { Style::new() },
                ),
                Span::raw(format!("{:>3} decisions  ", r.decisions)),
                Span::styled(last, Style::new().fg(Color::DarkGray)),
            ]))
        })
        .collect::<Vec<_>>();
    // Folders where agents worked without a project, so the human can set them up.
    let mut rows = rows;
    for u in &app.unlinked {
        let notes = if u.inbox_entries > 0 { format!(" · {} notes in inbox", u.inbox_entries) } else { String::new() };
        rows.push(ListItem::new(Line::from(vec![
            Span::styled("⚠ not set up ", Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:<26}", first_line(&display_path(std::path::Path::new(&u.cwd)), 24)), Style::new()),
            Span::styled(format!("{} · {}{notes}", u.agent, ago(&u.last_seen)), Style::new().fg(Color::DarkGray)),
        ])));
    }
    let empty = rows.is_empty();
    let mut st = ListState::default().with_selected(if empty { None } else { Some(app.proj_sel) });
    let title = if app.unlinked.is_empty() {
        "Projects".to_string()
    } else {
        format!("Projects · {} folder(s) not set up: press i", app.unlinked.len())
    };
    f.render_stateful_widget(
        List::new(rows).block(panel(&title, true)).highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
        left,
        &mut st,
    );
    let text = if let Some(u) = app.selected_folder() {
        let mut lines = vec![
            Line::from(Span::styled("Not linked to a t9 project", Style::new().bold().fg(Color::Yellow))),
            Line::from(""),
            Line::from(format!("folder   {}", display_path(std::path::Path::new(&u.cwd)))),
            Line::from(format!("seen     {} by {}", ago(&u.last_seen), u.agent)),
        ];
        if u.inbox_entries > 0 {
            lines.push(Line::from(format!("inbox    {} notes waiting for a project", u.inbox_entries)));
        }
        lines.push(Line::from(""));
        lines.push(Line::from("Agents working here record nothing under a project until it is set up."));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::raw("Press "),
            Span::styled("i", Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(format!(" to create {} ({}) here, or run in that folder:", u.name, u.key)),
        ]));
        lines.push(Line::from(Span::styled(
            format!("  {}", t9_core::unlinked::init_command(&u.name, &u.key)),
            Style::new().fg(Color::Cyan),
        )));
        lines.push(Line::from("  t9 project link KEY      (if it belongs to an existing project)"));
        Text::from(lines)
    } else {
        match app.projects.get(app.proj_sel) {
            Some(r) => {
                let mut lines = vec![Line::from(Span::styled(r.project.meta.name.clone(), Style::new().bold()))];
                for root in &r.project.meta.roots {
                    lines.push(Line::from(format!("root    {}", display_path(std::path::Path::new(root)))));
                }
                for rem in &r.project.meta.remotes {
                    lines.push(Line::from(format!("remote  {rem}")));
                }
                lines.push(Line::from(""));
                lines.extend(r.project.description.lines().map(|l| Line::from(l.to_string())));
                Text::from(lines)
            }
            None => Text::from("No projects yet.\n\nIn a project folder run:\n  t9 project init --name \"Name\""),
        }
    };
    f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }).block(panel("Details", false)), right);
}

fn draw_timeline(f: &mut Frame, app: &App, area: Rect) {
    let w = area.width.saturating_sub(4) as usize;
    let mut lines: Vec<Line<'static>> = vec![];
    let mut day = String::new();
    for e in &app.timeline {
        let d = e.at.with_timezone(&chrono::Local).format("%A, %b %-d").to_string();
        if d != day {
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(d.clone(), Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD))));
            day = d;
        }
        let time = e.at.with_timezone(&chrono::Local).format("%l:%M %p").to_string();
        let id = if e.source == "entry" { e.item.clone().unwrap_or_default() } else { e.id.clone() };
        let style = match e.kind.as_str() {
            "decision" => Style::new().fg(Color::Cyan),
            "handoff" => Style::new().fg(Color::Magenta),
            "blocker" => Style::new().fg(Color::Red),
            "finding" => Style::new().fg(Color::Green),
            k if k.starts_with("new ") => Style::new().fg(Color::Yellow),
            _ => Style::new().fg(Color::DarkGray),
        };
        let used = 10 + 16 + 9;
        lines.push(Line::from(vec![
            Span::styled(format!("{time}  "), Style::new().fg(Color::DarkGray)),
            Span::styled(format!("{:<16}", e.kind), style),
            Span::styled(format!("{id:<9}"), Style::new().fg(Color::DarkGray)),
            Span::raw(fit(&e.title, w.saturating_sub(used))),
        ]));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled("Nothing in the last 60 days.", Style::new().fg(Color::DarkGray))));
    }
    f.render_widget(
        Paragraph::new(lines)
            .scroll((app.timeline_scroll, 0))
            .block(panel("Timeline: what happened when (last 60 days)", true)),
        area,
    );
}

fn md_lines(body: &str) -> Vec<Line<'static>> {
    body.lines()
        .map(|l| {
            if let Some(h) = l.strip_prefix("## ").or_else(|| l.strip_prefix("# ")) {
                Line::from(Span::styled(h.to_string(), Style::new().bold().fg(Color::Blue)))
            } else {
                Line::from(l.to_string())
            }
        })
        .collect()
}

fn draw_brief(f: &mut Frame, app: &App, area: Rect) {
    if app.raw_brief {
        let text = Text::from(md_lines(&app.brief));
        f.render_widget(
            Paragraph::new(text).wrap(Wrap { trim: false }).scroll((app.brief_scroll, 0)).block(panel(
                "Agent brief: exactly what an agent receives at session start (b: back to overview)",
                true,
            )),
            area,
        );
        return;
    }
    let Some(b) = &app.overview else { return };
    let block = panel("Overview (b: the agent's brief)", true);
    let inner = block.inner(area);
    f.render_widget(block, area);
    // Two columns when there is room, one otherwise.
    if inner.width >= 150 {
        let [l, r] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(inner);
        let (left, right) = overview_columns(app, b, (l.width.saturating_sub(2)) as usize, true);
        f.render_widget(Paragraph::new(left).scroll((app.brief_scroll, 0)), pad(l));
        f.render_widget(Paragraph::new(right).scroll((app.brief_scroll, 0)), pad(r));
    } else {
        let (all, _) = overview_columns(app, b, inner.width.saturating_sub(2) as usize, false);
        f.render_widget(Paragraph::new(all).scroll((app.brief_scroll, 0)), pad(inner));
    }
}

fn pad(r: Rect) -> Rect {
    Rect { x: r.x + 1, width: r.width.saturating_sub(2), ..r }
}

/// Truncates to `w` characters, ending with "…" when cut.
fn fit(s: &str, w: usize) -> String {
    let s = s.lines().next().unwrap_or("");
    if s.chars().count() <= w {
        s.to_string()
    } else if w == 0 {
        String::new()
    } else {
        format!("{}…", s.chars().take(w - 1).collect::<String>().trim_end())
    }
}

/// Word-wraps to `w` columns, at most `max` lines (the last one ends with "…" if text remains).
fn wrap_text(s: &str, w: usize, max: usize) -> Vec<String> {
    let w = w.max(10);
    let mut lines: Vec<String> = vec![];
    for para in s.lines() {
        let mut cur = String::new();
        for word in para.split_whitespace() {
            if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > w {
                lines.push(std::mem::take(&mut cur));
            }
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(word);
        }
        lines.push(cur);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    if lines.len() > max {
        lines.truncate(max);
        let last = lines.last_mut().unwrap();
        *last = fit(&format!("{last} …"), w);
        if !last.ends_with('…') {
            last.push('…');
        }
    }
    lines
}

/// "just now", "12m ago", "3h ago", "yesterday", "4d ago", or the date.
fn ago(t: &Timestamp) -> String {
    let secs = (now() - *t).num_seconds().max(0);
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", secs / 60),
        3600..86400 => format!("{}h ago", secs / 3600),
        86400..172800 => "yesterday".into(),
        172800..604800 => format!("{}d ago", secs / 86400),
        _ => t.with_timezone(&chrono::Local).format("%b %d").to_string(),
    }
}

fn header(title: &str, count: Option<usize>, color: Color, note: &str) -> Line<'static> {
    let mut spans =
        vec![Span::styled(format!(" {title} "), Style::new().bg(color).fg(Color::Black).add_modifier(Modifier::BOLD))];
    if let Some(n) = count {
        spans.push(Span::styled(format!(" {n}"), Style::new().fg(color).add_modifier(Modifier::BOLD)));
    }
    if !note.is_empty() {
        spans.push(Span::styled(format!("  {note}"), Style::new().fg(Color::DarkGray)));
    }
    Line::from(spans)
}

/// A two-cell marker for how recently something changed: ● bright green (15 min), ● green (1 h),
/// • yellow (24 h), · grey (7 days), blank (older).
fn recency(t: &Timestamp) -> Span<'static> {
    let mins = (now() - *t).num_minutes();
    match mins {
        ..15 => Span::styled("● ", Style::new().fg(Color::LightGreen).add_modifier(Modifier::BOLD)),
        15..60 => Span::styled("● ", Style::new().fg(Color::Green)),
        60..1440 => Span::styled("• ", Style::new().fg(Color::Yellow)),
        1440..10080 => Span::styled("· ", Style::new().fg(Color::DarkGray)),
        _ => Span::raw("  "),
    }
}

/// The legend for [`recency`] markers.
fn recency_legend() -> Line<'static> {
    let dim = Style::new().fg(Color::DarkGray);
    Line::from(vec![
        Span::styled("● ", Style::new().fg(Color::LightGreen).add_modifier(Modifier::BOLD)),
        Span::styled("15 min  ", dim),
        Span::styled("● ", Style::new().fg(Color::Green)),
        Span::styled("1 hour  ", dim),
        Span::styled("• ", Style::new().fg(Color::Yellow)),
        Span::styled("today  ", dim),
        Span::styled("· ", dim),
        Span::styled("this week  ", dim),
        Span::styled(" NEW ", Style::new().bg(Color::LightGreen).fg(Color::Black).add_modifier(Modifier::BOLD)),
        Span::styled(" most recent", dim),
    ])
}

fn new_tag() -> Span<'static> {
    Span::styled(" NEW ", Style::new().bg(Color::LightGreen).fg(Color::Black).add_modifier(Modifier::BOLD))
}

/// `● BIL-22  bug       active   p0  Title…   extra`, fitted to `width`. The dot shows how recently the item
/// changed; the most recent item gets a NEW tag.
fn row(app: &App, i: &Item, width: usize, show_status: bool, extra: &str) -> Line<'static> {
    let m = &i.meta;
    let is_new = app.newest.as_deref() == Some(m.id.as_str());
    let mut spans = vec![
        recency(&app.touched_at(i)),
        Span::styled(format!("{:<8}", m.id), Style::new().fg(if is_new { Color::LightGreen } else { Color::DarkGray })),
        Span::styled(format!("{:<9}", m.kind.as_str()), kind_style(m.kind)),
    ];
    let mut used = 19 + if is_new { 6 } else { 0 };
    if show_status {
        spans.push(Span::styled(format!("{:<9}", m.status.as_str()), status_style(m.status)));
        used += 9;
    }
    let pri = m.priority.map(|p| format!("p{p} ")).unwrap_or_default();
    used += pri.len();
    spans.push(Span::styled(pri, Style::new().fg(if m.priority == Some(0) { Color::Red } else { Color::Yellow })));
    let extra_w = if extra.is_empty() { 0 } else { extra.chars().count() + 2 };
    let title_w = width.saturating_sub(used + extra_w);
    let title = fit(&m.title, title_w);
    let gap = title_w.saturating_sub(title.chars().count());
    spans.push(Span::raw(title));
    if is_new {
        spans.push(Span::raw(" "));
        spans.push(new_tag());
    }
    if !extra.is_empty() {
        spans.push(Span::raw(" ".repeat(gap + 2)));
        spans.push(Span::styled(extra.to_string(), Style::new().fg(Color::DarkGray)));
    }
    Line::from(spans)
}

/// A row with the item's age in the right-hand column.
fn fresh_row(app: &App, i: &Item, width: usize, show_status: bool, extra: &str) -> Line<'static> {
    let age = ago(&app.touched_at(i));
    let extra = if extra.is_empty() { age } else { format!("{extra} · {age}") };
    row(app, i, width, show_status, &extra)
}

fn more(n: usize) -> Line<'static> {
    Line::from(Span::styled(format!("        … {n} more"), Style::new().fg(Color::DarkGray)))
}

fn section(out: &mut Vec<Line<'static>>, head: Line<'static>, rows: Vec<Line<'static>>, cap: usize) {
    if rows.is_empty() {
        return;
    }
    if !out.is_empty() {
        out.push(Line::from(""));
    }
    out.push(head);
    let total = rows.len();
    out.extend(rows.into_iter().take(cap));
    if total > cap {
        out.push(more(total - cap));
    }
}

fn progress(done: usize, total: usize, width: usize) -> Line<'static> {
    let bar_w = width.saturating_sub(16).clamp(10, 40);
    let filled = (done * bar_w).checked_div(total).unwrap_or(0);
    Line::from(vec![
        Span::styled("█".repeat(filled), Style::new().fg(Color::Green)),
        Span::styled("░".repeat(bar_w - filled), Style::new().fg(Color::DarkGray)),
        Span::styled(format!("  {done}/{total} done"), Style::new().fg(Color::Green)),
    ])
}

/// The Overview: what a human reviewing the project wants first. Returns (left, right) columns; with
/// `two == false` everything is in the left one.
fn overview_columns(
    app: &App,
    b: &t9_core::context::Brief,
    w: usize,
    two: bool,
) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
    let mut left: Vec<Line<'static>> = vec![];
    let mut right: Vec<Line<'static>> = vec![];

    left.push(recency_legend());

    // What changed last, so it is obvious how current this is.
    if let Some(l) = &app.latest {
        let rows = vec![Line::from(vec![
            Span::styled(format!("{} ", l.who), Style::new().fg(Color::Green)),
            Span::raw(fit(&l.what, w.saturating_sub(l.who.chars().count() + 1))),
        ])];
        let when = format!("{} · {}", ago(&l.at), t9_core::clock::weekday_time(&l.at));
        section(&mut left, header("LATEST", None, Color::Green, &when), rows, 1);
    }

    // About: what the project is, in a line or two.
    if let Some(what) = b.about.iter().find(|s| s.key == "what" && !s.text.trim().is_empty()) {
        let rows: Vec<Line> =
            wrap_text(&what.text.replace("\n- ", "; ").replace("- ", ""), w, 3).into_iter().map(Line::from).collect();
        section(&mut left, header("ABOUT", None, Color::Blue, "t9 about"), rows, 3);
    } else {
        section(
            &mut left,
            header("ABOUT", None, Color::Blue, ""),
            vec![Line::from(Span::styled(
                "Not written yet: t9 about set what \"…\" (agents draft it from the README)",
                Style::new().fg(Color::DarkGray),
            ))],
            1,
        );
    }

    // Waiting on you: questions, proposed decisions, blocked work.
    let mut waiting: Vec<Line> = vec![];
    for i in b.issues.iter().filter(|i| i.meta.kind == Kind::Question) {
        waiting.push(row(app, i, w, false, ""));
    }
    for d in app.decisions.iter().filter(|d| d.meta.status == Status::Proposed) {
        waiting.push(row(app, d, w, true, "accept or reject: s"));
    }
    for i in b.in_progress.iter().filter(|i| i.meta.status == Status::Blocked) {
        waiting.push(row(app, i, w, true, ""));
    }
    section(
        &mut left,
        header("WAITING ON YOU", Some(waiting.len()), Color::Red, "questions, proposals, blocked"),
        waiting,
        8,
    );

    if let Some(h) = &b.handoff {
        let who = if h.meta.agent.is_empty() { "unknown".to_string() } else { h.meta.agent.clone() };
        let rows: Vec<Line> = wrap_text(&h.body, w, 8).into_iter().map(Line::from).collect();
        section(
            &mut left,
            header("LAST HANDOFF", None, Color::Magenta, &format!("{who} · {}", ago(&h.meta.at))),
            rows,
            8,
        );
    }
    if let Some(focus) = &b.focus {
        let agent = b.session.as_ref().map(|s| s.meta.agent.clone()).unwrap_or_default();
        section(&mut left, header("AGENT FOCUS", None, Color::Green, &agent), vec![row(app, focus, w, true, "")], 1);
    }
    let working: Vec<Line> = b
        .in_progress
        .iter()
        .filter(|i| i.meta.status == Status::Active)
        .map(|i| fresh_row(app, i, w, false, ""))
        .collect();
    section(&mut left, header("IN PROGRESS", Some(working.len()), Color::Green, ""), working, 10);

    for pv in &b.plans {
        let mut rows = vec![
            Line::from(Span::styled(fit(&pv.plan.meta.title, w), Style::new().add_modifier(Modifier::BOLD))),
            progress(pv.done, pv.total, w),
        ];
        for (depth, i, open) in pv.outline.iter().take(10) {
            let mut l = fresh_row(app, i, w.saturating_sub(2 * (depth - 1)), true, &format!("{open} open"));
            l.spans.insert(0, Span::raw("  ".repeat(depth - 1)));
            rows.push(l);
        }
        section(&mut left, header("PLAN", None, Color::Blue, &pv.plan.meta.id), rows, 14);
    }

    let col = if two { &mut right } else { &mut left };
    let issues: Vec<&Item> = b.issues.iter().filter(|i| i.meta.kind != Kind::Question).collect();
    let count = |k: Kind| issues.iter().filter(|i| i.meta.kind == k).count();
    let note = [("bugs", Kind::Bug), ("regressions", Kind::Regression), ("revisits", Kind::Revisit)]
        .iter()
        .filter(|(_, k)| count(*k) > 0)
        .map(|(n, k)| format!("{} {n}", count(*k)))
        .collect::<Vec<_>>()
        .join(" · ");
    let rows: Vec<Line> = issues.iter().map(|i| row(app, i, w, false, "")).collect();
    section(col, header("OPEN ISSUES", Some(rows.len()), Color::Yellow, &note), rows, 12);

    let exps: Vec<Line> = b.experiments.iter().map(|i| fresh_row(app, i, w, true, "")).collect();
    section(col, header("EXPERIMENTS", Some(exps.len()), Color::Green, "running"), exps, 6);
    let retry: Vec<Line> = b
        .worth_retrying
        .iter()
        .map(|i| row(app, i, w, true, &i.meta.retry_when.clone().map(|r| format!("when: {r}")).unwrap_or_default()))
        .collect();
    section(col, header("WORTH RETRYING", Some(retry.len()), Color::Green, "retry date passed"), retry, 5);
    let next: Vec<Line> = b.next_up.iter().map(|i| fresh_row(app, i, w, false, "")).collect();
    section(col, header("NEXT UP", Some(next.len()), Color::Cyan, "open tasks by priority"), next, 6);

    let mut decs: Vec<Line> = b.applicable.iter().map(|r| row(app, &r.item, w, true, "applies to focus")).collect();
    decs.extend(b.decisions.iter().map(|d| row(app, d, w, false, &ago(&d.meta.updated))));
    section(
        col,
        header("DECISIONS", Some(b.decisions.len() + b.applicable.len()), Color::Cyan, "in force; full log: 3"),
        decs,
        8,
    );
    let ruled: Vec<Line> = b.ruled_out.iter().map(|d| row(app, d, w, false, "")).collect();
    section(col, header("RULED OUT", Some(ruled.len()), Color::DarkGray, "do not re-propose"), ruled, 5);
    let global: Vec<Line> = b.global_rules.iter().map(|d| row(app, d, w, false, "")).collect();
    section(col, header("RULES FOR EVERY PROJECT", Some(global.len()), Color::Cyan, ""), global, 5);
    let res: Vec<Line> = b
        .resources
        .iter()
        .map(|r| {
            let t = r.meta.resource_type.clone().unwrap_or_default();
            let loc = r.meta.location.clone().unwrap_or_default();
            row(app, r, w, false, &format!("{t} · {loc}"))
        })
        .collect();
    section(col, header("RESOURCES", Some(res.len()), Color::LightBlue, "datasets, models, services …"), res, 8);
    let ideas: Vec<Line> = b.ideas.iter().map(|i| row(app, i, w, false, "")).collect();
    section(col, header("PARKED IDEAS", Some(ideas.len()), Color::Magenta, ""), ideas, 5);

    let recent: Vec<Line> = b
        .recent
        .iter()
        .map(|e| {
            let when = format!("{:<10}", ago(&e.meta.at));
            let item = format!("{:<8}", e.meta.item.clone().unwrap_or_else(|| "—".into()));
            let agent = format!("{:<12}", fit(&e.meta.agent, 11));
            let used = 10 + 8 + 12;
            Line::from(vec![
                Span::styled(when, Style::new().fg(Color::DarkGray)),
                Span::styled(item, Style::new().fg(Color::Blue)),
                Span::styled(agent, Style::new().fg(Color::DarkGray)),
                Span::raw(fit(&e.body, w.saturating_sub(used))),
            ])
        })
        .collect();
    section(col, header("RECENT ACTIVITY", None, Color::Blue, "full journal: 4"), recent, 8);

    let mut attention: Vec<Line> = vec![];
    if b.unfiled > 0 {
        attention.push(Line::from(format!("{} recent entries not filed under an item (tab 5)", b.unfiled)));
    }
    if b.inbox > 0 {
        attention.push(Line::from(format!("{} inbox entries from unlinked directories (tab 5)", b.inbox)));
    }
    for a in &b.attention.items {
        let color = if a.severity >= 4 { Color::Red } else { Color::Yellow };
        attention.push(Line::from(vec![
            Span::styled(format!("{:<8} ", a.id), Style::new().fg(Color::DarkGray)),
            Span::styled(format!("{} ", a.detail), Style::new().fg(color)),
            Span::raw(a.title.chars().take(w.saturating_sub(a.detail.len() + 12)).collect::<String>()),
        ]));
    }
    if b.attention.backlog > 0 {
        attention.push(Line::from(format!("{} open items untouched for 30+ days", b.attention.backlog)));
    }
    section(col, header("NEEDS ATTENTION", Some(attention.len()), Color::Yellow, "t9 pending"), attention, 8);

    if left.is_empty() && right.is_empty() {
        left.push(Line::from(Span::styled(
            "Nothing recorded yet. Agents fill this in as they work; press a to add an item.",
            Style::new().fg(Color::DarkGray),
        )));
    }
    (left, right)
}

fn split(area: Rect) -> [Rect; 2] {
    Layout::horizontal([Constraint::Percentage(48), Constraint::Percentage(52)]).areas(area)
}

fn item_row(app: &App, depth: usize, i: &Item, width: usize) -> ListItem<'static> {
    let is_new = app.newest.as_deref() == Some(i.meta.id.as_str());
    let mut spans = vec![
        recency(&app.touched_at(i)),
        Span::raw("  ".repeat(depth)),
        Span::styled(
            format!("{:<8}", i.meta.id),
            Style::new().fg(if is_new { Color::LightGreen } else { Color::DarkGray }),
        ),
        Span::styled(format!("{:<9}", i.meta.kind.as_str()), kind_style(i.meta.kind)),
        Span::styled(format!("{:<9}", i.meta.status.as_str()), status_style(i.meta.status)),
    ];
    if let Some(p) = i.meta.priority {
        spans.push(Span::styled(format!("p{p} "), Style::new().fg(Color::Yellow)));
    }
    let used = 2 + 2 * depth + 8 + 9 + 9 + i.meta.priority.map(|_| 3).unwrap_or(0) + 3 + if is_new { 6 } else { 0 };
    spans.push(Span::raw(fit(&i.meta.title, width.saturating_sub(used))));
    if is_new {
        spans.push(Span::raw(" "));
        spans.push(new_tag());
    }
    ListItem::new(Line::from(spans))
}

fn draw_items(f: &mut Frame, app: &mut App, area: Rect) {
    let [left, right] = split(area);
    let w = left.width.saturating_sub(2) as usize;
    let rows: Vec<ListItem> = app.items.iter().map(|(d, i)| item_row(app, *d, i, w)).collect();
    let title = format!("Items ({}{})", app.items.len(), if app.show_closed { ", incl. closed" } else { "" });
    let mut st = ListState::default().with_selected(if app.items.is_empty() { None } else { Some(app.item_sel) });
    f.render_stateful_widget(
        List::new(rows).block(panel(&title, true)).highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
        left,
        &mut st,
    );
    draw_detail(f, app, right);
}

fn draw_decisions(f: &mut Frame, app: &mut App, area: Rect) {
    let [left, right] = split(area);
    let rows: Vec<ListItem> = app
        .decisions
        .iter()
        .map(|d| {
            let mut spans = vec![
                recency(&app.touched_at(d)),
                Span::styled(format!("{:<8}", d.meta.id), Style::new().fg(Color::DarkGray)),
                Span::styled(format!("{:<11}", d.meta.status.as_str()), status_style(d.meta.status)),
                Span::raw(fit(
                    &d.meta.title,
                    (left.width as usize).saturating_sub(24 + d.meta.areas.join(", ").len().min(20)),
                )),
            ];
            if !d.meta.areas.is_empty() {
                spans.push(Span::styled(format!("  ({})", d.meta.areas.join(", ")), Style::new().fg(Color::DarkGray)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let mut st = ListState::default().with_selected(if app.decisions.is_empty() { None } else { Some(app.dec_sel) });
    let title = format!("Decision log ({})", app.decisions.len());
    f.render_stateful_widget(
        List::new(rows).block(panel(&title, true)).highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
        left,
        &mut st,
    );
    draw_detail(f, app, right);
}

fn entry_row(e: &Entry, show_project: bool, width: usize) -> ListItem<'static> {
    let mut spans = vec![recency(&e.meta.at)];
    let mut used = 2;
    if show_project {
        spans.push(Span::styled(format!("{:<7}", e.project), Style::new().fg(Color::DarkGray)));
        used += 7;
    }
    // Compact in the list; the detail pane shows the full date, time and zone.
    let when = e.meta.at.with_timezone(&chrono::Local).format("%b %e %l:%M %p ").to_string();
    used += when.chars().count() + 14 + 8;
    spans.push(Span::styled(when, Style::new().fg(Color::DarkGray)));
    let kind_style = match e.meta.kind {
        EntryKind::Handoff => Style::new().fg(Color::Magenta).bold(),
        EntryKind::Decision => Style::new().fg(Color::Cyan),
        EntryKind::Blocker => Style::new().fg(Color::Red),
        EntryKind::Finding => Style::new().fg(Color::Green),
        EntryKind::StatusChange => Style::new().fg(Color::DarkGray),
        _ => Style::new(),
    };
    spans.push(Span::styled(format!("{:<14}", e.meta.kind.as_str()), kind_style));
    spans.push(Span::styled(
        format!("{:<8}", e.meta.item.clone().unwrap_or_else(|| "—".into())),
        Style::new().fg(Color::Blue),
    ));
    spans.push(Span::raw(fit(&first_line(&e.body, 200), width.saturating_sub(used))));
    ListItem::new(Line::from(spans))
}

fn draw_entries(f: &mut Frame, app: &mut App, area: Rect, triage: bool) {
    let [left, right] = split(area);
    let (entries, sel, title) = if triage {
        (&app.triage, app.t_sel, format!("Triage: unfiled and inbox ({})", app.triage.len()))
    } else {
        (&app.journal, app.j_sel, format!("Journal ({})", app.journal.len()))
    };
    let w = left.width.saturating_sub(2) as usize;
    let rows: Vec<ListItem> = entries.iter().map(|e| entry_row(e, triage, w)).collect();
    let mut st = ListState::default().with_selected(if entries.is_empty() { None } else { Some(sel) });
    f.render_stateful_widget(
        List::new(rows).block(panel(&title, true)).highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
        left,
        &mut st,
    );
    draw_detail(f, app, right);
}

fn kv(k: &str, v: String) -> Line<'static> {
    Line::from(vec![Span::styled(format!("{k:<9}"), Style::new().fg(Color::DarkGray)), Span::raw(v)])
}

fn draw_detail(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line<'static>> = vec![];
    let title = match &app.detail {
        None => "Details".to_string(),
        Some(Detail::Item(d)) => {
            let m = &d.item.meta;
            lines.push(Line::from(vec![
                Span::styled(format!("{} ", m.id), Style::new().fg(Color::DarkGray)),
                Span::styled(format!("{} ", m.kind), kind_style(m.kind)),
                Span::styled(m.status.as_str().to_string(), status_style(m.status)),
                Span::raw(m.priority.map(|p| format!(" · p{p}")).unwrap_or_default()),
            ]));
            lines.push(Line::from(Span::styled(m.title.clone(), Style::new().bold())));
            lines.push(Line::from(""));
            if let Some(p) = &m.parent {
                lines.push(kv("parent", p.clone()));
            }
            if !m.areas.is_empty() {
                lines.push(kv("areas", m.areas.join(", ")));
            }
            if !m.tags.is_empty() {
                lines.push(kv("tags", m.tags.join(", ")));
            }
            for l in &m.links {
                lines.push(kv(&l.rel, l.to.clone()));
            }
            let by = if m.created_by.is_empty() { "?".to_string() } else { m.created_by.clone() };
            lines.push(kv("created", format!("{} by {by}", local_time(&m.created))));
            lines.push(kv("updated", format!("{} · rev {}", local_time(&m.updated), m.rev)));
            lines.push(kv("file", display_path(std::path::Path::new(&d.item.path))));
            if !d.item.body.trim().is_empty() {
                lines.push(Line::from(""));
                lines.extend(md_lines(d.item.body.trim_end()));
            }
            if !d.applicable.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled("From history", Style::new().bold().fg(Color::Cyan))));
                for r in &d.applicable {
                    let i = &r.item;
                    let w = why(i);
                    lines.push(Line::from(vec![
                        Span::styled(format!("{} ", i.meta.id), Style::new().fg(Color::DarkGray)),
                        Span::styled(format!("[{}] ", i.meta.status), status_style(i.meta.status)),
                        Span::raw(i.meta.title.clone()),
                        Span::styled(
                            if w.is_empty() { String::new() } else { format!(" — {w}") },
                            Style::new().fg(Color::DarkGray),
                        ),
                    ]));
                    lines.push(Line::from(Span::styled(
                        format!("    ({})", r.reasons.join("; ")),
                        Style::new().fg(Color::DarkGray),
                    )));
                }
            }
            if !d.children.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled("Children", Style::new().bold())));
                for c in &d.children {
                    lines.push(Line::from(vec![
                        Span::styled(format!("{:<8}", c.meta.id), Style::new().fg(Color::DarkGray)),
                        Span::styled(format!("{:<9}", c.meta.status.as_str()), status_style(c.meta.status)),
                        Span::raw(c.meta.title.clone()),
                    ]));
                }
            }
            if !d.entries.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled("Journal", Style::new().bold())));
                for e in &d.entries {
                    lines.push(Line::from(vec![
                        Span::styled(format!("{} ", local_time(&e.meta.at)), Style::new().fg(Color::DarkGray)),
                        Span::styled(format!("{} ", e.meta.kind), Style::new().fg(Color::Blue)),
                        Span::styled(format!("({}) ", e.meta.agent), Style::new().fg(Color::DarkGray)),
                        Span::raw(first_line(&e.body, 200)),
                    ]));
                }
            }
            m.id.clone()
        }
        Some(Detail::Entry(e)) => {
            let m = &e.meta;
            lines.push(Line::from(vec![
                Span::styled(format!("{} ", m.kind), Style::new().bold()),
                Span::styled(local_time(&m.at), Style::new().fg(Color::DarkGray)),
            ]));
            lines.push(kv("project", e.project.clone()));
            lines.push(kv("item", m.item.clone().unwrap_or_else(|| "— (unfiled)".into())));
            lines.push(kv("agent", m.agent.clone()));
            if let Some(s) = &m.session {
                lines.push(kv("session", short_id(s).to_string()));
            }
            if let Some(c) = &m.cwd {
                lines.push(kv("cwd", display_path(std::path::Path::new(c))));
            }
            for r in &m.refs {
                lines.push(kv("ref", r.clone()));
            }
            lines.push(Line::from(""));
            lines.extend(md_lines(&e.body));
            format!("entry {}", short_id(&m.id))
        }
    };
    if lines.is_empty() {
        lines.push(Line::from(Span::styled("Nothing selected", Style::new().fg(Color::DarkGray))));
    }
    f.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false })
            .scroll((app.detail_scroll, 0))
            .block(panel(&title, false)),
        area,
    );
}

fn draw_help(f: &mut Frame) {
    let area = centered(f.area(), 78, 30);
    f.render_widget(Clear, area);
    let rows = [
        ("Navigation", ""),
        ("1-6, Tab, Shift-Tab", "switch tab: Overview, Items, Decisions, Journal, Triage, Timeline"),
        ("↑↓ j k, PgUp PgDn, g G", "move in the list (scroll the brief)"),
        ("J K", "scroll the detail pane"),
        ("Esc, ←", "back to the project list"),
        ("/", "search (Tab inside: this project or all projects)"),
        ("r", "refresh now (it also refreshes every 2 s)"),
        ("", ""),
        ("Recording", ""),
        ("d", "record a decision: `Title | why`"),
        ("x", "record a rejected option: `Title | why`"),
        ("a / A", "add an item (`bug Crash on save`) / add a child of the selected item"),
        ("n", "add a note to the journal (on the selected item, if any)"),
        ("s", "change status (accept or reject a proposed decision, close a task, …)"),
        ("e", "open the selected item's Markdown file in $EDITOR"),
        ("c", "Items: show or hide closed items"),
        ("f", "Triage: file the selected entry under an item"),
        ("", ""),
        ("q, Ctrl-C", "quit"),
        ("", ""),
        ("Writes are made as `human` and committed to git in the background.", ""),
        ("Press any key to close.", ""),
    ];
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, v)| {
            if v.is_empty() {
                Line::from(Span::styled(k.to_string(), Style::new().bold()))
            } else {
                Line::from(vec![
                    Span::styled(format!("  {k:<24}"), Style::new().fg(Color::Cyan)),
                    Span::raw(v.to_string()),
                ])
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(panel("Help", true)), area);
}
