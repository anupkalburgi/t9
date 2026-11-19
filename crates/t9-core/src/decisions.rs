//! The decision log: recording decisions and rejections, and recalling them when they matter.
//!
//! A decision is an item of kind `decision` with status `accepted`, `proposed`, `rejected` or `superseded`. Its
//! body follows a small ADR shape (`## Why`, `## Rejected alternatives`, `## Notes`). Decisions carry `areas`
//! (topics such as `testing`) and `applies-to` links that scope them to part of the plan. Decisions in the
//! `GLOBAL` project apply to every project.
//!
//! Recall happens in three places, all built on [`Store::applicable_decisions`] / [`Store::check`]:
//! the brief (decisions that apply to the current focus), `t9 focus`, and the per-message hook
//! ([`prompt_context`]), which surfaces earlier decisions as soon as the user mentions their topic.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs;

use chrono::Local;
use serde::Serialize;

use crate::index::{self, ItemFilter};
use crate::model::*;
use crate::store::{Caller, GLOBAL, ItemPatch, LogInput, NewItem, Store, normalize_areas};
use crate::{Error, Result, frontmatter};

#[derive(Debug, Clone)]
pub struct DecisionInput {
    pub title: String,
    /// Free-form notes (become `## Notes`, or the whole body if nothing else is given).
    pub body: String,
    pub because: Option<String>,
    /// Alternatives considered and rejected, each "option: why not".
    pub rejected: Vec<String>,
    pub areas: Vec<String>,
    /// Items the decision applies to (it then applies to everything under them).
    pub applies_to: Vec<String>,
    pub supersedes: Vec<String>,
    pub parent: Option<String>,
    /// `accepted` (default), `proposed` or `rejected`.
    pub status: Status,
    /// Record in the global log (applies to every project).
    pub global: bool,
}

impl Default for DecisionInput {
    fn default() -> Self {
        DecisionInput {
            title: String::new(),
            body: String::new(),
            because: None,
            rejected: vec![],
            areas: vec![],
            applies_to: vec![],
            supersedes: vec![],
            parent: None,
            status: Status::Accepted,
            global: false,
        }
    }
}

/// A decision recalled for some context, with why it was picked.
#[derive(Debug, Clone, Serialize)]
pub struct Relevant {
    #[serde(flatten)]
    pub item: Item,
    pub reasons: Vec<String>,
    #[serde(skip)]
    strength: usize,
}

// ---------------------------------------------------------------- text helpers

const STOP: &[&str] = &[
    "the",
    "and",
    "for",
    "are",
    "but",
    "not",
    "you",
    "all",
    "any",
    "can",
    "had",
    "her",
    "was",
    "one",
    "our",
    "out",
    "has",
    "have",
    "him",
    "his",
    "how",
    "its",
    "may",
    "new",
    "now",
    "old",
    "see",
    "two",
    "way",
    "who",
    "did",
    "get",
    "got",
    "let",
    "lets",
    "say",
    "she",
    "too",
    "use",
    "used",
    "using",
    "this",
    "that",
    "with",
    "from",
    "what",
    "when",
    "where",
    "which",
    "while",
    "will",
    "would",
    "should",
    "could",
    "there",
    "their",
    "them",
    "then",
    "than",
    "these",
    "those",
    "they",
    "into",
    "onto",
    "about",
    "also",
    "just",
    "like",
    "make",
    "made",
    "more",
    "most",
    "much",
    "need",
    "needs",
    "only",
    "over",
    "some",
    "such",
    "sure",
    "take",
    "very",
    "want",
    "were",
    "been",
    "being",
    "does",
    "doing",
    "done",
    "each",
    "even",
    "here",
    "same",
    "want",
    "well",
    "your",
    "yours",
    "yes",
    "okay",
    "please",
    "thanks",
    "think",
    "know",
    "go",
    "going",
    "lets",
    "let's",
    "dont",
    "don't",
    "does",
    "really",
    "thing",
    "things",
    "something",
    "anything",
    "everything",
    "again",
    "still",
    "because",
    "why",
    "do",
    "is",
    "it",
    "in",
    "on",
    "of",
    "to",
    "we",
    "us",
    "be",
    "or",
    "an",
    "as",
    "at",
    "by",
    "if",
    "so",
    "no",
    "me",
    "my",
    "am",
    "up",
    "ok",
    "hi",
    "oh",
    "he",
    "go",
    "im",
    "ive",
    "id",
    "ll",
    "re",
    "ve",
    "s",
    "t",
];

