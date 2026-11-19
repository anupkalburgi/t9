//! SQLite index over the vault. Derived data only: deleting `index.db` loses nothing but `last_seen` times.
//!
//! Each row stores the full front matter as JSON (`meta`) plus the columns needed for filtering, so reads
//! return exactly what the file holds. Functions take `&Connection` so they work inside transactions.

use std::path::Path;

use chrono::SecondsFormat;
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};

use crate::model::*;
use crate::{Error, Result};

const SCHEMA_VERSION: i64 = 4;

const SCHEMA: &str = r#"
CREATE TABLE projects(
  key TEXT PRIMARY KEY, slug TEXT NOT NULL UNIQUE, name TEXT NOT NULL, meta TEXT NOT NULL, description TEXT NOT NULL
);
CREATE TABLE project_roots(key TEXT NOT NULL, root TEXT NOT NULL, PRIMARY KEY(key, root));
CREATE TABLE project_remotes(key TEXT NOT NULL, remote TEXT NOT NULL, PRIMARY KEY(key, remote));
CREATE INDEX project_remotes_remote ON project_remotes(remote);

-- `rid` is a stable integer key for the external-content FTS tables.
CREATE TABLE items(
  rid INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE, project TEXT NOT NULL, num INTEGER NOT NULL, kind TEXT NOT NULL, title TEXT NOT NULL,
  status TEXT NOT NULL, parent TEXT, priority INTEGER, updated TEXT NOT NULL,
  meta TEXT NOT NULL, body TEXT NOT NULL, path TEXT NOT NULL
);
CREATE INDEX items_project ON items(project, status);
CREATE INDEX items_parent ON items(parent);
CREATE INDEX items_path ON items(path);

CREATE TABLE entries(
  rid INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE, project TEXT NOT NULL, item TEXT, session TEXT, kind TEXT NOT NULL, at TEXT NOT NULL,
  meta TEXT NOT NULL, body TEXT NOT NULL, file TEXT NOT NULL
);
CREATE INDEX entries_project_at ON entries(project, at, id);
CREATE INDEX entries_project_kind ON entries(project, kind, at, id);
CREATE INDEX entries_kind_at ON entries(kind, at, id);
CREATE INDEX entries_item ON entries(item);
CREATE INDEX entries_session ON entries(session);
CREATE INDEX entries_file ON entries(file);

CREATE TABLE sessions(
  id TEXT PRIMARY KEY, project TEXT NOT NULL, agent TEXT NOT NULL, harness_id TEXT, cwd TEXT NOT NULL,
  started TEXT NOT NULL, ended TEXT, last_seen TEXT, meta TEXT NOT NULL, path TEXT NOT NULL
);
CREATE INDEX sessions_project ON sessions(project, ended);
CREATE INDEX sessions_harness ON sessions(agent, harness_id);

-- mtime/size of every vault file the index has read, to detect edits made outside t9.
CREATE TABLE files(path TEXT PRIMARY KEY, project TEXT NOT NULL, kind TEXT NOT NULL, mtime INTEGER NOT NULL, size INTEGER NOT NULL);
CREATE INDEX files_project ON files(project);

-- Full-text search: external-content FTS5 kept in sync by insert, update and delete triggers. Rows are written
-- with a single `INSERT … ON CONFLICT DO UPDATE` (atomic, so concurrent writers never race) and never REPLACEd
-- (REPLACE does not fire delete triggers).
CREATE VIRTUAL TABLE items_fts USING fts5(title, body, content = 'items', content_rowid = 'rid', tokenize = 'porter unicode61');
CREATE TRIGGER items_ai AFTER INSERT ON items BEGIN
  INSERT INTO items_fts(rowid, title, body) VALUES (new.rid, new.title, new.body);
END;
CREATE TRIGGER items_ad AFTER DELETE ON items BEGIN
  INSERT INTO items_fts(items_fts, rowid, title, body) VALUES ('delete', old.rid, old.title, old.body);
