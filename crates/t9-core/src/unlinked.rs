//! Folders where agents worked without a linked project.
//!
//! Without a project, an agent's notes go to the inbox and nothing tells the human. t9 remembers each such
//! folder (in the state dir, not the vault), suggests a name and key for it, and surfaces it in the brief, the
//! per-message hook (once per agent session) and the TUI, where one key creates the project.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::index::EntryFilter;
use crate::model::*;
use crate::store::{Caller, INBOX, Store, unique_key};
use crate::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UnlinkedFolder {
    /// The folder (its git work-tree top, if inside one).
    pub cwd: String,
    /// Suggested project name and key for `t9 project init`.
    pub name: String,
    pub key: String,
    pub agent: String,
    pub last_seen: Timestamp,
    /// Inbox entries written from this folder.
    #[serde(default)]
    pub inbox_entries: usize,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Registry {
    #[serde(default)]
    folders: BTreeMap<String, Seen>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Seen {
    agent: String,
    last_seen: Timestamp,
}

impl Store {
    fn unlinked_path(&self) -> PathBuf {
        self.env.home.join("unlinked.json")
    }

    fn read_registry(&self) -> Registry {
        fs::read_to_string(self.unlinked_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    fn write_registry(&self, r: &Registry) -> Result<()> {
        let p = self.unlinked_path();
        let tmp = p.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(r).expect("serializable")).map_err(|e| Error::io(&tmp, e))?;
        fs::rename(&tmp, &p).map_err(|e| Error::io(&p, e))
    }

    /// The folder t9 would register for `cwd`: its git work-tree top, or `cwd` itself.
    pub fn folder_of(&self, cwd: &Path) -> String {
        self.root_and_remote(cwd).0
    }

    /// Suggested name and key for a new project in `cwd`: the folder's name, and a key no project uses yet.
    pub fn suggested_project(&self, cwd: &Path) -> Result<(String, String)> {
        let folder = PathBuf::from(self.folder_of(cwd));
        let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "project".into());
        let existing: HashSet<String> = self.projects()?.into_iter().map(|p| p.meta.key).collect();
        Ok((name.clone(), unique_key(&name, &existing)))
    }

    /// Records that `agent` worked in `cwd`, which is not linked to a project.
    pub fn note_unlinked(&self, cwd: &Path, agent: &str) -> Result<()> {
        let mut r = self.read_registry();
        r.folders.insert(self.folder_of(cwd), Seen { agent: agent.into(), last_seen: now() });
        self.write_registry(&r)
    }

    /// Folders where agents worked without a project (from the registry and from inbox entries), most recent
    /// first. Folders that have since been linked are dropped.
    pub fn unlinked_folders(&mut self) -> Result<Vec<UnlinkedFolder>> {
        let mut registry = self.read_registry();
        self.sync_project(INBOX)?;
        let mut seen: BTreeMap<String, (Seen, usize)> =
            registry.folders.iter().map(|(k, v)| (k.clone(), (v.clone(), 0))).collect();
        for e in self.entries(&EntryFilter { project: Some(INBOX.into()), ..Default::default() })? {
            let Some(cwd) = &e.meta.cwd else { continue };
            let folder = self.folder_of(Path::new(cwd));
            let slot = seen.entry(folder).or_insert((Seen { agent: e.meta.agent.clone(), last_seen: e.meta.at }, 0));
            slot.1 += 1;
            if e.meta.at > slot.0.last_seen {
                slot.0 = Seen { agent: e.meta.agent.clone(), last_seen: e.meta.at };
            }
        }
        let mut out = vec![];
        let mut linked = vec![];
        for (folder, (s, inbox_entries)) in seen {
            let path = PathBuf::from(&folder);
            if !path.exists() || self.resolve(&Caller { cwd: path.clone(), ..Default::default() })?.project.is_some() {
                linked.push(folder);
                continue;
            }
            let (name, key) = self.suggested_project(&path)?;
            out.push(UnlinkedFolder { cwd: folder, name, key, agent: s.agent, last_seen: s.last_seen, inbox_entries });
        }
        if linked.iter().any(|f| registry.folders.contains_key(f)) {
            registry.folders.retain(|k, _| !linked.contains(k));
            self.write_registry(&registry)?;
        }
        out.sort_by_key(|u| std::cmp::Reverse(u.last_seen));
        Ok(out)
    }

    /// `true` the first time it is called for an agent session (keyed by the harness's session id).
    pub fn first_time_for(&self, what: &str, session: &str) -> Result<bool> {
        let dir = self.env.home.join("hints");
        fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        let safe: String = format!("{what}-{session}")
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
            .collect();
        let marker = dir.join(safe);
        if marker.exists() {
            return Ok(false);
        }
        fs::write(&marker, "").map_err(|e| Error::io(&marker, e))?;
        Ok(true)
    }
}

/// The exact command that creates the suggested project for a folder.
pub fn init_command(name: &str, key: &str) -> String {
    let name = name.replace('"', "'");
    format!("t9 project init --name \"{name}\" --key {key}")
}
