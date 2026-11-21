//! What needs attention, ranked once and shared by the brief, `t9 pending`, the agent hooks and the web UI.
//!
//! Freshness comes from commitments, not raw age: due dates the human set, what an item waits on, active and
//! blocked work, questions and proposals waiting on the human, closures without an outcome, and the
//! going-in-circles signals. Old open backlog is only counted.

use std::collections::{HashMap, HashSet};

use chrono::{Duration, NaiveDate};
use serde::Serialize;

use crate::Result;
use crate::index::ItemFilter;
use crate::insights::Signal;
use crate::model::*;
use crate::store::Store;

/// Days before an untouched item is flagged.
pub const STALE_ACTIVE_DAYS: i64 = 3;
pub const STALE_BLOCKED_DAYS: i64 = 5;
pub const QUESTION_WAIT_DAYS: i64 = 2;
pub const PROPOSAL_WAIT_DAYS: i64 = 3;
pub const DUE_SOON_DAYS: i64 = 2;
pub const UNEXPLAINED_DAYS: i64 = 7;
pub const BACKLOG_DAYS: i64 = 30;

#[derive(Debug, Clone, Serialize)]
pub struct Attention {
    pub id: String,
    pub kind: Kind,
    pub status: Status,
    pub title: String,
    /// `overdue`, `due-soon`, `ripe`, `loop`, `waiting`, `stale`, `unexplained`.
    pub reason: &'static str,
    /// 1 (worth a look) to 5 (act now).
    pub severity: u8,
    /// Days overdue, until due, idle, or waiting, depending on the reason.
    pub days: i64,
    /// One short phrase: "overdue 3d (due Nov 17)", "ripe: QR-126 closed", "idle 6d".
    pub detail: String,
    /// What to do about it, for agents.
    pub hint: &'static str,
    pub due: Option<NaiveDate>,
    /// Days since created / since last touched.
    pub age: i64,
    pub idle: i64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct AttentionReport {
    pub items: Vec<Attention>,
    /// Open items with no priority untouched for `BACKLOG_DAYS`+: counted, never listed.
    pub backlog: usize,
}

impl AttentionReport {
    /// Reasons worth interrupting for: shown to the agent mid-session, told to the user.
    pub fn reminders(&self) -> impl Iterator<Item = &Attention> {
        self.items.iter().filter(|a| matches!(a.reason, "overdue" | "due-soon" | "ripe" | "waiting"))
    }
    pub fn get(&self, id: &str) -> Option<&Attention> {
        self.items.iter().find(|a| a.id == id)
    }
}

/// `today`, `tomorrow`, `mon`…`sun` (the next one, never today), `2026-12-01`, or a delay: `3d`, `2w`, `1m`.
pub fn parse_when(s: &str, today: NaiveDate) -> std::result::Result<NaiveDate, String> {
    let s = s.trim().to_ascii_lowercase();
    if let Ok(d) = NaiveDate::parse_from_str(&s, "%Y-%m-%d") {
        return Ok(d);
    }
    match s.as_str() {
        "today" => return Ok(today),
        "tomorrow" => return Ok(today + Duration::days(1)),
        _ => {}
    }
    let days = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
    if let Some(n) = days.iter().position(|d| s.starts_with(d)) {
        let now = chrono::Datelike::weekday(&today).num_days_from_monday() as i64;
        let ahead = (n as i64 - now).rem_euclid(7);
        return Ok(today + Duration::days(if ahead == 0 { 7 } else { ahead }));
    }
    let bad = || format!("'{s}' is not a date: use 2026-12-01, today, tomorrow, mon…sun, or a delay like 3d, 2w, 1m");
    let (n, unit) = s.split_at(s.len().saturating_sub(1));
    let n: i64 = n.parse().map_err(|_| bad())?;
    let d = match unit {
        "d" => n,
        "w" => n * 7,
        "m" => n * 30,
        _ => return Err(bad()),
    };
    Ok(today + Duration::days(d))
}

fn short_date(d: NaiveDate) -> String {
    d.format("%b %-d").to_string()
}

fn days_phrase(n: i64) -> String {
    match n {
        0 => "today".into(),
        1 => "1 day".into(),
        n => format!("{n} days"),
    }
}

impl Store {
    /// What needs attention in a project. `signals` (from `insights`) adds the going-in-circles items; pass
    /// `None` where speed matters (the message hook).
    pub fn attention(&mut self, key: &str, signals: Option<&[Signal]>) -> Result<AttentionReport> {
        self.sync_project(key)?;
        let items =
            self.items(&ItemFilter { project: Some(key.into()), include_closed: true, ..Default::default() })?;
        let by_id: HashMap<&str, &Item> = items.iter().map(|i| (i.meta.id.as_str(), i)).collect();
        let rows = crate::index::last_entry_per_item(&self.conn, key)?;
        let last_id: HashMap<String, String> = rows.iter().map(|(i, _, e)| (i.clone(), e.clone())).collect();
        let last: HashMap<String, Timestamp> = rows
            .into_iter()
            .filter_map(|(id, at, _)| chrono::DateTime::parse_from_rfc3339(&at).ok().map(|d| (id, d.to_utc())))
            .collect();
        let touched = |i: &Item| last.get(&i.meta.id).copied().map_or(i.meta.updated, |t| t.max(i.meta.updated));
        let closings = self.closings(key)?;
        let mut checked_in: HashMap<String, Timestamp> = HashMap::new();
        for e in self.entries(&crate::index::EntryFilter {
            project: Some(key.into()),
            kinds: vec![EntryKind::Checkin],
            limit: Some(20_000),
            ..Default::default()
        })? {
            if let Some(i) = e.meta.item {
                checked_in.entry(i).or_insert(e.meta.at);
            }
        }
        let now = now();
        let today = chrono::Local::now().date_naive();
        let days_since = |t: Timestamp| (now - t).num_days().max(0);
        let structural = |k: Kind| matches!(k, Kind::Plan | Kind::Phase | Kind::Feature);

        let mut out: Vec<Attention> = vec![];
        let mut backlog = 0;
        let mk =
            |i: &Item, reason: &'static str, severity: u8, days: i64, detail: String, hint: &'static str| Attention {
                id: i.meta.id.clone(),
                kind: i.meta.kind,
                status: i.meta.status,
                title: i.meta.title.clone(),
                reason,
                severity,
                days,
                detail,
                hint,
                due: i.meta.due,
                age: days_since(i.meta.created),
                idle: days_since(touched(i)),
            };

        for i in &items {
            let open = !i.meta.status.is_closed() && i.meta.status != Status::Accepted;
            let idle = days_since(touched(i));
            if open && let Some(due) = i.meta.due {
                let left = (due - today).num_days();
                if left < 0 {
                    out.push(mk(
                        i,
                        "overdue",
                        5,
                        -left,
                        format!("overdue {} (due {})", days_phrase(-left), short_date(due)),
                        "finish it, or ask the user for a new date (`t9 update ID --due …`)",
                    ));
                    continue;
                }
                if left <= DUE_SOON_DAYS {
                    let when = if left == 0 {
                        "due today".to_string()
                    } else {
                        format!("due in {} ({})", days_phrase(left), short_date(due))
                    };
                    out.push(mk(
                        i,
                        "due-soon",
                        if left == 0 { 5 } else { 4 },
                        left,
                        when,
                        "plan it in, or tell the user it will slip",
                    ));
                    continue;
                }
            }
            if open {
                // Ripe: something it waits on closed after it was last touched.
                let ripe = i.meta.links.iter().filter(|l| l.rel == "blocked-by").find_map(|l| {
                    let c = closings.get(&l.to)?;
                    let target = by_id.get(l.to.as_str())?;
                    // Closed after this item was last touched: no newer entry on it, no later edit.
                    let after_entries = last_id.get(&i.meta.id).is_none_or(|e| c.entry > *e);
                    (target.meta.status.is_closed() && after_entries && c.at >= i.meta.updated)
                        .then(|| (l.to.clone(), c.at))
                });
                if let Some((t, at)) = ripe {
                    out.push(mk(
                        i,
                        "ripe",
                        4,
                        days_since(at),
                        format!(
                            "ripe: {t} closed {}",
                            if days_since(at) == 0 {
                                "today".into()
                            } else {
                                format!("{} ago", days_phrase(days_since(at)))
                            }
                        ),
                        "it was waiting on that: pick it up now, or tell the user it is unblocked",
                    ));
                    continue;
                }
            }
            if matches!(i.meta.status, Status::Failed | Status::Inconclusive)
                && let Some(d) = i.meta.retry_after
                && d <= today
                && checked_in.get(&i.meta.id).is_none_or(|t| t.with_timezone(&chrono::Local).date_naive() < d)
            {
                let w = i.meta.retry_when.as_deref().map(|w| format!("; retry when {w}")).unwrap_or_default();
                out.push(mk(
                    i,
                    "ripe",
                    3,
                    (today - d).num_days(),
                    format!("worth retrying since {}{w}", short_date(d)),
                    "check whether what made it fail has changed; propose the retry or `t9 still ID`",
                ));
                continue;
            }
            if i.meta.status == Status::Active && !structural(i.meta.kind) && idle >= STALE_ACTIVE_DAYS {
                out.push(mk(
                    i,
                    "stale",
                    2,
                    idle,
                    format!("active, idle {idle}d"),
                    "log progress, finish it, or `t9 still ID -n \"waiting on …\"`; drop it if it is no longer needed",
                ));
                continue;
            }
            if i.meta.status == Status::Blocked && idle >= STALE_BLOCKED_DAYS {
                out.push(mk(
                    i,
                    "stale",
                    2,
                    idle,
                    format!("blocked, idle {idle}d"),
                    "check whether it is still blocked",
                ));
                continue;
            }
            if i.meta.kind == Kind::Question && i.meta.status == Status::Open {
                let age = days_since(i.meta.created);
                if age >= QUESTION_WAIT_DAYS {
                    out.push(mk(
                        i,
                        "waiting",
                        3,
                        age,
                        format!("waiting on the user for {}", days_phrase(age)),
                        "ask the user (it is in `t9 web` too)",
                    ));
                    continue;
                }
            }
            if i.meta.kind == Kind::Decision && i.meta.status == Status::Proposed {
                let age = days_since(i.meta.created);
                if age >= PROPOSAL_WAIT_DAYS {
                    out.push(mk(
                        i,
                        "waiting",
                        3,
                        age,
                        format!("proposed {} ago, not accepted", days_phrase(age)),
                        "ask the user to accept or reject it",
                    ));
                    continue;
                }
            }
            if i.meta.status.is_closed()
                && let Some(c) = closings.get(&i.meta.id)
                && c.note.is_none()
                && days_since(c.at) <= UNEXPLAINED_DAYS
                && !matches!(i.meta.kind, Kind::Decision | Kind::Note)
            {
                out.push(mk(
                    i,
                    "unexplained",
                    1,
                    days_since(c.at),
                    format!("{} without an outcome", c.to),
                    "record what came out of it: `t9 log -i ID -k finding \"…\"`",
                ));
                continue;
            }
            if i.meta.status == Status::Open
                && i.meta.priority.is_none()
                && !matches!(i.meta.kind, Kind::Idea | Kind::Revisit | Kind::Resource | Kind::Note | Kind::Decision)
                && idle >= BACKLOG_DAYS
            {
                backlog += 1;
            }
        }

        if let Some(signals) = signals {
            let listed: HashSet<String> = out.iter().map(|a| a.id.clone()).collect();
            for s in signals.iter().filter(|s| s.severity >= 2) {
                let id = if s.kind == "flip-flop" { s.items.last() } else { s.items.first() };
                let Some(i) = id.and_then(|id| by_id.get(id.as_str())) else { continue };
                if listed.contains(&i.meta.id) || out.iter().any(|a| a.id == i.meta.id) {
                    continue;
                }
                out.push(mk(
                    i,
                    "loop",
                    s.severity + 1,
                    0,
                    format!("{}: {}", s.kind, s.detail),
                    "going in circles: say so to the user and suggest a different approach or a decision",
                ));
            }
        }

        out.sort_by(|a, b| b.severity.cmp(&a.severity).then(b.days.cmp(&a.days)).then(a.id.cmp(&b.id)));
        Ok(AttentionReport { items: out, backlog })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_weekdays() {
        let fri = NaiveDate::from_ymd_opt(2026, 11, 20).unwrap(); // a Friday
        assert_eq!(parse_when("today", fri).unwrap(), fri);
        assert_eq!(parse_when("tomorrow", fri).unwrap().to_string(), "2026-11-21");
        assert_eq!(parse_when("mon", fri).unwrap().to_string(), "2026-11-23");
        assert_eq!(parse_when("friday", fri).unwrap().to_string(), "2026-11-27", "never today");
        assert_eq!(parse_when("2w", fri).unwrap().to_string(), "2026-12-04");
        assert_eq!(parse_when("2026-12-01", fri).unwrap().to_string(), "2026-12-01");
        assert!(parse_when("soon", fri).is_err());
    }
}
