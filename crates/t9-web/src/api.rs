//! The JSON API behind the web UI. Reads sync the index first, so changes made by agents show up; writes are
//! made as `human` (in a session rooted at the vault) and committed to git in the background.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use t9_core::decisions::DecisionInput;
use t9_core::index::{EntryFilter, ItemFilter};
use t9_core::*;
use tiny_http::Method;

#[derive(Serialize)]
struct ItemOut {
    #[serde(flatten)]
    item: Item,
    /// When the item last changed: its own update or its newest journal entry.
    touched: Timestamp,
    /// Why it needs attention, if it does: overdue, due-soon, ripe, waiting, stale, loop, unexplained.
    #[serde(skip_serializing_if = "Option::is_none")]
    fresh: Option<Fresh>,
    /// For closed items: done ✓, merged ⇢, dead ✗, partial ~.
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<OutcomeOut>,
}

#[derive(Serialize, Clone)]
struct Fresh {
    reason: &'static str,
    detail: String,
    severity: u8,
}

#[derive(Serialize, Clone)]
struct OutcomeOut {
    class: &'static str,
    glyph: &'static str,
    why: Option<&'static str>,
    note: Option<String>,
}

/// Attention and outcome per item, for badges and map glyphs.
struct Marks {
    fresh: HashMap<String, Fresh>,
    outcome: HashMap<String, OutcomeOut>,
}

fn marks(store: &mut Store, key: &str, items: &[Item]) -> Result<Marks> {
    let signals = store.insights(key)?.signals;
    let att = store.attention(key, Some(&signals))?;
    let fresh = att
        .items
        .into_iter()
        .map(|a| (a.id, Fresh { reason: a.reason, detail: a.detail, severity: a.severity }))
        .collect();
    let closings = store.closings(key)?;
    let mut outcome = HashMap::new();
    for i in items.iter().filter(|i| i.meta.status.is_closed()) {
        let note = closings.get(&i.meta.id).and_then(|c| c.note.clone());
        let (class, why) = t9_core::story::classify(i.meta.status, note.as_deref().unwrap_or(""));
        outcome.insert(i.meta.id.clone(), OutcomeOut { class, glyph: t9_core::story::glyph(class), why, note });
    }
    Ok(Marks { fresh, outcome })
}

fn param<'a>(params: &'a [(String, String)], key: &str) -> Option<&'a str> {
    params.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn human(store: &Store, project: &str) -> Caller {
    Caller {
        cwd: store.env.vault.clone(),
        agent: Some("human".into()),
        session: None,
        project: Some(project.to_string()),
    }
}

/// Newest journal entry per item, for "touched" times.
fn touched_map(store: &Store, project: &str) -> Result<HashMap<String, Timestamp>> {
    let mut m: HashMap<String, Timestamp> = HashMap::new();
    for e in store.entries(&EntryFilter { project: Some(project.into()), limit: Some(5000), ..Default::default() })? {
        if let Some(i) = e.meta.item {
            let t = m.entry(i).or_insert(e.meta.at);
            if e.meta.at > *t {
                *t = e.meta.at;
            }
        }
    }
    Ok(m)
}

fn out(i: Item, touched: &HashMap<String, Timestamp>) -> ItemOut {
    let t = touched.get(&i.meta.id).copied().map_or(i.meta.updated, |t| t.max(i.meta.updated));
    ItemOut { item: i, touched: t, fresh: None, outcome: None }
}

fn out_marked(i: Item, touched: &HashMap<String, Timestamp>, m: &Marks) -> ItemOut {
    let mut o = out(i, touched);
    o.fresh = m.fresh.get(&o.item.meta.id).cloned();
    o.outcome = m.outcome.get(&o.item.meta.id).cloned();
    o
}

fn committed(store: &mut Store) {
    if store.wrote.get() && store.env.git {
        t9_core::vcs::spawn_background(&store.env);
    }
    store.wrote.set(false);
}

