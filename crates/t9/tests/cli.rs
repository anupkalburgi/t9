//! End-to-end tests: run the real `t9` binary against a temporary vault and state dir.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as Std;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

struct Sandbox {
    _tmp: TempDir,
    home: PathBuf,
    vault: PathBuf,
    root: PathBuf,
}

impl Sandbox {
    fn new() -> Sandbox {
        let tmp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        let s = Sandbox { home: root.join("state"), vault: root.join("vault"), root: root.clone(), _tmp: tmp };
        fs::create_dir_all(s.root.join("work")).unwrap();
        s
    }

    /// A fake git repo with an origin remote (written directly; no git binary needed).
    fn repo(&self, name: &str, remote: &str) -> PathBuf {
        let dir = self.root.join("work").join(name);
        let git = dir.join(".git");
        fs::create_dir_all(git.join("refs/heads")).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(git.join("refs/heads/main"), "0123456789abcdef\n").unwrap();
        fs::write(git.join("config"), format!("[remote \"origin\"]\n\turl = {remote}\n")).unwrap();
        dir
    }

    /// A linked worktree of `repo` at `work/<name>` on `branch`.
    fn worktree(&self, repo: &Path, name: &str, branch: &str) -> PathBuf {
        let wt_git = repo.join(".git/worktrees").join(name);
        fs::create_dir_all(&wt_git).unwrap();
        fs::write(wt_git.join("HEAD"), format!("ref: refs/heads/{branch}\n")).unwrap();
        fs::write(wt_git.join("commondir"), "../..\n").unwrap();
        let dir = self.root.join("work").join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(".git"), format!("gitdir: {}\n", wt_git.display())).unwrap();
        dir
    }

    fn cmd(&self, cwd: &Path) -> Command {
        let mut c = Command::cargo_bin("t9").unwrap();
        c.current_dir(cwd)
            .env("T9_HOME", &self.home)
            .env("T9_VAULT", &self.vault)
            .env("T9_GIT", "0")
            .env_remove("T9_AGENT")
            .env_remove("T9_SESSION")
            .env_remove("T9_PROJECT")
            .env_remove("CLAUDE_ENV_FILE");
        c
    }

    fn ok(&self, cwd: &Path, args: &[&str]) -> String {
        let out = self.cmd(cwd).args(args).output().unwrap();
        assert!(out.status.success(), "t9 {args:?} failed:\n{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    }

    fn json(&self, cwd: &Path, args: &[&str]) -> Value {
        let mut a = vec!["--json"];
        a.extend_from_slice(args);
        serde_json::from_str(&self.ok(cwd, &a)).unwrap()
    }

    fn err(&self, cwd: &Path, args: &[&str]) -> String {
        let out = self.cmd(cwd).args(args).output().unwrap();
        assert!(!out.status.success(), "t9 {args:?} unexpectedly succeeded");
        String::from_utf8(out.stderr).unwrap()
    }
}

#[test]
fn full_flow_and_context() {
    let s = Sandbox::new();
    let repo = s.repo("billing-service", "git@github.com:acme/billing-service.git");
    s.ok(&repo, &["project", "init", "--name", "Billing Service", "--key", "BIL"]);
    s.ok(&repo, &["add", "plan", "Move invoicing to the new payments API", "-p", "1"]);
    s.ok(&repo, &["add", "phase", "Numeric", "--parent", "bil-1"]);
    s.ok(&repo, &["add", "revisit", "Totals off by one cent", "--parent", "BIL-2", "-p", "1"]);
    let task = s.json(
        &repo,
        &["--agent", "claude-code", "add", "task", "Why do partial refunds fail", "--parent", "BIL-2", "--focus"],
    );
    assert_eq!(task["id"], "BIL-4");
    assert_eq!(task["status"], "active");

    // Flags after the message are flags, and the focus is used when no item is given.
    let e = s.json(
        &repo,
        &["--agent", "claude-code", "log", "provider returns 422 for partial refunds", "-r", "local/runs/v7"],
    );
    assert_eq!(e["item"], "BIL-4");
    assert_eq!(e["body"], "provider returns 422 for partial refunds");
    assert_eq!(e["refs"][0], "local/runs/v7");
    assert_eq!(e["agent"], "claude-code");

    s.ok(&repo, &["add", "decision", "Keep the XML export"]);
    let d = s.json(&repo, &["decide", "Drop the XML export", "-s", "BIL-5", "-b", "simpler"]);
    assert_eq!(d["links"][0]["rel"], "supersedes");
    let old = s.json(&repo, &["show", "BIL-5"]);
    assert_eq!(old["item"]["status"], "superseded");

    s.ok(&repo, &["--agent", "claude-code", "handoff", "Next: pass temp-table schemas to the binder"]);
    let brief = s.ok(&repo, &["--agent", "claude-code", "context"]);
    for want in [
        "# Billing Service (BIL)",
        "## Last handoff (claude-code",
        "Next: pass temp-table schemas",
        "## Current focus\n- BIL-4 task [active]",
        "## Plan BIL-1",
        "BIL-3 revisit [open] p1",
        "## Current decisions\n- BIL-6",
    ] {
        assert!(brief.contains(want), "brief is missing {want:?}:\n{brief}");
    }
    assert!(!brief.contains("Keep the XML export"), "superseded decisions are not current:\n{brief}");

    // A tiny budget still yields a well-formed, truncated brief.
    let small = s.ok(&repo, &["context", "--budget", "200"]);
    assert!(small.len() <= 200 * 4 + 200, "{}", small.len());
    assert!(small.contains("truncated"));

    let hits = s.json(&repo, &["search", "cent", "totals"]);
    assert_eq!(hits[0]["id"], "BIL-3");

    s.ok(&repo, &["done", "BIL-4", "-n", "found it"]);
    // A closed focus stops attracting entries and leaves the brief.
    let e = s.json(&repo, &["--agent", "claude-code", "log", "unrelated note"]);
    assert!(e["item"].is_null(), "{e}");
    assert!(!s.ok(&repo, &["--agent", "claude-code", "context"]).contains("## Current focus"));
    let open = s.json(&repo, &["ls"]);
    assert!(open.as_array().unwrap().iter().all(|i| i["id"] != "BIL-4"));
    let all = s.json(&repo, &["ls", "--all"]);
    assert_eq!(all.as_array().unwrap().len(), 6);
}

#[test]
fn worktrees_and_subdirs_resolve_to_the_project() {
    let s = Sandbox::new();
    let repo = s.repo("widgets", "https://github.com/Acme/Widgets.git");
    s.ok(&repo, &["project", "init"]);
    let wt = s.worktree(&repo, "widgets-feature", "feature/x");
    fs::create_dir_all(wt.join("src/deep")).unwrap();
    let w = s.json(&wt.join("src/deep"), &["whereami"]);
    assert_eq!(w["resolution"]["project"]["key"], "WID");
    assert_eq!(w["resolution"]["via"], "git-remote");
    assert_eq!(w["resolution"]["git"]["branch"], "feature/x");

    // An unrelated directory does not resolve, and suggestions point at the similarly named project.
    let other = s.root.join("work/widgets-old");
    fs::create_dir_all(&other).unwrap();
    let w = s.json(&other, &["whereami"]);
    assert!(w["resolution"]["project"].is_null());
    let brief = s.ok(&other, &["context"]);
    assert!(brief.contains("not linked to a t9 project") && brief.contains("(WID)"), "{brief}");
    let err = s.err(&other, &["add", "task", "x"]);
    assert!(err.contains("t9 project link"), "{err}");

    // .t9 pins a directory explicitly.
    fs::write(other.join(".t9"), "WID\n").unwrap();
    assert_eq!(s.json(&other, &["whereami"])["resolution"]["via"], ".t9");
}

#[test]
fn inbox_entries_are_adopted_on_init_and_triaged() {
    let s = Sandbox::new();
    let dir = s.root.join("work/scratchpad");
    fs::create_dir_all(&dir).unwrap();
    let e = s.json(&dir, &["log", "idea before the project existed"]);
    assert_eq!(e["project"], "INBOX");
    assert!(e["session"].is_null(), "unlinked writes must not start sessions");
    let out = s.ok(&dir, &["project", "init", "--key", "SP"]);
    assert!(out.contains("moved 1 inbox entries"), "{out}");
    let t = s.json(&dir, &["triage"]);
    assert_eq!(t["inbox"].as_array().unwrap().len(), 0);
    let unfiled = t["unfiled"].as_array().unwrap();
    assert_eq!(unfiled.len(), 1);
    let id = unfiled[0]["id"].as_str().unwrap().to_string();

    s.ok(&dir, &["add", "note", "Ideas"]);
    s.ok(&dir, &["attach", &id[id.len() - 6..], "SP-1"]);
    let show = s.json(&dir, &["show", "SP-1"]);
    assert_eq!(show["entries"][0]["body"], "idea before the project existed");
    assert_eq!(s.json(&dir, &["triage"])["unfiled"].as_array().unwrap().len(), 0);
}

#[test]
fn hand_edits_are_picked_up_and_reindex_is_lossless() {
    let s = Sandbox::new();
    let repo = s.repo("app", "git@github.com:acme/app.git");
    s.ok(&repo, &["project", "init", "--key", "APP"]);
    let item = s.json(&repo, &["add", "bug", "Crash on save", "-b", "steps: click save"]);
    s.ok(&repo, &["log", "-i", "APP-1", "reproduced"]);
    let path = PathBuf::from(item["path"].as_str().unwrap());

    // A human edits status and body in an editor; body edits survive later t9 writes.
    let text = fs::read_to_string(&path).unwrap();
    let text = text.replace("status: open", "status: blocked").replace("steps: click save", "steps: click save twice");
    fs::write(&path, text).unwrap();
    let show = s.json(&repo, &["show", "APP-1"]);
    assert_eq!(show["item"]["status"], "blocked");
    s.ok(&repo, &["update", "APP-1", "-t", "ui"]);
    assert!(fs::read_to_string(&path).unwrap().contains("click save twice"));

    // Hand-edited journal bodies are re-read too.
    let journal = fs::read_dir(s.vault.join("projects/app/journal")).unwrap().next().unwrap().unwrap().path();
    let jt = fs::read_to_string(&journal).unwrap().replace("reproduced", "reproduced on 1.2.3");
    fs::write(&journal, jt).unwrap();
    assert_eq!(s.json(&repo, &["show", "APP-1"])["entries"][0]["body"], "reproduced on 1.2.3");

    // Deleting the index loses nothing.
    let before = s.ok(&repo, &["ls", "--all"]);
    let before_j = s.ok(&repo, &["journal"]);
    fs::remove_file(s.home.join("index.db")).unwrap();
    assert_eq!(s.ok(&repo, &["ls", "--all"]), before);
    assert_eq!(s.ok(&repo, &["journal"]), before_j);

    // Optimistic concurrency.
    let rev = s.json(&repo, &["show", "APP-1"])["item"]["rev"].as_u64().unwrap();
    let err = s.err(&repo, &["update", "APP-1", "--title", "x", "--rev", &(rev - 1).to_string()]);
    assert!(err.contains("conflict"), "{err}");
    s.ok(&repo, &["update", "APP-1", "--title", "Crash on save (Mac)", "--rev", &rev.to_string()]);
    // Search follows updates: the new title is found, and the item is listed once.
    let hits = s.json(&repo, &["search", "mac"]);
    assert_eq!(hits.as_array().unwrap().iter().filter(|h| h["id"] == "APP-1").count(), 1, "{hits}");

    // Deleting an item file removes it from the index.
    fs::remove_file(&path).unwrap();
    assert!(s.err(&repo, &["show", "APP-1"]).contains("not found"));
}

#[test]
fn concurrent_writers_do_not_lose_or_duplicate() {
    let s = Sandbox::new();
    let repo = s.repo("busy", "git@github.com:acme/busy.git");
    s.ok(&repo, &["project", "init", "--key", "BZ"]);
    let bin = assert_cmd::cargo::cargo_bin("t9");
    let spawn = |args: Vec<String>| {
        Std::new(&bin)
            .args(args)
            .current_dir(&repo)
            .env("T9_HOME", &s.home)
            .env("T9_VAULT", &s.vault)
            .env("T9_AGENT", "agent")
            .env("T9_GIT", "0")
            .spawn()
            .unwrap()
    };
    let mut kids: Vec<_> = (0..16).map(|i| spawn(vec!["log".into(), format!("parallel entry {i}")])).collect();
    kids.extend((0..8).map(|i| spawn(vec!["add".into(), "task".into(), format!("task {i}")])));
    for mut k in kids {
        assert!(k.wait().unwrap().success());
    }
    let entries = s.json(&repo, &["journal", "-n", "100"]);
    assert_eq!(entries.as_array().unwrap().len(), 16);
    let items = s.json(&repo, &["ls"]);
    let mut ids: Vec<String> =
        items.as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap().to_string()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 8);
    // And the files agree with the index.
    fs::remove_file(s.home.join("index.db")).unwrap();
    assert_eq!(s.json(&repo, &["journal", "-n", "100"]).as_array().unwrap().len(), 16);
}

