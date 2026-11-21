//! Data model: projects, items, journal entries and sessions.
//!
//! The structs that end in `Meta` are exactly what is stored in a file's YAML front matter, so the file format,
//! the JSON API and the index never drift apart.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, SubsecRound, Utc};
use serde::{Deserialize, Serialize};

use crate::Error;

pub type Timestamp = DateTime<Utc>;

/// A new entry or session id: a ULID from a monotonic generator, so ids created one after another in this
/// process always sort in creation order (plain ULIDs are random within the same millisecond).
pub fn new_id() -> String {
    static GENERATOR: std::sync::Mutex<Option<ulid::Generator>> = std::sync::Mutex::new(None);
    let mut guard = GENERATOR.lock().unwrap_or_else(|e| e.into_inner());
    let generator = guard.get_or_insert_with(ulid::Generator::new);
    match generator.generate() {
        Ok(id) => id.to_string(),
        // Overflow of the random part within one millisecond: astronomically unlikely; fall back.
        Err(_) => ulid::Ulid::generate().to_string(),
    }
}

/// Current time truncated to whole seconds, so files stay readable.
pub fn now() -> Timestamp {
    Utc::now().trunc_subsecs(0)
}

macro_rules! string_enum {
    ($(#[$m:meta])* $name:ident, $what:literal { $($variant:ident => $s:literal $(| $alias:literal)*),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub enum $name { $(#[serde(rename = $s)] $variant),+ }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];
            pub fn as_str(self) -> &'static str { match self { $($name::$variant => $s),+ } }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.as_str()) }
        }

        impl FromStr for $name {
            type Err = Error;
            fn from_str(s: &str) -> Result<Self, Error> {
                let l = s.trim().to_ascii_lowercase();
                match l.as_str() {
                    $($s $(| $alias)* => Ok($name::$variant),)+
                    _ => Err(Error::Invalid(format!(
                        "unknown {} '{}' (expected one of: {})", $what, s,
                        [$($s),+].join(", ")
                    ))),
                }
            }
        }
    };
}

string_enum!(
    /// What an item is. The tree (`parent`) gives the hierarchy; the kind gives its meaning.
    Kind, "kind" {
        Plan => "plan" | "plans",
        Phase => "phase" | "phases" | "milestone",
        Feature => "feature" | "features" | "feat",
        Task => "task" | "tasks" | "todo",
        Bug => "bug" | "bugs",
        Regression => "regression" | "regressions" | "reg",
        Revisit => "revisit" | "revisits",
        Decision => "decision" | "decisions",
        Question => "question" | "questions",
        Idea => "idea" | "ideas",
        Experiment => "experiment" | "experiments" | "exp",
        Resource => "resource" | "resources" | "res" | "asset",
        Note => "note" | "notes",
    }
);

string_enum!(
    Status, "status" {
        Open => "open" | "todo",
        Active => "active" | "doing" | "wip" | "in-progress",
        Blocked => "blocked",
        Done => "done" | "closed" | "fixed" | "succeeded" | "success" | "passed",
        Dropped => "dropped" | "wontfix" | "cancelled",
        Proposed => "proposed",
        Accepted => "accepted",
        Superseded => "superseded",
        Rejected => "rejected",
        Failed => "failed",
        Inconclusive => "inconclusive",
    }
);

string_enum!(
    EntryKind, "entry kind" {
        Progress => "progress",
        Note => "note",
        Blocker => "blocker",
        Finding => "finding",
        Handoff => "handoff",
        Decision => "decision",
        StatusChange => "status-change" | "status",
        // "Still relevant" (`t9 still`): resets the idle clock without counting as progress.
        Checkin => "checkin" | "check-in" | "still",
    }
);

