//! Human-readable output.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use anyhow::Result;
use chrono::Duration;
use serde::Serialize;
use t9_core::context::{first_line, local_time};
use t9_core::env::display_path;
use t9_core::index::{EntryFilter, Hit, ItemFilter};
use t9_core::*;

pub fn whereami(res: &Resolution, session: Option<&Session>, vault: &str) {
    println!("cwd      {}", display_path(Path::new(&res.cwd)));
    match &res.project {
        Some(p) => println!("project  {} ({}) via {}", p.meta.name, p.meta.key, res.via.as_deref().unwrap_or("?")),
        None => println!("project  none (entries go to the inbox; see `t9 context`)"),
    }
    if let Some(g) = &res.git {
        println!(
            "git      {} · {} · {}",
            g.remote.as_deref().unwrap_or("(no remote)"),
            g.branch.as_deref().unwrap_or("(detached)"),
            g.head.as_deref().map(|h| &h[..h.len().min(10)]).unwrap_or("")
        );
    }
    match session {
        Some(s) => println!(
            "session  {} ({}) started {}{}",
            s.short_id(),
            s.meta.agent,
            local_time(&s.meta.started),
            s.meta.focus.as_ref().map(|f| format!(" · focus {f}")).unwrap_or_default()
        ),
        None => println!("session  none open here"),
    }
    println!("vault    {}", display_path(Path::new(vault)));
}

pub fn logged(e: &Entry) {
    let target = match (&e.meta.item, e.project.as_str()) {
        (Some(i), _) => i.clone(),
        (None, INBOX) => "the inbox (directory not linked to a project)".into(),
        (None, p) => format!("{p} (unfiled)"),
    };
    println!("logged {} {} → {}", e.meta.kind, short_id(&e.meta.id), target);
}

fn item_row(i: &Item, indent: usize, show_project: bool) -> String {
    let pri = i.meta.priority.map(|p| format!("p{p}")).unwrap_or_default();
    let proj = if show_project { format!("{:<6} ", i.project) } else { String::new() };
    let tags = if i.meta.tags.is_empty() { String::new() } else { format!("  #{}", i.meta.tags.join(" #")) };
    format!(
        "{proj}{:<9} {}{:<10} {:<10} {:<3} {}{tags}",
        i.meta.id,
        "  ".repeat(indent),
        i.meta.kind.as_str(),
        i.meta.status.as_str(),
        pri,
        i.meta.title
    )
}

pub fn items(items: &[Item], tree: bool, show_project: bool) {
    if items.is_empty() {
        println!("no items");
        return;
    }
    if !tree {
        for i in items {
            println!("{}", item_row(i, 0, show_project));
        }
        return;
    }
    let ids: HashSet<&str> = items.iter().map(|i| i.meta.id.as_str()).collect();
    let mut kids: BTreeMap<&str, Vec<&Item>> = BTreeMap::new();
    let mut roots = vec![];
    for i in items {
        match i.meta.parent.as_deref() {
            Some(p) if ids.contains(p) => kids.entry(p).or_default().push(i),
            _ => roots.push(i),
        }
    }
    fn walk(i: &Item, depth: usize, kids: &BTreeMap<&str, Vec<&Item>>, show_project: bool) {
        println!("{}", item_row(i, depth, show_project));
        for k in kids.get(i.meta.id.as_str()).into_iter().flatten() {
            walk(k, depth + 1, kids, show_project);
        }
    }
    for r in roots {
        walk(r, 0, &kids, show_project);
    }
}

pub fn entry_row(e: &Entry, show_project: bool) -> String {
    let proj = if show_project { format!("{:<6} ", e.project) } else { String::new() };
    let item = e.meta.item.clone().unwrap_or_else(|| "—".into());
    format!(
        "{proj}{} {}  {:<14} {:<8} {:<12} {}",
        local_time(&e.meta.at),
        short_id(&e.meta.id),
        e.meta.kind.as_str(),
        item,
        e.meta.agent,
        first_line(&e.body, 100)
    )
}

