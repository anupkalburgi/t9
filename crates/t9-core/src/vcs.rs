//! Automatic git history for the vault.
//!
//! After a command writes to the vault, the CLI starts a detached `t9 vault commit --background` and exits, so
//! writes never wait for git. The committer waits `git_delay_ms` (a burst of writes becomes one commit), takes
//! `locks/git.lock` (committers never race), stages everything and commits with a message derived from the files
//! that changed. Failures go to `<state>/git.log`; `t9 doctor` reports the last one.

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::env::Env;
use crate::{Error, Result};

// `.*.tmp*`: t9 writes files atomically through a temp file next to the target.
const GITIGNORE: &str = ".state/\nintegrations/\n.DS_Store\n.*.tmp*\n";

fn git(vault: &Path, args: &[&str]) -> Result<Output> {
    let mut c = Command::new("git");
    c.arg("-C").arg(vault).args(args).stdin(Stdio::null());
    c.output().map_err(|e| Error::Invalid(format!("running git: {e} (is git installed?)")))
}

fn ok(out: &Output, what: &str) -> Result<()> {
    if out.status.success() {
        Ok(())
    } else {
        Err(Error::Invalid(format!("git {what} failed: {}", String::from_utf8_lossy(&out.stderr).trim())))
    }
}

/// Writes the vault's `.gitignore` if it does not exist (state and generated plugins are not history).
pub fn ensure_gitignore(vault: &Path) -> Result<()> {
    let p = vault.join(".gitignore");
    if !p.exists() {
        fs::create_dir_all(vault).map_err(|e| Error::io(vault, e))?;
        fs::write(&p, GITIGNORE).map_err(|e| Error::io(&p, e))?;
    }
    Ok(())
}

/// Turns changed vault paths into a short label: item ids, journals, sessions, projects.
pub fn summarize(paths: &[String]) -> String {
    let mut labels: BTreeSet<String> = BTreeSet::new();
    let mut items: Vec<String> = vec![];
    for p in paths {
        let parts: Vec<&str> = p.split('/').collect();
        let label = match parts.as_slice() {
            ["projects", _, "items", file] => {
                let id: String =
                    file.split('-').take(2).collect::<Vec<_>>().join("-").trim_end_matches(".md").to_string();
                if !items.contains(&id) {
                    items.push(id);
                }
                continue;
            }
            ["projects", slug, "journal", _] => format!("journal {slug}"),
            ["projects", slug, "sessions", _] => format!("session {slug}"),
            ["projects", slug, "project.md"] => format!("project {slug}"),
            ["inbox", ..] => "inbox".to_string(),
            _ => p.clone(),
        };
        labels.insert(label);
    }
    let mut all: Vec<String> = items;
    all.extend(labels);
    let mut msg = format!("t9: {}", all.join(", "));
    if msg.chars().count() > 72 {
        msg = format!("{}…", msg.chars().take(71).collect::<String>());
    }
    msg
}

/// Commits all vault changes now. Returns the commit subject, or `None` if there was nothing to commit.
pub fn commit(env: &Env) -> Result<Option<String>> {
    let vault = &env.vault;
    fs::create_dir_all(env.locks_dir()).map_err(|e| Error::io(&env.locks_dir(), e))?;
    let lock_path = env.locks_dir().join("git.lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| Error::io(&lock_path, e))?;
    lock.lock().map_err(|e| Error::io(&lock_path, e))?;

    ensure_gitignore(vault)?;
    if !vault.join(".git").exists() {
        ok(&git(vault, &["init", "-q"])?, "init")?;
    }
    ok(&git(vault, &["add", "-A"])?, "add")?;
    let staged = git(vault, &["diff", "--cached", "--name-only", "-z"])?;
    ok(&staged, "diff")?;
    let paths: Vec<String> =
        String::from_utf8_lossy(&staged.stdout).split('\0').filter(|s| !s.is_empty()).map(String::from).collect();
    if paths.is_empty() {
        return Ok(None);
    }
    let subject = summarize(&paths);
    let mut body = paths.iter().take(50).map(|p| format!("- {p}")).collect::<Vec<_>>().join("\n");
    if paths.len() > 50 {
        body.push_str(&format!("\n- … {} more", paths.len() - 50));
    }
    let message = format!("{subject}\n\n{body}\n");

    let mut c = Command::new("git");
    c.arg("-C").arg(vault).args(["commit", "-q", "-F", "-"]).stdin(Stdio::piped()).stdout(Stdio::null());
    c.stderr(Stdio::piped());
    // Use the user's git identity if there is one; otherwise commit as t9.
    let has_identity = git(vault, &["config", "user.email"]).map(|o| o.status.success()).unwrap_or(false);
    if !has_identity {
        for (k, v) in [
            ("GIT_AUTHOR_NAME", "t9"),
            ("GIT_AUTHOR_EMAIL", "t9@localhost"),
            ("GIT_COMMITTER_NAME", "t9"),
            ("GIT_COMMITTER_EMAIL", "t9@localhost"),
        ] {
            c.env(k, v);
        }
    }
    let mut child = c.spawn().map_err(|e| Error::Invalid(format!("running git: {e}")))?;
    child.stdin.take().expect("piped").write_all(message.as_bytes()).map_err(|e| Error::io(vault, e))?;
    let out = child.wait_with_output().map_err(|e| Error::io(vault, e))?;
    ok(&out, "commit")?;
    Ok(Some(subject))
}

/// Starts `<current exe> vault commit --background` detached from the terminal and returns immediately.
/// The current executable must be the `t9` CLI.
pub fn spawn_background(env: &Env) {
    use std::os::unix::process::CommandExt;
    let Ok(exe) = std::env::current_exe() else { return };
    let spawned = Command::new(exe)
        .args(["vault", "commit", "--background"])
        .env("T9_VAULT", &env.vault)
        .env("T9_HOME", &env.home)
        .env("T9_GIT_DELAY_MS", env.git_delay_ms.to_string())
        .env_remove("T9_TRACE")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
    if let Err(e) = spawned {
        log_line(env, &format!("error: could not start background commit: {e}"));
    }
}

pub fn log_path(env: &Env) -> PathBuf {
    env.home.join("git.log")
}

/// Appends a line to `<state>/git.log` (background commits have no terminal to report to).
pub fn log_line(env: &Env, line: &str) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(log_path(env)) {
        let _ = writeln!(f, "{} {line}", chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"));
    }
}

/// The last line of `git.log` if it records a failure.
pub fn last_error(env: &Env) -> Option<String> {
    let text = fs::read_to_string(log_path(env)).ok()?;
    let last = text.lines().last()?;
    last.contains(" error: ").then(|| last.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries() {
        let paths = [
            "projects/billing-service/items/BIL-12-fix-the-thing.md",
            "projects/billing-service/items/BIL-3.md",
            "projects/billing-service/journal/2026-11-18.md",
            "projects/billing-service/sessions/x.md",
        ]
        .map(String::from);
        assert_eq!(summarize(&paths), "t9: BIL-12, BIL-3, journal billing-service, session billing-service");
        let many: Vec<String> = (1..40).map(|i| format!("projects/p/items/P-{i}-x.md")).collect();
        assert!(summarize(&many).chars().count() <= 72);
    }
}