impl Kind {
    pub fn default_status(self) -> Status {
        match self {
            Kind::Decision => Status::Accepted,
            _ => Status::Open,
        }
    }
    /// Kinds that represent problems or loose ends and are surfaced in the context brief.
    pub fn is_issue(self) -> bool {
        matches!(self, Kind::Bug | Kind::Regression | Kind::Revisit | Kind::Question)
    }
}

/// Resource types t9 knows; other short names are accepted too.
pub const RESOURCE_TYPES: &[&str] = &[
    "dataset",
    "table",
    "pipeline",
    "dashboard",
    "notebook",
    "model",
    "endpoint",
    "prompt",
    "eval",
    "index",
    "feature",
    "service",
    "api",
    "database",
    "queue",
    "bucket",
    "environment",
    "repo",
    "doc",
    "other",
];

/// Refuses text that looks like a credential: resources should point at a secret manager, never hold secrets.
pub fn looks_like_secret(s: &str) -> Option<&'static str> {
    let l = s.to_ascii_lowercase();
    let patterns: &[(&str, &str)] = &[
        ("-----begin", "a private key"),
        ("password=", "a password"),
        ("passwd=", "a password"),
        ("secret=", "a secret"),
        ("token=", "a token"),
        ("api_key=", "an API key"),
        ("apikey=", "an API key"),
        ("authorization: bearer", "a bearer token"),
    ];
    for (p, what) in patterns {
        if l.contains(p) {
            return Some(what);
        }
    }
    let words = s.split(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',');
    for w in words {
        let looks = (w.starts_with("AKIA")
            && w.len() == 20
            && w[4..].chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()))
            || ((w.starts_with("ghp_")
                || w.starts_with("github_pat_")
                || w.starts_with("sk-")
                || w.starts_with("xox"))
                && w.len() >= 20)
            || (w.starts_with("dapi") && w.len() >= 32 && w[4..].chars().all(|c| c.is_ascii_hexdigit()));
        if looks {
            return Some("an access token");
        }
    }
    None
}

impl Status {
    pub fn is_closed(self) -> bool {
        matches!(
            self,
            Status::Done
                | Status::Dropped
                | Status::Superseded
                | Status::Rejected
                | Status::Failed
                | Status::Inconclusive
        )
    }
}

/// A typed link between items.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub rel: String,
    pub to: String,
}

pub const LINK_RELS: &[&str] = &[
    "blocks",
    "blocked-by",
    "relates",
    "regression-of",
    "supersedes",
    "superseded-by",
    "duplicate-of",
    "applies-to",
    "about",
];

impl FromStr for Link {
    type Err = Error;
    /// Parses `rel:ID`, e.g. `regression-of:BIL-31`.
    fn from_str(s: &str) -> Result<Self, Error> {
        let (rel, to) = s
            .split_once(':')
            .ok_or_else(|| Error::Invalid(format!("link '{s}' must look like rel:ID, e.g. relates:BIL-3")))?;
        let rel = rel.trim().to_ascii_lowercase();
        if !LINK_RELS.contains(&rel.as_str()) {
            return Err(Error::Invalid(format!(
                "unknown link rel '{rel}' (expected one of: {})",
                LINK_RELS.join(", ")
            )));
        }
        Ok(Link { rel, to: normalize_item_id(to)? })
    }
}

/// Front matter of `projects/<slug>/project.md`. The body is the project description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub key: String,
    pub name: String,
    #[serde(default = "default_project_status")]
    pub status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remotes: Vec<String>,
    pub created: Timestamp,
    /// When the project's About was last written or confirmed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about_reviewed: Option<Timestamp>,
    /// Keys a human added that t9 does not know about; preserved on rewrite.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_norway::Value>,
}