pub fn entries(es: &[Entry], show_project: bool) {
    if es.is_empty() {
        println!("no entries");
    }
    for e in es {
        println!("{}", entry_row(e, show_project));
    }
}

pub fn show(i: &Item, children: &[Item], entries: &[Entry]) {
    let m = &i.meta;
    println!("{} · {} · {}{}", m.id, m.kind, m.status, m.priority.map(|p| format!(" · p{p}")).unwrap_or_default());
    println!("{}", m.title);
    let mut meta = vec![format!(
        "created {} by {}",
        local_time(&m.created),
        if m.created_by.is_empty() { "?" } else { &m.created_by }
    )];
    meta.push(format!("updated {} (rev {})", local_time(&m.updated), m.rev));
    if let Some(p) = &m.parent {
        meta.push(format!("parent {p}"));
    }
    println!("{}", meta.join(" · "));
    if !m.tags.is_empty() {
        println!("tags: {}", m.tags.join(", "));
    }
    for l in &m.links {
        println!("{} {}", l.rel, l.to);
    }
    println!("file: {}", display_path(Path::new(&i.path)));
    if !i.body.trim().is_empty() {
        println!("\n{}", i.body.trim_end());
    }
    if !children.is_empty() {
        println!("\nchildren:");
        for c in children {
            println!("  {}", item_row(c, 0, false));
        }
    }
    if !entries.is_empty() {
        println!("\njournal:");
        for e in entries {
            println!("  {}", entry_row(e, false));
        }
    }
}

pub fn hits(hits: &[Hit], show_project: bool) {
    if hits.is_empty() {
        println!("no matches");
    }
    for h in hits {
        let proj = if show_project { format!("{:<6} ", h.project) } else { String::new() };
        let id = if h.kind == "entry" { format!("entry {}", short_id(&h.id)) } else { h.id.clone() };
        let snippet = h.snippet.replace('\n', " ");
        if h.title.is_empty() {
            println!("{proj}{id:<12} {snippet}");
        } else {
            println!("{proj}{id:<12} {} — {snippet}", h.title);
        }
    }
}

pub fn triage(unfiled: &[Entry], inbox: &[Entry]) {
    if unfiled.is_empty() && inbox.is_empty() {
        println!("nothing to triage");
        return;
    }
    if !unfiled.is_empty() {
        println!("Not filed under an item ({}):", unfiled.len());
        for e in unfiled {
            println!("  {}", entry_row(e, false));
        }
    }
    if !inbox.is_empty() {
        println!("Inbox — written from directories not linked to a project ({}):", inbox.len());
        for e in inbox {
            let cwd = e.meta.cwd.as_deref().map(|c| display_path(Path::new(c))).unwrap_or_default();
            println!("  {}  [{}]", entry_row(e, false), cwd);
        }
    }
    println!("\nFile one with `t9 attach ENTRY ITEM` (ENTRY is the 6-character id above).");
}

pub fn project(p: &Project) {
    println!("{} ({}) · {}", p.meta.name, p.meta.key, p.meta.status);
    for r in &p.meta.roots {
        println!("  root    {}", display_path(Path::new(r)));
    }
    for r in &p.meta.remotes {
        println!("  remote  {r}");
    }
}

/// Recalled decisions: `BIL-12 [accepted] 2026-11-10 Title (areas) — why  [reasons]`.
pub fn relevant(found: &[t9_core::decisions::Relevant], empty: &str) {
    if found.is_empty() && !empty.is_empty() {
        println!("{empty}");
    }
    for r in found {
        let w = t9_core::decisions::why(&r.item);
        let mut line = format!("  {}", t9_core::decisions::decision_line(&r.item));
        if r.item.project == GLOBAL {
            line.push_str(" · global");
        }
        if !w.is_empty() {
            line.push_str(&format!(" — {w}"));
        }
        println!("{line}");
        if !r.reasons.is_empty() {
            println!("      ({})", r.reasons.join("; "));
        }
    }
}

