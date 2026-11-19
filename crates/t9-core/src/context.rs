//! The context brief: what a fresh agent session (or a human) needs to know about a project, most valuable first,
//! cut to a token budget.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

use chrono::Duration;
use serde::Serialize;

use crate::Result;
use crate::decisions::{Relevant, decision_line, why};
use crate::env::display_path;
use crate::index::{EntryFilter, ItemFilter};
use crate::model::*;
use crate::store::{Caller, GLOBAL, INBOX, Resolution, Store};

#[derive(Debug, Serialize)]
pub struct PlanView {
    pub plan: Item,
    pub done: usize,
    pub total: usize,
    /// Open phases/features under the plan, with depth (1 = direct child).
    pub outline: Vec<(usize, Item, usize)>,
}

#[derive(Debug, Serialize)]
pub struct Brief {
    pub resolution: Resolution,
    pub session: Option<Session>,
    pub handoff: Option<Entry>,
    pub focus: Option<Item>,
    pub in_progress: Vec<Item>,
    pub plans: Vec<PlanView>,
    pub issues: Vec<Item>,
    pub next_up: Vec<Item>,
    /// Decisions (and rejections) that apply to the focus: scoped to it, same area, or same topic.
    pub applicable: Vec<Relevant>,
    /// Accepted decisions of the GLOBAL project: rules for every project.
    pub global_rules: Vec<Item>,
    pub decisions: Vec<Item>,
    /// Rejected decisions and ideas: things not to re-propose without new information.
    pub ruled_out: Vec<Item>,
    /// Open ideas, parked for later.
    pub ideas: Vec<Item>,
    /// For an unlinked folder: the suggested project name and key.
    pub suggested: Option<(String, String)>,
    pub recent: Vec<Entry>,
    pub unfiled: usize,
    pub inbox: usize,
    pub stale: Vec<Item>,
}

const STALE_DAYS: i64 = 3;

/// Accepted GLOBAL decisions, newest first (empty if there is no GLOBAL project yet).
fn global_rules(store: &mut Store) -> Result<Vec<Item>> {
    if store.project(GLOBAL).is_err() {
        return Ok(vec![]);
    }
    store.sync_project(GLOBAL)?;
    let mut v = store.items(&ItemFilter {
        project: Some(GLOBAL.into()),
        kinds: vec![Kind::Decision],
        statuses: vec![Status::Accepted],
        include_closed: true,
        ..Default::default()
    })?;
    v.sort_by_key(|i| std::cmp::Reverse(i.meta.updated));
    Ok(v)
}

fn decision_with_why(d: &Item) -> String {
    let w = why(d);
    if w.is_empty() { format!("- {}", decision_line(d)) } else { format!("- {} — {w}", decision_line(d)) }
}