pub fn route(store: &mut Store, m: &Method, segs: &[&str], params: &[(String, String)], body: &str) -> Result<Value> {
    match (m, segs) {
        (Method::Get, ["projects"]) => projects(store),
        (Method::Get, ["p", key, "overview"]) => overview(store, key),
        (Method::Get, ["p", key, "items"]) => items(store, key),
        (Method::Get, ["p", key, "journal"]) => journal(store, key, params),
        (Method::Get, ["p", key, "timeline"]) => {
            let days: i64 = param(params, "days").and_then(|d| d.parse().ok()).unwrap_or(60);
            let topic = param(params, "topic").filter(|t| !t.trim().is_empty());
            Ok(serde_json::to_value(store.timeline(key, Some(now() - chrono::Duration::days(days)), topic)?)?)
        }
        (Method::Get, ["p", key, "sessions"]) => Ok(serde_json::to_value(store.sessions(Some(key), false, 200)?)?),
        (Method::Get, ["p", key, "insights"]) => Ok(serde_json::to_value(store.insights(key)?)?),
        (Method::Get, ["p", key, "attention"]) => {
            let signals = store.insights(key)?.signals;
            Ok(serde_json::to_value(store.attention(key, Some(&signals))?)?)
        }
        (Method::Get, ["p", key, "about"]) => Ok(serde_json::to_value(store.about(key)?)?),
        (Method::Get, ["p", key, "changes"]) => changes(store, key),
        (Method::Get, ["history"]) => {
            let q = param(params, "q").unwrap_or("");
            Ok(serde_json::to_value(store.history(q, param(params, "p"))?)?)
        }
        (Method::Get, ["search"]) => {
            let q = param(params, "q").unwrap_or("");
            if param(params, "p").is_none() {
                store.sync_all()?;
            }
            Ok(serde_json::to_value(store.search(q, param(params, "p"), 50)?)?)
        }
        (Method::Get, ["item", id]) => item_page(store, id),
        (Method::Get, ["p", key, "file"]) => file(store, key, param(params, "path").unwrap_or("")),
        (Method::Post, ["item", id, "status"]) => {
            #[derive(Deserialize)]
            struct B {
                status: Status,
                note: Option<String>,
            }
            let b: B = serde_json::from_str(body)?;
            let it = store.item(id)?;
            let caller = human(store, &it.project);
            let i = store.update_item(
                &caller,
                id,
                ItemPatch { status: Some(b.status), note: b.note, ..Default::default() },
            )?;
            committed(store);
            Ok(serde_json::to_value(i)?)
        }
        (Method::Post, ["item", id, "note"]) => {
            #[derive(Deserialize)]
            struct B {
                text: String,
            }
            let b: B = serde_json::from_str(body)?;
            let it = store.item(id)?;
            let caller = human(store, &it.project);
            let e = store
                .log(&caller, LogInput { kind: EntryKind::Note, body: b.text, item: Some(it.meta.id), refs: vec![] })?;
            committed(store);
            Ok(serde_json::to_value(e)?)
        }
        (Method::Post, ["item", id, "answer"]) => answer(store, id, body),
        (Method::Post, ["item", id, "fields"]) => {
            #[derive(Deserialize)]
            struct B {
                goal: Option<String>,
                /// `YYYY-MM-DD`, `today`, `fri`, `3d` …; empty clears.
                due: Option<String>,
            }
            let b: B = serde_json::from_str(body)?;
            let due = match b.due.as_deref().map(str::trim) {
                None => None,
                Some("") => Some(None),
                Some(d) => Some(Some(
                    t9_core::attention::parse_when(d, chrono::Local::now().date_naive())
                        .map_err(|e| anyhow::anyhow!(e))?,
                )),
            };
            let it = store.item(id)?;
            let caller = human(store, &it.project);
            let i = store.update_item(&caller, id, ItemPatch { goal: b.goal, due, ..Default::default() })?;
            committed(store);
            Ok(serde_json::to_value(i)?)
        }
        (Method::Post, ["item", id, "still"]) => {
            #[derive(Deserialize)]
            struct B {
                text: String,
            }
            let b: B = serde_json::from_str(body)?;
            if b.text.trim().is_empty() {
                bail!("say why it is still relevant");
            }
            let it = store.item(id)?;
            let caller = human(store, &it.project);
            let e = store.log(
                &caller,
                LogInput { kind: EntryKind::Checkin, body: b.text, item: Some(it.meta.id), refs: vec![] },
            )?;
            committed(store);
            Ok(serde_json::to_value(e)?)
        }
        (Method::Post, ["p", key, "decide"]) => {
            #[derive(Deserialize)]
            struct B {
                title: String,
                because: Option<String>,
                #[serde(default)]
                rejected: Vec<String>,
                #[serde(default)]
                areas: Vec<String>,
                status: Option<Status>,
            }
            let b: B = serde_json::from_str(body)?;
            let caller = human(store, key);
            let d = store.decide(
                &caller,
                DecisionInput {
                    title: b.title,
                    because: b.because,
                    rejected: b.rejected,
                    areas: b.areas,
                    status: b.status.unwrap_or(Status::Accepted),
                    ..Default::default()
                },
            )?;
            committed(store);
            Ok(serde_json::to_value(d)?)
        }
        _ => bail!("not found: {}", segs.join("/")),
    }
}