fn default_project_status() -> String {
    "active".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct Project {
    #[serde(flatten)]
    pub meta: ProjectMeta,
    pub slug: String,
    pub description: String,
}

/// Front matter of an item file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemMeta {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Topics such as `architecture`, `testing`, `storage`. Decisions are recalled by area.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub areas: Vec<String>,
    /// Why it exists: what it serves. Shown with the item; inherited by children that have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// When the human wants it done (a local date).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<chrono::NaiveDate>,
    /// Experiments: what is being tested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hypothesis: Option<String>,
    /// Experiments: the condition under which a failed or inconclusive one is worth trying again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_when: Option<String>,
    /// Experiments: the date from which it is worth trying again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<chrono::NaiveDate>,
    /// Resources: dataset, table, model, endpoint, service, api, …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<String>,
    /// Resources: where it lives (table name, URI, path, URL).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// Resources: who owns or maintains it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    pub created: Timestamp,
    pub updated: Timestamp,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created_by: String,
    /// Session that created the item, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_in: Option<String>,
    #[serde(default)]
    pub rev: u32,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_norway::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    #[serde(flatten)]
    pub meta: ItemMeta,
    pub project: String,
    pub body: String,
    pub path: String,
}

/// Header of a journal block (stored as JSON inside an HTML comment).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryMeta {
    pub id: String,
    pub kind: EntryKind,
    pub at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent: String,
    /// Recorded for entries that could not be tied to a project (inbox), so they can be triaged later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    #[serde(flatten)]
    pub meta: EntryMeta,
    pub project: String,
    pub body: String,
}

/// Front matter of `projects/<slug>/sessions/<id>.md`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_id: Option<String>,
    pub project: String,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    pub started: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<Timestamp>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Session {
    #[serde(flatten)]
    pub meta: SessionMeta,
    pub last_seen: Option<Timestamp>,
}

impl Session {
    /// Last 6 characters of the ULID: the random part, short enough to type.
    pub fn short_id(&self) -> &str {
        short_id(&self.meta.id)
    }
}

pub fn short_id(id: &str) -> &str {
    &id[id.len().saturating_sub(6)..]
}

/// Validates and upper-cases an item id like `bil-42` → `BIL-42`.
pub fn normalize_item_id(s: &str) -> Result<String, Error> {
    let s = s.trim().to_ascii_uppercase();
    match split_item_id(&s) {
        Some(_) => Ok(s),
        None => Err(Error::Invalid(format!("'{s}' is not an item id (expected KEY-NUMBER, e.g. BIL-42)"))),
    }
}

/// `BIL-42` → `("BIL", 42)`.
pub fn split_item_id(s: &str) -> Option<(&str, u32)> {
    let (key, num) = s.rsplit_once('-')?;
    if !is_valid_key(key) {
        return None;
    }
    Some((key, num.parse().ok()?))
}

/// Project keys: 2–10 chars, upper-case letters and digits, starting with a letter.
pub fn is_valid_key(key: &str) -> bool {
    let b = key.as_bytes();
    (2..=10).contains(&b.len())
        && b[0].is_ascii_uppercase()
        && b.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_sort_in_creation_order() {
        let ids: Vec<String> = (0..1000).map(|_| new_id()).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
    }

    #[test]
    fn parses_aliases() {
        assert_eq!("WIP".parse::<Status>().unwrap(), Status::Active);
        assert_eq!("reg".parse::<Kind>().unwrap(), Kind::Regression);
        assert!("nope".parse::<Kind>().is_err());
    }

    #[test]
    fn item_ids() {
        assert_eq!(normalize_item_id("bil-42").unwrap(), "BIL-42");
        assert_eq!(split_item_id("AB2-7"), Some(("AB2", 7)));
        assert!(normalize_item_id("42").is_err());
        assert!(normalize_item_id("Q-1").is_err());
    }

    #[test]
    fn links() {
        let l: Link = "regression-of:bil-3".parse().unwrap();
        assert_eq!(l, Link { rel: "regression-of".into(), to: "BIL-3".into() });
        assert!("foo:BIL-1".parse::<Link>().is_err());
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("Billing Service!"), "billing-service");
        assert_eq!(slugify("billing_service"), "billing-service");
    }
}
