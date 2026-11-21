//! Asking the agents: the human asks a question about an item (from the web UI or `t9 ask`); the next agent
//! session in the project sees it (the brief, or the message hook if a session is running), answers from evidence
//! with `t9 answer`, and the answer is recorded on the question and on the item it is about.
//!
//! A question for the agents is an ordinary `question` item tagged `for-agents`, linked `about:ID`.

use std::collections::HashMap;

use serde::Serialize;

use crate::decisions::DecisionInput;
use crate::index::{EntryFilter, ItemFilter};
use crate::model::*;
use crate::store::{Caller, LogInput, NewItem, Store};
use crate::{Error, ItemPatch, Result};

pub const FOR_AGENTS: &str = "for-agents";
const ANSWER_PREFIX: &str = "Answer: ";

pub fn is_for_agents(i: &Item) -> bool {
    i.meta.kind == Kind::Question && i.meta.tags.iter().any(|t| t == FOR_AGENTS)
}

/// The item a question for the agents is about.
pub fn about_of(i: &Item) -> Option<&str> {
    i.meta.links.iter().find(|l| l.rel == "about").map(|l| l.to.as_str())
}

/// A question asked of the agents, with its answer once there is one.
#[derive(Debug, Clone, Serialize)]
pub struct Asked {
    pub id: String,
    pub question: String,
    pub detail: String,
    pub about: Option<String>,
    pub about_title: Option<String>,
    pub status: Status,
    pub asked_at: Timestamp,
    pub asked_by: String,
    pub answer: Option<String>,
    pub answered_by: Option<String>,
    pub answered_at: Option<Timestamp>,
}

impl Store {
    /// Records a question for the agents about an item.
    pub fn ask_agents(&mut self, caller: &Caller, about: &str, text: &str) -> Result<Item> {
        let about = self.item(about)?;
        let text = text.trim();
        if text.is_empty() {
            return Err(Error::Invalid("the question is empty".into()));
        }
        let first = text.lines().next().unwrap_or(text).trim();
        let title: String = if first.chars().count() > 140 {
            format!("{}…", first.chars().take(139).collect::<String>().trim_end())
        } else {
            first.to_string()
        };
        let body = if text.lines().count() > 1 || title != first { text.to_string() } else { String::new() };
        let caller = Caller { project: Some(about.project.clone()), ..caller.clone() };
        self.add_item(
            &caller,
            NewItem {
                kind: Some(Kind::Question),
                title,
                body,
                tags: vec![FOR_AGENTS.into()],
                areas: about.meta.areas.clone(),
                links: vec![Link { rel: "about".into(), to: about.meta.id.clone() }],
                ..Default::default()
            },
        )
    }

    /// Answers a question: records the answer on it (and, for a question about an item, on that item), closes it,
    /// and optionally records the answer as a decision. Returns the closed question and the decision id.
    pub fn answer(&mut self, caller: &Caller, id: &str, text: &str, decide: bool) -> Result<(Item, Option<String>)> {
        let answer = text.trim().to_string();
        if answer.is_empty() {
            return Err(Error::Invalid("the answer is empty".into()));
        }
        let q = self.item(id)?;
        if q.meta.kind != Kind::Question {
            return Err(Error::Invalid(format!("{} is a {}, not a question", q.meta.id, q.meta.kind)));
        }
        let caller = Caller { project: Some(q.project.clone()), ..caller.clone() };
        self.log(
            &caller,
            LogInput {
                kind: EntryKind::Note,
                body: format!("{ANSWER_PREFIX}{answer}"),
                item: Some(q.meta.id.clone()),
                refs: vec![],
            },
        )?;
        if let Some(about) = about_of(&q).map(String::from) {
            self.log(
                &caller,
                LogInput {
                    kind: EntryKind::Finding,
                    body: format!("{} ({}): {answer}", q.meta.id, q.meta.title),
                    item: Some(about),
                    refs: vec![],
                },
            )?;
        }
        let mut decision = None;
        if decide {
            let title: String = answer.lines().next().unwrap_or(&answer).chars().take(140).collect();
            let mut applies_to: Vec<String> = q.meta.parent.iter().cloned().collect();
            applies_to.extend(about_of(&q).map(String::from));
            let d = self.decide(
                &caller,
                DecisionInput {
                    title,
                    because: Some(format!("Answer to {} ({})", q.meta.id, q.meta.title)),
                    body: if answer.contains('\n') { answer.clone() } else { String::new() },
                    areas: q.meta.areas.clone(),
                    applies_to,
                    ..Default::default()
                },
            )?;
            decision = Some(d.meta.id.clone());
        }
        let note = match &decision {
            Some(d) => format!("answered; recorded as {d}"),
            None => "answered".to_string(),
        };
        let closed = self.update_item(
            &caller,
            &q.meta.id,
            ItemPatch { status: Some(Status::Done), note: Some(note), ..Default::default() },
        )?;
        Ok((closed, decision))
    }

    /// Questions asked of the agents in a project, newest first, with their answers.
    pub fn asked(&mut self, key: &str) -> Result<Vec<Asked>> {
        self.sync_project(key)?;
        let items = self.items(&ItemFilter {
            project: Some(key.into()),
            kinds: vec![Kind::Question],
            include_closed: true,
            ..Default::default()
        })?;
        let qs: Vec<&Item> = items.iter().filter(|i| is_for_agents(i)).collect();
        if qs.is_empty() {
            return Ok(vec![]);
        }
        let mut answers: HashMap<String, Entry> = HashMap::new();
        for e in self.entries(&EntryFilter {
            project: Some(key.into()),
            kinds: vec![EntryKind::Note],
            limit: Some(20_000),
            ..Default::default()
        })? {
            if let Some(i) = e.meta.item.clone()
                && e.body.starts_with(ANSWER_PREFIX)
            {
                answers.entry(i).or_insert(e);
            }
        }
        let mut out = vec![];
        for q in qs {
            let about = about_of(q).map(String::from);
            let about_title = match &about {
                Some(a) => self.item(a).ok().map(|i| i.meta.title),
                None => None,
            };
            let a = answers.get(&q.meta.id);
            out.push(Asked {
                id: q.meta.id.clone(),
                question: q.meta.title.clone(),
                detail: q.body.trim().to_string(),
                about,
                about_title,
                status: q.meta.status,
                asked_at: q.meta.created,
                asked_by: q.meta.created_by.clone(),
                answer: a.map(|e| e.body.trim_start_matches(ANSWER_PREFIX).to_string()),
                answered_by: a.map(|e| e.meta.agent.clone()),
                answered_at: a.map(|e| e.meta.at),
            });
        }
        out.sort_by_key(|a| std::cmp::Reverse(a.asked_at));
        Ok(out)
    }
}
