//! [`Store`]: the one API every front end uses. Writes go to the vault first (atomically, under a per-project
//! lock), then to the index. Reads sync the index with the vault by comparing file mtimes/sizes, so edits made
//! in an editor are picked up without a daemon.

use std::cell::Cell;
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use chrono::{Duration, Local};
use rusqlite::{Connection, Transaction, TransactionBehavior};
use serde::Serialize;

use crate::env::{Env, display_path};
use crate::git::{self, GitInfo};
use crate::index::{self, EntryFilter, ItemFilter};
use crate::model::*;
use crate::{Error, Result, frontmatter, journal};

pub const INBOX: &str = "INBOX";
/// Project holding decisions that apply to every project (`t9 decide --global`).
pub const GLOBAL: &str = "GLOBAL";

/// Areas are lower-case slugs: `Test Kit` → `test-kit`.
pub fn normalize_areas(areas: &[String]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for a in areas.iter().flat_map(|a| a.split(',')) {
        let a = slugify(a);
        if !a.is_empty() && !out.contains(&a) {
            out.push(a);
        }
    }
    out
}
/// How often reads do a full check of a project's files (catches in-place edits the mtime fast path misses).
pub const FULL_CHECK_SECS: i64 = 300;
/// Sessions with no activity for this long are no longer picked up implicitly.
const SESSION_IDLE_HOURS: i64 = 12;

/// Who is calling and from where. Built by the front end from flags and environment.
#[derive(Debug, Clone, Default)]
pub struct Caller {
    pub cwd: PathBuf,
    /// `claude-code`, `codex`, `human`, … (`T9_AGENT`).
    pub agent: Option<String>,
    /// Explicit session id or suffix (`T9_SESSION`).
    pub session: Option<String>,
    /// Explicit project key or slug (`T9_PROJECT`); bypasses cwd resolution.
    pub project: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Suggestion {
    pub key: String,
    pub name: String,
    pub reason: String,
    pub roots: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Resolution {
    pub cwd: String,
    pub project: Option<Project>,
    /// How the project was found: `explicit`, `.t9`, `root`, `git-remote`.
    pub via: Option<String>,
    pub git: Option<GitInfo>,
    pub suggestions: Vec<Suggestion>,
}

impl Resolution {
    pub fn key(&self) -> &str {
        self.project.as_ref().map(|p| p.meta.key.as_str()).unwrap_or(INBOX)
    }
}

#[derive(Debug, Clone, Default)]
pub struct NewItem {
    pub kind: Option<Kind>,
    pub title: String,
    pub body: String,
    pub parent: Option<String>,
    pub priority: Option<u8>,
    pub tags: Vec<String>,
    pub areas: Vec<String>,
    pub links: Vec<Link>,
    pub status: Option<Status>,
    pub goal: Option<String>,
    pub due: Option<chrono::NaiveDate>,
    pub hypothesis: Option<String>,
    pub retry_when: Option<String>,
    pub retry_after: Option<chrono::NaiveDate>,
    pub resource_type: Option<String>,
    pub location: Option<String>,
    pub owner: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ItemPatch {
    pub title: Option<String>,
    pub status: Option<Status>,
    pub parent: Option<Option<String>>,
    pub priority: Option<Option<u8>>,
    pub add_tags: Vec<String>,
    pub add_areas: Vec<String>,
    pub remove_tags: Vec<String>,
    pub add_links: Vec<Link>,
    pub body: Option<String>,
    /// `Some("")` clears the goal.
    pub goal: Option<String>,
    pub due: Option<Option<chrono::NaiveDate>>,
    /// Experiment and resource fields: `Some("")` clears a field.
    pub hypothesis: Option<String>,
    pub retry_when: Option<String>,
    pub retry_after: Option<Option<chrono::NaiveDate>>,
    pub resource_type: Option<String>,
    pub location: Option<String>,
    pub owner: Option<String>,
    /// Optimistic concurrency: fail if the file's `rev` differs.
    pub expect_rev: Option<u32>,
    /// Reason recorded with a status change.
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LogInput {
    pub kind: EntryKind,
    pub body: String,
    pub item: Option<String>,
    pub refs: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct ReindexStats {
    pub projects: usize,
    pub items: usize,
    pub entries: usize,
    pub sessions: usize,
    pub warnings: Vec<String>,
}

impl Drop for Store {
    fn drop(&mut self) {
        if self.commit_on_drop && self.wrote.get() && self.env.git {
            crate::vcs::spawn_background(&self.env);
        }
    }
}

pub struct Store {
    pub env: Env,
    pub conn: Connection,
    /// Problems found while reading the vault (unparseable files, duplicate keys). Shown by `t9 doctor`.
    pub warnings: Vec<String>,
    /// Set when this process changed a vault file.
    pub wrote: Cell<bool>,
    /// If set (the CLI does), dropping a store that wrote starts a background git commit of the vault.
    pub commit_on_drop: bool,
}

// ---------------------------------------------------------------- file helpers

fn file_sig(path: &Path) -> Option<(i64, i64)> {
    let md = fs::metadata(path).ok()?;
    let mtime = md.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos() as i64;
    Some((mtime, md.len() as i64))
}

fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let dir = path.parent().expect("vault paths have a parent");
    fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    let name = path.file_name().unwrap().to_string_lossy();
    let tmp = dir.join(format!(".{name}.tmp{}", std::process::id()));
    fs::write(&tmp, content).map_err(|e| Error::io(&tmp, e))?;
    fs::rename(&tmp, path).map_err(|e| Error::io(path, e))?;
    Ok(())
}

fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|e| Error::io(path, e))
}

fn md_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(dir) else { return vec![] };
    let mut v: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().is_some_and(|x| x == "md")
                && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
        })
        .collect();
    v.sort();
    v
}

