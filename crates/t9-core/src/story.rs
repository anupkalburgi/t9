//! An item's story: the goal it served, what came out of it, and what it led to. Derived from what is already
//! recorded (the `goal` field, the parent chain, the closing status-change note, links and the ids a closing note
//! names), so the CLI, the brief and the web UI tell the same story.

use std::collections::HashMap;

use serde::Serialize;

use crate::Result;
use crate::index::{EntryFilter, ItemFilter};
use crate::insights::parse_status_change;
use crate::model::*;
use crate::store::Store;

/// The latest change into a closed status, with its note.
#[derive(Debug, Clone, Serialize)]
pub struct Closing {
    pub at: Timestamp,
    /// The status-change entry's id (ULIDs sort in time order, finer than `at`).
    pub entry: String,
    pub from: Status,
    pub to: Status,
    pub note: Option<String>,
    /// How many status changes were recorded in the same second (a batch closure when large).
    pub batch: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Goal {
    pub text: String,
    /// `stated` (the item's own goal or hypothesis), `inherited` (an ancestor's goal) or `parent` (the parent's
    /// title, the best guess when nothing is stated).
    pub source: &'static str,
    /// The ancestor it came from, when not stated.
    pub from: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    /// `done` ✓, `merged` ⇢ (combined into, duplicate of, superseded by), `dead` ✗ (dropped, failed, rejected,
    /// obsolete), `partial` ~ (inconclusive), or `open` for work not finished.
    pub class: &'static str,
    pub glyph: &'static str,
    /// A short reason when one can be read from the note: obsolete, duplicate, combined, superseded, failed …
    pub why: Option<&'static str>,
    pub text: Option<String>,
    pub at: Option<Timestamp>,
    pub batch: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LedTo {
    pub id: String,
    pub title: Option<String>,
    /// The link rel (`superseded-by`, `blocks`, `← blocked-by` for a backlink) or the words before an id in the
    /// closing note ("combined into", "follows").
    pub how: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Rollup {
    pub class: &'static str,
    pub glyph: &'static str,
    pub count: usize,
    /// One representative closing note.
    pub example: Option<String>,
    pub why: Vec<(&'static str, usize)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Story {
    pub goal: Option<Goal>,
    pub outcome: Outcome,
    pub led_to: Vec<LedTo>,
    /// For items with children: their outcomes, grouped.
    pub children: Vec<Rollup>,
    /// The latest progress or finding, for work still open.
    pub latest: Option<Entry>,
    /// Questions the user asked the agents about it, with their answers.
    pub asked: Vec<crate::ask::Asked>,
}

/// Reads the closing note's meaning: `(class, why)`.
pub fn classify(to: Status, note: &str) -> (&'static str, Option<&'static str>) {
    let n = note.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    let merged = if has(&["duplicate of", "dup of", "duplicate"]) {
        Some("duplicate")
    } else if has(&["combined into", "merged into", "folded into", "rolled into", "absorbed into", "covered by"]) {
        Some("combined")
    } else if has(&["superseded", "replaced by"]) || to == Status::Superseded {
        Some("superseded")
    } else {
        None
    };
    if let Some(w) = merged {
        return ("merged", Some(w));
    }
    match to {
        Status::Done => ("done", None),
        Status::Inconclusive => ("partial", Some("inconclusive")),
        Status::Failed => ("dead", Some("failed")),
        Status::Rejected => ("dead", Some("rejected")),
        _ if has(&["obsolete", "no longer", "not needed", "moot", "irrelevant"]) => ("dead", Some("obsolete")),
        _ if has(&["won't", "wont", "not worth", "out of scope"]) => ("dead", Some("not worth it")),
        _ if has(&["fail", "broke", "does not work", "doesn't work"]) => ("dead", Some("failed")),
        _ if to.is_closed() => ("dead", Some("dropped")),
        _ => ("open", None),
    }
}

pub fn glyph(class: &str) -> &'static str {
    match class {
        "done" => "✓",
        "merged" => "⇢",
        "dead" => "✗",
        "partial" => "~",
        _ => "●",
    }
}

/// Item ids a note names, each with the (up to two) words before it: `("QR-19", "combined into")`.
pub fn mentions(note: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![];
    let words: Vec<&str> = note.split_whitespace().collect();
    for (i, w) in words.iter().enumerate() {
        let token = w.trim_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '-'));
        let Ok(id) = normalize_item_id(token) else { continue };
        if token.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
            continue;
        }
        // Up to two words before the id, stopping at punctuation ("merged (follows QR-45)" → "follows").
        let mut how: Vec<String> = vec![];
        for raw in words[i.saturating_sub(2)..i].iter().rev() {
            if raw.ends_with([':', ',', ';', '.', ')']) {
                break;
            }
            let w = raw.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
            if w.is_empty() || normalize_item_id(&w).is_ok() {
                break;
            }
            how.insert(0, w);
            if raw.starts_with('(') {
                break;
            }
        }
        if !out.iter().any(|(x, _)| *x == id) {
            out.push((id, how.join(" ")));
        }
    }
    out
}