#[test]
fn claude_hook_starts_and_resumes_sessions() {
    let s = Sandbox::new();
    let repo = s.repo("hooked", "git@github.com:acme/hooked.git");
    s.ok(&repo, &["project", "init", "--key", "HK"]);
    let env_file = s.root.join("claude.env");
    let payload = serde_json::json!({"session_id": "abc-123", "cwd": repo, "source": "startup"}).to_string();
    let out = s
        .cmd(&s.root)
        .env("CLAUDE_ENV_FILE", &env_file)
        .args(["hook", "claude", "session-start"])
        .write_stdin(payload)
        .output()
        .unwrap();
    assert!(out.status.success());
    let brief = String::from_utf8(out.stdout).unwrap();
    assert!(brief.contains("# hooked (HK)") && brief.contains("t9 log"), "{brief}");
    let env = fs::read_to_string(&env_file).unwrap();
    let sid = env.lines().next().unwrap().strip_prefix("export T9_SESSION=").unwrap().to_string();

    // After compaction Claude sends the same session id: same t9 session, brief says so.
    let payload = serde_json::json!({"session_id": "abc-123", "cwd": repo, "source": "compact"}).to_string();
    let out = s.cmd(&s.root).args(["hook", "claude", "session-start"]).write_stdin(payload).output().unwrap();
    assert!(String::from_utf8(out.stdout).unwrap().starts_with("(The conversation was compacted"));
    let sessions = s.json(&repo, &["session", "ls"]);
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    assert_eq!(sessions[0]["id"], sid.as_str());
    assert_eq!(sessions[0]["agent"], "claude-code");

    // Logs from the agent's shell pick the session up by cwd even without T9_SESSION.
    let e = s.json(&repo, &["log", "did a thing"]);
    assert_eq!(e["session"], sid.as_str());

    let payload = serde_json::json!({"session_id": "abc-123", "cwd": repo}).to_string();
    s.cmd(&s.root).args(["hook", "claude", "session-end"]).write_stdin(payload).assert().success();
    assert!(s.json(&repo, &["session", "ls", "--open"]).as_array().unwrap().is_empty());

    // A broken payload never fails the hook.
    s.cmd(&s.root).args(["hook", "claude", "session-start"]).write_stdin("not json").assert().success();
}

