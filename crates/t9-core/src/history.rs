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

// ---------------------------------------------------------------- the timeline chart

/// One lane of the timeline chart: a feature or phase (or a plan, for work directly under it), or a group of
/// loose items.
#[derive(Debug, Clone, Serialize)]
pub struct Lane {
    pub id: String,
    pub title: String,
    pub kind: Option<Kind>,
    pub status: Option<Status>,
    pub open: usize,
    pub closed: usize,
    pub last: Option<Timestamp>,
}

/// An item's lifespan: created → closed (or still open).
#[derive(Debug, Clone, Serialize)]
pub struct Span {
    pub id: String,
    pub lane: String,
    pub kind: Kind,
    pub status: Status,
    pub title: String,
    pub created: Timestamp,
    pub closed: Option<Timestamp>,
    /// done, merged, dead, partial, open.
    pub outcome: &'static str,
    pub due: Option<chrono::NaiveDate>,
}

/// A mark on the chart.
#[derive(Debug, Clone, Serialize)]
pub struct Mark {
    pub at: Timestamp,
    /// new, done, merged, dead, partial, reopened, started, blocked, decision, finding, blocker, handoff, progress,
    /// checkin, note.
    pub kind: &'static str,
    pub glyph: &'static str,
    pub item: Option<String>,
    pub lane: String,
    pub text: String,
    pub agent: String,
    pub session: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSpan {
    pub id: String,
    pub agent: String,
    pub started: Timestamp,
    pub ended: Option<Timestamp>,
    pub last_seen: Option<Timestamp>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelineView {
    pub lanes: Vec<Lane>,
    pub spans: Vec<Span>,
    pub marks: Vec<Mark>,
    pub sessions: Vec<SessionSpan>,
    pub now: Timestamp,
}

/// Lane for an item: its feature or phase (the first one below the plan, from the top), else its plan, else a group
/// by kind for loose items. Structural items without a parent are their own lane.
fn lane_of(i: &Item, by_id: &std::collections::HashMap<&str, &Item>) -> String {
    let structural = |k: Kind| matches!(k, Kind::Plan | Kind::Phase | Kind::Feature);
    let mut chain = vec![i];
    let mut cur = i.meta.parent.as_deref();
    while let Some(p) = cur {
        let Some(a) = by_id.get(p) else { break };
        if chain.iter().any(|c| c.meta.id == a.meta.id) {
            break;
        }
        chain.push(a);
        cur = a.meta.parent.as_deref();
    }
    chain.reverse(); // top first
    if let Some(f) = chain.iter().find(|c| matches!(c.meta.kind, Kind::Feature | Kind::Phase)) {
        return f.meta.id.clone();
    }
    if structural(chain[0].meta.kind) {
        return chain[0].meta.id.clone();
    }
    match i.meta.kind {
        Kind::Decision => "~decisions".into(),
        Kind::Question => "~questions".into(),
        Kind::Idea => "~ideas".into(),
        Kind::Experiment => "~experiments".into(),
        Kind::Resource => "~resources".into(),
        _ => "~other".into(),
    }
}

impl Store {
    pub fn timeline_view(&mut self, project: &str, since: Option<Timestamp>) -> Result<TimelineView> {
        self.sync_project(project)?;
        let items =
            self.items(&ItemFilter { project: Some(project.into()), include_closed: true, ..Default::default() })?;
        let by_id: std::collections::HashMap<&str, &Item> = items.iter().map(|i| (i.meta.id.as_str(), i)).collect();
        let closings = self.closings(project)?;
        let lane: std::collections::HashMap<String, String> =
            items.iter().map(|i| (i.meta.id.clone(), lane_of(i, &by_id))).collect();
        let in_range = |t: Timestamp| since.is_none_or(|s| t >= s);

        let mut spans = vec![];
        for i in items.iter().filter(|i| !matches!(i.meta.kind, Kind::Resource | Kind::Note)) {
            let c = closings.get(&i.meta.id).filter(|_| i.meta.status.is_closed());
            if !in_range(c.map_or(crate::model::now(), |c| c.at)) {
                continue;
            }
            let outcome = if i.meta.status.is_closed() {
                crate::story::classify(i.meta.status, c.and_then(|c| c.note.as_deref()).unwrap_or("")).0
            } else {
                "open"
            };
            spans.push(Span {
                id: i.meta.id.clone(),
                lane: lane[&i.meta.id].clone(),
                kind: i.meta.kind,
                status: i.meta.status,
                title: i.meta.title.clone(),
                created: i.meta.created,
                closed: c.map(|c| c.at),
                outcome,
                due: i.meta.due,
            });
        }

        let mut marks = vec![];
        for i in items.iter().filter(|i| in_range(i.meta.created)) {
            let glyph = match i.meta.kind {
                Kind::Question => "?",
                Kind::Decision => "◆",
                Kind::Idea => "✦",
                Kind::Experiment => "⚗",
                _ => "●",
            };
            marks.push(Mark {
                at: i.meta.created,
                kind: if i.meta.kind == Kind::Decision { "decision" } else { "new" },
                glyph,
                item: Some(i.meta.id.clone()),
                lane: lane[&i.meta.id].clone(),
                text: format!("new {}: {}", i.meta.kind, i.meta.title),
                agent: i.meta.created_by.clone(),
                session: i.meta.created_in.clone(),
            });
        }
        let entries = self.entries(&EntryFilter {
            project: Some(project.into()),
            since,
            limit: Some(20_000),
            ..Default::default()
        })?;
        for e in &entries {
            // "Decided X" entries repeat the new decision item: keep one.
            if e.meta.kind == EntryKind::Decision
                && let Some(d) = e.meta.item.as_deref().and_then(|i| by_id.get(i))
                && d.meta.kind == Kind::Decision
                && (e.meta.at - d.meta.created).num_seconds().abs() <= 60
            {
                continue;
            }
            let item_lane =
                e.meta.item.as_ref().and_then(|i| lane.get(i)).cloned().unwrap_or_else(|| "~sessions".into());
            let (kind, glyph): (&'static str, &'static str) = match e.meta.kind {
                EntryKind::StatusChange => match crate::insights::parse_status_change(&e.body) {
                    Some((_, _, to)) if to.is_closed() => {
                        let note = e.body.split_once(" — ").map(|x| x.1).unwrap_or("");
                        let c = crate::story::classify(to, note).0;
                        (c, crate::story::glyph(c))
                    }
                    Some((_, from, _)) if from.is_closed() => ("reopened", "↺"),
                    Some((_, _, Status::Active)) => ("started", "▸"),
                    Some((_, _, Status::Blocked)) => ("blocked", "⚑"),
                    _ => ("note", "·"),
                },
                EntryKind::Decision => ("decision", "◆"),
                EntryKind::Finding => ("finding", "★"),
                EntryKind::Blocker => ("blocker", "⚑"),
                EntryKind::Handoff => ("handoff", "✉"),
                EntryKind::Progress => ("progress", "·"),
                EntryKind::Checkin => ("checkin", "·"),
                EntryKind::Note => ("note", "·"),
            };
            marks.push(Mark {
                at: e.meta.at,
                kind,
                glyph,
                item: e.meta.item.clone(),
                lane: item_lane,
                text: crate::context::first_line(&e.body, 200),
                agent: e.meta.agent.clone(),
                session: e.meta.session.clone(),
            });
        }
        marks.sort_by_key(|m| m.at);

        let mut lanes: Vec<Lane> = vec![];
        let group_title = |id: &str| match id {
            "~decisions" => "Decisions",
            "~questions" => "Questions",
            "~ideas" => "Ideas",
            "~experiments" => "Experiments",
            "~resources" => "Resources",
            "~sessions" => "Sessions and handoffs",
            _ => "Other work",
        };
        for s in &spans {
            let l = match lanes.iter_mut().position(|l| l.id == s.lane) {
                Some(p) => &mut lanes[p],
                None => {
                    let it = by_id.get(s.lane.as_str());
                    lanes.push(Lane {
                        id: s.lane.clone(),
                        title: it.map_or_else(|| group_title(&s.lane).to_string(), |i| i.meta.title.clone()),
                        kind: it.map(|i| i.meta.kind),
                        status: it.map(|i| i.meta.status),
                        open: 0,
                        closed: 0,
                        last: None,
                    });
                    lanes.last_mut().unwrap()
                }
            };
            if s.id != s.lane {
                if s.closed.is_some() { l.closed += 1 } else { l.open += 1 }
            }
        }
        for m in &marks {
            if let Some(l) = lanes.iter_mut().find(|l| l.id == m.lane) {
                l.last = l.last.max(Some(m.at));
            }
        }
        lanes.sort_by_key(|l| std::cmp::Reverse(l.last));

        let sessions = self
            .sessions(Some(project), false, 500)?
            .into_iter()
            .filter(|s| in_range(s.meta.ended.or(s.last_seen).unwrap_or(s.meta.started)))
            .map(|s| SessionSpan {
                id: s.meta.id.clone(),
                agent: s.meta.agent.clone(),
                started: s.meta.started,
                ended: s.meta.ended,
                last_seen: s.last_seen,
            })
            .collect();
        Ok(TimelineView { lanes, spans, marks, sessions, now: crate::model::now() })
    }
}