pub fn build(store: &mut Store, caller: &Caller) -> Result<Brief> {
    let resolution = store.resolve(caller)?;
    let session = store.current_session(caller)?;
    let since_inbox = now() - Duration::days(14);
    store.sync_project(INBOX)?;
    let inbox = store.count_entries(&EntryFilter {
        project: Some(INBOX.into()),
        since: Some(since_inbox),
        ..Default::default()
    })?;
    let Some(project) = resolution.project.clone() else {
        // An agent is working in a folder without a project: remember it so the human is told (TUI, hooks).
        if let Some(agent) = caller.agent.as_deref().filter(|a| *a != "human") {
            store.note_unlinked(&caller.cwd, agent)?;
        }
        return Ok(Brief {
            resolution,
            session,
            handoff: None,
            focus: None,
            in_progress: vec![],
            plans: vec![],
            issues: vec![],
            next_up: vec![],
            applicable: vec![],
            global_rules: global_rules(store)?,
            decisions: vec![],
            ruled_out: vec![],
            ideas: vec![],
            suggested: Some(store.suggested_project(&caller.cwd)?),
            recent: vec![],
            unfiled: 0,
            inbox,
            stale: vec![],
        });
    };
    let key = project.meta.key.clone();
    store.sync_project(&key)?;

    let all = store.items(&ItemFilter { project: Some(key.clone()), include_closed: true, ..Default::default() })?;
    let by_id: BTreeMap<&str, &Item> = all.iter().map(|i| (i.meta.id.as_str(), i)).collect();
    let mut children: BTreeMap<&str, Vec<&Item>> = BTreeMap::new();
    for i in &all {
        if let Some(p) = &i.meta.parent {
            children.entry(p.as_str()).or_default().push(i);
        }
    }
    let mut shown: HashSet<String> = HashSet::new();

    let handoff = store
        .entries(&EntryFilter {
            project: Some(key.clone()),
            kinds: vec![EntryKind::Handoff],
            limit: Some(1),
            ..Default::default()
        })?
        .into_iter()
        .next();

    // A focus that has been closed (done, dropped) no longer counts.
    let focus = session
        .as_ref()
        .and_then(|s| s.meta.focus.as_deref())
        .and_then(|f| by_id.get(f).map(|i| (*i).clone()))
        .filter(|i| !i.meta.status.is_closed());
    if let Some(f) = &focus {
        shown.insert(f.meta.id.clone());
    }

    let structural = |k: Kind| matches!(k, Kind::Plan | Kind::Phase | Kind::Feature);
    let mut in_progress: Vec<Item> = all
        .iter()
        .filter(|i| matches!(i.meta.status, Status::Active | Status::Blocked) && !structural(i.meta.kind))
        .filter(|i| !shown.contains(&i.meta.id))
        .cloned()
        .collect();
    in_progress.sort_by_key(|i| {
        (i.meta.status != Status::Active, i.meta.priority.unwrap_or(99), std::cmp::Reverse(i.meta.updated))
    });
    shown.extend(in_progress.iter().map(|i| i.meta.id.clone()));

    let mut plans = vec![];
    for plan in all.iter().filter(|i| i.meta.kind == Kind::Plan && !i.meta.status.is_closed()) {
        let (mut done, mut total, mut outline) = (0, 0, vec![]);
        let mut stack: Vec<(usize, &Item)> =
            children.get(plan.meta.id.as_str()).into_iter().flatten().map(|c| (1, *c)).collect();
        stack.reverse();
        while let Some((depth, it)) = stack.pop() {
            total += 1;
            if it.meta.status.is_closed() {
                done += 1;
                continue;
            }
            let kids: Vec<&Item> = children.get(it.meta.id.as_str()).cloned().unwrap_or_default();
            if structural(it.meta.kind) && depth <= 2 {
                let open_kids = kids.iter().filter(|k| !k.meta.status.is_closed()).count();
                outline.push((depth, it.clone(), open_kids));
            }
            for k in kids.into_iter().rev() {
                stack.push((depth + 1, k));
            }
        }
        plans.push(PlanView { plan: plan.clone(), done, total, outline });
    }
    plans.sort_by_key(|p| {
        (p.plan.meta.status != Status::Active, p.plan.meta.priority.unwrap_or(99), p.plan.meta.id.clone())
    });

    let mut issues: Vec<Item> = all
        .iter()
        .filter(|i| i.meta.kind.is_issue() && !i.meta.status.is_closed() && !shown.contains(&i.meta.id))
        .cloned()
        .collect();
    issues.sort_by_key(|i| (i.meta.priority.unwrap_or(99), std::cmp::Reverse(i.meta.updated)));
    shown.extend(issues.iter().map(|i| i.meta.id.clone()));

    let mut next_up: Vec<Item> = all
        .iter()
        .filter(|i| i.meta.kind == Kind::Task && i.meta.status == Status::Open && !shown.contains(&i.meta.id))
        .cloned()
        .collect();
    next_up.sort_by_key(|i| (i.meta.priority.unwrap_or(99), split_item_id(&i.meta.id).map(|x| x.1).unwrap_or(0)));

    let applicable = match &focus {
        Some(f) => store.applicable_decisions(f, 5)?,
        None => vec![],
    };
    let applied: HashSet<&str> = applicable.iter().map(|r| r.item.meta.id.as_str()).collect();
    let newest = |v: &mut Vec<Item>| v.sort_by_key(|i| std::cmp::Reverse(i.meta.updated));
    let mut decisions: Vec<Item> = all
        .iter()
        .filter(|i| i.meta.kind == Kind::Decision && i.meta.status == Status::Accepted)
        .filter(|i| !applied.contains(i.meta.id.as_str()))
        .cloned()
        .collect();
    newest(&mut decisions);
    let mut ruled_out: Vec<Item> = all
        .iter()
        .filter(|i| matches!(i.meta.kind, Kind::Decision | Kind::Idea) && i.meta.status == Status::Rejected)
        .filter(|i| !applied.contains(i.meta.id.as_str()))
        .cloned()
        .collect();
    newest(&mut ruled_out);
    let mut ideas: Vec<Item> =
        all.iter().filter(|i| i.meta.kind == Kind::Idea && i.meta.status == Status::Open).cloned().collect();
    newest(&mut ideas);
    let global_rules: Vec<Item> = if key == GLOBAL { vec![] } else { global_rules(store)? }
        .into_iter()
        .filter(|g| !applied.contains(g.meta.id.as_str()))
        .collect();

    let recent = store.entries(&EntryFilter {
        project: Some(key.clone()),
        exclude_kinds: vec![EntryKind::Handoff],
        limit: Some(10),
        ..Default::default()
    })?;
    let unfiled = store.count_entries(&EntryFilter {
        project: Some(key.clone()),
        unfiled: true,
        exclude_kinds: vec![EntryKind::Handoff],
        since: Some(since_inbox),
        ..Default::default()
    })?;

    let stale_before = now() - Duration::days(STALE_DAYS);
    let mut stale = vec![];
    for i in all.iter().filter(|i| i.meta.status == Status::Active && !structural(i.meta.kind)) {
        let last = store
            .last_activity(&i.meta.id)?
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok().map(|d| d.to_utc()));
        if last.unwrap_or(i.meta.updated).max(i.meta.updated) < stale_before {
            stale.push(i.clone());
        }
    }

    Ok(Brief {
        resolution,
        session,
        handoff,
        focus,
        in_progress,
        plans,
        issues,
        next_up,
        applicable,
        global_rules,
        decisions,
        ruled_out,
        ideas,
        suggested: None,
        recent,
        unfiled,
        inbox,
        stale,
    })
}

