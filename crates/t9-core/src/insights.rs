//! "Are we going in circles?": signals computed from a project's history.
//!
//! * flip-flop: a decision changed two or more times (a supersede chain of three or more);
//! * reopened: an item closed and then opened again;
//! * repeat: open work that resembles an earlier dead end (failed experiment, rejected option, dropped item);
//! * spinning: a branch of the plan with a lot of recent activity but nothing finished, or issues opening faster
//!   than they close;
//! * stuck: an item active for days with many entries but not finished.

use std::collections::{HashMap, HashSet};

use chrono::Duration;
use serde::Serialize;

use crate::Result;
use crate::decisions::keywords;
use crate::index::{EntryFilter, ItemFilter};
use crate::model::*;
use crate::store::Store;

#[derive(Debug, Clone, Serialize)]
pub struct Signal {
    /// `flip-flop`, `reopened`, `repeat`, `spinning`, `stuck`.
    pub kind: String,
    /// 1 (worth a look) to 3 (likely a loop).
    pub severity: u8,
    pub title: String,
    pub detail: String,
    pub items: Vec<String>,
}

/// Recent activity of one branch of the plan (an item and everything under it).
#[derive(Debug, Clone, Serialize)]
pub struct BranchStats {
    pub id: String,
    pub entries_7d: usize,
    pub closed_7d: usize,
    pub opened_7d: usize,
    pub spinning: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Insights {
    pub signals: Vec<Signal>,
    pub branches: Vec<BranchStats>,
}

/// Parses a status-change entry body: `BIL-5: active → done — note`.
pub fn parse_status_change(body: &str) -> Option<(String, Status, Status)> {
    let (id, rest) = body.split_once(": ")?;
    let (from, to) = rest.split_once(" → ")?;
    let to = to.split(" — ").next()?.trim();
    Some((id.trim().to_string(), from.trim().parse().ok()?, to.parse().ok()?))
}

fn is_dead_end(i: &Item) -> bool {
    matches!(i.meta.status, Status::Failed | Status::Inconclusive | Status::Rejected | Status::Dropped)
        || (i.meta.kind == Kind::Decision && i.meta.status == Status::Superseded)
}

fn similar(a: &[String], b: &[String]) -> (usize, f64) {
    let sa: HashSet<&String> = a.iter().collect();
    let sb: HashSet<&String> = b.iter().collect();
    let shared = sa.intersection(&sb).count();
    let union = sa.union(&sb).count().max(1);
    (shared, shared as f64 / union as f64)
}

impl Store {
    pub fn insights(&mut self, key: &str) -> Result<Insights> {
        self.sync_project(key)?;
        let items =
            self.items(&ItemFilter { project: Some(key.into()), include_closed: true, ..Default::default() })?;
        let by_id: HashMap<&str, &Item> = items.iter().map(|i| (i.meta.id.as_str(), i)).collect();
        let entries =
            self.entries(&EntryFilter { project: Some(key.into()), limit: Some(20_000), ..Default::default() })?;
        let week_ago = now() - Duration::days(7);
        let mut signals = vec![];

        // Flip-flops: follow supersede chains back from decisions that are in force.
        for d in items.iter().filter(|i| i.meta.kind == Kind::Decision && !matches!(i.meta.status, Status::Superseded))
        {
            let mut chain = vec![d.meta.id.clone()];
            let mut cur = d;
            while let Some(prev) =
                cur.meta.links.iter().find(|l| l.rel == "supersedes").and_then(|l| by_id.get(l.to.as_str()))
            {
                if chain.contains(&prev.meta.id) {
                    break;
                }
                chain.push(prev.meta.id.clone());
                cur = prev;
            }
            if chain.len() >= 3 {
                chain.reverse();
                let changes = chain.len() - 1;
                signals.push(Signal {
                    kind: "flip-flop".into(),
                    severity: if changes >= 3 { 3 } else { 2 },
                    title: format!("Decision changed {changes} times: {}", d.meta.title),
                    detail: chain.join(" → "),
                    items: chain,
                });
            }
        }

        // Reopened: status changes from closed back to open or active.
        let mut reopened: HashMap<String, usize> = HashMap::new();
        let mut closed_at: Vec<(String, Timestamp)> = vec![];
        for e in entries.iter().filter(|e| e.meta.kind == EntryKind::StatusChange) {
            if let Some((id, from, to)) = parse_status_change(&e.body) {
                if from.is_closed() && !to.is_closed() {
                    *reopened.entry(id.clone()).or_default() += 1;
                }
                if !from.is_closed() && to.is_closed() {
                    closed_at.push((id, e.meta.at));
                }
            }
        }
        for (id, n) in &reopened {
            if let Some(i) = by_id.get(id.as_str()) {
                signals.push(Signal {
                    kind: "reopened".into(),
                    severity: if *n >= 2 { 2 } else { 1 },
                    title: format!("Reopened {n} time{}: {}", if *n == 1 { "" } else { "s" }, i.meta.title),
                    detail: format!("{} was closed and opened again", i.meta.id),
                    items: vec![id.clone()],
                });
            }
        }

        // Repeats: open work that resembles an earlier dead end.
        let dead: Vec<(&Item, Vec<String>)> =
            items.iter().filter(|i| is_dead_end(i)).map(|i| (i, keywords(&i.meta.title))).collect();
        let mut repeats = 0;
        for i in items.iter().filter(|i| !i.meta.status.is_closed() && i.meta.kind != Kind::Resource) {
            let kw = keywords(&i.meta.title);
            if kw.len() < 2 {
                continue;
            }
            let best = dead
                .iter()
                .filter(|(d, _)| d.meta.id != i.meta.id && d.meta.created <= i.meta.created)
                .map(|(d, dk)| (d, similar(&kw, dk)))
                .filter(|(_, (shared, j))| *shared >= 2 && *j >= 0.4)
                .max_by(|a, b| a.1.1.total_cmp(&b.1.1));
            if let Some((d, (_, j))) = best {
                repeats += 1;
                if repeats > 12 {
                    break;
                }
                signals.push(Signal {
                    kind: "repeat".into(),
                    severity: if j >= 0.6 { 2 } else { 1 },
                    title: format!("{} resembles a dead end: {} ({})", i.meta.id, d.meta.id, d.meta.status),
                    detail: format!("“{}” vs “{}”", i.meta.title, d.meta.title),
                    items: vec![i.meta.id.clone(), d.meta.id.clone()],
                });
            }
        }

        // Branch activity: attribute entries, closures and new issues to every ancestor.
        let ancestors = |id: &str| -> Vec<String> {
            let mut out = vec![];
            let mut cur = by_id.get(id).and_then(|i| i.meta.parent.clone());
            while let Some(p) = cur {
                if out.contains(&p) {
                    break;
                }
                cur = by_id.get(p.as_str()).and_then(|i| i.meta.parent.clone());
                out.push(p);
            }
            out
        };
        let mut stats: HashMap<String, (usize, usize, usize)> = HashMap::new();
        for e in entries.iter().filter(|e| e.meta.at >= week_ago && e.meta.kind != EntryKind::Checkin) {
            if let Some(id) = &e.meta.item {
                for a in std::iter::once(id.clone()).chain(ancestors(id)) {
                    stats.entry(a).or_default().0 += 1;
                }
            }
        }
        for (id, at) in closed_at.iter().filter(|(_, at)| *at >= week_ago) {
            let _ = at;
            for a in ancestors(id) {
                stats.entry(a).or_default().1 += 1;
            }
        }
        for i in items.iter().filter(|i| i.meta.kind.is_issue() && i.meta.created >= week_ago) {
            for a in ancestors(&i.meta.id) {
                stats.entry(a).or_default().2 += 1;
            }
        }
        let has_children: HashSet<&str> = items.iter().filter_map(|i| i.meta.parent.as_deref()).collect();
        let mut branches = vec![];
        for (id, (n, closed, opened)) in &stats {
            if !has_children.contains(id.as_str()) {
                continue;
            }
            let Some(item) = by_id.get(id.as_str()) else { continue };
            let busy_no_progress = *n >= 12 && *closed == 0;
            let growing = *opened >= 5 && *opened > 2 * *closed;
            let spinning = busy_no_progress || growing;
            branches.push(BranchStats {
                id: id.clone(),
                entries_7d: *n,
                closed_7d: *closed,
                opened_7d: *opened,
                spinning,
            });
            if spinning && item.meta.kind != Kind::Plan {
                let detail = if busy_no_progress {
                    format!("{n} entries in 7 days, nothing under it finished")
                } else {
                    format!("{opened} new issues in 7 days, {closed} items finished")
                };
                signals.push(Signal {
                    kind: "spinning".into(),
                    severity: if *n >= 25 || *opened >= 10 { 3 } else { 2 },
                    title: format!("Spinning: {}", item.meta.title),
                    detail,
                    items: vec![id.clone()],
                });
            }
        }

        // Stuck: active for a while, many entries, not finished.
        let mut entry_count: HashMap<&str, usize> = HashMap::new();
        for e in entries.iter().filter(|e| e.meta.kind != EntryKind::Checkin) {
            if let Some(i) = e.meta.item.as_deref() {
                *entry_count.entry(i).or_default() += 1;
            }
        }
        for i in items.iter().filter(|i| i.meta.status == Status::Active && !has_children.contains(i.meta.id.as_str()))
        {
            let active_since = entries
                .iter()
                .filter(|e| {
                    e.meta.kind == EntryKind::StatusChange && e.meta.item.as_deref() == Some(i.meta.id.as_str())
                })
                .filter_map(|e| {
                    parse_status_change(&e.body).filter(|(_, _, to)| *to == Status::Active).map(|_| e.meta.at)
                })
                .max()
                .unwrap_or(i.meta.created);
            let days = (now() - active_since).num_days();
            let n = entry_count.get(i.meta.id.as_str()).copied().unwrap_or(0);
            if days >= 5 && n >= 8 {
                signals.push(Signal {
                    kind: "stuck".into(),
                    severity: if days >= 10 { 2 } else { 1 },
                    title: format!("Active for {days} days, {n} entries, not finished: {}", i.meta.title),
                    detail: format!(
                        "{} has been in progress since {}",
                        i.meta.id,
                        crate::clock::date_time(&active_since)
                    ),
                    items: vec![i.meta.id.clone()],
                });
            }
        }

        signals.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.kind.cmp(&b.kind)).then(a.title.cmp(&b.title)));
        branches.sort_by_key(|a| std::cmp::Reverse(a.entries_7d));
        Ok(Insights { signals, branches })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_change_bodies() {
        assert_eq!(
            parse_status_change("BIL-5: done → active — reopened"),
            Some(("BIL-5".into(), Status::Done, Status::Active))
        );
        assert_eq!(parse_status_change("BIL-5: open → failed"), Some(("BIL-5".into(), Status::Open, Status::Failed)));
        assert!(parse_status_change("Decided BIL-5: x").is_none());
    }
}