/// Content words of `text`: lower-case, 2+ characters (`uv`, `ci` matter), not stop words, unique, at most 16.
pub fn keywords(text: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for w in text.split(|c: char| !c.is_alphanumeric()) {
        let w = w.to_lowercase();
        if w.chars().count() < 2 || w.chars().all(|c| c.is_ascii_digit()) || STOP.contains(&w.as_str()) {
            continue;
        }
        if !out.contains(&w) {
            out.push(w);
        }
        if out.len() == 16 {
            break;
        }
    }
    out
}

fn tokens(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// Keywords found in `toks`; words of 4+ characters also match by prefix (`decimal` ~ `decimals`).
fn hits(kws: &[String], toks: &HashSet<String>) -> Vec<String> {
    kws.iter()
        .filter(|k| {
            toks.contains(*k)
                || (k.len() >= 4
                    && toks.iter().any(|t| t.len() >= 4 && (t.starts_with(k.as_str()) || k.starts_with(t.as_str()))))
        })
        .cloned()
        .collect()
}

/// Does the user's message read like it makes or rules out a choice?
pub fn looks_like_decision(text: &str) -> bool {
    const PHRASES: &[&str] = &[
        "let's go with",
        "lets go with",
        "go with ",
        "we'll use",
        "we will use",
        "let's use",
        "lets use",
        "instead of",
        "rather than",
        "don't use",
        "do not use",
        "don't do",
        "do not do",
        "never ",
        "always ",
        "stick with",
        "switch to",
        "switch the",
        "switch from",
        "replace ",
        "move to",
        "go back to",
        "revert to",
        "decided",
        "decision",
        "we chose",
        "i chose",
        "i prefer",
        "we prefer",
        "not going to",
        "no, ",
        "reject",
        "get rid of",
        "from now on",
        "going forward",
        "drop the",
        "let's not",
        "lets not",
        "we should not",
        "we shouldn't",
        "should always",
        "should never",
    ];
    let t = format!(" {} ", text.to_lowercase().replace('’', "'"));
    PHRASES.iter().any(|p| t.contains(p))
}

/// The ADR-shaped body of a decision.
pub fn decision_body(because: Option<&str>, rejected: &[String], notes: &str) -> String {
    let mut out = String::new();
    if let Some(b) = because.map(str::trim).filter(|b| !b.is_empty()) {
        let _ = write!(out, "## Why\n\n{b}\n\n");
    }
    let rejected: Vec<&str> = rejected.iter().map(|r| r.trim()).filter(|r| !r.is_empty()).collect();
    if !rejected.is_empty() {
        out.push_str("## Rejected alternatives\n\n");
        for r in rejected {
            let _ = writeln!(out, "- {r}");
        }
        out.push('\n');
    }
    let notes = notes.trim();
    if !notes.is_empty() {
        if !out.is_empty() {
            out.push_str("## Notes\n\n");
        }
        out.push_str(notes);
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// One line of reasoning for a decision: the first line under `## Why`, else the first line of the body.
pub fn why(item: &Item) -> String {
    let mut in_why = false;
    let mut first_plain: Option<&str> = None;
    for line in item.body.lines() {
        let l = line.trim();
        if l.starts_with("## ") {
            in_why = l.eq_ignore_ascii_case("## why");
            continue;
        }
        if l.is_empty() {
            continue;
        }
        if in_why {
            return clip(l, 140);
        }
        first_plain.get_or_insert(l);
    }
    first_plain.map(|l| clip(l, 140)).unwrap_or_default()
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>().trim_end())
    }
}

/// `BIL-12 [accepted] 2026-11-10 Use FTS5 for search, not tantivy (search)`.
pub fn decision_line(i: &Item) -> String {
    let date = i.meta.created.with_timezone(&Local).format("%Y-%m-%d");
    let mut s = format!("{} [{}] {date} {}", i.meta.id, i.meta.status, i.meta.title);
    if !i.meta.areas.is_empty() {
        let _ = write!(s, " ({})", i.meta.areas.join(", "));
    }
    s
}

// ---------------------------------------------------------------- store operations

const RECALL_KINDS: &[Kind] = &[Kind::Decision, Kind::Idea];

impl Store {
    /// The `GLOBAL` project, created on first use (no roots; it never resolves from a directory).
    pub fn ensure_global(&mut self) -> Result<Project> {
        if let Ok(p) = self.project(GLOBAL) {
            return Ok(p);
        }
        let dir = self.env.projects_dir().join("_global");
        for sub in ["items", "journal", "sessions"] {
            fs::create_dir_all(dir.join(sub)).map_err(|e| Error::io(&dir, e))?;
        }
        let meta = ProjectMeta {
            key: GLOBAL.into(),
            name: "Global".into(),
            status: "active".into(),
            roots: vec![],
            remotes: vec![],
            created: now(),
            extra: Default::default(),
        };
        let body = "# Global\n\nDecisions and rules that apply to every project. They appear in every brief.\n";
        self.put(&dir.join("project.md"), &frontmatter::render(&meta, body)?)?;
        self.sync_projects()?;
        self.project(GLOBAL)
    }

    /// Records a decision (or a rejection, or a proposal). Supersedes earlier decisions and links both ways.
    pub fn decide(&mut self, caller: &Caller, d: DecisionInput) -> Result<Item> {
        let mut caller = caller.clone();
        if d.global {
            self.ensure_global()?;
            caller.project = Some(GLOBAL.into());
        }
        let olds: Vec<Item> = d.supersedes.iter().map(|id| self.item(id)).collect::<Result<_>>()?;
        let targets: Vec<Item> = d.applies_to.iter().map(|id| self.item(id)).collect::<Result<_>>()?;
        // From an unlinked directory, record it where the items it refers to live.
        if caller.project.is_none()
            && d.parent.is_none()
            && self.resolve(&caller)?.project.is_none()
            && let Some(i) = olds.first().or(targets.first())
        {
            caller.project = Some(i.project.clone());
        }
        // A change of mind stays in the same topics unless new ones are given.
        let mut areas = d.areas.clone();
        if areas.is_empty() {
            for a in olds.iter().flat_map(|o| o.meta.areas.iter()) {
                if !areas.contains(a) {
                    areas.push(a.clone());
                }
            }
        }
        let mut links: Vec<Link> =
            olds.iter().map(|o| Link { rel: "supersedes".into(), to: o.meta.id.clone() }).collect();
        links.extend(targets.iter().map(|t| Link { rel: "applies-to".into(), to: t.meta.id.clone() }));
        let item = self.add_item(
            &caller,
            NewItem {
                kind: Some(Kind::Decision),
                title: d.title.clone(),
                body: decision_body(d.because.as_deref(), &d.rejected, &d.body),
                parent: d.parent.clone(),
                areas,
                links,
                status: Some(d.status),
                ..Default::default()
            },
        )?;
        for o in &olds {
            self.update_item(
                &caller,
                &o.meta.id,
                ItemPatch {
                    status: Some(Status::Superseded),
                    add_links: vec![Link { rel: "superseded-by".into(), to: item.meta.id.clone() }],
                    note: Some(format!("superseded by {}", item.meta.id)),
                    ..Default::default()
                },
            )?;
        }
        let verb = match d.status {
            Status::Rejected => "Rejected",
            Status::Proposed => "Proposed",
            _ => "Decided",
        };
        let mut text = format!("{verb} {}: {}", item.meta.id, item.meta.title);
        if !olds.is_empty() {
            let ids: Vec<&str> = olds.iter().map(|o| o.meta.id.as_str()).collect();
            let _ = write!(text, " (supersedes {})", ids.join(", "));
        }
        self.log(
            &caller,
            LogInput { kind: EntryKind::Decision, body: text, item: Some(item.meta.id.clone()), refs: vec![] },
        )?;
        Ok(item)
    }

    /// A project's decisions, newest first. Superseded ones only with `include_superseded`.
    pub fn decision_log(&mut self, project: &str, areas: &[String], include_superseded: bool) -> Result<Vec<Item>> {
        self.sync_project(project)?;
        let statuses =
            if include_superseded { vec![] } else { vec![Status::Accepted, Status::Proposed, Status::Rejected] };
        let mut v = self.items(&ItemFilter {
            project: Some(project.into()),
            kinds: vec![Kind::Decision],
            statuses,
            include_closed: true,
            ..Default::default()
        })?;
        let areas = normalize_areas(areas);
        if !areas.is_empty() {
            v.retain(|i| i.meta.areas.iter().any(|a| areas.contains(a)));
        }
        v.sort_by(|a, b| b.meta.created.cmp(&a.meta.created).then(b.meta.id.cmp(&a.meta.id)));
        Ok(v)
    }

    /// Projects searched for recall: the given one plus GLOBAL (if it exists), synced.
    fn recall_projects(&mut self, project: Option<&str>) -> Result<Vec<String>> {
        let mut v = vec![];
        if let Some(p) = project {
            self.sync_project(p)?;
            v.push(p.to_string());
        }
        if project != Some(GLOBAL) && self.project(GLOBAL).is_ok() {
            self.sync_project(GLOBAL)?;
            v.push(GLOBAL.to_string());
        }
        Ok(v)
    }

    /// Decisions and ideas whose text matches `text`. `strict` requires two matching words (or a long word in
    /// the title) and skips superseded decisions and parked ideas, for unprompted recall.
    fn text_matches(&mut self, text: &str, projects: &[String], strict: bool, limit: usize) -> Result<Vec<Relevant>> {
        let kws = keywords(text);
        if kws.is_empty() {
            return Ok(vec![]);
        }
        let candidates = index::search_items(&self.conn, &index::fts_query_any(&kws), RECALL_KINDS, projects, 40)?;
        let mut out = vec![];
        for c in candidates {
            let st = c.meta.status;
            if strict && (st == Status::Superseded || (c.meta.kind == Kind::Idea && st == Status::Open)) {
                continue;
            }
            let all = hits(&kws, &tokens(&format!("{} {} {}", c.meta.title, c.meta.areas.join(" "), c.body)));
            let title = hits(&kws, &tokens(&c.meta.title));
            let keep =
                if strict { all.len() >= 2 || title.iter().any(|k| k.chars().count() >= 5) } else { !all.is_empty() };
            if !keep {
                continue;
            }
            let strength =
                all.len() * 2 + title.len() * 3 + usize::from(matches!(st, Status::Accepted | Status::Rejected));
            out.push(Relevant { reasons: vec![format!("mentions {}", all.join(", "))], strength, item: c });
        }
        // Decisions in force first; superseded ones last, as history.
        out.sort_by(|a, b| {
            let old = |r: &Relevant| r.item.meta.status == Status::Superseded;
            old(a).cmp(&old(b)).then(b.strength.cmp(&a.strength)).then(b.item.meta.updated.cmp(&a.item.meta.updated))
        });
        out.truncate(limit);
        Ok(out)
    }

    /// `t9 check`: has anything been decided, rejected or parked about this? Searches all statuses.
    pub fn check(&mut self, text: &str, project: Option<&str>, limit: usize) -> Result<Vec<Relevant>> {
        let projects = self.recall_projects(project)?;
        self.text_matches(text, &projects, false, limit)
    }

    /// Decisions that bear on a user's message (strict, for the per-message hook).
    pub fn prompt_decisions(&mut self, text: &str, project: Option<&str>, limit: usize) -> Result<Vec<Relevant>> {
        let projects = self.recall_projects(project)?;
        self.text_matches(text, &projects, true, limit)
    }

    /// Decisions that apply to `item`: scoped to it or an ancestor (`applies-to` or tree), sharing an area or
    /// tag with it or its ancestors, or matching its text. Includes rejections ("ruled out") and GLOBAL ones.
    pub fn applicable_decisions(&mut self, item: &Item, limit: usize) -> Result<Vec<Relevant>> {
        let mut chain: Vec<Item> = vec![item.clone()];
        while let Some(p) = chain.last().and_then(|i| i.meta.parent.clone()) {
            match index::item(&self.conn, &p)? {
                Some(parent) if !chain.iter().any(|c| c.meta.id == parent.meta.id) => chain.push(parent),
                _ => break,
            }
        }
        let chain_ids: Vec<&str> = chain.iter().map(|c| c.meta.id.as_str()).collect();
        let topics: HashSet<String> =
            chain.iter().flat_map(|c| c.meta.areas.iter().chain(c.meta.tags.iter())).map(|t| slugify(t)).collect();
        let projects = self.recall_projects(Some(&item.project))?;
        let mut found: Vec<Relevant> = vec![];
        for p in &projects {
            let cands = self.items(&ItemFilter {
                project: Some(p.clone()),
                kinds: RECALL_KINDS.to_vec(),
                statuses: vec![Status::Accepted, Status::Proposed, Status::Rejected],
                include_closed: true,
                ..Default::default()
            })?;
            for c in cands {
                if c.meta.id == item.meta.id {
                    continue;
                }
                let mut reasons = vec![];
                let mut strength = 0;
                for l in c.meta.links.iter().filter(|l| l.rel == "applies-to") {
                    if chain_ids.contains(&l.to.as_str()) {
                        reasons.push(format!("applies to {}", l.to));
                        strength += 100;
                    }
                }
                if let Some(parent) = &c.meta.parent
                    && chain_ids.contains(&parent.as_str())
                {
                    reasons.push(format!("under {parent}"));
                    strength += 80;
                }
                let shared: Vec<&String> = c.meta.areas.iter().filter(|a| topics.contains(*a)).collect();
                if !shared.is_empty() {
                    reasons.push(format!("area {}", shared.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
                    strength += 50;
                }
                if strength > 0 {
                    found.push(Relevant { item: c, reasons, strength });
                }
            }
        }
        let text: String = format!("{} {}", item.meta.title, item.body.chars().take(400).collect::<String>());
        for m in self.text_matches(&text, &projects, true, 10)? {
            match found.iter_mut().find(|f| f.item.meta.id == m.item.meta.id) {
                Some(f) => {
                    f.reasons.extend(m.reasons);
                    f.strength += m.strength;
                }
                None => found.push(m),
            }
        }
        found.retain(|f| f.item.meta.id != item.meta.id);
        found.sort_by(|a, b| b.strength.cmp(&a.strength).then(b.item.meta.updated.cmp(&a.item.meta.updated)));
        found.truncate(limit);
        Ok(found)
    }
}

/// What the per-message hook adds to the agent's context: earlier decisions on the topic of `prompt`, and a
/// reminder to record a decision if the message reads like one. `None` when there is nothing worth saying.
pub fn prompt_context(store: &mut Store, caller: &Caller, prompt: &str) -> Result<Option<String>> {
    if prompt.trim().chars().count() < 3 {
        return Ok(None);
    }
    let res = store.resolve(caller)?;
    let project = res.project.as_ref().map(|p| p.meta.key.clone());
    let found = store.prompt_decisions(prompt, project.as_deref(), 3)?;
    let mut out = String::new();
    if !found.is_empty() {
        out.push_str(
            "t9: earlier decisions that may bear on this message. Follow them; if the user is changing one, record \
             that with `t9 decide \"…\" -s ID --because \"…\"`.\n",
        );
        for f in &found {
            let w = why(&f.item);
            let _ = write!(out, "- {}", decision_line(&f.item));
            if !w.is_empty() {
                let _ = write!(out, " — {w}");
            }
            out.push('\n');
        }
    }
    if looks_like_decision(prompt) {
        out.push_str(
            "t9: if this message makes or rules out a choice, record it now: `t9 decide \"…\" --because \"…\"` or \
             `t9 reject \"…\" --because \"…\"` (add --area AREA, --for ID; --global for rules across projects).\n",
        );
    }
    Ok((!out.is_empty()).then_some(out))
}

/// The decision log as a Markdown document (ADR style), newest first.
pub fn render_log_md(project: &Project, items: &[Item]) -> String {
    let mut out = format!("# Decision log: {} ({})\n\n", project.meta.name, project.meta.key);
    if items.is_empty() {
        out.push_str("No decisions recorded yet.\n");
        return out;
    }
    out.push_str("| Id | Date | Status | Areas | Decision |\n|---|---|---|---|---|\n");
    for i in items {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} |",
            i.meta.id,
            i.meta.created.with_timezone(&Local).format("%Y-%m-%d"),
            i.meta.status,
            i.meta.areas.join(", "),
            i.meta.title.replace('|', "\\|")
        );
    }
    for i in items {
        let _ = write!(
            out,
            "\n## {}: {}\n\n{} · {}",
            i.meta.id,
            i.meta.title,
            i.meta.status,
            i.meta.created.with_timezone(&Local).format("%Y-%m-%d")
        );
        if !i.meta.areas.is_empty() {
            let _ = write!(out, " · areas: {}", i.meta.areas.join(", "));
        }
        for l in &i.meta.links {
            let _ = write!(out, " · {} {}", l.rel, l.to);
        }
        out.push_str("\n\n");
        if !i.body.trim().is_empty() {
            out.push_str(i.body.trim_end());
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_extraction() {
        assert_eq!(keywords("Should we use tantivy for search?"), vec!["tantivy", "search"]);
        assert!(keywords("ok so do it").is_empty());
    }

    #[test]
    fn decision_language() {
        assert!(looks_like_decision("No, let's go with FTS5 instead"));
        assert!(looks_like_decision("We should never deploy on Fridays"));
        assert!(looks_like_decision("Should we switch the search engine to tantivy?"));
        assert!(!looks_like_decision("Can you run the tests?"));
    }

    #[test]
    fn body_shape() {
        let b = decision_body(Some("fast enough"), &["tantivy: extra index".into()], "see bench");
        assert_eq!(
            b,
            "## Why\n\nfast enough\n\n## Rejected alternatives\n\n- tantivy: extra index\n\n## Notes\n\nsee bench"
        );
        assert_eq!(decision_body(None, &[], "just notes"), "just notes");
    }

    #[test]
    fn prefix_hits() {
        let t = tokens("Exact DECIMAL arithmetic over temp tables");
        assert_eq!(hits(&["decimals".into(), "table".into(), "zzz".into()], &t), vec!["decimals", "table"]);
    }
}
