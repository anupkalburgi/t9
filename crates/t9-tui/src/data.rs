//! Every read and write the TUI makes goes through this module, so it can later be pointed at `t9 serve`
//! (REST) instead of the in-process store without touching the screens.

use std::collections::{BTreeMap, HashSet};

use anyhow::Result;
use t9_core::context;
use t9_core::decisions::{DecisionInput, Relevant};
use t9_core::index::{EntryFilter, Hit, ItemFilter};
use t9_core::*;

pub struct ProjectRow {
    pub project: Project,
    pub open: usize,
    pub issues: usize,
    pub decisions: usize,
    pub last: Option<Timestamp>,
}

/// One project per row (GLOBAL last), with counts for the dashboard.
pub fn projects(store: &mut Store) -> Result<Vec<ProjectRow>> {
    store.sync_projects()?;
    let mut rows = vec![];
    for project in store.projects()? {
        let key = project.meta.key.clone();
        store.sync_project(&key)?;
        let items = store.items(&ItemFilter { project: Some(key.clone()), ..Default::default() })?;
        let decisions = store
            .items(&ItemFilter {
                project: Some(key.clone()),
                kinds: vec![Kind::Decision],
                statuses: vec![Status::Accepted],
                include_closed: true,
                ..Default::default()
            })?
            .len();
        let last = store
            .entries(&EntryFilter { project: Some(key.clone()), limit: Some(1), ..Default::default() })?
            .first()
            .map(|e| e.meta.at);
        rows.push(ProjectRow {
            open: items.iter().filter(|i| !matches!(i.meta.kind, Kind::Decision)).count(),
            issues: items.iter().filter(|i| i.meta.kind.is_issue()).count(),
            decisions,
            last,
            project,
        });
    }
    rows.sort_by(|a, b| {
        (a.project.meta.key == GLOBAL)
            .cmp(&(b.project.meta.key == GLOBAL))
            .then(b.last.cmp(&a.last))
            .then(a.project.meta.name.cmp(&b.project.meta.name))
    });
    Ok(rows)
}

/// The project's items except decisions, as a depth-first tree.
pub fn tree(store: &mut Store, key: &str, show_closed: bool) -> Result<Vec<(usize, Item)>> {
    store.sync_project(key)?;
    let items: Vec<Item> = store
        .items(&ItemFilter { project: Some(key.into()), include_closed: show_closed, ..Default::default() })?
        .into_iter()
        .filter(|i| i.meta.kind != Kind::Decision)
        .collect();
    let ids: HashSet<String> = items.iter().map(|i| i.meta.id.clone()).collect();
    let mut kids: BTreeMap<String, Vec<Item>> = BTreeMap::new();
    let mut roots = vec![];
    for i in items {
        match i.meta.parent.clone() {
            Some(p) if ids.contains(&p) => kids.entry(p).or_default().push(i),
            _ => roots.push(i),
        }
    }
    fn walk(i: Item, depth: usize, kids: &mut BTreeMap<String, Vec<Item>>, out: &mut Vec<(usize, Item)>) {
        let children = kids.remove(&i.meta.id).unwrap_or_default();
        out.push((depth, i));
        for c in children {
            walk(c, depth + 1, kids, out);
        }
    }
    let mut out = vec![];
    for r in roots {
        walk(r, 0, &mut kids, &mut out);
    }
    Ok(out)
}

/// All decisions of a project, including superseded ones, newest first.
pub fn decisions(store: &mut Store, key: &str) -> Result<Vec<Item>> {
    Ok(store.decision_log(key, &[], true)?)
}

pub fn journal(store: &mut Store, key: &str) -> Result<Vec<Entry>> {
    store.sync_project(key)?;
    Ok(store.entries(&EntryFilter { project: Some(key.into()), limit: Some(300), ..Default::default() })?)
}

/// Unfiled entries of the project, then inbox entries.
pub fn triage(store: &mut Store, key: &str) -> Result<Vec<Entry>> {
    store.sync_project(INBOX)?;
    let mut v = store.entries(&EntryFilter {
        project: Some(key.into()),
        unfiled: true,
        exclude_kinds: vec![EntryKind::Handoff],
        limit: Some(200),
        ..Default::default()
    })?;
    v.extend(store.entries(&EntryFilter { project: Some(INBOX.into()), limit: Some(200), ..Default::default() })?);
    Ok(v)
}

