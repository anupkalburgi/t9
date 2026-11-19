//! Where things live.
//!
//! ```text
//! ~/.t9/                  vault (T9_VAULT): the Markdown files, the source of truth, a git repository
//!   config.toml           settings, committed with the vault
//!   projects/ inbox/      data
//!   .state/               state dir (T9_HOME): index.db, locks/, git.log. Disposable, git-ignored
//!   integrations/         generated harness plugins (`t9 install`), git-ignored
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::Error;

#[derive(Debug, Clone)]
pub struct Env {
    /// State dir: index, locks, logs.
    pub home: PathBuf,
    pub vault: PathBuf,
    /// Commit vault changes to git automatically after each write (default on).
    pub git: bool,
    /// How long the background committer waits, so a burst of writes becomes one commit.
    pub git_delay_ms: u64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    git: Option<bool>,
    git_delay_ms: Option<u64>,
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

pub fn expand_tilde(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home_dir().join(rest),
        None if p == "~" => home_dir(),
        None => PathBuf::from(p),
    }
}

/// Shortens a path under `$HOME` to `~/…` for display.
pub fn display_path(p: &Path) -> String {
    let home = home_dir();
    match p.strip_prefix(&home) {
        Ok(rest) if !rest.as_os_str().is_empty() => format!("~/{}", rest.display()),
        Ok(_) => "~".into(),
        Err(_) => p.display().to_string(),
    }
}

fn env_bool(name: &str) -> Option<bool> {
    let v = std::env::var(name).ok()?;
    Some(!matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off" | "no" | ""))
}

impl Env {
    pub fn from_env() -> Result<Env, Error> {
        let vault = std::env::var("T9_VAULT").map(|v| expand_tilde(&v)).unwrap_or_else(|_| home_dir().join(".t9"));
        let home = std::env::var("T9_HOME").map(|h| expand_tilde(&h)).unwrap_or_else(|_| vault.join(".state"));
        let config_path = vault.join("config.toml");
        let config: Config = match fs::read_to_string(&config_path) {
            Ok(text) => toml::from_str(&text).map_err(|e| Error::Parse(format!("{}: {e}", config_path.display())))?,
            Err(_) => Config::default(),
        };
        let git = env_bool("T9_GIT").or(config.git).unwrap_or(true);
        let git_delay_ms =
            std::env::var("T9_GIT_DELAY_MS").ok().and_then(|v| v.parse().ok()).or(config.git_delay_ms).unwrap_or(1500);
        Ok(Env { home, vault, git, git_delay_ms })
    }

    pub fn new(home: impl Into<PathBuf>, vault: impl Into<PathBuf>) -> Env {
        Env { home: home.into(), vault: vault.into(), git: false, git_delay_ms: 0 }
    }

    pub fn db_path(&self) -> PathBuf {
        self.home.join("index.db")
    }
    pub fn locks_dir(&self) -> PathBuf {
        self.home.join("locks")
    }
    pub fn projects_dir(&self) -> PathBuf {
        self.vault.join("projects")
    }
    /// Pseudo-project for entries written from directories that are not linked to a project.
    pub fn inbox_dir(&self) -> PathBuf {
        self.vault.join("inbox")
    }
    /// Where `t9 install` writes generated harness plugins.
    pub fn integrations_dir(&self) -> PathBuf {
        self.vault.join("integrations")
    }
}