fn projects(store: &mut Store) -> Result<Value> {
    store.sync_projects()?;
    let mut rows = vec![];
    for p in store.projects()? {
        let key = p.meta.key.clone();
        store.sync_project(&key)?;
        let open = store.items(&ItemFilter { project: Some(key.clone()), ..Default::default() })?;
        let last = store
            .entries(&EntryFilter { project: Some(key.clone()), limit: Some(1), ..Default::default() })?
            .first()
            .map(|e| e.meta.at);
        rows.push(json!({
            "key": key,
            "name": p.meta.name,
            "roots": p.meta.roots,
            "open": open.iter().filter(|i| i.meta.kind != Kind::Decision && i.meta.kind != Kind::Resource).count(),
            "issues": open.iter().filter(|i| i.meta.kind.is_issue() && i.meta.kind != Kind::Question).count(),
            "questions": open.iter().filter(|i| i.meta.kind == Kind::Question).count(),
            "last": last,
            "global": key == GLOBAL,
        }));
    }
    Ok(json!({ "projects": rows, "unlinked": store.unlinked_folders()? }))
}

fn overview(store: &mut Store, key: &str) -> Result<Value> {
    let p = store.project(key)?;
    let root = p.meta.roots.first().cloned();
    let caller = Caller {
        cwd: root.map(std::path::PathBuf::from).unwrap_or_else(|| store.env.vault.clone()),
        project: Some(p.meta.key.clone()),
        ..Default::default()
    };
    let brief = context::build(store, &caller)?;
    let proposed = store.items(&ItemFilter {
        project: Some(p.meta.key.clone()),
        kinds: vec![Kind::Decision],
        statuses: vec![Status::Proposed],
        include_closed: true,
        ..Default::default()
    })?;
    Ok(json!({ "project": p, "brief": brief, "proposed": proposed, "agent_brief": context::render(&brief, 6000) }))
}

fn items(store: &mut Store, key: &str) -> Result<Value> {
    store.sync_project(key)?;
    let touched = touched_map(store, key)?;
    let all = store.items(&ItemFilter { project: Some(key.into()), include_closed: true, ..Default::default() })?;
    let m = marks(store, key, &all)?;
    let v: Vec<ItemOut> = all.into_iter().map(|i| out_marked(i, &touched, &m)).collect();
    Ok(serde_json::to_value(v)?)
}

fn journal(store: &mut Store, key: &str, params: &[(String, String)]) -> Result<Value> {
    store.sync_project(key)?;
    let limit = param(params, "limit").and_then(|l| l.parse().ok()).unwrap_or(300);
    let kinds = param(params, "kind").and_then(|k| k.parse::<EntryKind>().ok()).map(|k| vec![k]).unwrap_or_default();
    let session = param(params, "session").map(String::from);
    let e = store.entries(&EntryFilter {
        project: Some(key.into()),
        kinds,
        session,
        limit: Some(limit),
        ..Default::default()
    })?;
    Ok(serde_json::to_value(e)?)
}

/// The newest change in a project, so the UI can refresh only when something changed.
fn changes(store: &mut Store, key: &str) -> Result<Value> {
    store.sync_project(key)?;
    let last_entry = store
        .entries(&EntryFilter { project: Some(key.into()), limit: Some(1), ..Default::default() })?
        .first()
        .map(|e| e.meta.at);
    let last_item = store
        .items(&ItemFilter { project: Some(key.into()), include_closed: true, ..Default::default() })?
        .iter()
        .map(|i| i.meta.updated)
        .max();
    let latest = last_entry.max(last_item);
    Ok(json!({ "latest": latest, "key": key }))
}