/// The brief as data (for the Overview) and as the text an agent receives.
pub fn brief(store: &mut Store, caller: &Caller) -> Result<(context::Brief, String)> {
    let b = context::build(store, caller)?;
    let text = context::render(&b, 6000);
    Ok((b, text))
}

pub struct ItemDetail {
    pub item: Item,
    pub children: Vec<Item>,
    pub entries: Vec<Entry>,
    pub applicable: Vec<Relevant>,
}

pub fn item_detail(store: &mut Store, id: &str) -> Result<ItemDetail> {
    let item = store.item(id)?;
    let children = store.items(&ItemFilter {
        parent: Some(Some(item.meta.id.clone())),
        include_closed: true,
        ..Default::default()
    })?;
    let entries =
        store.entries(&EntryFilter { item: Some(item.meta.id.clone()), limit: Some(30), ..Default::default() })?;
    let applicable = if item.meta.kind == Kind::Decision { vec![] } else { store.applicable_decisions(&item, 5)? };
    Ok(ItemDetail { item, children, entries, applicable })
}

pub fn search(store: &mut Store, q: &str, key: Option<&str>) -> Result<Vec<Hit>> {
    if key.is_none() {
        store.sync_all()?;
    }
    Ok(store.search(q, key, 50)?)
}

pub fn entry(store: &Store, id: &str) -> Result<Entry> {
    Ok(store.entry(id)?)
}

// ---------------------------------------------------------------- writes

pub fn set_status(store: &mut Store, caller: &Caller, id: &str, status: Status) -> Result<Item> {
    Ok(store.update_item(caller, id, ItemPatch { status: Some(status), ..Default::default() })?)
}

/// `bug Crash on save` → a bug titled "Crash on save" (kind defaults to task).
pub fn add(store: &mut Store, caller: &Caller, text: &str, parent: Option<String>) -> Result<Item> {
    let text = text.trim();
    let (kind, title) = match text.split_once(' ') {
        Some((k, rest)) if k.parse::<Kind>().is_ok() => (k.parse::<Kind>().ok(), rest.trim()),
        _ => (Some(Kind::Task), text),
    };
    Ok(store.add_item(caller, NewItem { kind, title: title.into(), parent, ..Default::default() })?)
}

pub fn log(store: &mut Store, caller: &Caller, text: &str, item: Option<String>) -> Result<Entry> {
    Ok(store.log(caller, LogInput { kind: EntryKind::Note, body: text.into(), item, refs: vec![] })?)
}

/// `Title | why` records a decision (or rejection) with its reason.
pub fn decide(store: &mut Store, caller: &Caller, text: &str, status: Status) -> Result<Item> {
    let (title, because) = match text.split_once('|') {
        Some((t, w)) => (t.trim().to_string(), Some(w.trim().to_string()).filter(|w| !w.is_empty())),
        None => (text.trim().to_string(), None),
    };
    Ok(store.decide(caller, DecisionInput { title, because, status, ..Default::default() })?)
}

pub fn unlinked(store: &mut Store) -> Result<Vec<t9_core::unlinked::UnlinkedFolder>> {
    Ok(store.unlinked_folders()?)
}

/// `Name | KEY` (the key is optional) creates a project for `folder` and moves its inbox notes into it.
pub fn create_project(store: &mut Store, folder: &str, text: &str) -> Result<(Project, usize)> {
    let (name, key) = match text.split_once('|') {
        Some((n, k)) => (n.trim().to_string(), Some(k.trim().to_string()).filter(|k| !k.is_empty())),
        None => (text.trim().to_string(), None),
    };
    let p = store.init_project(&name, key.as_deref(), "", Some(std::path::Path::new(folder)))?;
    let moved = store.adopt_inbox(&p.meta.key)?;
    Ok((p, moved))
}

pub fn attach(store: &mut Store, entry: &str, item: &str) -> Result<Entry> {
    Ok(store.attach(entry, item)?)
}