/// The goal: stated on the item (or its hypothesis), else an ancestor's goal, else the parent's title.
pub fn goal_of(item: &Item, by_id: &HashMap<&str, &Item>) -> Option<Goal> {
    let own = item.meta.goal.clone().or_else(|| item.meta.hypothesis.clone()).filter(|g| !g.trim().is_empty());
    if let Some(text) = own {
        return Some(Goal { text, source: "stated", from: None });
    }
    let mut cur = item.meta.parent.clone();
    let mut seen = vec![item.meta.id.clone()];
    let mut parent_title = None;
    while let Some(p) = cur {
        if seen.contains(&p) {
            break;
        }
        let Some(a) = by_id.get(p.as_str()) else { break };
        if parent_title.is_none() {
            parent_title = Some((a.meta.title.clone(), a.meta.id.clone()));
        }
        if let Some(g) = a.meta.goal.clone().filter(|g| !g.trim().is_empty()) {
            return Some(Goal { text: g, source: "inherited", from: Some(a.meta.id.clone()) });
        }
        seen.push(p.clone());
        cur = a.meta.parent.clone();
    }
    parent_title.map(|(text, from)| Goal { text, source: "parent", from: Some(from) })
}

impl Store {
    /// The latest closing status change of every item in a project.
    pub fn closings(&self, key: &str) -> Result<HashMap<String, Closing>> {
        let changes = self.entries(&EntryFilter {
            project: Some(key.into()),
            kinds: vec![EntryKind::StatusChange],
            limit: Some(50_000),
            ..Default::default()
        })?;
        let mut per_second: HashMap<Timestamp, usize> = HashMap::new();
        for e in &changes {
            *per_second.entry(e.meta.at).or_default() += 1;
        }
        let mut out: HashMap<String, Closing> = HashMap::new();
        // Newest first: the first closing seen per item is the latest.
        for e in &changes {
            let Some((id, from, to)) = parse_status_change(&e.body) else { continue };
            if !to.is_closed() || out.contains_key(&id) {
                continue;
            }
            let note = e.body.split_once(" — ").map(|(_, n)| n.trim().to_string()).filter(|n| !n.is_empty());
            out.insert(
                id,
                Closing { at: e.meta.at, entry: e.meta.id.clone(), from, to, note, batch: per_second[&e.meta.at] },
            );
        }
        Ok(out)
    }