END;
CREATE TRIGGER items_au AFTER UPDATE ON items BEGIN
  INSERT INTO items_fts(items_fts, rowid, title, body) VALUES ('delete', old.rid, old.title, old.body);
  INSERT INTO items_fts(rowid, title, body) VALUES (new.rid, new.title, new.body);
END;
CREATE VIRTUAL TABLE entries_fts USING fts5(body, content = 'entries', content_rowid = 'rid', tokenize = 'porter unicode61');
-- Title matches weigh 4x body matches.
INSERT INTO items_fts(items_fts, rank) VALUES ('rank', 'bm25(4.0, 1.0)');
CREATE TRIGGER entries_ai AFTER INSERT ON entries BEGIN
  INSERT INTO entries_fts(rowid, body) VALUES (new.rid, new.body);
END;
CREATE TRIGGER entries_ad AFTER DELETE ON entries BEGIN
  INSERT INTO entries_fts(entries_fts, rowid, body) VALUES ('delete', old.rid, old.body);
END;
CREATE TRIGGER entries_au AFTER UPDATE ON entries BEGIN
  INSERT INTO entries_fts(entries_fts, rowid, body) VALUES ('delete', old.rid, old.body);
  INSERT INTO entries_fts(rowid, body) VALUES (new.rid, new.body);
END;
"#;

/// Opens (creating if needed) the index. Returns `true` if it is new or was reset and must be rebuilt.
pub fn open(path: &Path) -> Result<(Connection, bool)> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version == SCHEMA_VERSION {
        return Ok((conn, false));
    }
    // Derived data: on any version mismatch, drop everything and rebuild from the vault.
    conn.execute_batch("BEGIN IMMEDIATE")?;
    // Another process may have built the schema while we waited for the lock.
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version == SCHEMA_VERSION {
        conn.execute_batch("COMMIT")?;
        return Ok((conn, false));
    }
    reset(&conn)?;
    conn.execute_batch("COMMIT")?;
    Ok((conn, true))
}

const TABLES: &[&str] = &[
    "projects",
    "project_roots",
    "project_remotes",
    "items_fts",
    "entries_fts",
    "fts",
    "items",
    "entries",
    "sessions",
    "files",
];

/// Drops and recreates every table (inside the caller's transaction).
fn reset(c: &Connection) -> Result<()> {
    for t in TABLES {
        c.execute_batch(&format!("DROP TABLE IF EXISTS {t};"))?;
    }
    c.execute_batch(SCHEMA)?;
    c.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

/// Empties the index before a rebuild.
pub fn clear(c: &Connection) -> Result<()> {
    c.execute_batch("BEGIN IMMEDIATE")?;
    reset(c)?;
    c.execute_batch("COMMIT")?;
    Ok(())
}

pub fn ts(t: &Timestamp) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).expect("model types always serialize")
}

fn from_json<T: serde::de::DeserializeOwned>(s: &str) -> rusqlite::Result<T> {
    serde_json::from_str(s)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, e.into()))
}

// ---------------------------------------------------------------- files