/// (Re)indexes one vault file inside a transaction. Unparseable files are reported in `warnings` and still
/// recorded, so they are not re-read until they change.
fn index_file(
    tx: &Connection,
    key: &str,
    kind: &str,
    file: &Path,
    sig: (i64, i64),
    warnings: &mut Vec<String>,
) -> Result<()> {
    let fstr = path_str(file);
    if index::file_state(tx, &fstr)? == Some(sig) {
        return Ok(());
    }
    index::forget_file(tx, &fstr)?;
    let text = read(file)?;
    match kind {
        "item" => match frontmatter::parse::<ItemMeta>(&text, &fstr) {
            Ok((meta, body)) => {
                if index::item(tx, &meta.id)?.is_some_and(|i| i.path != fstr) {
                    warnings.push(format!("{fstr}: duplicate item id {}, ignored", meta.id));
                } else {
                    index::upsert_item(tx, &Item { meta, project: key.into(), body, path: fstr.clone() })?;
                }
            }
            Err(e) => warnings.push(e.to_string()),
        },
        "journal" => {
            let (blocks, errors) = journal::parse(&text);
            warnings.extend(errors.into_iter().map(|e| format!("{fstr}: {e}")));
            for (meta, body) in blocks {
                index::insert_entry(tx, &Entry { meta, project: key.into(), body }, &fstr)?;
            }
        }
        _ => match frontmatter::parse::<SessionMeta>(&text, &fstr) {
            Ok((meta, _)) => index::upsert_session(tx, &Session { meta, last_seen: None }, &fstr)?,
            Err(e) => warnings.push(e.to_string()),
        },
    }
    index::set_file_state(tx, &fstr, key, kind, sig.0, sig.1)?;
    Ok(())
}

fn canonical(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// `true` if `path` is `root` or inside it.
fn is_within(path: &str, root: &str) -> bool {
    path == root || (path.starts_with(root) && path.as_bytes().get(root.len()) == Some(&b'/'))
}

fn derive_key(name: &str) -> String {
    let words: Vec<String> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_ascii_uppercase())
        .collect();
    let mut key: String = match words.len() {
        0 => "PRJ".into(),
        1 => words[0].chars().take(3).collect(),
        _ => words.iter().take(4).filter_map(|w| w.chars().next()).collect(),
    };
    if !key.starts_with(|c: char| c.is_ascii_uppercase()) {
        key.insert(0, 'P');
    }
    if key.len() < 2 {
        key.push_str(&words[0].chars().skip(1).take(2).collect::<String>());
        if key.len() < 2 {
            key.push('X');
        }
    }
    key.truncate(8);
    key
}

/// A key derived from `name` that no project uses yet (`BS`, then `BS2`, …).
pub(crate) fn unique_key(name: &str, existing: &HashSet<String>) -> String {
    let base = derive_key(name);
    let mut k = base.clone();
    let mut n = 2;
    while existing.contains(&k) || k == INBOX || k == GLOBAL {
        k = format!("{base}{n}");
        n += 1;
    }
    k
}

/// `Some("  ")` → `None`; trims.
fn clean_opt(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// t9 never stores credentials: refuse text that looks like one and say what to do instead.
fn refuse_secrets<'a>(fields: impl IntoIterator<Item = Option<&'a String>>) -> Result<()> {
    for f in fields.into_iter().flatten() {
        if let Some(what) = looks_like_secret(f) {
            return Err(Error::Invalid(format!(
                "this looks like it contains {what}; t9 does not store credentials. Record where the secret is kept \
                 instead (e.g. \"in 1Password: billing/prod-db\" or \"env var DB_PASSWORD\")"
            )));
        }
    }
    Ok(())
}

fn item_file_name(id: &str, title: &str) -> String {
    let mut slug = slugify(title);
    if slug.len() > 60 {
        let mut cut = 60;
        while !slug.is_char_boundary(cut) {
            cut -= 1;
        }
        slug.truncate(cut);
        while slug.ends_with('-') {
            slug.pop();
        }
    }
    if slug.is_empty() { format!("{id}.md") } else { format!("{id}-{slug}.md") }
}