// ---------------------------------------------------------------- rendering

/// Accumulates lines until the character budget runs out. A section header is only written if at least one of
/// its lines fits.
struct Budget {
    out: String,
    left: usize,
    truncated: bool,
}

impl Budget {
    fn line(&mut self, s: &str) -> bool {
        if s.len() + 1 > self.left {
            self.truncated = true;
            return false;
        }
        self.left -= s.len() + 1;
        self.out.push_str(s);
        self.out.push('\n');
        true
    }

    fn section<I: IntoIterator<Item = String>>(&mut self, title: &str, lines: I, more: usize) {
        let mut lines = lines.into_iter().peekable();
        if lines.peek().is_none() {
            return;
        }
        let header = format!("\n## {title}");
        if header.len() + 40 > self.left {
            self.truncated = true;
            return;
        }
        self.line(&header);
        let mut dropped = more;
        for l in lines.by_ref() {
            if !self.line(&l) {
                dropped += 1;
                break;
            }
        }
        dropped += lines.count();
        if dropped > 0 {
            self.line(&format!("- … {dropped} more"));
        }
    }
}

pub fn first_line(s: &str, max: usize) -> String {
    let line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    if line.chars().count() <= max {
        let more = s.trim().lines().count() > 1;
        return if more { format!("{line} …") } else { line.to_string() };
    }
    let cut: String = line.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

pub fn item_line(i: &Item) -> String {
    let mut s = format!("- {} {} [{}]", i.meta.id, i.meta.kind, i.meta.status);
    if let Some(p) = i.meta.priority {
        let _ = write!(s, " p{p}");
    }
    let _ = write!(s, " {}", i.meta.title);
    if let Some(p) = &i.meta.parent {
        let _ = write!(s, " (in {p})");
    }
    s
}

/// For tables: `2026-11-18  8:11 PM EST` (local zone, 12-hour clock, hour padded so columns line up).
pub fn local_time(t: &Timestamp) -> String {
    crate::clock::date_time_col(t)
}

fn entry_line(e: &Entry) -> String {
    let mut s = format!("- {} {}", local_time(&e.meta.at), e.meta.kind);
    if let Some(i) = &e.meta.item {
        let _ = write!(s, " {i}");
    }
    if !e.meta.agent.is_empty() {
        let _ = write!(s, " ({})", e.meta.agent);
    }
    let _ = write!(s, ": {}", first_line(&e.body, 140));
    s
}

pub fn render(b: &Brief, budget_tokens: usize) -> String {
    let mut w = Budget { out: String::new(), left: budget_tokens.max(200) * 4, truncated: false };
    let res = &b.resolution;
    let cwd = display_path(Path::new(&res.cwd));
    let branch = res.git.as_ref().and_then(|g| g.branch.clone());

    let Some(p) = &res.project else {
        w.line("# t9: this folder is not linked to a t9 project yet");
        let mut where_ = format!("cwd {cwd}");
        if let Some(g) = &res.git {
            if let Some(r) = &g.remote {
                let _ = write!(where_, " · git {r}");
            }
            if let Some(br) = &branch {
                let _ = write!(where_, " · branch {br}");
            }
        }
        w.line(&where_);
        if !res.suggestions.is_empty() {
            w.line("\n## Existing projects that may match");
            for s in &res.suggestions {
                let roots = s.roots.iter().map(|r| display_path(Path::new(r))).collect::<Vec<_>>().join(", ");
                w.line(&format!("- {} ({}): {} — {}", s.name, s.key, s.reason, roots));
            }
        }
        w.line("\n## Tell the user in your first reply");
        w.line("This folder is not linked to a t9 project, so nothing recorded here is filed under a project.");
        w.line(
            "Ask whether to create one (they may choose another name or key), or to link an existing one, then run:",
        );
        if let Some((name, key)) = &b.suggested {
            w.line(&format!("- `{}` — new project", crate::unlinked::init_command(name, key)));
        }
        w.line("- `t9 project link KEY` — this folder belongs to an existing project");
        w.line("Until then `t9 log` still works: notes go to the inbox and move into the project once it exists.");
        w.section(
            "Rules for every project (global)",
            b.global_rules.iter().take(6).map(decision_with_why),
            b.global_rules.len().saturating_sub(6),
        );
        if b.inbox > 0 {
            w.line(&format!("Inbox: {} entries in the last 14 days (`t9 triage`).", b.inbox));
        }
        return w.out;
    };

    w.line(&format!("# {} ({}) — context at {}", p.meta.name, p.meta.key, crate::clock::date_time(&now())));
    let mut where_ = format!("cwd {cwd}");
    if let Some(br) = &branch {
        let _ = write!(where_, " · branch {br}");
    }
    if let Some(s) = &b.session {
        let _ = write!(where_, " · session {} ({})", s.short_id(), s.meta.agent);
    }
    w.line(&where_);
    if !p.description.is_empty() {
        let desc: String = p
            .description
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .take(3)
            .collect::<Vec<_>>()
            .join(" ");
        if !desc.is_empty() {
            w.line(&first_line(&desc, 300));
        }
    }

    if let Some(h) = &b.handoff {
        let title = format!(
            "Last handoff ({}, {})",
            if h.meta.agent.is_empty() { "unknown" } else { &h.meta.agent },
            crate::clock::date_time(&h.meta.at)
        );
        let body: String = h.body.chars().take(1200).collect();
        w.section(&title, body.lines().map(String::from), 0);
    }
    if let Some(f) = &b.focus {
        let mut lines = vec![item_line(f)];
        lines.extend(f.body.lines().filter(|l| !l.trim().is_empty()).take(6).map(|l| format!("  {l}")));
        w.section("Current focus", lines, 0);
    }
    w.section(
        "Decisions that apply to your focus",
        b.applicable.iter().map(|r| format!("{} [{}]", decision_with_why(&r.item), r.reasons.join("; "))),
        0,
    );
    w.section(
        "Rules for every project (global)",
        b.global_rules.iter().take(6).map(decision_with_why),
        b.global_rules.len().saturating_sub(6),
    );
    w.section("In progress", b.in_progress.iter().take(10).map(item_line), b.in_progress.len().saturating_sub(10));
    for pv in &b.plans {
        let title = format!(
            "Plan {} {} [{}] — {}/{} done",
            pv.plan.meta.id, pv.plan.meta.title, pv.plan.meta.status, pv.done, pv.total
        );
        let lines = pv.outline.iter().take(12).map(|(d, i, open)| {
            format!(
                "{}- {} {} [{}] {} ({} open)",
                "  ".repeat(d - 1),
                i.meta.id,
                i.meta.kind,
                i.meta.status,
                i.meta.title,
                open
            )
        });
        let lines: Vec<String> = lines.collect();
        if lines.is_empty() {
            w.section(&title, ["- (no open phases or features)".to_string()], 0);
        } else {
            w.section(&title, lines, 0);
        }
    }
    w.section(
        "Open bugs, regressions, revisits, questions",
        b.issues.iter().take(10).map(item_line),
        b.issues.len().saturating_sub(10),
    );
    w.section("Next up (open tasks)", b.next_up.iter().take(8).map(item_line), b.next_up.len().saturating_sub(8));
    w.section(
        "Current decisions",
        b.decisions.iter().take(6).map(|d| format!("- {}", decision_line(d))),
        b.decisions.len().saturating_sub(6),
    );
    w.section(
        "Ruled out (do not re-propose without new information)",
        b.ruled_out.iter().take(5).map(decision_with_why),
        b.ruled_out.len().saturating_sub(5),
    );
    w.section("Parked ideas", b.ideas.iter().take(5).map(item_line), b.ideas.len().saturating_sub(5));
    w.section("Recent activity", b.recent.iter().take(8).map(entry_line), 0);

    let mut attention = vec![];
    if b.unfiled > 0 {
        attention.push(format!("- {} recent entries are not filed under an item (`t9 triage`)", b.unfiled));
    }
    if b.inbox > 0 {
        attention.push(format!("- {} inbox entries from unlinked directories (`t9 triage`)", b.inbox));
    }
    for s in &b.stale {
        attention.push(format!(
            "- {} is active but has had no activity for {STALE_DAYS}+ days: {}",
            s.meta.id, s.meta.title
        ));
    }
    w.section("Needs attention", attention, 0);

    if w.truncated {
        w.out.push_str("\n(brief truncated to fit the budget; `t9 ls`, `t9 show ID`, `t9 search` for more)\n");
    }
    w.out
}