fn git_log(vault: &Path) -> Vec<String> {
    let out = Std::new("git").arg("-C").arg(vault).args(["log", "--format=%s"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().map(String::from).collect()
}

#[test]
fn vault_changes_are_committed_to_git_in_the_background() {
    let s = Sandbox::new();
    let repo = s.repo("gitted", "git@github.com:acme/gitted.git");
    let git_on = |args: &[&str]| {
        let out = s.cmd(&repo).env("T9_GIT", "1").env("T9_GIT_DELAY_MS", "0").args(args).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    };
    git_on(&["project", "init", "--key", "GT"]);
    git_on(&["add", "bug", "Crash on save"]);
    git_on(&["log", "-i", "GT-1", "reproduced"]);

    // The commit happens in a detached process; wait for it (up to 10 s).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let log = git_log(&s.vault);
        let all = log.join("\n");
        if all.contains("GT-1") && all.contains("journal gitted") {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "no commit yet; git log: {log:?}");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    // Everything is committed; the state dir and temp files are ignored.
    let status = Std::new("git").arg("-C").arg(&s.vault).args(["status", "--porcelain"]).output().unwrap();
    assert!(String::from_utf8_lossy(&status.stdout).trim().is_empty(), "{}", String::from_utf8_lossy(&status.stdout));
    assert!(fs::read_to_string(s.vault.join(".gitignore")).unwrap().contains(".state/"));
    assert_eq!(s.ok(&repo, &["vault", "commit"]).trim(), "nothing to commit");

    // Reads do not commit; `t9 vault git` runs git in the vault.
    let before = git_log(&s.vault).len();
    s.cmd(&repo).env("T9_GIT", "1").env("T9_GIT_DELAY_MS", "0").args(["ls"]).assert().success();
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(git_log(&s.vault).len(), before);
    assert!(s.ok(&repo, &["vault", "git", "log", "--oneline"]).contains("t9: "));
}

#[test]
fn default_layout_keeps_state_inside_the_vault() {
    let s = Sandbox::new();
    let vault = s.root.join("dot-t9");
    let dir = s.root.join("work/plain");
    fs::create_dir_all(&dir).unwrap();
    let out = s.cmd(&dir).env_remove("T9_HOME").env("T9_VAULT", &vault).args(["vault", "path"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), vault.display().to_string());
    s.cmd(&dir)
        .env_remove("T9_HOME")
        .env("T9_VAULT", &vault)
        .args(["project", "init", "--key", "PL"])
        .assert()
        .success();
    assert!(vault.join(".state/index.db").exists());
    assert!(vault.join("projects/plain/project.md").exists());
    // config.toml in the vault is read (and unknown keys are rejected).
    fs::write(vault.join("config.toml"), "git = false\n").unwrap();
    s.cmd(&dir).env_remove("T9_HOME").env_remove("T9_GIT").env("T9_VAULT", &vault).args(["ls"]).assert().success();
    fs::write(vault.join("config.toml"), "colour = 1\n").unwrap();
    let out = s.cmd(&dir).env_remove("T9_HOME").env("T9_VAULT", &vault).args(["ls"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).contains("config.toml"));
}

#[test]
fn decisions_are_recorded_and_recalled_by_scope_area_and_topic() {
    let s = Sandbox::new();
    let repo = s.repo("qr", "git@github.com:acme/qr.git");
    s.ok(&repo, &["project", "init", "--key", "BIL"]);
    s.ok(&repo, &["add", "plan", "Ship v2"]);
    s.ok(&repo, &["add", "feature", "Search", "--parent", "BIL-1", "--area", "Search"]);
    s.ok(&repo, &["add", "task", "Index journal entries", "--parent", "BIL-2"]);

    // A full ADR-style decision.
    let d = s.json(
        &repo,
        &[
            "decide",
            "Use FTS5 for search",
            "--because",
            "no extra process",
            "-r",
            "tantivy: separate index to sync",
            "--area",
            "search",
        ],
    );
    assert_eq!(d["id"], "BIL-4");
    assert_eq!(d["status"], "accepted");
    assert_eq!(d["areas"][0], "search");
    assert_eq!(
        d["body"].as_str().unwrap().trim(),
        "## Why\n\nno extra process\n\n## Rejected alternatives\n\n- tantivy: separate index to sync"
    );
    let r = s.json(
        &repo,
        &["reject", "Use a daemon for all writes", "--because", "CLI must work alone", "--area", "architecture"],
    );
    assert_eq!(r["status"], "rejected");
    s.ok(&repo, &["decide", "Pin the tokenizer to porter", "--for", "BIL-2"]); // BIL-6, scoped
    s.ok(&repo, &["decide", "Integration tests use real temp vaults", "--area", "testing"]); // BIL-7
    s.ok(&repo, &["decide", "Always use uv for Python", "--global", "--because", "one tool for envs"]);
    s.ok(&repo, &["add", "idea", "Trigram tokenizer for identifiers"]); // BIL-8

    // Focusing a task recalls what applies through its ancestors: scope (BIL-6) and area (BIL-4), not testing.
    let f = s.json(&repo, &["--agent", "claude-code", "focus", "BIL-3"]);
    let ids: Vec<&str> = f["decisions"].as_array().unwrap().iter().map(|d| d["id"].as_str().unwrap()).collect();
    assert_eq!(ids[0], "BIL-6", "{ids:?}");
    assert!(ids.contains(&"BIL-4") && !ids.contains(&"BIL-7"), "{ids:?}");
    let reasons = f["decisions"][0]["reasons"][0].as_str().unwrap();
    assert_eq!(reasons, "applies to BIL-2");

    let brief = s.ok(&repo, &["--agent", "claude-code", "context"]);
    for want in [
        "## From history: applies to your focus (decisions, experiments, resources)\n- BIL-6",
        "## Rules for every project (global)\n- GLOBAL-1 [accepted]",
        "Always use uv for Python — one tool for envs",
        "## Current decisions\n- BIL-7",
        "## Ruled out (do not re-propose without new information)\n- BIL-5 [rejected]",
        "CLI must work alone",
        "## Parked ideas\n- BIL-8 idea [open]",
    ] {
        assert!(brief.contains(want), "brief is missing {want:?}:\n{brief}");
    }

    // check: finds rejected alternatives inside a decision's body, and explicit rejections.
    assert_eq!(s.json(&repo, &["check", "tantivy"])[0]["id"], "BIL-4");
    assert_eq!(s.json(&repo, &["check", "daemon"])[0]["status"], "rejected");
    assert!(s.ok(&repo, &["check", "kubernetes"]).contains("nothing decided"));

    // The per-message hook surfaces earlier decisions and nudges to record new ones; silent otherwise.
    let hook = |prompt: &str| {
        let payload = serde_json::json!({"session_id": "s1", "cwd": repo, "prompt": prompt}).to_string();
        let out = s.cmd(&s.root).args(["hook", "claude", "prompt"]).write_stdin(payload).output().unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    };
    let out = hook("Should we switch the search engine to tantivy?");
    assert!(out.contains("BIL-4 [accepted]") && out.contains("record it now"), "{out}");
    assert_eq!(hook("hello there, how are you"), "");
    let out = hook("python env setup for the bench script");
    assert!(out.contains("GLOBAL-1"), "{out}");

    // The decision log, by area and as an ADR document.
    let log = s.json(&repo, &["decisions", "--area", "architecture"]);
    assert_eq!(log.as_array().unwrap().len(), 1);
    let md = s.ok(&repo, &["decisions", "--md"]);
    assert!(md.starts_with("# Decision log: qr (BIL)") && md.contains("## Rejected alternatives"), "{md}");

    // Changing a decision supersedes it; the log hides superseded ones unless --all.
    let changed = s.json(&repo, &["decide", "Use tantivy after all", "-s", "BIL-4", "--because", "need fuzzy search"]);
    assert_eq!(changed["areas"][0], "search", "areas are inherited from the superseded decision");
    let checked = s.json(&repo, &["check", "tantivy"]);
    assert_eq!(checked[0]["id"], changed["id"], "current decisions come before superseded ones");
    let ids =
        |v: Value| v.as_array().unwrap().iter().map(|d| d["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert!(!ids(s.json(&repo, &["decisions"])).contains(&"BIL-4".to_string()));
    assert!(ids(s.json(&repo, &["decisions", "--all"])).contains(&"BIL-4".to_string()));

    // Global rules reach other projects and their checks.
    let other = s.repo("other", "git@github.com:acme/other.git");
    s.ok(&other, &["project", "init", "--key", "OT"]);
    assert!(s.ok(&other, &["context"]).contains("GLOBAL-1"));
    assert_eq!(s.json(&other, &["check", "uv"])[0]["id"], "GLOBAL-1");
    // GLOBAL is never suggested for an unlinked directory and cannot be taken as a key.
    assert!(s.err(&other, &["project", "init", "--key", "GLOBAL", "--no-root"]).contains("must be"));
}

#[test]
fn an_unlinked_folder_is_surfaced_to_the_human_until_set_up() {
    // A co-worker installs t9 and starts Claude Code in a repository that has no project yet.
    let s = Sandbox::new();
    let repo = s.repo("billing-service", "git@github.com:acme/billing-service.git");
    let hook = |event: &str, extra: serde_json::Value| {
        let mut payload = serde_json::json!({"session_id": "s1", "cwd": repo});
        payload.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let out = s.cmd(&s.root).args(["hook", "claude", event]).write_stdin(payload.to_string()).output().unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    };

    // Session start: the brief tells Claude to tell the user, with a ready-to-run command.
    let brief = hook("session-start", serde_json::json!({"source": "startup"}));
    assert!(brief.contains("Tell the user in your first reply"), "{brief}");
    assert!(brief.contains("`t9 project init --name \"billing-service\" --key BS`"), "{brief}");

    // First message: one reminder; later messages: none.
    let first = hook("prompt", serde_json::json!({"prompt": "fix the refund bug"}));
    assert!(first.contains("not linked to a t9 project") && first.contains("--key BS"), "{first}");
    assert!(!hook("prompt", serde_json::json!({"prompt": "and the tests"})).contains("not linked"));

    // The agent's notes go to the inbox; the human sees the folder in `project ls` and `doctor`.
    s.ok(&repo, &["--agent", "claude-code", "log", "refund handler returns 422"]);
    let ls = s.ok(&s.root, &["project", "ls"]);
    assert!(ls.contains("Not set up yet") && ls.contains("billing-service") && ls.contains("--key BS"), "{ls}");
    assert!(s.ok(&s.root, &["doctor"]).contains("has no project (claude-code worked there)"));

    // Running the suggested command creates the project, adopts the note, and clears the warning.
    let out = s.ok(&repo, &["project", "init", "--name", "billing-service", "--key", "BS"]);
    assert!(out.contains("moved 1 inbox entries"), "{out}");
    assert!(!s.ok(&s.root, &["project", "ls"]).contains("Not set up yet"));
    let brief = hook("session-start", serde_json::json!({"source": "startup", "session_id": "s2"}));
    assert!(brief.contains("# billing-service (BS)"), "{brief}");
}

#[test]
fn project_context_experiments_resources_history_and_timeline() {
    let s = Sandbox::new();
    let repo = s.repo("billing", "git@github.com:acme/billing.git");
    s.ok(&repo, &["project", "init", "--name", "Billing Service", "--key", "BIL"]);

    // About: sections, aliases, refusal of credentials, and a compact form in the brief.
    s.ok(&repo, &["about", "set", "what", "Charges customers and sends invoices."]);
    s.ok(&repo, &["about", "add", "Rust and Postgres", "--section", "tech"]);
    let about = s.json(&repo, &["about", "add", "cargo test", "--section", "run"]);
    let keys: Vec<&str> = about["sections"].as_array().unwrap().iter().map(|x| x["key"].as_str().unwrap()).collect();
    assert_eq!(keys, ["what", "stack", "run"]);
    assert!(!about["reviewed"].is_null());
    assert!(s.err(&repo, &["about", "add", "db password=EXAMPLE", "--section", "run"]).contains("credentials"));
    assert!(s.err(&repo, &["about", "add", "x", "--section", "nope"]).contains("unknown section"));
    let brief = s.ok(&repo, &["context"]);
    assert!(
        brief.contains("- What it is: Charges customers") && brief.contains("- Stack: Rust and Postgres"),
        "{brief}"
    );

    // Resources: typed, located, owned; never credentials.
    let r = s.json(
        &repo,
        &[
            "add",
            "resource",
            "Invoices table",
            "--type",
            "Table",
            "--at",
            "analytics.billing.invoices",
            "--owner",
            "data-eng",
        ],
    );
    assert_eq!(
        (r["resource_type"].as_str(), r["location"].as_str()),
        (Some("table"), Some("analytics.billing.invoices"))
    );
    assert!(s.err(&repo, &["add", "resource", "Prod DB", "--at", "prod db, password=EXAMPLE"]).contains("credentials"));
    assert_eq!(s.json(&repo, &["resources", "--type", "table"]).as_array().unwrap().len(), 1);
    assert!(s.ok(&repo, &["context"]).contains("BIL-1 Invoices table [table] @ analytics.billing.invoices"));

    // An idea, an experiment that fails with a retry condition, and the decision that followed.
    s.ok(&repo, &["add", "idea", "Batch webhooks to cut API calls"]); // BIL-2
    s.ok(&repo, &["add", "experiment", "Batch webhooks in groups of 50", "--hypothesis", "fewer 429 errors"]); // BIL-3
    s.ok(&repo, &["log", "-i", "BIL-3", "-k", "finding", "provider rejects webhook batches above 10"]);
    let x = s.json(
        &repo,
        &[
            "status",
            "BIL-3",
            "failed",
            "-n",
            "batch cap 10",
            "--retry-when",
            "provider raises the batch limit",
            "--retry-after",
            "2020-01-01",
        ],
    );
    assert_eq!((x["status"].as_str(), x["retry_after"].as_str()), (Some("failed"), Some("2020-01-01")));
    s.ok(&repo, &["decide", "Send webhooks one at a time", "--because", "batches above 10 are rejected"]); // BIL-4
    let brief = s.ok(&repo, &["context"]);
    assert!(
        brief.contains("## Needs attention") && brief.contains("BIL-3 experiment [failed] worth retrying since"),
        "{brief}"
    );

    // History: the whole story, in order, each thing once, with the retry flag.
    let h = s.json(&repo, &["history", "webhook", "batching"]);
    let ids: Vec<String> = h
        .as_array()
        .unwrap()
        .iter()
        .map(|e| format!("{}:{}", e["kind"].as_str().unwrap(), e["item"].as_str().or(e["id"].as_str()).unwrap()))
        .collect();
    let pos = |k: &str| ids.iter().position(|x| x == k).unwrap_or_else(|| panic!("{k} missing from {ids:?}"));
    assert!(pos("idea:BIL-2") < pos("experiment:BIL-3") && pos("experiment:BIL-3") < pos("finding:BIL-3"), "{ids:?}");
    assert_eq!(ids.iter().filter(|x| x.ends_with("BIL-4")).count(), 1, "the decision appears once: {ids:?}");
    let exp = h.as_array().unwrap().iter().find(|e| e["id"] == "BIL-3").unwrap();
    assert!(exp["flag"].as_str().unwrap().contains("has passed"), "{exp}");

    // Timeline: newest first, with outcomes and new items; a topic narrows it.
    let t = s.json(&repo, &["timeline"]);
    let kinds: Vec<&str> = t.as_array().unwrap().iter().map(|e| e["kind"].as_str().unwrap()).collect();
    assert!(
        kinds.contains(&"new experiment") && kinds.contains(&"status-change") && kinds.contains(&"decision"),
        "{kinds:?}"
    );
    let topical = s.json(&repo, &["timeline", "--topic", "invoices table"]);
    assert!(topical.as_array().unwrap().iter().all(|e| e["title"].as_str().unwrap().contains("Invoices")), "{topical}");
    assert!(s.ok(&repo, &["timeline"]).contains("new resource"));

    // The message hook brings the failed experiment back when the topic comes up.
    let payload =
        serde_json::json!({"session_id": "s1", "cwd": repo, "prompt": "should we batch the webhooks?"}).to_string();
    let out = s.cmd(&s.root).args(["hook", "claude", "prompt"]).write_stdin(payload).output().unwrap();
    let out = String::from_utf8(out.stdout).unwrap();
    assert!(out.contains("BIL-3 experiment [failed]") && out.contains("t9 history"), "{out}");

    // Clearing a retry date.
    assert!(s.json(&repo, &["update", "BIL-3", "--retry-after", "none"])["retry_after"].is_null());
}

#[test]
fn insights_flag_loops_flip_flops_and_repeats() {
    let s = Sandbox::new();
    let repo = s.repo("bil", "git@github.com:acme/bil.git");
    s.ok(&repo, &["project", "init", "--key", "BIL"]);
    s.ok(&repo, &["add", "feature", "Webhooks"]); // BIL-1
    // A decision changed twice: BIL-2 → BIL-3 → BIL-4.
    s.ok(&repo, &["decide", "Retry webhooks with a fixed delay"]);
    s.ok(&repo, &["decide", "Retry webhooks with exponential backoff", "-s", "BIL-2"]);
    s.ok(&repo, &["decide", "Retry webhooks with a fixed delay after all", "-s", "BIL-3"]);
    // An experiment that failed, and new work that looks just like it.
    s.ok(&repo, &["add", "experiment", "Batch webhook deliveries per merchant", "--parent", "BIL-1", "-s", "failed"]); // BIL-5
    s.ok(&repo, &["add", "task", "Batch webhook deliveries per merchant again", "--parent", "BIL-1"]); // BIL-6
    // Reopened: done, then open again.
    s.ok(&repo, &["update", "BIL-6", "--status", "done"]);
    s.ok(&repo, &["update", "BIL-6", "--status", "open"]);
    // Issues piling up under the feature with nothing finished.
    for n in 0..5 {
        s.ok(&repo, &["add", "bug", &format!("Webhook signature mismatch case {n}"), "--parent", "BIL-1"]);
    }

    let ins = s.json(&repo, &["insights"]);
    let kinds: Vec<&str> = ins["signals"].as_array().unwrap().iter().map(|x| x["kind"].as_str().unwrap()).collect();
    for k in ["flip-flop", "reopened", "repeat", "spinning"] {
        assert!(kinds.contains(&k), "{k} missing: {ins}");
    }
    let ff = ins["signals"].as_array().unwrap().iter().find(|x| x["kind"] == "flip-flop").unwrap();
    assert_eq!(ff["detail"], "BIL-2 → BIL-3 → BIL-4");
    let repeats: Vec<&Value> = ins["signals"].as_array().unwrap().iter().filter(|x| x["kind"] == "repeat").collect();
    assert!(repeats.iter().any(|r| r["items"] == serde_json::json!(["BIL-6", "BIL-5"])), "{repeats:?}");
    // Going back to a superseded decision is a repeat too.
    assert!(repeats.iter().any(|r| r["items"] == serde_json::json!(["BIL-4", "BIL-2"])), "{repeats:?}");
    assert!(ins["branches"].as_array().unwrap().iter().any(|b| b["id"] == "BIL-1" && b["spinning"] == true), "{ins}");

    // The brief warns the agent; the text output is readable.
    let brief = s.ok(&repo, &["context"]);
    assert!(brief.contains("## Needs attention") && brief.contains("[open] spinning:"), "{brief}");
    assert!(s.ok(&repo, &["loops"]).contains("Spinning: Webhooks"));
}

#[test]
fn goals_due_dates_ripe_work_and_timely_reminders() {
    let s = Sandbox::new();
    let repo = s.repo("bil", "git@github.com:acme/bil.git");
    s.ok(&repo, &["project", "init", "--key", "BIL"]);
    s.ok(&repo, &["add", "feature", "Webhooks", "--goal", "merchants hear about payments within a minute"]); // BIL-1
    s.ok(&repo, &["add", "task", "Sign payloads", "--parent", "BIL-1"]); // BIL-2
    s.ok(&repo, &["add", "task", "Ship the invoice export", "--due", "2020-01-01"]); // BIL-3, overdue
    s.ok(&repo, &["add", "task", "Re-measure latency", "--after", "BIL-2"]); // BIL-4 waits on BIL-2
    let x = s.json(&repo, &["add", "task", "Write the runbook", "--due", "today"]); // BIL-5
    assert!(x["due"].is_string(), "{x}");

    // The goal is inherited, labelled with where it came from.
    let st = s.json(&repo, &["--agent", "claude-code", "focus", "BIL-2"]);
    assert!(st["item"]["goal"].is_null());
    let brief = s.ok(&repo, &["--agent", "claude-code", "context"]);
    assert!(brief.contains("goal (inferred from BIL-1"), "{brief}");
    assert!(brief.contains("merchants hear about payments"), "{brief}");
    s.ok(&repo, &["--agent", "claude-code", "focus", "BIL-2", "--goal", "receivers can verify who sent a webhook"]);
    assert!(
        s.ok(&repo, &["--agent", "claude-code", "context"]).contains("  goal: receivers can verify who sent a webhook")
    );

    // Closing without an outcome nudges; closing what others wait on says they are ripe now.
    let out = s.cmd(&repo).args(["done", "BIL-2"]).output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("closed without an outcome") && err.contains("BIL-4 was waiting on BIL-2"), "{err}");

    // One ranked list: overdue and due today first, then ripe, then closed without an outcome.
    let p = s.json(&repo, &["pending"]);
    let reasons: Vec<(String, String)> = p["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| (a["id"].as_str().unwrap().to_string(), a["reason"].as_str().unwrap().to_string()))
        .collect();
    let has = |id: &str, r: &str| reasons.iter().any(|(i, x)| i == id && x == r);
    assert!(
        has("BIL-3", "overdue") && has("BIL-5", "due-soon") && has("BIL-4", "ripe") && has("BIL-2", "unexplained"),
        "{reasons:?}"
    );
    assert_eq!(reasons[0].1, "overdue", "{reasons:?}");
    let brief = s.ok(&repo, &["--agent", "claude-code", "context"]);
    assert!(brief.contains("## Needs attention") && brief.contains("BIL-3 task [open] overdue"), "{brief}");

    // A check-in or progress clears ripe; check-ins need a reason.
    assert!(!s.cmd(&repo).args(["still", "BIL-4"]).output().unwrap().status.success());
    s.ok(&repo, &["still", "BIL-4", "-n", "waiting for a quiet hour to measure"]);
    let p = s.json(&repo, &["pending"]);
    assert!(!p["items"].as_array().unwrap().iter().any(|a| a["id"] == "BIL-4"), "{p}");

    // The message hook reminds once per item and reason; things the session-start brief listed are not repeated.
    let start = serde_json::json!({"session_id": "h1", "cwd": repo, "source": "startup"}).to_string();
    let brief = s.cmd(&s.root).args(["hook", "claude", "session-start"]).write_stdin(start).output().unwrap();
    assert!(String::from_utf8_lossy(&brief.stdout).contains("BIL-3"));
    let prompt = serde_json::json!({"session_id": "h1", "cwd": repo, "prompt": "what next?"}).to_string();
    let first = s.cmd(&s.root).args(["hook", "claude", "prompt"]).write_stdin(prompt.clone()).output().unwrap();
    assert!(!String::from_utf8_lossy(&first.stdout).contains("t9 reminder"), "already in the brief");
    // Something new becomes ripe mid-session (another session closes what it waits on).
    s.ok(&repo, &["add", "task", "Rotate the signing key", "--after", "BIL-5"]); // BIL-6
    s.ok(&repo, &["done", "BIL-5", "-n", "runbook in docs/runbook.md"]);
    let second = s.cmd(&s.root).args(["hook", "claude", "prompt"]).write_stdin(prompt.clone()).output().unwrap();
    let second = String::from_utf8_lossy(&second.stdout).to_string();
    assert!(second.contains("t9 reminder") && second.contains("BIL-6 ripe: BIL-5 closed"), "{second}");
    let third = s.cmd(&s.root).args(["hook", "claude", "prompt"]).write_stdin(prompt).output().unwrap();
    assert!(!String::from_utf8_lossy(&third.stdout).contains("BIL-6"), "shown once");
}