/// Parses the item number from a file name like `BIL-42-some-title.md`.
fn file_item_num(name: &str, key: &str) -> Option<u32> {
    let rest = name.strip_prefix(key)?.strip_prefix('-')?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

impl Store {
    pub fn open(env: Env) -> Result<Store> {
        for dir in [env.projects_dir(), env.inbox_dir().join("journal"), env.locks_dir()] {
            fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        }
        crate::vcs::ensure_gitignore(&env.vault)?;
        let (conn, fresh) = index::open(&env.db_path())?;
        let mut store = Store { env, conn, warnings: vec![], wrote: Cell::new(false), commit_on_drop: false };
        if fresh {
            store.reindex()?;
        } else {
            store.sync_projects()?;
        }
        Ok(store)
    }

    // ------------------------------------------------------------ locking and paths

    /// Every vault write goes through here (or `append_entry`), so `wrote` is reliable.
    pub(crate) fn put(&self, path: &Path, content: &str) -> Result<()> {
        self.wrote.set(true);
        write_atomic(path, content)
    }

    pub(crate) fn lock(&self, key: &str) -> Result<File> {
        let path = self.env.locks_dir().join(format!("{key}.lock"));
        let f =
            OpenOptions::new().create(true).truncate(false).write(true).open(&path).map_err(|e| Error::io(&path, e))?;
        f.lock().map_err(|e| Error::io(&path, e))?;
        Ok(f)
    }

    pub fn project_dir(&self, key: &str) -> Result<PathBuf> {
        if key == INBOX {
            return Ok(self.env.inbox_dir());
        }
        let p = self.project(key)?;
        Ok(self.env.projects_dir().join(p.slug))
    }

    /// Was the file's directory unchanged since the last sync? (Then t9's own write may update its state.)
    fn dir_clean(&self, path: &Path) -> Result<bool> {
        let dir = path.parent().expect("vault paths have a parent");
        let sig = file_sig(dir);
        Ok(sig.is_some() && index::file_state(&self.conn, &path_str(dir))? == sig)
    }

    /// Writes a vault file atomically. Returns whether its directory was in sync before the write.
    fn write_file(&self, path: &Path, content: &str) -> Result<bool> {
        let clean = self.dir_clean(path)?;
        self.put(path, content)?;
        Ok(clean)
    }

    /// Records the state of a file t9 just wrote (and of its directory, if that was in sync before), so t9's
    /// own writes never look like outside edits and never force a directory rescan.
    fn record_written(&self, path: &Path, project: &str, kind: &str, dir_was_clean: bool) -> Result<()> {
        if let Some((m, s)) = file_sig(path) {
            index::set_file_state(&self.conn, &path_str(path), project, kind, m, s)?;
        }
        let dir = path.parent().expect("vault paths have a parent");
        if dir_was_clean && let Some((m, s)) = file_sig(dir) {
            index::set_file_state(&self.conn, &path_str(dir), project, "dir", m, s)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------ sync

    /// Picks up added, edited and removed `project.md` files.
    pub fn sync_projects(&mut self) -> Result<()> {
        let Ok(rd) = fs::read_dir(self.env.projects_dir()) else { return Ok(()) };
        let mut dirs: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect();
        dirs.sort();
        let files: Vec<(PathBuf, (i64, i64))> =
            dirs.iter().filter_map(|d| file_sig(&d.join("project.md")).map(|sig| (d.clone(), sig))).collect();
        // Read-only check first: the common case is "nothing changed" and must not take the write lock.
        let indexed = self.conn.query_row("SELECT count(*) FROM projects", [], |r| r.get::<_, i64>(0))? as usize;
        let mut dirty = indexed != files.len();
        for (d, sig) in &files {
            dirty |= index::file_state(&self.conn, &path_str(&d.join("project.md")))? != Some(*sig);
        }
        if !dirty {
            return Ok(());
        }
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let mut seen: HashSet<String> = HashSet::new();
        for (dir, (m, s)) in files {
            let file = dir.join("project.md");
            let fstr = path_str(&file);
            let slug = dir.file_name().unwrap().to_string_lossy().into_owned();
            if index::file_state(&tx, &fstr)? == Some((m, s)) {
                if let Some(p) = index::project_by_slug(&tx, &slug)? {
                    seen.insert(p.meta.key);
                }
                continue;
            }
            let text = read(&file)?;
            match frontmatter::parse::<ProjectMeta>(&text, &fstr) {
                Ok((meta, body)) => {
                    if seen.contains(&meta.key) {
                        self.warnings.push(format!("{fstr}: duplicate project key {}, ignored", meta.key));
                        continue;
                    }
                    if let Some(old) = index::project_by_slug(&tx, &slug)?
                        && old.meta.key != meta.key
                    {
                        index::delete_project(&tx, &old.meta.key)?;
                    }
                    seen.insert(meta.key.clone());
                    let key = meta.key.clone();
                    index::upsert_project(&tx, &Project { meta, slug, description: body.trim().to_string() })?;
                    index::set_file_state(&tx, &fstr, &key, "project", m, s)?;
                }
                Err(e) => self.warnings.push(e.to_string()),
            }
        }
        for p in index::projects(&tx)? {
            if !seen.contains(&p.meta.key) {
                index::delete_project(&tx, &p.meta.key)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Brings the index up to date with a project's items, journal and session files.
    ///
    /// Cheap by design (filesystem calls can cost 30+ µs each): a subdirectory is only listed when its mtime
    /// changed (files added, removed or atomically replaced), plus a full check every [`FULL_CHECK_SECS`] that
    /// also catches in-place edits. `t9 reindex` forces a full rebuild; single-item reads check their own file.
    pub fn sync_project(&mut self, key: &str) -> Result<()> {
        self.sync_project_with(key, false)
    }

    fn sync_project_with(&mut self, key: &str, force_full: bool) -> Result<()> {
        let t = std::time::Instant::now();
        let dir = self.project_dir(key)?;
        let states = index::file_states(&self.conn, key)?;
        let full_marker = format!("{}#full-check", path_str(&dir));
        let now_ns = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as i64).unwrap_or(0);
        let full =
            force_full || states.get(&full_marker).is_none_or(|(at, _)| now_ns - at > FULL_CHECK_SECS * 1_000_000_000);

        let mut files: Vec<(PathBuf, &'static str, (i64, i64))> = vec![];
        let mut scanned: Vec<String> = vec![];
        let mut dir_states: Vec<(String, (i64, i64))> = vec![];
        for (sub, kind) in [("items", "item"), ("journal", "journal"), ("sessions", "session")] {
            let d = dir.join(sub);
            let dstr = path_str(&d);
            // Read the directory's state before listing it: a change during the listing triggers a rescan.
            let Some(dsig) = file_sig(&d) else { continue };
            if !full && states.get(&dstr) == Some(&dsig) {
                continue;
            }
            scanned.push(format!("{dstr}/"));
            dir_states.push((dstr, dsig));
            for file in md_files(&d) {
                if let Some(sig) = file_sig(&file) {
                    files.push((file, kind, sig));
                }
            }
        }
        // Recent journals are often appended to or edited in place (no directory change): always check them.
        let journal_dir = format!("{}/", path_str(&dir.join("journal")));
        if !scanned.contains(&journal_dir) {
            let cutoff = (Local::now() - Duration::days(7)).format("%Y-%m-%d.md").to_string();
            for (path, sig) in &states {
                let recent = path.strip_prefix(journal_dir.as_str()).is_some_and(|name| name >= cutoff.as_str());
                if recent && file_sig(Path::new(path)) != Some(*sig) {
                    match file_sig(Path::new(path)) {
                        Some(now_sig) => files.push((PathBuf::from(path), "journal", now_sig)),
                        None => scanned.push(path.clone()),
                    }
                }
            }
        }
        let seen: HashSet<String> = files.iter().map(|f| path_str(&f.0)).collect();
        let removed: Vec<String> = states
            .keys()
            .filter(|k| scanned.iter().any(|d| k.starts_with(d.as_str())) && !seen.contains(*k))
            .cloned()
            .collect();
        let changed: Vec<_> = files.into_iter().filter(|(f, _, sig)| states.get(&path_str(f)) != Some(sig)).collect();
        let dirs_moved = dir_states.iter().any(|(d, sig)| states.get(d) != Some(sig));
        crate::trace(&format!("sync {key} check{}", if full { " (full)" } else { "" }), t);
        if changed.is_empty() && removed.is_empty() && !dirs_moved && !full {
            return Ok(());
        }
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        for (file, kind, sig) in changed {
            index_file(&tx, key, kind, &file, sig, &mut self.warnings)?;
        }
        for path in removed {
            index::forget_file(&tx, &path)?;
        }
        for (d, (m, s)) in dir_states {
            index::set_file_state(&tx, &d, key, "dir", m, s)?;
        }
        if full {
            index::set_file_state(&tx, &full_marker, key, "full-check", now_ns, 0)?;
        }
        tx.commit()?;
        crate::trace(&format!("sync {key} apply"), t);
        Ok(())
    }

    /// Re-reads one file if it changed on disk (e.g. edited in place by an editor) or forgets it if deleted.
    fn refresh_file(&mut self, key: &str, kind: &'static str, path: &str) -> Result<()> {
        let p = Path::new(path);
        let current = file_sig(p);
        if index::file_state(&self.conn, path)? == current {
            return Ok(());
        }
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        match current {
            Some(sig) => index_file(&tx, key, kind, p, sig, &mut self.warnings)?,
            None => index::forget_file(&tx, path)?,
        }
        tx.commit()?;
        Ok(())
    }

    pub fn sync_all(&mut self) -> Result<()> {
        self.sync_all_with(false)
    }

    fn sync_all_with(&mut self, full: bool) -> Result<()> {
        self.sync_projects()?;
        let keys: Vec<String> = index::projects(&self.conn)?.into_iter().map(|p| p.meta.key).collect();
        for k in keys.iter().map(String::as_str).chain([INBOX]) {
            self.sync_project_with(k, full)?;
        }
        Ok(())
    }

    /// Rebuilds the whole index from the vault.
    pub fn reindex(&mut self) -> Result<ReindexStats> {
        index::clear(&self.conn)?;
        self.warnings.clear();
        self.sync_all_with(true)?;
        let count = |sql: &str| -> Result<usize> { Ok(self.conn.query_row(sql, [], |r| r.get::<_, i64>(0))? as usize) };
        Ok(ReindexStats {
            projects: count("SELECT count(*) FROM projects")?,
            items: count("SELECT count(*) FROM items")?,
            entries: count("SELECT count(*) FROM entries")?,
            sessions: count("SELECT count(*) FROM sessions")?,
            warnings: self.warnings.clone(),
        })
    }

    // ------------------------------------------------------------ projects

    pub fn projects(&self) -> Result<Vec<Project>> {
        index::projects(&self.conn)
    }

    /// Looks a project up by key (case-insensitive) or slug.
    pub fn project(&self, key_or_slug: &str) -> Result<Project> {
        let upper = key_or_slug.trim().to_ascii_uppercase();
        if let Some(p) = index::project(&self.conn, &upper)? {
            return Ok(p);
        }
        if let Some(p) = index::project_by_slug(&self.conn, &slugify(key_or_slug))? {
            return Ok(p);
        }
        Err(Error::NotFound(format!("project '{key_or_slug}' (see `t9 project ls`)")))
    }

    pub fn resolve(&self, caller: &Caller) -> Result<Resolution> {
        let cwd = canonical(&caller.cwd);
        let cwd_s = path_str(&cwd);
        let git = git::discover(&cwd);
        let mut res = Resolution { cwd: cwd_s.clone(), project: None, via: None, git, suggestions: vec![] };

        if let Some(p) = &caller.project {
            res.project = Some(self.project(p)?);
            res.via = Some("explicit".into());
            return Ok(res);
        }
        for dir in cwd.ancestors() {
            if let Ok(text) = fs::read_to_string(dir.join(".t9")) {
                let key = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#')).unwrap_or("");
                let key = key.strip_prefix("project:").unwrap_or(key).trim();
                res.project = Some(self.project(key)?);
                res.via = Some(".t9".into());
                return Ok(res);
            }
        }
        let best_root = index::all_roots(&self.conn)?
            .into_iter()
            .filter(|(_, root)| is_within(&cwd_s, root))
            .max_by_key(|(_, root)| root.len());
        if let Some((key, _)) = best_root {
            res.project = Some(self.project(&key)?);
            res.via = Some("root".into());
            return Ok(res);
        }
        if let Some(remote) = res.git.as_ref().and_then(|g| g.remote.clone())
            && let Some(key) = index::project_for_remote(&self.conn, &remote)?.into_iter().next()
        {
            res.project = Some(self.project(&key)?);
            res.via = Some("git-remote".into());
            return Ok(res);
        }
        res.suggestions = self.suggest(&cwd, res.git.as_ref())?;
        Ok(res)
    }

    fn suggest(&self, cwd: &Path, git: Option<&GitInfo>) -> Result<Vec<Suggestion>> {
        let base_dir = git.map(|g| g.root.as_path()).unwrap_or(cwd);
        let base = slugify(&base_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        let parent =
            base_dir.parent().and_then(|p| p.file_name()).map(|n| slugify(&n.to_string_lossy())).unwrap_or_default();
        let squash = |s: &str| s.replace('-', "");
        let tokens = |s: &str| s.split('-').filter(|t| t.len() >= 3).map(String::from).collect::<HashSet<_>>();
        let mut out: Vec<(u8, Suggestion)> = vec![];
        let all: Vec<Project> = self.projects()?.into_iter().filter(|p| p.meta.key != GLOBAL).collect();
        for p in &all {
            let names = [p.slug.clone(), slugify(&p.meta.name)];
            let root_bases: Vec<String> = p
                .meta
                .roots
                .iter()
                .filter_map(|r| Path::new(r).file_name().map(|n| slugify(&n.to_string_lossy())))
                .collect();
            let mut score = 0u8;
            let mut reason = String::new();
            for cand in [&base, &parent] {
                if cand.is_empty() {
                    continue;
                }
                for n in names.iter().chain(root_bases.iter()) {
                    let (a, b) = (squash(cand), squash(n));
                    let (s, r) = if a == b {
                        (3, format!("same name as '{cand}'"))
                    } else if a.len() >= 3 && b.len() >= 3 && (a.contains(&b) || b.contains(&a)) {
                        (2, format!("name similar to '{cand}'"))
                    } else if !tokens(cand).is_disjoint(&tokens(n)) {
                        (1, format!("shares a word with '{cand}'"))
                    } else {
                        (0, String::new())
                    };
                    if s > score {
                        score = s;
                        reason = r;
                    }
                }
            }
            if score > 0 {
                out.push((
                    score,
                    Suggestion {
                        key: p.meta.key.clone(),
                        name: p.meta.name.clone(),
                        reason,
                        roots: p.meta.roots.clone(),
                    },
                ));
            }
        }
        out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.key.cmp(&b.1.key)));
        let mut v: Vec<Suggestion> = out.into_iter().map(|(_, s)| s).take(5).collect();
        if v.is_empty() && all.len() <= 8 {
            v = all
                .iter()
                .map(|p| Suggestion {
                    key: p.meta.key.clone(),
                    name: p.meta.name.clone(),
                    reason: "existing project".into(),
                    roots: p.meta.roots.clone(),
                })
                .collect();
        }
        Ok(v)
    }

    /// Root and remote to register for `cwd`: the git work tree top if inside one, else `cwd`.
    pub(crate) fn root_and_remote(&self, cwd: &Path) -> (String, Option<String>) {
        let cwd = canonical(cwd);
        match git::discover(&cwd) {
            Some(g) => (path_str(&g.root), g.remote),
            None => (path_str(&cwd), None),
        }
    }

    pub fn init_project(
        &mut self,
        name: &str,
        key: Option<&str>,
        description: &str,
        cwd: Option<&Path>,
    ) -> Result<Project> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("project name is empty".into()));
        }
        let existing: HashSet<String> = self.projects()?.into_iter().map(|p| p.meta.key).collect();
        let key = match key {
            Some(k) => {
                let k = k.trim().to_ascii_uppercase();
                if !is_valid_key(&k) || k == INBOX || k == GLOBAL {
                    return Err(Error::Invalid(format!(
                        "project key '{k}' must be 2-10 upper-case letters/digits, starting with a letter"
                    )));
                }
                if existing.contains(&k) {
                    return Err(Error::Conflict(format!("project key {k} is taken")));
                }
                k
            }
            None => unique_key(name, &existing),
        };
        let (mut roots, mut remotes) = (vec![], vec![]);
        if let Some(cwd) = cwd {
            let r = self.resolve(&Caller { cwd: cwd.to_path_buf(), ..Default::default() })?;
            if let Some(p) = r.project {
                return Err(Error::Conflict(format!(
                    "{} already belongs to project {} ({}); use `t9 project link` to move it",
                    display_path(Path::new(&r.cwd)),
                    p.meta.name,
                    p.meta.key
                )));
            }
            let (root, remote) = self.root_and_remote(cwd);
            roots.push(root);
            remotes.extend(remote);
        }
        let base_slug = {
            let s = slugify(name);
            if s.is_empty() { key.to_ascii_lowercase() } else { s }
        };
        let mut slug = base_slug.clone();
        let mut n = 2;
        while self.env.projects_dir().join(&slug).exists() {
            slug = format!("{base_slug}-{n}");
            n += 1;
        }
        let meta = ProjectMeta {
            key: key.clone(),
            name: name.into(),
            status: "active".into(),
            roots,
            remotes,
            created: now(),
            about_reviewed: None,
            extra: Default::default(),
        };
        let dir = self.env.projects_dir().join(&slug);
        for sub in ["items", "journal", "sessions"] {
            fs::create_dir_all(dir.join(sub)).map_err(|e| Error::io(&dir, e))?;
        }
        let body = if description.trim().is_empty() {
            format!("# {name}\n")
        } else {
            format!("# {name}\n\n{}\n", description.trim())
        };
        let file = dir.join("project.md");
        self.put(&file, &frontmatter::render(&meta, &body)?)?;
        self.sync_projects()?;
        self.project(&key)
    }

    /// Adds `cwd` (its git root) and git remote to a project, so worktrees and clones resolve to it.
    pub fn link_project(&mut self, key: &str, cwd: &Path) -> Result<Project> {
        let project = self.project(key)?;
        let key = project.meta.key.clone();
        let (root, remote) = self.root_and_remote(cwd);
        let _lock = self.lock(&key)?;
        let file = self.env.projects_dir().join(&project.slug).join("project.md");
        let (mut meta, body) = frontmatter::parse::<ProjectMeta>(&read(&file)?, &path_str(&file))?;
        // A directory belongs to one project: remove it from any other project that lists it.
        for other in self.projects()? {
            if other.meta.key != key && other.meta.roots.contains(&root) {
                self.unlink_root(&other, &root)?;
            }
        }
        if !meta.roots.contains(&root) {
            meta.roots.push(root);
        }
        if let Some(r) = remote
            && !meta.remotes.contains(&r)
        {
            meta.remotes.push(r);
        }
        self.put(&file, &frontmatter::render(&meta, &body)?)?;
        self.sync_projects()?;
        self.project(&key)
    }

    fn unlink_root(&self, project: &Project, root: &str) -> Result<()> {
        let file = self.env.projects_dir().join(&project.slug).join("project.md");
        let (mut meta, body) = frontmatter::parse::<ProjectMeta>(&read(&file)?, &path_str(&file))?;
        meta.roots.retain(|r| r != root);
        self.put(&file, &frontmatter::render(&meta, &body)?)
    }

    /// The project for a write: explicit, or resolved from cwd. `None` means the inbox.
    fn caller_project(&self, caller: &Caller) -> Result<Option<Project>> {
        Ok(self.resolve(caller)?.project)
    }

    // ------------------------------------------------------------ sessions

    fn session_path(&self, key: &str, meta: &SessionMeta) -> Result<PathBuf> {
        let stamp = meta.started.format("%Y%m%dT%H%MZ");
        let agent = slugify(&meta.agent);
        Ok(self
            .project_dir(key)?
            .join("sessions")
            .join(format!("{stamp}-{agent}-{}.md", short_id(&meta.id).to_ascii_lowercase())))
    }

    fn write_session(&self, meta: &SessionMeta, path: &Path, last_seen: Option<Timestamp>) -> Result<Session> {
        let body = fs::read_to_string(path)
            .ok()
            .and_then(|t| frontmatter::split(&t).map(|(_, b)| b.to_string()))
            .unwrap_or_else(|| format!("# Session {} · {}\n", short_id(&meta.id), meta.agent));
        let clean = self.write_file(path, &frontmatter::render(meta, &body)?)?;
        let s = Session { meta: meta.clone(), last_seen };
        index::upsert_session(&self.conn, &s, &path_str(path))?;
        self.record_written(path, &meta.project, "session", clean)?;
        Ok(s)
    }

    /// Starts a session, or resumes the one with the same agent and harness id (e.g. after compaction).
    pub fn start_session(&mut self, caller: &Caller, harness_id: Option<&str>) -> Result<Session> {
        let res = self.resolve(caller)?;
        let key = res.key().to_string();
        self.sync_project(&key)?;
        let agent = caller.agent.clone().unwrap_or_else(|| "unknown".into());
        let at = now();
        if let Some(h) = harness_id
            && let Some((s, path)) = index::session_by_harness(&self.conn, &agent, h)?
            && s.meta.project == key
        {
            let mut meta = s.meta;
            meta.ended = None;
            meta.cwd = res.cwd.clone();
            if let Some(g) = &res.git {
                meta.branch = g.branch.clone();
                meta.head = g.head.clone();
            }
            let _lock = self.lock(&key)?;
            return self.write_session(&meta, Path::new(&path), Some(at));
        }
        let meta = SessionMeta {
            id: new_id(),
            agent,
            harness_id: harness_id.map(String::from),
            project: key.clone(),
            cwd: res.cwd.clone(),
            branch: res.git.as_ref().and_then(|g| g.branch.clone()),
            head: res.git.as_ref().and_then(|g| g.head.clone()),
            focus: None,
            started: at,
            ended: None,
        };
        let path = self.session_path(&key, &meta)?;
        let _lock = self.lock(&key)?;
        self.write_session(&meta, &path, Some(at))
    }

    fn find_session(&self, id: &str) -> Result<(Session, String)> {
        if let Some(s) = index::session(&self.conn, id)? {
            return Ok(s);
        }
        match index::session_ids_by_suffix(&self.conn, id)?.as_slice() {
            [one] => Ok(index::session(&self.conn, one)?.expect("id just listed")),
            [] => Err(Error::NotFound(format!("session '{id}'"))),
            many => Err(Error::Invalid(format!("session '{id}' is ambiguous: {}", many.join(", ")))),
        }
    }

    /// The caller's session: explicit id, else the most recent open session in this directory (or a parent).
    pub fn current_session(&mut self, caller: &Caller) -> Result<Option<Session>> {
        if let Some(id) = &caller.session {
            return Ok(Some(self.find_session(id)?.0));
        }
        let res = self.resolve(caller)?;
        let since = now() - Duration::hours(SESSION_IDLE_HOURS);
        let found = index::open_sessions_for_cwd(&self.conn, &res.cwd, caller.agent.as_deref(), &since)?
            .into_iter()
            .find(|(s, _)| s.meta.project == res.key());
        Ok(found.map(|(s, _)| s))
    }

    fn session_for_write(&mut self, caller: &Caller) -> Result<Session> {
        match self.current_session(caller)? {
            Some(s) => {
                index::touch_session(&self.conn, &s.meta.id, &now())?;
                Ok(s)
            }
            None => self.start_session(caller, None),
        }
    }

    pub fn end_session(&mut self, caller: &Caller, handoff: Option<&str>) -> Result<Option<Session>> {
        let Some(s) = self.current_session(caller)? else { return Ok(None) };
        let caller = Caller { session: Some(s.meta.id.clone()), ..caller.clone() };
        if let Some(text) = handoff.filter(|t| !t.trim().is_empty()) {
            self.log(&caller, LogInput { kind: EntryKind::Handoff, body: text.into(), item: None, refs: vec![] })?;
        }
        let (s, path) = self.find_session(&s.meta.id)?;
        let mut meta = s.meta;
        meta.ended = Some(now());
        let _lock = self.lock(&meta.project)?;
        Ok(Some(self.write_session(&meta, Path::new(&path), Some(now()))?))
    }

    /// Sets the session's focus item; later logs attach to it. Activates an `open` item.
    pub fn focus(&mut self, caller: &Caller, item_id: Option<&str>, activate: bool) -> Result<(Session, Option<Item>)> {
        let item = match item_id {
            Some(id) => Some(self.item(id)?),
            None => None,
        };
        let s = self.session_for_write(caller)?;
        let (s, path) = self.find_session(&s.meta.id)?;
        let mut meta = s.meta;
        meta.focus = item.as_ref().map(|i| i.meta.id.clone());
        let session = {
            let _lock = self.lock(&meta.project)?;
            self.write_session(&meta, Path::new(&path), Some(now()))?
        };
        let caller = Caller { session: Some(session.meta.id.clone()), ..caller.clone() };
        let item = match item {
            Some(i) if activate && i.meta.status == Status::Open => Some(self.update_item(
                &caller,
                &i.meta.id,
                ItemPatch { status: Some(Status::Active), ..Default::default() },
            )?),
            other => other,
        };
        Ok((session, item))
    }

    pub fn sessions(&self, project: Option<&str>, open_only: bool, limit: usize) -> Result<Vec<Session>> {
        index::sessions(&self.conn, project, open_only, limit)
    }

    // ------------------------------------------------------------ items

    /// Finds an item by id, syncing its project first so the result reflects the file on disk.
    pub fn item(&mut self, id: &str) -> Result<Item> {
        let id = normalize_item_id(id)?;
        let (key, _) = split_item_id(&id).expect("normalized");
        if index::project(&self.conn, key)?.is_some() {
            self.sync_project(key)?;
        }
        if let Some(found) = index::item(&self.conn, &id)? {
            self.refresh_file(key, "item", &found.path)?;
        }
        index::item(&self.conn, &id)?.ok_or_else(|| Error::NotFound(format!("item {id}")))
    }

    pub fn items(&self, f: &ItemFilter) -> Result<Vec<Item>> {
        let t = std::time::Instant::now();
        let r = index::items(&self.conn, f);
        crate::trace("items query", t);
        r
    }

    pub fn add_item(&mut self, caller: &Caller, ni: NewItem) -> Result<Item> {
        let kind = ni.kind.unwrap_or(Kind::Task);
        let title = ni.title.trim().to_string();
        if title.is_empty() {
            return Err(Error::Invalid("title is empty".into()));
        }
        refuse_secrets([Some(&title), Some(&ni.body), ni.location.as_ref(), ni.owner.as_ref(), ni.goal.as_ref()])?;
        let parent = match &ni.parent {
            Some(p) => Some(self.item(p)?),
            None => None,
        };
        let project = match (&parent, &caller.project) {
            (Some(p), None) => self.project(&p.project)?,
            _ => self.caller_project(caller)?.ok_or_else(|| {
                Error::Invalid(
                    "this directory is not linked to a project: run `t9 context` for suggestions, then \
                     `t9 project link KEY` or `t9 project init --name NAME` (or pass --project KEY)"
                        .into(),
                )
            })?,
        };
        let key = project.meta.key.clone();
        self.sync_project(&key)?;
        for l in &ni.links {
            self.item(&l.to)?;
        }
        let session = self.session_for_write(&Caller { project: Some(key.clone()), ..caller.clone() }).ok();
        let at = now();
        let dir = self.env.projects_dir().join(&project.slug).join("items");
        let _lock = self.lock(&key)?;
        let on_disk = md_files(&dir)
            .iter()
            .filter_map(|p| file_item_num(&p.file_name()?.to_string_lossy(), &key))
            .max()
            .unwrap_or(0);
        let num = on_disk.max(index::max_item_num(&self.conn, &key)?) + 1;
        let id = format!("{key}-{num}");
        let meta = ItemMeta {
            id: id.clone(),
            kind,
            title: title.clone(),
            status: ni.status.unwrap_or(kind.default_status()),
            parent: parent.map(|p| p.meta.id),
            priority: ni.priority,
            tags: ni.tags,
            areas: normalize_areas(&ni.areas),
            goal: clean_opt(ni.goal),
            due: ni.due,
            hypothesis: clean_opt(ni.hypothesis),
            retry_when: clean_opt(ni.retry_when),
            retry_after: ni.retry_after,
            resource_type: clean_opt(ni.resource_type).map(|t| slugify(&t)),
            location: clean_opt(ni.location),
            owner: clean_opt(ni.owner),
            links: ni.links,
            created: at,
            updated: at,
            created_by: session.as_ref().map(|s| s.meta.agent.clone()).or(caller.agent.clone()).unwrap_or_default(),
            created_in: session.map(|s| s.meta.id),
            rev: 1,
            extra: Default::default(),
        };
        let path = dir.join(item_file_name(&id, &title));
        let body = if ni.body.trim().is_empty() { String::new() } else { format!("{}\n", ni.body.trim_end()) };
        let clean = self.write_file(&path, &frontmatter::render(&meta, &body)?)?;
        let item = Item { meta, project: key.clone(), body, path: path_str(&path) };
        index::upsert_item(&self.conn, &item)?;
        self.record_written(&path, &key, "item", clean)?;
        Ok(item)
    }

    pub fn update_item(&mut self, caller: &Caller, id: &str, patch: ItemPatch) -> Result<Item> {
        let current = self.item(id)?;
        let key = current.project.clone();
        if let Some(Some(p)) = &patch.parent {
            // Reject cycles: the new parent must not be this item or one of its descendants.
            let mut cursor = Some(normalize_item_id(p)?);
            while let Some(c) = cursor {
                if c == current.meta.id {
                    return Err(Error::Invalid(format!("{p} is {} or one of its children", current.meta.id)));
                }
                cursor = self.item(&c)?.meta.parent;
            }
        }
        for l in &patch.add_links {
            self.item(&l.to)?;
        }
        let path = PathBuf::from(&current.path);
        let (old_status, item) = {
            let _lock = self.lock(&key)?;
            let (mut meta, mut body) = frontmatter::parse::<ItemMeta>(&read(&path)?, &current.path)?;
            if let Some(rev) = patch.expect_rev
                && rev != meta.rev
            {
                return Err(Error::Conflict(format!(
                    "{} is at rev {}, not {rev}; re-read it and retry",
                    meta.id, meta.rev
                )));
            }
            let old_status = meta.status;
            if let Some(t) = patch.title.filter(|t| !t.trim().is_empty()) {
                meta.title = t.trim().into();
            }
            if let Some(s) = patch.status {
                meta.status = s;
            }
            if let Some(p) = patch.parent {
                meta.parent = p.map(|p| normalize_item_id(&p)).transpose()?;
            }
            if let Some(p) = patch.priority {
                meta.priority = p;
            }
            refuse_secrets([patch.body.as_ref(), patch.location.as_ref(), Some(&meta.title), patch.goal.as_ref()])?;
            if let Some(v) = patch.goal {
                meta.goal = clean_opt(Some(v));
            }
            if let Some(v) = patch.due {
                meta.due = v;
            }
            if let Some(v) = patch.hypothesis {
                meta.hypothesis = clean_opt(Some(v));
            }
            if let Some(v) = patch.retry_when {
                meta.retry_when = clean_opt(Some(v));
            }
            if let Some(v) = patch.retry_after {
                meta.retry_after = v;
            }
            if let Some(v) = patch.resource_type {
                meta.resource_type = clean_opt(Some(v)).map(|t| slugify(&t));
            }
            if let Some(v) = patch.location {
                meta.location = clean_opt(Some(v));
            }
            if let Some(v) = patch.owner {
                meta.owner = clean_opt(Some(v));
            }
            for a in normalize_areas(&patch.add_areas) {
                if !meta.areas.contains(&a) {
                    meta.areas.push(a);
                }
            }
            for t in patch.add_tags {
                if !meta.tags.contains(&t) {
                    meta.tags.push(t);
                }
            }
            meta.tags.retain(|t| !patch.remove_tags.contains(t));
            for l in patch.add_links {
                if !meta.links.contains(&l) {
                    meta.links.push(l);
                }
            }
            if let Some(b) = patch.body {
                body = if b.trim().is_empty() { String::new() } else { format!("{}\n", b.trim_end()) };
            }
            meta.rev += 1;
            meta.updated = now();
            let clean = self.write_file(&path, &frontmatter::render(&meta, &body)?)?;
            let item = Item { meta, project: key.clone(), body, path: current.path.clone() };
            index::upsert_item(&self.conn, &item)?;
            self.record_written(&path, &key, "item", clean)?;
            (old_status, item)
        };
        if old_status != item.meta.status {
            let mut text = format!("{}: {old_status} → {}", item.meta.id, item.meta.status);
            if let Some(n) = patch.note.filter(|n| !n.trim().is_empty()) {
                text.push_str(" — ");
                text.push_str(n.trim());
            }
            self.log(
                caller,
                LogInput { kind: EntryKind::StatusChange, body: text, item: Some(item.meta.id.clone()), refs: vec![] },
            )?;
        }
        Ok(item)
    }

    // ------------------------------------------------------------ journal

    fn journal_path(&self, key: &str, at: &Timestamp) -> Result<PathBuf> {
        let date = at.with_timezone(&Local).format("%Y-%m-%d");
        Ok(self.project_dir(key)?.join("journal").join(format!("{date}.md")))
    }

    /// Appends a block to a project's journal and indexes it.
    fn append_entry(&self, key: &str, meta: EntryMeta, body: &str) -> Result<Entry> {
        let path = self.journal_path(key, &meta.at)?;
        let pstr = path_str(&path);
        let _lock = self.lock(key)?;
        let before = file_sig(&path);
        let stale = before.is_some() && index::file_state(&self.conn, &pstr)? != before;
        let clean = self.dir_clean(&path)?;
        self.wrote.set(true);
        let mut f = OpenOptions::new().create(true).append(true).open(&path).map_err(|e| Error::io(&path, e))?;
        if before.is_none() {
            let name = if key == INBOX { "Inbox".to_string() } else { self.project(key)?.meta.name };
            let date = meta.at.with_timezone(&Local).format("%Y-%m-%d").to_string();
            f.write_all(journal::file_header(&name, &date).as_bytes()).map_err(|e| Error::io(&path, e))?;
        }
        f.write_all(journal::render_block(&meta, body)?.as_bytes()).map_err(|e| Error::io(&path, e))?;
        drop(f);
        let entry = Entry { meta, project: key.into(), body: body.trim().into() };
        if stale {
            // Someone edited the file outside t9: re-read all of it.
            index::forget_file(&self.conn, &pstr)?;
            let (blocks, _) = journal::parse(&read(&path)?);
            for (m, b) in blocks {
                index::insert_entry(&self.conn, &Entry { meta: m, project: key.into(), body: b }, &pstr)?;
            }
        } else {
            index::insert_entry(&self.conn, &entry, &pstr)?;
        }
        self.record_written(&path, key, "journal", clean)?;
        Ok(entry)
    }

    /// Writes a journal entry. Never fails because the project is unknown: such entries go to the inbox.
    pub fn log(&mut self, caller: &Caller, input: LogInput) -> Result<Entry> {
        if input.body.trim().is_empty() {
            return Err(Error::Invalid("message is empty".into()));
        }
        let explicit_item = match &input.item {
            Some(id) => Some(self.item(id)?),
            None => None,
        };
        let res = self.resolve(caller)?;
        let caller_key = res.key().to_string();
        // The session belongs to the caller's project; an explicit item may belong to another one. Writes from
        // unlinked directories reuse an open session but never start one, so the inbox collects no sessions.
        let session = if res.project.is_some() || caller.session.is_some() {
            Some(self.session_for_write(caller)?)
        } else {
            self.current_session(caller)?
        };
        let focus = session.as_ref().filter(|s| s.meta.project == caller_key).and_then(|s| s.meta.focus.clone());
        let item = match explicit_item {
            Some(i) => Some(i),
            // Entries do not attach to a focus that has since been closed.
            None => focus.and_then(|f| self.item(&f).ok()).filter(|i| !i.meta.status.is_closed()),
        };
        let key = item.as_ref().map(|i| i.project.clone()).unwrap_or(caller_key);
        let meta = EntryMeta {
            id: new_id(),
            kind: input.kind,
            at: now(),
            item: item.map(|i| i.meta.id),
            session: session.as_ref().map(|s| s.meta.id.clone()),
            agent: session.map(|s| s.meta.agent).or(caller.agent.clone()).unwrap_or_else(|| "unknown".into()),
            cwd: (key == INBOX).then(|| res.cwd.clone()),
            refs: input.refs,
        };
        if key == INBOX {
            self.note_unlinked(Path::new(&res.cwd), &meta.agent)?;
        }
        self.append_entry(&key, meta, &input.body)
    }

    /// Moves an entry to another project's journal (same id and timestamp), optionally filing it under an item.
    fn move_entry(&mut self, entry: &Entry, file: &str, key: &str, item: Option<String>) -> Result<Entry> {
        let mut meta = entry.meta.clone();
        meta.item = item;
        meta.cwd = None;
        let moved = self.append_entry(key, meta, &entry.body)?;
        let file_path = PathBuf::from(file);
        let _lock = self.lock(&entry.project)?;
        let mut clean = false;
        if let Some(new) = journal::remove_block(&read(&file_path)?, &entry.meta.id) {
            clean = self.write_file(&file_path, &new)?;
        }
        self.record_written(&file_path, &entry.project, "journal", clean)?;
        Ok(moved)
    }

    /// Moves inbox entries written from inside the project's roots into its journal (unfiled). Called after
    /// `init`/`link`, so notes taken before the directory was linked are not lost in the inbox.
    pub fn adopt_inbox(&mut self, key: &str) -> Result<usize> {
        self.sync_project(INBOX)?;
        let inbox = self.entries(&EntryFilter { project: Some(INBOX.into()), ..Default::default() })?;
        let mut moved = 0;
        for e in inbox {
            let Some(cwd) = e.meta.cwd.clone() else { continue };
            let res = self.resolve(&Caller { cwd: PathBuf::from(cwd), ..Default::default() })?;
            if res.key() != key {
                continue;
            }
            let (_, file) = self.find_entry(&e.meta.id)?;
            self.move_entry(&e, &file, key, None)?;
            moved += 1;
        }
        Ok(moved)
    }

    pub fn entries(&self, f: &EntryFilter) -> Result<Vec<Entry>> {
        index::entries(&self.conn, f)
    }

    pub fn count_entries(&self, f: &EntryFilter) -> Result<usize> {
        index::count_entries(&self.conn, f)
    }

    /// An entry by id or id suffix.
    pub fn entry(&self, id: &str) -> Result<Entry> {
        Ok(self.find_entry(id)?.0)
    }

    fn find_entry(&self, id: &str) -> Result<(Entry, String)> {
        let id = id.trim().to_ascii_uppercase();
        if let Some(e) = index::entry(&self.conn, &id)? {
            return Ok(e);
        }
        match index::entries_by_suffix(&self.conn, &id)?.as_slice() {
            [one] => Ok(index::entry(&self.conn, one)?.expect("id just listed")),
            [] => Err(Error::NotFound(format!("entry '{id}'"))),
            many => Err(Error::Invalid(format!("entry '{id}' is ambiguous: {}", many.join(", ")))),
        }
    }

    /// Files an entry under an item, moving it to the item's project journal if needed (e.g. from the inbox).
    pub fn attach(&mut self, entry_id: &str, item_id: &str) -> Result<Entry> {
        let item = self.item(item_id)?;
        let (entry, file) = self.find_entry(entry_id)?;
        let mut meta = entry.meta.clone();
        meta.item = Some(item.meta.id.clone());
        let file_path = PathBuf::from(&file);
        if entry.project == item.project {
            let _lock = self.lock(&item.project)?;
            let new = journal::rewrite_header(&read(&file_path)?, &meta)?
                .ok_or_else(|| Error::NotFound(format!("entry {} in {file}", meta.id)))?;
            let clean = self.write_file(&file_path, &new)?;
            index::insert_entry(
                &self.conn,
                &Entry { meta: meta.clone(), project: item.project.clone(), body: entry.body.clone() },
                &file,
            )?;
            self.record_written(&file_path, &item.project, "journal", clean)?;
            return Ok(Entry { meta, project: item.project, body: entry.body });
        }
        self.move_entry(&entry, &file, &item.project, Some(item.meta.id.clone()))
    }

    // ------------------------------------------------------------ search

    pub fn search(&self, q: &str, project: Option<&str>, limit: usize) -> Result<Vec<index::Hit>> {
        index::search(&self.conn, q, project, limit)
    }

    pub fn last_activity(&self, item_id: &str) -> Result<Option<String>> {
        index::last_entry_at(&self.conn, item_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(derive_key("Billing Service"), "BS");
        assert_eq!(derive_key("Payments"), "PAY");
        assert_eq!(derive_key("9lives"), "P9LI");
        assert_eq!(derive_key("a"), "AX");
    }

    #[test]
    fn file_names() {
        assert_eq!(item_file_name("BIL-1", "Fix: the thing!"), "BIL-1-fix-the-thing.md");
        assert_eq!(file_item_num("BIL-12-fix.md", "BIL"), Some(12));
        assert_eq!(file_item_num("QRS-12-fix.md", "BIL"), None);
    }

    #[test]
    fn within() {
        assert!(is_within("/a/b", "/a/b"));
        assert!(is_within("/a/b/c", "/a/b"));
        assert!(!is_within("/a/bc", "/a/b"));
    }
}