/// Everything about one item: its place in the plan, links both ways, its journal, what applies to it, and,
/// for anything an agent created, the context in which it was created and what happened since.
fn item_page(store: &mut Store, id: &str) -> Result<Value> {
    let item = store.item(id)?;
    let key = item.project.clone();
    let touched = touched_map(store, &key)?;
    let all = store.items(&ItemFilter { project: Some(key.clone()), include_closed: true, ..Default::default() })?;
    let m = marks(store, &key, &all)?;
    let by_id: BTreeMap<&str, &Item> = all.iter().map(|i| (i.meta.id.as_str(), i)).collect();

    let mut chain = vec![];
    let mut cur = item.meta.parent.clone();
    while let Some(p) = cur {
        match by_id.get(p.as_str()) {
            Some(pi) if !chain.iter().any(|c: &Item| c.meta.id == pi.meta.id) => {
                chain.push((*pi).clone());
                cur = pi.meta.parent.clone();
            }
            _ => break,
        }
    }
    chain.reverse();
    let children: Vec<ItemOut> = all
        .iter()
        .filter(|i| i.meta.parent.as_deref() == Some(item.meta.id.as_str()))
        .cloned()
        .map(|i| out_marked(i, &touched, &m))
        .collect();
    let backlinks: Vec<Value> = all
        .iter()
        .flat_map(|i| {
            i.meta.links.iter().filter(|l| l.to == item.meta.id).map(move |l| json!({ "rel": l.rel, "item": i }))
        })
        .collect();
    let links: Vec<Value> =
        item.meta.links.iter().map(|l| json!({ "rel": l.rel, "to": l.to, "item": by_id.get(l.to.as_str()) })).collect();
    let mut entries = store.entries(&EntryFilter { item: Some(item.meta.id.clone()), ..Default::default() })?;
    entries.reverse();
    let applicable = if item.meta.kind == Kind::Decision { vec![] } else { store.applicable_decisions(&item, 8)? };

    // The context it was created in: the creating session, and what was happening just before.
    let created = item.meta.created;
    let session = match &item.meta.created_in {
        Some(s) => store.sessions(None, false, 2000)?.into_iter().find(|x| &x.meta.id == s),
        None => None,
    };
    let pool = match &session {
        Some(s) => {
            store.entries(&EntryFilter { session: Some(s.meta.id.clone()), limit: Some(2000), ..Default::default() })?
        }
        None => store.entries(&EntryFilter { project: Some(key.clone()), limit: Some(2000), ..Default::default() })?,
    };
    let earlier: Vec<&Entry> =
        pool.iter().filter(|e| e.meta.at <= created && e.meta.item.as_deref() != Some(&item.meta.id)).collect();
    // Status changes often come in bursts (closing a batch of items); summarize them instead of listing them.
    let mut before: Vec<&Entry> =
        earlier.iter().filter(|e| e.meta.kind != EntryKind::StatusChange).take(8).copied().collect();
    before.reverse();
    let window = created - chrono::Duration::minutes(30);
    let status_changes =
        earlier.iter().filter(|e| e.meta.kind == EntryKind::StatusChange && e.meta.at >= window).count();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for e in &before {
        if let Some(i) = e.meta.item.as_deref() {
            *counts.entry(i).or_default() += 1;
        }
    }
    let working_on = counts.into_iter().max_by_key(|(_, n)| *n).and_then(|(i, _)| by_id.get(i).map(|x| (*x).clone()));

    // What happened since: in the project, after it was created.
    let after_all = store.entries(&EntryFilter {
        project: Some(key.clone()),
        since: Some(created),
        limit: Some(5000),
        ..Default::default()
    })?;
    let after: Vec<&Entry> = after_all.iter().filter(|e| e.meta.at > created).collect();
    let mut after_items: HashMap<&str, usize> = HashMap::new();
    for e in &after {
        if let Some(i) = e.meta.item.as_deref() {
            *after_items.entry(i).or_default() += 1;
        }
    }
    let mut busiest: Vec<(&str, usize)> = after_items.into_iter().collect();
    busiest.sort_by_key(|b| std::cmp::Reverse(b.1));
    let busiest: Vec<Value> = busiest
        .into_iter()
        .take(5)
        .filter_map(|(i, n)| by_id.get(i).map(|x| json!({ "item": x, "entries": n })))
        .collect();
    let recent_after: Vec<&Entry> = after.iter().take(6).copied().collect();

    let story = store.story(&item.meta.id)?;
    Ok(json!({
        "story": story,
        "item": out_marked(item.clone(), &touched, &m),
        "chain": chain,
        "children": children,
        "links": links,
        "backlinks": backlinks,
        "entries": entries,
        "applicable": applicable,
        "asked": {
            "session": session,
            "before": before,
            "working_on": working_on,
            "status_changes": status_changes,
        },
        "since": {
            "entries": after.len(),
            "recent": recent_after,
            "busiest": busiest,
            "last": after.first(),
        },
    }))
}