/// All file states of a project in one query (used by the sync check).
pub fn file_states(c: &Connection, project: &str) -> Result<std::collections::HashMap<String, (i64, i64)>> {
    let mut st = c.prepare_cached("SELECT path, mtime, size FROM files WHERE project = ?")?;
    let rows = st.query_map([project], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn file_state(c: &Connection, path: &str) -> Result<Option<(i64, i64)>> {
    Ok(c.prepare_cached("SELECT mtime, size FROM files WHERE path = ?")?
        .query_row([path], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?)
}

pub fn set_file_state(c: &Connection, path: &str, project: &str, kind: &str, mtime: i64, size: i64) -> Result<()> {
    c.prepare_cached("INSERT OR REPLACE INTO files(path, project, kind, mtime, size) VALUES (?,?,?,?,?)")?
        .execute(params![path, project, kind, mtime, size])?;
    Ok(())
}

pub fn project_files(c: &Connection, project: &str) -> Result<Vec<(String, String)>> {
    let mut st = c.prepare_cached("SELECT path, kind FROM files WHERE project = ?")?;
    let rows = st.query_map([project], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Removes a file and every row derived from it.
pub fn forget_file(c: &Connection, path: &str) -> Result<()> {
    c.execute("DELETE FROM entries WHERE file = ?1", [path])?;
    c.execute("DELETE FROM items WHERE path = ?1", [path])?;
    c.execute("DELETE FROM sessions WHERE path = ?1", [path])?;
    c.execute("DELETE FROM files WHERE path = ?1", [path])?;
    Ok(())
}

// ---------------------------------------------------------------- projects

pub fn upsert_project(c: &Connection, p: &Project) -> Result<()> {
    let key = &p.meta.key;
    c.execute("DELETE FROM project_roots WHERE key = ?", [key])?;
    c.execute("DELETE FROM project_remotes WHERE key = ?", [key])?;
    c.execute(
        "INSERT OR REPLACE INTO projects(key, slug, name, meta, description) VALUES (?,?,?,?,?)",
        params![key, p.slug, p.meta.name, json(&p.meta), p.description],
    )?;
    for root in &p.meta.roots {
        c.execute("INSERT OR IGNORE INTO project_roots(key, root) VALUES (?,?)", params![key, root])?;
    }
    for remote in &p.meta.remotes {
        c.execute("INSERT OR IGNORE INTO project_remotes(key, remote) VALUES (?,?)", params![key, remote])?;
    }
    Ok(())
}

pub fn delete_project(c: &Connection, key: &str) -> Result<()> {
    for path in project_files(c, key)? {
        forget_file(c, &path.0)?;
    }
    c.execute("DELETE FROM project_roots WHERE key = ?", [key])?;
    c.execute("DELETE FROM project_remotes WHERE key = ?", [key])?;
    c.execute("DELETE FROM projects WHERE key = ?", [key])?;
    Ok(())
}

fn project_row(r: &Row) -> rusqlite::Result<Project> {
    Ok(Project { slug: r.get(0)?, meta: from_json(&r.get::<_, String>(1)?)?, description: r.get(2)? })
}

pub fn projects(c: &Connection) -> Result<Vec<Project>> {
    let mut st = c.prepare_cached("SELECT slug, meta, description FROM projects ORDER BY name")?;
    let rows = st.query_map([], project_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn project(c: &Connection, key: &str) -> Result<Option<Project>> {
    Ok(c.prepare_cached("SELECT slug, meta, description FROM projects WHERE key = ?")?
        .query_row([key], project_row)
        .optional()?)
}

pub fn project_by_slug(c: &Connection, slug: &str) -> Result<Option<Project>> {
    Ok(c.prepare_cached("SELECT slug, meta, description FROM projects WHERE slug = ?")?
        .query_row([slug], project_row)
        .optional()?)
}

pub fn all_roots(c: &Connection) -> Result<Vec<(String, String)>> {
    let mut st = c.prepare_cached("SELECT key, root FROM project_roots")?;
    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn project_for_remote(c: &Connection, remote: &str) -> Result<Vec<String>> {
    let mut st = c.prepare_cached("SELECT key FROM project_remotes WHERE remote = ? ORDER BY key")?;
    let rows = st.query_map([remote], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

// ---------------------------------------------------------------- items

pub fn upsert_item(c: &Connection, it: &Item) -> Result<()> {
    let m = &it.meta;
    let num = split_item_id(&m.id).map(|(_, n)| n).unwrap_or(0);
    c.prepare_cached(
        "INSERT INTO items(id, project, num, kind, title, status, parent, priority, updated, meta, body, path)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
         ON CONFLICT(id) DO UPDATE SET project=?2, num=?3, kind=?4, title=?5, status=?6, parent=?7, priority=?8,
           updated=?9, meta=?10, body=?11, path=?12",
    )?
    .execute(params![
        m.id,
        it.project,
        num,
        m.kind.as_str(),
        m.title,
        m.status.as_str(),
        m.parent,
        m.priority,
        ts(&m.updated),
        json(m),
        it.body,
        it.path
    ])?;
    Ok(())
}

const ITEM_COLS: &str = "meta, project, body, path";

fn item_row(r: &Row) -> rusqlite::Result<Item> {
    Ok(Item { meta: from_json(&r.get::<_, String>(0)?)?, project: r.get(1)?, body: r.get(2)?, path: r.get(3)? })
}

pub fn item(c: &Connection, id: &str) -> Result<Option<Item>> {
    Ok(c.prepare_cached(&format!("SELECT {ITEM_COLS} FROM items WHERE id = ?"))?
        .query_row([id], item_row)
        .optional()?)
}

#[derive(Debug, Default, Clone)]
pub struct ItemFilter {
    pub project: Option<String>,
    pub kinds: Vec<Kind>,
    pub statuses: Vec<Status>,
    /// When `statuses` is empty: include done/dropped/superseded items?
    pub include_closed: bool,
    /// `Some(None)`: only top-level items. `Some(Some(id))`: children of `id`.
    pub parent: Option<Option<String>>,
    pub limit: Option<usize>,
}

pub fn items(c: &Connection, f: &ItemFilter) -> Result<Vec<Item>> {
    let mut sql = format!("SELECT {ITEM_COLS} FROM items WHERE 1=1");
    let mut args: Vec<String> = Vec::new();
    if let Some(p) = &f.project {
        sql.push_str(" AND project = ?");
        args.push(p.clone());
    }
    let in_list = |sql: &mut String, args: &mut Vec<String>, col: &str, vals: Vec<&str>| {
        sql.push_str(&format!(" AND {col} IN ({})", vec!["?"; vals.len()].join(",")));
        args.extend(vals.into_iter().map(String::from));
    };
    if !f.kinds.is_empty() {
        in_list(&mut sql, &mut args, "kind", f.kinds.iter().map(|k| k.as_str()).collect());
    }
    if !f.statuses.is_empty() {
        in_list(&mut sql, &mut args, "status", f.statuses.iter().map(|s| s.as_str()).collect());
    } else if !f.include_closed {
        sql.push_str(" AND status NOT IN ('done','dropped','superseded')");
    }
    match &f.parent {
        Some(Some(p)) => {
            sql.push_str(" AND parent = ?");
            args.push(p.clone());
        }
        Some(None) => sql.push_str(" AND parent IS NULL"),
        None => {}
    }
    sql.push_str(" ORDER BY project, coalesce(priority, 99), num");
    if let Some(l) = f.limit {
        sql.push_str(&format!(" LIMIT {l}"));
    }
    let mut st = c.prepare_cached(&sql)?;
    let rows = st.query_map(params_from_iter(args), item_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn max_item_num(c: &Connection, project: &str) -> Result<u32> {
    Ok(c.prepare_cached("SELECT coalesce(max(num), 0) FROM items WHERE project = ?")?
        .query_row([project], |r| r.get(0))?)
}

// ---------------------------------------------------------------- entries

pub fn insert_entry(c: &Connection, e: &Entry, file: &str) -> Result<()> {
    let m = &e.meta;
    c.prepare_cached(
        "INSERT INTO entries(id, project, item, session, kind, at, meta, body, file) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
         ON CONFLICT(id) DO UPDATE SET project=?2, item=?3, session=?4, kind=?5, at=?6, meta=?7, body=?8, file=?9",
    )?
    .execute(params![m.id, e.project, m.item, m.session, m.kind.as_str(), ts(&m.at), json(m), e.body, file])?;
    Ok(())
}

fn entry_row(r: &Row) -> rusqlite::Result<Entry> {
    Ok(Entry { meta: from_json(&r.get::<_, String>(0)?)?, project: r.get(1)?, body: r.get(2)? })
}

pub fn entry(c: &Connection, id: &str) -> Result<Option<(Entry, String)>> {
    Ok(c.prepare_cached("SELECT meta, project, body, file FROM entries WHERE id = ?")?
        .query_row([id], |r| Ok((entry_row(r)?, r.get(3)?)))
        .optional()?)
}

pub fn entries_by_suffix(c: &Connection, suffix: &str) -> Result<Vec<String>> {
    let mut st = c.prepare_cached("SELECT id FROM entries WHERE id LIKE ? LIMIT 5")?;
    let rows = st.query_map([format!("%{}", suffix.to_ascii_uppercase())], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[derive(Debug, Default, Clone)]
pub struct EntryFilter {
    pub project: Option<String>,
    pub item: Option<String>,
    pub session: Option<String>,
    pub kinds: Vec<EntryKind>,
    pub exclude_kinds: Vec<EntryKind>,
    pub unfiled: bool,
    pub since: Option<Timestamp>,
    pub limit: Option<usize>,
}

/// Newest first.
pub fn entries(c: &Connection, f: &EntryFilter) -> Result<Vec<Entry>> {
    let (w, args) = entry_where(f);
    let mut sql = format!("SELECT meta, project, body FROM entries{w}");
    sql.push_str(" ORDER BY at DESC, id DESC");
    if let Some(l) = f.limit {
        sql.push_str(&format!(" LIMIT {l}"));
    }
    let mut st = c.prepare_cached(&sql)?;
    let rows = st.query_map(params_from_iter(args), entry_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Builds the WHERE clause shared by `entries` and `count_entries`.
fn entry_where(f: &EntryFilter) -> (String, Vec<String>) {
    let mut sql = String::from(" WHERE 1=1");
    let mut args: Vec<String> = Vec::new();
    for (col, val) in [("project", &f.project), ("item", &f.item), ("session", &f.session)] {
        if let Some(v) = val {
            sql.push_str(&format!(" AND {col} = ?"));
            args.push(v.clone());
        }
    }
    for (op, kinds) in [("IN", &f.kinds), ("NOT IN", &f.exclude_kinds)] {
        if !kinds.is_empty() {
            sql.push_str(&format!(" AND kind {op} ({})", vec!["?"; kinds.len()].join(",")));
            args.extend(kinds.iter().map(|k| k.as_str().to_string()));
        }
    }
    if f.unfiled {
        sql.push_str(" AND item IS NULL");
    }
    if let Some(s) = &f.since {
        sql.push_str(" AND at >= ?");
        args.push(ts(s));
    }
    (sql, args)
}

pub fn count_entries(c: &Connection, f: &EntryFilter) -> Result<usize> {
    let (w, args) = entry_where(f);
    let n: i64 = c
        .prepare_cached(&format!("SELECT count(*) FROM entries{w}"))?
        .query_row(params_from_iter(args), |r| r.get(0))?;
    Ok(n as usize)
}

pub fn last_entry_at(c: &Connection, item: &str) -> Result<Option<String>> {
    Ok(c.prepare_cached("SELECT max(at) FROM entries WHERE item = ?")?.query_row([item], |r| r.get(0))?)
}

// ---------------------------------------------------------------- sessions

pub fn upsert_session(c: &Connection, s: &Session, path: &str) -> Result<()> {
    let m = &s.meta;
    let last_seen = s.last_seen.as_ref().map(ts);
    c.prepare_cached(
        "INSERT INTO sessions(id, project, agent, harness_id, cwd, started, ended, last_seen, meta, path)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
         ON CONFLICT(id) DO UPDATE SET project=?2, agent=?3, harness_id=?4, cwd=?5, started=?6, ended=?7,
           last_seen=coalesce(?8, last_seen), meta=?9, path=?10",
    )?
    .execute(params![
        m.id,
        m.project,
        m.agent,
        m.harness_id,
        m.cwd,
        ts(&m.started),
        m.ended.as_ref().map(ts),
        last_seen,
        json(m),
        path
    ])?;
    Ok(())
}

pub fn touch_session(c: &Connection, id: &str, at: &Timestamp) -> Result<()> {
    c.prepare_cached("UPDATE sessions SET last_seen = ? WHERE id = ?")?.execute(params![ts(at), id])?;
    Ok(())
}

fn session_row(r: &Row) -> rusqlite::Result<(Session, String)> {
    let last_seen: Option<String> = r.get(1)?;
    Ok((
        Session {
            meta: from_json(&r.get::<_, String>(0)?)?,
            last_seen: last_seen.and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok()).map(|d| d.to_utc()),
        },
        r.get(2)?,
    ))
}

pub fn session(c: &Connection, id: &str) -> Result<Option<(Session, String)>> {
    Ok(c.prepare_cached("SELECT meta, last_seen, path FROM sessions WHERE id = ?")?
        .query_row([id], session_row)
        .optional()?)
}

pub fn session_ids_by_suffix(c: &Connection, suffix: &str) -> Result<Vec<String>> {
    let mut st = c.prepare_cached("SELECT id FROM sessions WHERE id LIKE ? LIMIT 5")?;
    let rows = st.query_map([format!("%{}", suffix.to_ascii_uppercase())], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn session_by_harness(c: &Connection, agent: &str, harness_id: &str) -> Result<Option<(Session, String)>> {
    Ok(c.prepare_cached(
        "SELECT meta, last_seen, path FROM sessions WHERE agent = ? AND harness_id = ? ORDER BY started DESC LIMIT 1",
    )?
    .query_row([agent, harness_id], session_row)
    .optional()?)
}

/// Open sessions whose cwd is `cwd` or one of its ancestors, most recently active first.
pub fn open_sessions_for_cwd(
    c: &Connection,
    cwd: &str,
    agent: Option<&str>,
    since: &Timestamp,
) -> Result<Vec<(Session, String)>> {
    let mut st = c.prepare_cached(
        "SELECT meta, last_seen, path FROM sessions
         WHERE ended IS NULL AND (cwd = ?1 OR ?1 LIKE cwd || '/%')
           AND (?2 IS NULL OR agent = ?2) AND coalesce(last_seen, started) >= ?3
         ORDER BY coalesce(last_seen, started) DESC, length(cwd) DESC LIMIT 5",
    )?;
    let rows = st.query_map(params![cwd, agent, ts(since)], session_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn sessions(c: &Connection, project: Option<&str>, open_only: bool, limit: usize) -> Result<Vec<Session>> {
    let mut st = c.prepare_cached(
        "SELECT meta, last_seen, path FROM sessions WHERE (?1 IS NULL OR project = ?1) AND (?2 = 0 OR ended IS NULL)
         ORDER BY coalesce(last_seen, started) DESC LIMIT ?3",
    )?;
    let rows = st.query_map(params![project, open_only as i32, limit as i64], session_row)?;
    Ok(rows.map(|r| r.map(|(s, _)| s)).collect::<rusqlite::Result<_>>()?)
}

// ---------------------------------------------------------------- search

#[derive(Debug, Clone, serde::Serialize)]
pub struct Hit {
    /// Item id or entry id.
    pub id: String,
    pub project: String,
    /// `item` or `entry`.
    pub kind: String,
    pub title: String,
    pub snippet: String,
    pub score: f64,
}

/// Turns free text into an FTS5 query: every word must match, as a prefix. No FTS syntax leaks through.
pub fn fts_query(q: &str) -> String {
    q.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn search(c: &Connection, q: &str, project: Option<&str>, limit: usize) -> Result<Vec<Hit>> {
    let query = fts_query(q);
    if query.is_empty() {
        return Ok(vec![]);
    }
    // One ranked query per table. `ORDER BY rank LIMIT` is evaluated inside FTS5, so snippets are only built
    // for the rows returned, not for every match.
    let mut hits = Vec::new();
    for sql in [
        "SELECT i.id, i.project, 'item', i.title, snippet(items_fts, 1, '«', '»', '…', 14), rank
           FROM items_fts JOIN items i ON i.rid = items_fts.rowid
          WHERE items_fts MATCH ?1 AND (?2 IS NULL OR i.project = ?2) ORDER BY rank LIMIT ?3",
        "SELECT e.id, e.project, 'entry', '', snippet(entries_fts, 0, '«', '»', '…', 14), rank
           FROM entries_fts JOIN entries e ON e.rid = entries_fts.rowid
          WHERE entries_fts MATCH ?1 AND (?2 IS NULL OR e.project = ?2) ORDER BY rank LIMIT ?3",
    ] {
        let mut st = c.prepare_cached(sql)?;
        let rows = st.query_map(params![query, project, limit as i64], |r| {
            Ok(Hit {
                id: r.get(0)?,
                project: r.get(1)?,
                kind: r.get(2)?,
                title: r.get(3)?,
                snippet: r.get(4)?,
                score: r.get(5)?,
            })
        })?;
        for h in rows {
            hits.push(h?);
        }
    }
    // bm25 is lower-is-better; give items a small edge over journal entries of equal relevance.
    let key = |h: &Hit| if h.kind == "item" { h.score * 1.2 } else { h.score };
    hits.sort_by(|a, b| key(a).total_cmp(&key(b)));
    hits.truncate(limit);
    Ok(hits)
}

/// FTS query where any of `words` may match (each as a prefix). Words must be plain alphanumerics.
pub fn fts_query_any(words: &[String]) -> String {
    words
        .iter()
        .filter(|w| w.chars().all(char::is_alphanumeric))
        .map(|w| format!("\"{w}\"*"))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// Ranked full-text candidates among items of the given kinds in the given projects (for decision recall).
pub fn search_items(c: &Connection, fts: &str, kinds: &[Kind], projects: &[String], limit: usize) -> Result<Vec<Item>> {
    if fts.is_empty() || kinds.is_empty() || projects.is_empty() {
        return Ok(vec![]);
    }
    let sql = format!(
        "SELECT i.meta, i.project, i.body, i.path FROM items_fts JOIN items i ON i.rid = items_fts.rowid
          WHERE items_fts MATCH ? AND i.kind IN ({}) AND i.project IN ({}) ORDER BY rank LIMIT {limit}",
        vec!["?"; kinds.len()].join(","),
        vec!["?"; projects.len()].join(",")
    );
    let mut args: Vec<String> = vec![fts.to_string()];
    args.extend(kinds.iter().map(|k| k.as_str().to_string()));
    args.extend(projects.iter().cloned());
    let mut st = c.prepare_cached(&sql)?;
    let rows = st.query_map(params_from_iter(args), item_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Ranked full-text candidates among journal entries of the given projects (for `t9 history`).
pub fn search_entries(c: &Connection, fts: &str, projects: &[String], limit: usize) -> Result<Vec<Entry>> {
    if fts.is_empty() || projects.is_empty() {
        return Ok(vec![]);
    }
    let sql = format!(
        "SELECT e.meta, e.project, e.body FROM entries_fts JOIN entries e ON e.rid = entries_fts.rowid
          WHERE entries_fts MATCH ? AND e.project IN ({}) ORDER BY rank LIMIT {limit}",
        vec!["?"; projects.len()].join(",")
    );
    let mut args: Vec<String> = vec![fts.to_string()];
    args.extend(projects.iter().cloned());
    let mut st = c.prepare_cached(&sql)?;
    let rows = st.query_map(params_from_iter(args), entry_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
