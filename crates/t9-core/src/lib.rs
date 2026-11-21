//! t9 core: model, vault (Markdown files, the source of truth), SQLite index (derived), project
//! resolution, context brief and search. Front ends (CLI, MCP, REST) are thin layers over [`Store`].

pub mod about;
pub mod attention;
pub mod clock;
pub mod context;
pub mod decisions;
pub mod env;
pub mod frontmatter;
pub mod git;
pub mod history;
pub mod index;
pub mod insights;
pub mod journal;
pub mod model;
pub mod store;
pub mod story;
pub mod unlinked;
pub mod vcs;

pub use env::Env;
pub use model::*;
pub use store::*;

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("index: {0}")]
    Db(#[from] rusqlite::Error),
}

impl Error {
    pub fn io(path: &Path, source: std::io::Error) -> Error {
        Error::Io { path: path.to_path_buf(), source }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Prints `label` and the time since `since` to stderr when `T9_TRACE` is set. For finding slow paths.
pub fn trace(label: &str, since: std::time::Instant) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("T9_TRACE").is_some()) {
        eprintln!("trace: {label:<24} {:?}", since.elapsed());
    }
}