pub fn decision_log(items: &[Item]) {
    if items.is_empty() {
        println!("no decisions yet (record one with `t9 decide \"…\" --because \"…\"`)");
    }
    for i in items {
        let w = t9_core::decisions::why(i);
        if w.is_empty() {
            println!("{}", t9_core::decisions::decision_line(i));
        } else {
            println!("{} — {w}", t9_core::decisions::decision_line(i));
        }
    }
}

pub fn about(a: &t9_core::about::About) {
    println!("{} ({}) · About", a.name, a.project);
    match &a.reviewed {
        Some(t) => println!("last reviewed {}", t9_core::clock::date_time(t)),
        None => println!("never reviewed"),
    }
    if a.is_empty() {
        println!("\nNothing written yet. Start with what it is, then the stack and how to run it:");
        println!("  t9 about set what \"…\"");
        println!("  t9 about add \"…\" --section stack");
        println!("  t9 about edit");
        return;
    }
    for s in a.sections.iter().filter(|s| !s.text.trim().is_empty()) {
        println!("\n## {}\n{}", s.title, s.text.trim_end());
    }
}

fn date(t: &Timestamp) -> String {
    t.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string()
}

pub fn history(events: &[t9_core::history::Event], show_project: bool) {
    if events.is_empty() {
        println!("nothing recorded about this yet");
        return;
    }
    for e in events {
        let proj = if show_project { format!("{:<6} ", e.project) } else { String::new() };
        let id = if e.source == "entry" {
            e.item.clone().map(|i| format!("({i})")).unwrap_or_else(|| "—".into())
        } else {
            e.id.clone()
        };
        let status = e.status.map(|s| format!("[{s}] ")).unwrap_or_default();
        let mut line = format!("{proj}{}  {:<13} {:<8} {status}{}", date(&e.at), e.kind, id, first_line(&e.title, 100));
        if !e.detail.is_empty() {
            line.push_str(&format!(" — {}", first_line(&e.detail, 120)));
        }
        if let Some(f) = &e.flag {
            line.push_str(&format!("   ← {f}"));
        }
        println!("{line}");
    }
}

pub fn timeline(events: &[t9_core::history::Event]) {
    if events.is_empty() {
        println!("nothing happened in this period");
        return;
    }
    let mut day = String::new();
    for e in events {
        let d = e.at.with_timezone(&chrono::Local).format("%a %b %d, %Y").to_string();
        if d != day {
            println!("\n{d}");
            day = d;
        }
        let id = if e.source == "entry" { e.item.clone().unwrap_or_default() } else { e.id.clone() };
        let time = e.at.with_timezone(&chrono::Local).format("%l:%M %p").to_string();
        println!("  {time}  {:<15} {:<8} {}", e.kind, id, first_line(&e.title, 110));
    }
}

pub fn resources(items: &[Item]) {
    if items.is_empty() {
        println!(
            "no resources yet (record one with `t9 add resource \"Orders table\" --type table --at analytics.orders`)"
        );
    }
    for i in items {
        println!(
            "{:<8} {:<11} {:<30} {}{}",
            i.meta.id,
            i.meta.resource_type.as_deref().unwrap_or("—"),
            first_line(&i.meta.title, 30),
            i.meta.location.as_deref().unwrap_or(""),
            i.meta.owner.as_ref().map(|o| format!("  (owner {o})")).unwrap_or_default()
        );
    }
}

pub fn adopted(n: usize) {
    if n > 0 {
        println!("moved {n} inbox entries written from here into this project (unfiled; see `t9 triage`)");
    }
}

