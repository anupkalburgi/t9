//! Minimal, read-only git discovery by reading `.git` files directly (no `git` process, ~0.1 ms).
//!
//! Handles normal repos, linked worktrees (`.git` is a file pointing at `<common>/worktrees/<name>`), and
//! packed refs.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct GitInfo {
    /// Top of the working tree containing the cwd.
    pub root: PathBuf,
    /// Normalized `origin` URL (or first remote), e.g. `github.com/acme/widgets`.
    pub remote: Option<String>,
    pub branch: Option<String>,
    pub head: Option<String>,
}

pub fn discover(cwd: &Path) -> Option<GitInfo> {
    for dir in cwd.ancestors() {
        let dot_git = dir.join(".git");
        let Ok(md) = fs::metadata(&dot_git) else { continue };
        let git_dir = if md.is_dir() {
            dot_git
        } else {
            let text = fs::read_to_string(&dot_git).ok()?;
            let target = text.trim().strip_prefix("gitdir:")?.trim();
            let p = Path::new(target);
            if p.is_absolute() { p.to_path_buf() } else { dir.join(p) }
        };
        let common_dir = match fs::read_to_string(git_dir.join("commondir")) {
            Ok(c) => {
                let p = Path::new(c.trim());
                if p.is_absolute() { p.to_path_buf() } else { git_dir.join(p) }
            }
            Err(_) => git_dir.clone(),
        };
        let remote = fs::read_to_string(common_dir.join("config"))
            .ok()
            .and_then(|c| remote_url(&c))
            .map(|u| normalize_remote(&u));
        let (branch, head) = read_head(&git_dir, &common_dir);
        return Some(GitInfo { root: dir.to_path_buf(), remote, branch, head });
    }
    None
}

/// Picks `origin`'s URL, else the first remote's.
fn remote_url(config: &str) -> Option<String> {
    let mut section: Option<String> = None;
    let mut first: Option<String> = None;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section = Some(line.to_string());
            continue;
        }
        let Some(sec) = &section else { continue };
        if !sec.starts_with("[remote ") {
            continue;
        }
        if let Some((k, v)) = line.split_once('=')
            && k.trim() == "url"
        {
            let v = v.trim().to_string();
            if sec == "[remote \"origin\"]" {
                return Some(v);
            }
            first.get_or_insert(v);
        }
    }
    first
}

fn read_head(git_dir: &Path, common_dir: &Path) -> (Option<String>, Option<String>) {
    let Ok(head) = fs::read_to_string(git_dir.join("HEAD")) else { return (None, None) };
    let head = head.trim();
    let Some(reference) = head.strip_prefix("ref:").map(str::trim) else {
        return (None, Some(head.to_string())); // detached
    };
    let branch = reference.strip_prefix("refs/heads/").unwrap_or(reference).to_string();
    let sha = [git_dir, common_dir]
        .iter()
        .find_map(|d| fs::read_to_string(d.join(reference)).ok())
        .map(|s| s.trim().to_string())
        .or_else(|| {
            let packed = fs::read_to_string(common_dir.join("packed-refs")).ok()?;
            packed.lines().find_map(|l| {
                let (sha, name) = l.split_once(' ')?;
                (name.trim() == reference).then(|| sha.to_string())
            })
        });
    (Some(branch), sha)
}

/// `git@github.com:Acme/Widgets.git` and `https://user@github.com/acme/widgets/` → `github.com/acme/widgets`.
pub fn normalize_remote(url: &str) -> String {
    let mut s = url.trim();
    let had_scheme = if let Some((_, rest)) = s.split_once("://") {
        s = rest;
        true
    } else {
        false
    };
    if let Some(at) = s.find('@')
        && s[..at].find('/').is_none()
    {
        s = &s[at + 1..];
    }
    let mut out = s.trim_end_matches('/').trim_end_matches(".git").to_string();
    if !had_scheme && let Some(colon) = out.find(':') {
        out.replace_range(colon..colon + 1, "/");
    }
    out.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_remotes() {
        for u in [
            "git@github.com:Acme/Widgets.git",
            "https://github.com/acme/widgets",
            "https://user@github.com/acme/widgets.git/",
            "ssh://git@github.com/acme/widgets.git",
        ] {
            assert_eq!(normalize_remote(u), "github.com/acme/widgets", "{u}");
        }
    }

    #[test]
    fn worktree_and_packed_refs() {
        let t = tempfile::tempdir().unwrap();
        let main = t.path().join("main");
        let git = main.join(".git");
        fs::create_dir_all(git.join("worktrees/wt")).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(git.join("config"), "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = git@github.com:a/b.git\n")
            .unwrap();
        fs::write(git.join("packed-refs"), "# pack\nabc123 refs/heads/feature\n").unwrap();
        fs::write(git.join("worktrees/wt/HEAD"), "ref: refs/heads/feature\n").unwrap();
        fs::write(git.join("worktrees/wt/commondir"), "../..\n").unwrap();
        let wt = t.path().join("wt");
        fs::create_dir_all(wt.join("sub")).unwrap();
        fs::write(wt.join(".git"), format!("gitdir: {}\n", git.join("worktrees/wt").display())).unwrap();

        let info = discover(&wt.join("sub")).unwrap();
        assert_eq!(info.root, wt);
        assert_eq!(info.remote.as_deref(), Some("github.com/a/b"));
        assert_eq!(info.branch.as_deref(), Some("feature"));
        assert_eq!(info.head.as_deref(), Some("abc123"));
    }
}