/// Reads a text file that an item or note refers to (e.g. a report), for display. Only files inside one of the
/// project's roots, not hidden, not credential-like, under 1 MB, and valid UTF-8.
fn file(store: &mut Store, key: &str, rel: &str) -> Result<Value> {
    let p = store.project(key)?;
    let rel = rel.trim().trim_start_matches("./");
    if rel.is_empty() {
        bail!("no path given");
    }
    let name = std::path::Path::new(rel).file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let secretish =
        [".env", ".pem", ".key", ".p12", ".pfx", "id_rsa", "id_ed25519", ".netrc", ".npmrc", ".pypirc", "credentials"];
    if rel.split('/').any(|c| c == "..") {
        bail!("not shown: outside the project's folder");
    }
    if rel.split('/').any(|c| c.starts_with('.')) || secretish.iter().any(|s| name.ends_with(s) || name.starts_with(s))
    {
        bail!("not shown: hidden or credential-like file");
    }
    for root in &p.meta.roots {
        let root = std::fs::canonicalize(root)?;
        let candidate =
            if std::path::Path::new(rel).is_absolute() { std::path::PathBuf::from(rel) } else { root.join(rel) };
        let Ok(full) = std::fs::canonicalize(&candidate) else { continue };
        if !full.starts_with(&root) {
            bail!("not shown: outside the project's folder");
        }
        let meta = std::fs::metadata(&full)?;
        if !meta.is_file() {
            bail!("not a file");
        }
        if meta.len() > 1 << 20 {
            bail!("too large to show ({} KB)", meta.len() / 1024);
        }
        let bytes = std::fs::read(&full)?;
        let text = String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("not a text file"))?;
        let modified = meta.modified().ok().map(chrono::DateTime::<chrono::Utc>::from);
        return Ok(json!({
            "path": rel,
            "full": full.to_string_lossy(),
            "text": text,
            "modified": modified,
            "markdown": full.extension().is_some_and(|e| e == "md" || e == "markdown"),
        }));
    }
    bail!("not found: {rel} in the project's folder")
}

/// Answers a question: records the answer on it, closes it, and optionally records it as a decision so later
/// sessions follow it.
fn answer(store: &mut Store, id: &str, body: &str) -> Result<Value> {
    #[derive(Deserialize)]
    struct B {
        answer: String,
        #[serde(default)]
        decide: bool,
    }
    let b: B = serde_json::from_str(body)?;
    let answer = b.answer.trim().to_string();
    if answer.is_empty() {
        bail!("the answer is empty");
    }
    let q = store.item(id)?;
    let caller = human(store, &q.project);
    store.log(
        &caller,
        LogInput {
            kind: EntryKind::Note,
            body: format!("Answer: {answer}"),
            item: Some(q.meta.id.clone()),
            refs: vec![],
        },
    )?;
    let mut decision = None;
    if b.decide {
        let title: String = answer.lines().next().unwrap_or(&answer).chars().take(140).collect();
        let d = store.decide(
            &caller,
            DecisionInput {
                title,
                because: Some(format!("Answer to {} ({})", q.meta.id, q.meta.title)),
                body: if answer.contains('\n') { answer.clone() } else { String::new() },
                areas: q.meta.areas.clone(),
                applies_to: q.meta.parent.iter().cloned().collect(),
                ..Default::default()
            },
        )?;
        decision = Some(d.meta.id.clone());
    }
    let note = match &decision {
        Some(d) => format!("answered; recorded as {d}"),
        None => "answered".to_string(),
    };
    let closed = store.update_item(
        &caller,
        id,
        ItemPatch { status: Some(Status::Done), note: Some(note), ..Default::default() },
    )?;
    committed(store);
    Ok(json!({ "item": closed, "decision": decision }))
}