pub fn sessions(ss: &[Session]) {
    if ss.is_empty() {
        println!("no sessions");
    }
    for s in ss {
        let state = match &s.meta.ended {
            Some(e) => format!("ended {}", local_time(e)),
            None => "open".into(),
        };
        println!(
            "{}  {:<6} {:<12} {}  {:<22} {}{}",
            s.short_id(),
            s.meta.project,
            s.meta.agent,
            local_time(&s.meta.started),
            state,
            display_path(Path::new(&s.meta.cwd)),
            s.meta.focus.as_ref().map(|f| format!("  focus {f}")).unwrap_or_default()
        );
    }
}

#[derive(Serialize, Default)]
struct DoctorReport {
    vault: String,
    index: String,
    projects: usize,
    items: usize,
    entries: usize,
    warnings: Vec<String>,
    findings: Vec<String>,
}

pub fn doctor(store: &mut Store, json: bool) -> Result<()> {
    let stats = store.reindex()?;
    let mut r = DoctorReport {
        vault: store.env.vault.display().to_string(),
        index: store.env.db_path().display().to_string(),
        projects: stats.projects,
        items: stats.items,
        entries: stats.entries,
        warnings: stats.warnings,
        ..Default::default()
    };
    let all = store.items(&ItemFilter { include_closed: true, ..Default::default() })?;
    let ids: HashSet<&str> = all.iter().map(|i| i.meta.id.as_str()).collect();
    let stale = now() - Duration::days(3);
    for i in &all {
        if let Some(p) = &i.meta.parent
            && !ids.contains(p.as_str())
        {
            r.findings.push(format!("{}: parent {p} does not exist", i.meta.id));
        }
        for l in &i.meta.links {
            if !ids.contains(l.to.as_str()) {
                r.findings.push(format!("{}: {} link to missing {}", i.meta.id, l.rel, l.to));
            }
        }
        if i.meta.status == Status::Active && i.meta.kind != Kind::Plan {
            let last = store.last_activity(&i.meta.id)?.unwrap_or_default();
            if i.meta.updated < stale && last < t9_core::index::ts(&stale) {
                r.findings.push(format!("{}: active but untouched for 3+ days — {}", i.meta.id, i.meta.title));
            }
        }
    }
    for s in store.sessions(None, true, 500)? {
        if s.last_seen.unwrap_or(s.meta.started) < now() - Duration::hours(24) {
            r.findings.push(format!("session {} ({}, {}) never ended", s.short_id(), s.meta.agent, s.meta.project));
        }
    }
    if store.env.git {
        if !store.env.vault.join(".git").exists() && r.items + r.entries > 0 {
            r.findings.push("the vault is not a git repository yet (`t9 vault commit` creates it)".into());
        }
        if let Some(e) = t9_core::vcs::last_error(&store.env) {
            r.findings.push(format!(
                "last automatic git commit failed: {e} (see {})",
                t9_core::vcs::log_path(&store.env).display()
            ));
        }
    }
    for u in store.unlinked_folders()? {
        r.findings.push(format!(
            "{} has no project ({} worked there); run there: {}",
            display_path(Path::new(&u.cwd)),
            u.agent,
            t9_core::unlinked::init_command(&u.name, &u.key)
        ));
    }
    let unfiled = store
        .entries(&EntryFilter {
            unfiled: true,
            exclude_kinds: vec![EntryKind::Handoff],
            since: Some(now() - Duration::days(14)),
            ..Default::default()
        })?
        .len();
    if unfiled > 0 {
        r.findings.push(format!("{unfiled} entries from the last 14 days are not filed under an item (`t9 triage`)"));
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&r)?);
        return Ok(());
    }
    println!("vault   {}", display_path(Path::new(&r.vault)));
    println!("index   {}", display_path(Path::new(&r.index)));
    println!("{} projects, {} items, {} entries", r.projects, r.items, r.entries);
    for w in &r.warnings {
        println!("warning: {w}");
    }
    for f in &r.findings {
        println!("- {f}");
    }
    if r.warnings.is_empty() && r.findings.is_empty() {
        println!("all good");
    }
    Ok(())
}