    pub fn story(&mut self, id: &str) -> Result<Story> {
        let item = self.item(id)?;
        let key = item.project.clone();
        let all = self.items(&ItemFilter { project: Some(key.clone()), include_closed: true, ..Default::default() })?;
        let by_id: HashMap<&str, &Item> = all.iter().map(|i| (i.meta.id.as_str(), i)).collect();
        let closings = self.closings(&key)?;
        let outcome_of = |i: &Item| -> Outcome {
            match closings.get(&i.meta.id).filter(|_| i.meta.status.is_closed()) {
                Some(c) => {
                    let (class, why) = classify(i.meta.status, c.note.as_deref().unwrap_or(""));
                    Outcome { class, glyph: glyph(class), why, text: c.note.clone(), at: Some(c.at), batch: c.batch }
                }
                None if i.meta.status.is_closed() => {
                    let (class, why) = classify(i.meta.status, "");
                    Outcome { class, glyph: glyph(class), why, text: None, at: None, batch: 0 }
                }
                None => Outcome { class: "open", glyph: "●", why: None, text: None, at: None, batch: 0 },
            }
        };
        let outcome = outcome_of(&item);

        let mut led_to: Vec<LedTo> = vec![];
        let title = |id: &str| by_id.get(id).map(|i| i.meta.title.clone());
        for l in &item.meta.links {
            if l.rel != "applies-to" {
                led_to.push(LedTo { id: l.to.clone(), title: title(&l.to), how: l.rel.clone() });
            }
        }
        for other in &all {
            for l in
                other.meta.links.iter().filter(|l| l.to == item.meta.id && l.rel != "applies-to" && l.rel != "about")
            {
                led_to.push(LedTo {
                    id: other.meta.id.clone(),
                    title: Some(other.meta.title.clone()),
                    how: format!("← {}", l.rel),
                });
            }
        }
        if let Some(note) = &outcome.text {
            for (mid, how) in mentions(note) {
                if mid != item.meta.id && !led_to.iter().any(|l| l.id == mid) {
                    led_to.push(LedTo {
                        title: title(&mid),
                        id: mid,
                        how: if how.is_empty() { "mentioned".into() } else { how },
                    });
                }
            }
        }

        let mut groups: Vec<Rollup> = vec![];
        for c in all.iter().filter(|c| c.meta.parent.as_deref() == Some(item.meta.id.as_str())) {
            let o = outcome_of(c);
            let g = match groups.iter_mut().position(|g| g.class == o.class) {
                Some(p) => &mut groups[p],
                None => {
                    groups.push(Rollup { class: o.class, glyph: o.glyph, count: 0, example: None, why: vec![] });
                    groups.last_mut().unwrap()
                }
            };
            g.count += 1;
            if g.example.is_none() {
                g.example = o.text.clone().map(|t| format!("{}: {t}", c.meta.id));
            }
            if let Some(w) = o.why {
                match g.why.iter_mut().find(|(x, _)| *x == w) {
                    Some(x) => x.1 += 1,
                    None => g.why.push((w, 1)),
                }
            }
        }
        let order = |c: &str| ["open", "done", "merged", "partial", "dead"].iter().position(|x| *x == c).unwrap_or(9);
        groups.sort_by_key(|g| order(g.class));

        let latest = if item.meta.status.is_closed() {
            None
        } else {
            self.entries(&EntryFilter {
                item: Some(item.meta.id.clone()),
                kinds: vec![EntryKind::Progress, EntryKind::Finding, EntryKind::Blocker, EntryKind::Handoff],
                limit: Some(1),
                ..Default::default()
            })?
            .into_iter()
            .next()
        };
        let asked = self
            .asked(&key)?
            .into_iter()
            .filter(|a| a.about.as_deref() == Some(item.meta.id.as_str()) || a.id == item.meta.id)
            .collect();
        Ok(Story { goal: goal_of(&item, &by_id), outcome, led_to, children: groups, latest, asked })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_closing_notes() {
        assert_eq!(classify(Status::Done, "Verified on the full suite"), ("done", None));
        assert_eq!(classify(Status::Done, "Combined into QR-19: all lanes merged"), ("merged", Some("combined")));
        assert_eq!(classify(Status::Dropped, "Duplicate of QR-95"), ("merged", Some("duplicate")));
        assert_eq!(classify(Status::Dropped, "Obsolete for the corpora"), ("dead", Some("obsolete")));
        assert_eq!(classify(Status::Dropped, ""), ("dead", Some("dropped")));
        assert_eq!(classify(Status::Inconclusive, "noisy"), ("partial", Some("inconclusive")));
        assert_eq!(classify(Status::Superseded, ""), ("merged", Some("superseded")));
    }

    #[test]
    fn finds_mentions_with_their_verbs() {
        let m = mentions("Combined into QR-19: all lanes merged (follows QR-45). See qr-3 and QR-19 again.");
        assert_eq!(
            m,
            vec![("QR-19".to_string(), "combined into".to_string()), ("QR-45".to_string(), "follows".to_string())]
        );
    }
}
