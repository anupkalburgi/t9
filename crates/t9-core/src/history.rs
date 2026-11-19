//! Looking back: `t9 history TOPIC` (everything known about a topic, in time order) and `t9 timeline` (what
//! happened when). Both return [`Event`]s so the CLI, the TUI and agents see the same thing.

use std::collections::HashSet;

use chrono::Local;
use serde::Serialize;

use crate::Result;
use crate::decisions::{keywords, why};
use crate::index::{self, EntryFilter, ItemFilter};
use crate::model::*;
use crate::store::{GLOBAL, Store};

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub at: Timestamp,
    /// `item` (something recorded) or `entry` (a journal entry).
    pub source: String,
    /// The item kind (`experiment`, `decision`, …) or the entry kind (`finding`, `status-change`, …).
    pub kind: String,
    /// Item id, or entry id for entries.
    pub id: String,
    /// For entries: the item they are filed under.
    pub item: Option<String>,
    pub status: Option<Status>,
    pub project: String,
    pub title: String,
    /// The one line that matters: why (decisions), hypothesis/outcome (experiments), location (resources).
    pub detail: String,
    /// A hint for whoever plans next, e.g. "retry date passed".
    pub flag: Option<String>,
}

fn tokens(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// How many of `kws` appear in `text` (words of 4+ characters also match by prefix).
fn hit_count(kws: &[String], text: &str) -> usize {
    let toks = tokens(text);
    kws.iter()
        .filter(|k| {
            toks.contains(*k)
                || (k.len() >= 4
                    && toks.iter().any(|t| t.len() >= 4 && (t.starts_with(k.as_str()) || k.starts_with(t.as_str()))))
        })
        .count()
}

/// The "what matters" line for an item.
pub fn item_detail(i: &Item) -> String {
    let m = &i.meta;
    match m.kind {
        Kind::Decision => why(i),
        Kind::Experiment => {
            let mut parts = vec![];
            if let Some(h) = &m.hypothesis {
                parts.push(format!("hypothesis: {h}"));
            }
            if let Some(w) = &m.retry_when {
                parts.push(format!("retry when: {w}"));
            }
            if let Some(d) = m.retry_after {
                parts.push(format!("retry after {d}"));
            }
            parts.join(" · ")
        }
        Kind::Resource => {
            let mut parts = vec![];
            if let Some(t) = &m.resource_type {
                parts.push(t.clone());
            }
            if let Some(l) = &m.location {
                parts.push(l.clone());
            }
            if let Some(o) = &m.owner {
                parts.push(format!("owner {o}"));
            }
            parts.join(" · ")
        }
        _ => crate::context::first_line(&i.body, 160),
    }
}

/// Is a failed or inconclusive experiment due for another try?
pub fn retry_flag(i: &Item) -> Option<String> {
    let m = &i.meta;
    if m.kind != Kind::Experiment || !matches!(m.status, Status::Failed | Status::Inconclusive) {
        return None;
    }
    let today = Local::now().date_naive();
    match (m.retry_after, &m.retry_when) {
        (Some(d), _) if d <= today => Some(format!("retry date {d} has passed")),
        (_, Some(w)) => Some(format!("worth retrying when: {w}")),
        _ => None,
    }
}

fn order_key(e: &Event) -> (bool, u32, String) {
    let num = split_item_id(&e.id).map(|(_, n)| n).unwrap_or(0);
    (e.source == "entry", num, e.id.clone())
}

fn item_event(i: &Item) -> Event {
    Event {
        at: i.meta.created,
        source: "item".into(),
        kind: i.meta.kind.as_str().into(),
        id: i.meta.id.clone(),
        item: None,
        status: Some(i.meta.status),
        project: i.project.clone(),
        title: i.meta.title.clone(),
        detail: item_detail(i),
        flag: retry_flag(i),
    }
}

fn entry_event(e: &Entry) -> Event {
    Event {
        at: e.meta.at,
        source: "entry".into(),
        kind: e.meta.kind.as_str().into(),
        id: e.meta.id.clone(),
        item: e.meta.item.clone(),
        status: None,
        project: e.project.clone(),
        title: crate::context::first_line(&e.body, 200),
        detail: e.meta.refs.join(", "),
        flag: None,
    }
}

impl Store {
    fn history_projects(&mut self, project: Option<&str>) -> Result<Vec<String>> {
        let mut v: Vec<String> = match project {
            Some(p) => vec![p.to_string()],
            None => self.projects()?.into_iter().map(|p| p.meta.key).collect(),
        };
        if !v.iter().any(|p| p == GLOBAL) && self.project(GLOBAL).is_ok() {
            v.push(GLOBAL.into());
        }
        for p in &v {
            self.sync_project(p)?;
        }
        Ok(v)
    }

    /// Everything recorded about `topic`, oldest first: items of every kind (decisions with what they replaced,
    /// rejected options, experiments and their outcomes, ideas, resources, bugs, tasks) and the journal entries
    /// behind them. An item or entry matches if at least half of the topic's words appear in it.
    pub fn history(&mut self, topic: &str, project: Option<&str>) -> Result<Vec<Event>> {
        let kws = keywords(topic);
        if kws.is_empty() {
            return Ok(vec![]);
        }
        let need = kws.len().div_ceil(2);
        let projects = self.history_projects(project)?;
        let fts = index::fts_query_any(&kws);
        let mut events = vec![];
        let mut matched: HashSet<String> = HashSet::new();
        for i in index::search_items(&self.conn, &fts, Kind::ALL, &projects, 400)? {
            let text = format!("{} {} {} {}", i.meta.title, i.meta.areas.join(" "), item_detail(&i), i.body);
            if hit_count(&kws, &text) >= need {
                matched.insert(i.meta.id.clone());
                events.push(item_event(&i));
            }
        }
        // Decisions bring their history along: what they replaced and what replaced them.
        let mut extra = vec![];
        for e in &events {
            if e.kind != "decision" {
                continue;
            }
            if let Some(i) = index::item(&self.conn, &e.id)? {
                for l in i.meta.links.iter().filter(|l| l.rel == "supersedes" || l.rel == "superseded-by") {
                    if !matched.contains(&l.to)
                        && let Some(other) = index::item(&self.conn, &l.to)?
                    {
                        matched.insert(other.meta.id.clone());
                        extra.push(item_event(&other));
                    }
                }
            }
        }
        events.extend(extra);
        for e in index::search_entries(&self.conn, &fts, &projects, 600)? {
            let about_match = e.meta.item.as_ref().is_some_and(|i| matched.contains(i));
            // A matched decision already stands for its "Decided …" entry.
            if e.meta.kind == EntryKind::Decision && about_match {
                continue;
            }
            let useful = !matches!(e.meta.kind, EntryKind::StatusChange) || about_match;
            if useful && (about_match || hit_count(&kws, &e.body) >= need) {
                events.push(entry_event(&e));
            }
        }
        // Outcomes of matched items (e.g. an experiment moving to failed), even if the entry text doesn't match.
        for id in &matched {
            for e in self.entries(&EntryFilter {
                item: Some(id.clone()),
                kinds: vec![EntryKind::StatusChange],
                ..Default::default()
            })? {
                if !events.iter().any(|x| x.id == e.meta.id) {
                    events.push(entry_event(&e));
                }
            }
        }
        // Same second: items before the entries about them; items by number, entries in creation order.
        events.sort_by(|a, b| a.at.cmp(&b.at).then(order_key(a).cmp(&order_key(b))));
        events.truncate(300);
        Ok(events)
    }

    /// What happened when: work started and finished, decisions and rejections, experiment outcomes, blockers,
    /// findings, handoffs, and new plans, features, experiments, resources and bugs. Newest first.
    pub fn timeline(&mut self, project: &str, since: Option<Timestamp>, topic: Option<&str>) -> Result<Vec<Event>> {
        self.sync_project(project)?;
        let mut events = vec![];
        for e in self.entries(&EntryFilter {
            project: Some(project.into()),
            kinds: vec![
                EntryKind::StatusChange,
                EntryKind::Decision,
                EntryKind::Handoff,
                EntryKind::Blocker,
                EntryKind::Finding,
            ],
            since,
            limit: Some(2000),
            ..Default::default()
        })? {
            events.push(entry_event(&e));
        }
        let created_kinds = [
            Kind::Plan,
            Kind::Phase,
            Kind::Feature,
            Kind::Experiment,
            Kind::Resource,
            Kind::Bug,
            Kind::Regression,
            Kind::Idea,
            Kind::Question,
        ];
        for i in self.items(&ItemFilter {
            project: Some(project.into()),
            kinds: created_kinds.to_vec(),
            include_closed: true,
            ..Default::default()
        })? {
            if since.is_none_or(|s| i.meta.created >= s) {
                let mut ev = item_event(&i);
                ev.kind = format!("new {}", i.meta.kind);
                events.push(ev);
            }
        }
        if let Some(t) = topic {
            let kws = keywords(t);
            let need = kws.len().div_ceil(2).max(1);
            events.retain(|e| {
                hit_count(&kws, &format!("{} {} {}", e.title, e.detail, e.item.clone().unwrap_or_default())) >= need
            });
        }
        events.sort_by(|a, b| b.at.cmp(&a.at).then(b.id.cmp(&a.id)));
        Ok(events)
    }
}
