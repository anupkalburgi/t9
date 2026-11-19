//! `t9`: the t9 command line. Every command is a thin call into `t9_core::Store`; `--json` prints the
//! same data the REST API and MCP tools return.

mod hook;
mod install;
mod print;

use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use t9_core::decisions::DecisionInput;
use t9_core::index::{EntryFilter, ItemFilter};
use t9_core::*;

#[derive(Parser)]
#[command(
    name = "t9",
    version,
    about = "t9: shared, persistent project memory for coding agents and the humans running them"
)]
#[command(after_help = "Start here: `t9 context` prints the brief for the project in the current directory.")]
struct Cli {
    /// Print JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    /// Act as if run from this directory.
    #[arg(long, global = true, value_name = "DIR")]
    cwd: Option<PathBuf>,
    /// Who is writing: claude-code, codex, human, …
    #[arg(long, global = true, env = "T9_AGENT")]
    agent: Option<String>,
    /// Session id (or its last characters). Default: the open session for this directory.
    #[arg(long, global = true, env = "T9_SESSION")]
    session: Option<String>,
    /// Project key or slug. Default: resolved from the directory.
    #[arg(long, short = 'P', global = true, env = "T9_PROJECT")]
    project: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print the project brief for this directory: handoff, focus, plan, open issues, decisions, recent activity.
    Context {
        /// Approximate token budget for the brief.
        #[arg(long, default_value_t = 1500)]
        budget: usize,
    },
    /// Show which project and session this directory resolves to.
    Whereami,
    /// Register, link and list projects.
    #[command(subcommand)]
    Project(ProjectCmd),
    /// Start, end and list agent sessions.
    #[command(subcommand)]
    Session(SessionCmd),
    /// Set the item you are working on; later `t9 log` entries attach to it.
    Focus {
        id: Option<String>,
        /// Clear the focus.
        #[arg(long, conflicts_with = "id")]
        clear: bool,
        /// Do not move an `open` item to `active`.
        #[arg(long)]
        no_activate: bool,
    },
    /// Add an item: plan, phase, feature, task, bug, regression, revisit, decision, question, note.
    Add(AddArgs),
    /// Append a journal entry (progress by default). Use `-` to read the message from stdin.
    Log {
        #[arg(required = true, num_args = 1..)]
        message: Vec<String>,
        #[arg(long, short, default_value = "progress")]
        kind: EntryKind,
        /// Item to file the entry under (default: the session focus).
        #[arg(long, short)]
        item: Option<String>,
        /// Files, commits, run folders this entry refers to.
        #[arg(long = "ref", short)]
        refs: Vec<String>,
    },
    /// Change an item's status.
    Status {
        id: String,
        status: Status,
        #[arg(long, short)]
        note: Option<String>,
    },
    /// Mark an item done.
    Done {
        id: String,
        #[arg(long, short)]
        note: Option<String>,
    },
    /// Change an item's fields.
    Update(UpdateArgs),
    /// Record a decision: what was chosen, why, and what was rejected. Replaces earlier decisions with -s.
    Decide {
        #[arg(required = true, num_args = 1..)]
        title: Vec<String>,
        #[command(flatten)]
        d: DecisionArgs,
        /// Decisions this one replaces (they become `superseded`).
        #[arg(long, short)]
        supersedes: Vec<String>,
        /// Record it as proposed, for the human to accept later.
        #[arg(long)]
        proposed: bool,
    },
    /// Record an option that was considered and rejected, so it is not proposed again.
    Reject {
        #[arg(required = true, num_args = 1..)]
        title: Vec<String>,
        #[command(flatten)]
        d: DecisionArgs,
    },
    /// Has this been decided, rejected or parked before? Searches decisions and ideas (this project and global).
    Check {
        #[arg(required = true, num_args = 1..)]
        query: Vec<String>,
        #[arg(long, short = 'n', default_value_t = 10)]
        limit: usize,
    },
    /// The decision log: accepted, proposed and rejected decisions, newest first.
    Decisions {
        /// Only these areas.
        #[arg(long)]
        area: Vec<String>,
        /// Include superseded decisions.
        #[arg(long, short)]
        all: bool,
        /// The global log (decisions for every project).
        #[arg(long, short)]
        global: bool,
        /// Print a Markdown document (ADR style) instead of a list.
        #[arg(long)]
        md: bool,
    },
    /// Write a handoff for the next session: where you stopped and what comes next. `-` reads stdin.
    Handoff {
        #[arg(required = true, num_args = 1..)]
        message: Vec<String>,
    },
    /// List items (open ones by default).
    Ls(LsArgs),
    /// Show an item with its children, links and journal.
    Show {
        id: String,
        /// Number of journal entries to show.
        #[arg(long, short = 'n', default_value_t = 15)]
        entries: usize,
    },
    /// Show journal entries, newest first.
    #[command(alias = "timeline")]
    Journal {
        #[arg(long, short)]
        item: Option<String>,
        /// Only this session (id or suffix).
        #[arg(long = "of-session")]
        of_session: Option<String>,
        #[arg(long, short)]
        kind: Vec<EntryKind>,
        /// Only the last N days.
        #[arg(long)]
        days: Option<i64>,
        #[arg(long, short = 'n', default_value_t = 30)]
        limit: usize,
        #[arg(long, short = 'A')]
        all_projects: bool,
    },
    /// Full-text search over items and journal entries.
    Search {
        #[arg(required = true, num_args = 1..)]
        query: Vec<String>,
        #[arg(long, short = 'A')]
        all_projects: bool,
        #[arg(long, short = 'n', default_value_t = 20)]
        limit: usize,
    },
    /// List entries that are not filed under an item, and inbox entries from unlinked directories.
    Triage {
        #[arg(long, default_value_t = 14)]
        days: i64,
    },
    /// File a journal entry under an item (moves inbox entries into the item's project).
    Attach { entry: String, item: String },
    /// Open an item's file in $EDITOR.
    Edit { id: String },
    /// Rebuild the index from the vault.
    Reindex,
    /// Check the vault and index, and report stale or inconsistent items.
    Doctor,
    /// Install agent integrations: the Claude Code plugin, or an AGENTS.md section for Codex and others.
    #[command(subcommand)]
    Install(install::InstallCmd),
    /// Terminal UI: projects, brief, item tree, decision log, journal, triage, search. Opens on this directory's
    /// project.
    Tui,
    /// The vault's git history: commit now, show the path, or run git in it.
    #[command(subcommand)]
    Vault(VaultCmd),
    /// Entry points for agent harness hooks (read the hook payload from stdin).
    #[command(subcommand, hide = true)]
    Hook(hook::HookCmd),
}

#[derive(Subcommand)]
enum VaultCmd {
    /// Commit vault changes now (t9 also does this automatically after every write).
    Commit {
        /// Wait, commit quietly and log errors to git.log (used by t9 itself after writes).
        #[arg(long, hide = true)]
        background: bool,
    },
    /// Print the vault path.
    Path,
    /// Run git in the vault, e.g. `t9 vault git log --oneline -10`.
    Git {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

fn vault_cmd(env: &Env, cmd: VaultCmd) -> Result<()> {
    match cmd {
        VaultCmd::Commit { background: true } => {
            std::thread::sleep(std::time::Duration::from_millis(env.git_delay_ms));
            match t9_core::vcs::commit(env) {
                Ok(Some(subject)) => t9_core::vcs::log_line(env, &format!("ok: {subject}")),
                Ok(None) => {}
                Err(e) => t9_core::vcs::log_line(env, &format!("error: {e}")),
            }
        }
        VaultCmd::Commit { background: false } => match t9_core::vcs::commit(env)? {
            Some(subject) => println!("committed: {subject}"),
            None => println!("nothing to commit"),
        },
        VaultCmd::Path => println!("{}", env.vault.display()),
        VaultCmd::Git { args } => {
            let status = std::process::Command::new("git").arg("-C").arg(&env.vault).args(args).status()?;
            if !status.success() {
                bail!("git exited with {status}");
            }
        }
    }
    Ok(())
}

#[derive(Subcommand)]
enum ProjectCmd {
    /// Register this directory (its git work tree) as a new project.
    Init {
        /// Default: the directory name.
        #[arg(long, short)]
        name: Option<String>,
        /// Short id prefix for items, e.g. BIL. Default: derived from the name.
        #[arg(long, short)]
        key: Option<String>,
        #[arg(long, short)]
        description: Option<String>,
        /// Do not register the current directory as a root.
        #[arg(long)]
        no_root: bool,
    },
    /// Add this directory (and its git remote) to an existing project.
    Link { key: String },
    /// List projects.
    Ls,
    /// Show a project (default: the current one).
    Show { key: Option<String> },
}

#[derive(Subcommand)]
enum SessionCmd {
    /// Start (or resume, with the same --harness-id) a session in this directory.
    Start {
        #[arg(long)]
        harness_id: Option<String>,
        /// Also print the context brief.
        #[arg(long)]
        context: bool,
        #[arg(long, default_value_t = 1500)]
        budget: usize,
    },
    /// End the current session, optionally writing a handoff.
    End {
        #[arg(long)]
        handoff: Option<String>,
    },
    /// List sessions.
    Ls {
        #[arg(long)]
        open: bool,
        #[arg(long, short = 'A')]
        all_projects: bool,
        #[arg(long, short = 'n', default_value_t = 20)]
        limit: usize,
    },
}

#[derive(Args)]
struct DecisionArgs {
    /// Why. Becomes the `## Why` section.
    #[arg(long, short = 'w')]
    because: Option<String>,
    /// An alternative that was rejected, "option: why not" (repeatable).
    #[arg(long, short = 'r')]
    rejected: Vec<String>,
    /// Topic, e.g. architecture, testing, storage (repeatable).
    #[arg(long)]
    area: Vec<String>,
    /// Item this applies to; it then applies to everything under it (repeatable).
    #[arg(long = "for", value_name = "ID")]
    applies_to: Vec<String>,
    /// More notes; `-` reads stdin.
    #[arg(long, short)]
    body: Option<String>,
    #[arg(long)]
    parent: Option<String>,
    /// Record in the global log: applies to every project.
    #[arg(long, short)]
    global: bool,
}

impl DecisionArgs {
    fn into_input(self, title: String, status: Status, supersedes: Vec<String>) -> Result<DecisionInput> {
        Ok(DecisionInput {
            title,
            body: self.body.as_deref().map(text).transpose()?.unwrap_or_default(),
            because: self.because.as_deref().map(text).transpose()?,
            rejected: self.rejected,
            areas: self.area,
            applies_to: self.applies_to,
            supersedes,
            parent: self.parent,
            status,
            global: self.global,
        })
    }
}

#[derive(Args)]
struct AddArgs {
    kind: Kind,
    #[arg(required = true, num_args = 1..)]
    title: Vec<String>,
    /// Topic, e.g. architecture, testing (repeatable). Decisions in the same area are shown when it is focused.
    #[arg(long)]
    area: Vec<String>,
    #[arg(long)]
    parent: Option<String>,
    /// Description; `-` reads stdin.
    #[arg(long, short)]
    body: Option<String>,
    #[arg(long, short)]
    tag: Vec<String>,
    /// 0 (highest) to 9.
    #[arg(long, short = 'p', value_parser = clap::value_parser!(u8).range(0..=9))]
    priority: Option<u8>,
    #[arg(long, short)]
    status: Option<Status>,
    /// Typed link `rel:ID`, e.g. `regression-of:BIL-31`.
    #[arg(long, short)]
    link: Vec<Link>,
    /// Also set it as the session focus.
    #[arg(long, short)]
    focus: bool,
}

#[derive(Args)]
struct UpdateArgs {
    id: String,
    /// Add an area (repeatable).
    #[arg(long)]
    area: Vec<String>,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    status: Option<Status>,
    /// New parent; `none` makes it top-level.
    #[arg(long)]
    parent: Option<String>,
    /// 0-9, or `none`.
    #[arg(long, short = 'p')]
    priority: Option<String>,
    #[arg(long, short)]
    tag: Vec<String>,
    #[arg(long)]
    untag: Vec<String>,
    #[arg(long, short)]
    link: Vec<Link>,
    /// Replace the body; `-` reads stdin.
    #[arg(long, short)]
    body: Option<String>,
    /// Fail unless the item is at this rev.
    #[arg(long)]
    rev: Option<u32>,
    #[arg(long, short)]
    note: Option<String>,
}

#[derive(Args)]
struct LsArgs {
    #[arg(long, short)]
    kind: Vec<Kind>,
    #[arg(long, short)]
    status: Vec<Status>,
    /// Include done, dropped and superseded items.
    #[arg(long, short)]
    all: bool,
    /// Show as a tree.
    #[arg(long, short)]
    tree: bool,
    /// Only children of this item.
    #[arg(long)]
    parent: Option<String>,
    #[arg(long, short = 'A')]
    all_projects: bool,
    #[arg(long, short = 'n')]
    limit: Option<usize>,
}

/// `-` means stdin; anything else is the text itself.
fn text(arg: &str) -> Result<String> {
    if arg == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).context("reading stdin")?;
        Ok(s)
    } else {
        Ok(arg.to_string())
    }
}

fn words(v: &[String]) -> Result<String> {
    if v.len() == 1 { text(&v[0]) } else { Ok(v.join(" ")) }
}

fn emit<T: Serialize>(json: bool, value: &T, human: impl FnOnce(&T)) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        human(value);
    }
    Ok(())
}

struct App {
    store: Store,
    caller: Caller,
    json: bool,
}

impl App {
    /// The project for a read command: explicit or resolved. Errors with guidance if neither.
    fn project_key(&mut self) -> Result<String> {
        let res = self.store.resolve(&self.caller)?;
        match res.project {
            Some(p) => {
                self.store.sync_project(&p.meta.key)?;
                Ok(p.meta.key)
            }
            None => bail!(
                "this directory is not linked to a project; run `t9 context` for suggestions, or pass --project KEY / -A"
            ),
        }
    }

    fn scope(&mut self, all_projects: bool) -> Result<Option<String>> {
        if all_projects {
            self.store.sync_all()?;
            Ok(None)
        } else {
            Ok(Some(self.project_key()?))
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let cwd = match cli.cwd {
        Some(c) => c,
        None => std::env::current_dir().context("current directory")?,
    };
    // Agents run t9 without a terminal; a person typing at one is a human unless they say otherwise.
    let agent = cli.agent.or_else(|| std::io::stdin().is_terminal().then(|| "human".to_string()));
    let caller = Caller { cwd, agent, session: cli.session, project: cli.project };
    let trace = std::env::var_os("T9_TRACE").is_some();
    let t0 = std::time::Instant::now();
    let env = Env::from_env()?;
    if let Cmd::Vault(v) = cli.cmd {
        return vault_cmd(&env, v);
    }
    if let Cmd::Tui = cli.cmd {
        return t9_tui::run(Store::open(env)?, &caller);
    }
    let mut store = Store::open(env)?;
    store.commit_on_drop = true;
    if trace {
        eprintln!("trace: open {:?}", t0.elapsed());
    }
    let mut app = App { store, caller, json: cli.json };
    let json = app.json;

    match cli.cmd {
        Cmd::Context { budget } => {
            let brief = context::build(&mut app.store, &app.caller)?;
            emit(json, &brief, |b| print!("{}", context::render(b, budget)))?;
        }
        Cmd::Whereami => {
            let res = app.store.resolve(&app.caller)?;
            let session = app.store.current_session(&app.caller)?;
            #[derive(Serialize)]
            struct W<'a> {
                resolution: &'a Resolution,
                session: Option<Session>,
                vault: String,
                home: String,
            }
            let w = W {
                resolution: &res,
                session,
                vault: app.store.env.vault.display().to_string(),
                home: app.store.env.home.display().to_string(),
            };
            emit(json, &w, |w| print::whereami(w.resolution, w.session.as_ref(), &w.vault))?;
        }
        Cmd::Project(p) => project_cmd(&mut app, p)?,
        Cmd::Session(s) => session_cmd(&mut app, s)?,
        Cmd::Focus { id, clear, no_activate } => {
            if id.is_none() && !clear {
                let s = app.store.current_session(&app.caller)?;
                let focus = s.as_ref().and_then(|s| s.meta.focus.clone());
                return emit(json, &focus, |f| match f {
                    Some(f) => println!("{f}"),
                    None => println!("no focus (set one with `t9 focus ID`)"),
                });
            }
            let (session, item) = app.store.focus(&app.caller, id.as_deref(), !no_activate)?;
            let decisions = match &item {
                Some(i) => app.store.applicable_decisions(i, 5)?,
                None => vec![],
            };
            #[derive(Serialize)]
            struct F<'a> {
                session: &'a Session,
                item: &'a Option<Item>,
                decisions: &'a [t9_core::decisions::Relevant],
            }
            emit(json, &F { session: &session, item: &item, decisions: &decisions }, |f| match f.item {
                Some(i) => {
                    println!(
                        "focus {} [{}] {} (session {})",
                        i.meta.id,
                        i.meta.status,
                        i.meta.title,
                        f.session.short_id()
                    );
                    if !f.decisions.is_empty() {
                        println!("\ndecisions that apply:");
                        print::relevant(f.decisions, "");
                    }
                }
                None => println!("focus cleared (session {})", f.session.short_id()),
            })?;
        }
        Cmd::Add(a) => {
            let ni = NewItem {
                kind: Some(a.kind),
                title: a.title.join(" "),
                body: a.body.as_deref().map(text).transpose()?.unwrap_or_default(),
                parent: a.parent,
                priority: a.priority,
                tags: a.tag,
                areas: a.area,
                links: a.link,
                status: a.status,
            };
            let mut item = app.store.add_item(&app.caller, ni)?;
            if a.focus {
                let caller = Caller { project: Some(item.project.clone()), ..app.caller.clone() };
                item = app.store.focus(&caller, Some(&item.meta.id), true)?.1.unwrap_or(item);
            }
            emit(json, &item, |i| {
                println!("added {} {} [{}] {}", i.meta.id, i.meta.kind, i.meta.status, i.meta.title)
            })?;
        }
        Cmd::Log { message, kind, item, refs } => {
            let e = app.store.log(&app.caller, LogInput { kind, body: words(&message)?, item, refs })?;
            emit(json, &e, print::logged)?;
        }
        Cmd::Status { id, status, note } => {
            let i = app.store.update_item(
                &app.caller,
                &id,
                ItemPatch { status: Some(status), note, ..Default::default() },
            )?;
            emit(json, &i, |i| println!("{} is now {}", i.meta.id, i.meta.status))?;
        }
        Cmd::Done { id, note } => {
            let i = app.store.update_item(
                &app.caller,
                &id,
                ItemPatch { status: Some(Status::Done), note, ..Default::default() },
            )?;
            emit(json, &i, |i| println!("{} is now {}", i.meta.id, i.meta.status))?;
        }
        Cmd::Update(u) => {
            let none_or = |s: &str| s.eq_ignore_ascii_case("none");
            let patch = ItemPatch {
                title: u.title,
                status: u.status,
                parent: u.parent.map(|p| if none_or(&p) { None } else { Some(p) }),
                priority: u
                    .priority
                    .map(|p| {
                        if none_or(&p) {
                            Ok(None)
                        } else {
                            p.parse::<u8>().map(Some).context("priority must be 0-9 or none")
                        }
                    })
                    .transpose()?,
                add_tags: u.tag,
                add_areas: u.area,
                remove_tags: u.untag,
                add_links: u.link,
                body: u.body.as_deref().map(text).transpose()?,
                expect_rev: u.rev,
                note: u.note,
            };
            let i = app.store.update_item(&app.caller, &u.id, patch)?;
            emit(json, &i, |i| println!("updated {} (rev {})", i.meta.id, i.meta.rev))?;
        }
        Cmd::Decide { title, d, supersedes, proposed } => {
            let status = if proposed { Status::Proposed } else { Status::Accepted };
            let d = app.store.decide(&app.caller, d.into_input(title.join(" "), status, supersedes)?)?;
            emit(json, &d, |d| {
                let verb = if d.meta.status == Status::Proposed { "proposed" } else { "decided" };
                println!("{verb} {} {}", d.meta.id, d.meta.title);
                for l in d.meta.links.iter().filter(|l| l.rel == "supersedes") {
                    println!("  {} is now superseded", l.to);
                }
            })?;
        }
        Cmd::Reject { title, d } => {
            let d = app.store.decide(&app.caller, d.into_input(title.join(" "), Status::Rejected, vec![])?)?;
            emit(json, &d, |d| println!("rejected {} {}", d.meta.id, d.meta.title))?;
        }
        Cmd::Check { query, limit } => {
            let project = app.store.resolve(&app.caller)?.project.map(|p| p.meta.key);
            let found = app.store.check(&query.join(" "), project.as_deref(), limit)?;
            emit(json, &found, |f| print::relevant(f, "nothing decided, rejected or parked about this yet"))?;
        }
        Cmd::Decisions { area, all, global, md } => {
            let key = if global { app.store.ensure_global()?.meta.key } else { app.project_key()? };
            let items = app.store.decision_log(&key, &area, all)?;
            if md {
                let project = app.store.project(&key)?;
                print!("{}", t9_core::decisions::render_log_md(&project, &items));
            } else {
                emit(json, &items, |items| print::decision_log(items))?;
            }
        }
        Cmd::Handoff { message } => {
            let e = app.store.log(
                &app.caller,
                LogInput { kind: EntryKind::Handoff, body: words(&message)?, item: None, refs: vec![] },
            )?;
            emit(json, &e, print::logged)?;
        }
        Cmd::Ls(a) => {
            let project = app.scope(a.all_projects)?;
            let parent = a.parent.map(|p| normalize_item_id(&p)).transpose()?;
            let f = ItemFilter {
                project,
                kinds: a.kind,
                statuses: a.status,
                include_closed: a.all,
                parent: parent.map(Some),
                limit: a.limit,
            };
            let items = app.store.items(&f)?;
            emit(json, &items, |items| print::items(items, a.tree, a.all_projects))?;
        }
        Cmd::Show { id, entries } => {
            let item = app.store.item(&id)?;
            let children = app.store.items(&ItemFilter {
                parent: Some(Some(item.meta.id.clone())),
                include_closed: true,
                ..Default::default()
            })?;
            let journal = app.store.entries(&EntryFilter {
                item: Some(item.meta.id.clone()),
                limit: Some(entries),
                ..Default::default()
            })?;
            #[derive(Serialize)]
            struct S {
                item: Item,
                children: Vec<Item>,
                entries: Vec<Entry>,
            }
            emit(json, &S { item, children, entries: journal }, |s| print::show(&s.item, &s.children, &s.entries))?;
        }
        Cmd::Journal { item, of_session, kind, days, limit, all_projects } => {
            let project = if item.is_some() { None } else { app.scope(all_projects)? };
            let item = item.map(|i| app.store.item(&i).map(|i| i.meta.id)).transpose()?;
            let session = match of_session {
                Some(s) => Some(
                    app.store
                        .current_session(&Caller { session: Some(s), ..app.caller.clone() })?
                        .map(|s| s.meta.id)
                        .unwrap_or_default(),
                ),
                None => None,
            };
            let f = EntryFilter {
                project,
                item,
                session,
                kinds: kind,
                since: days.map(|d| now() - chrono::Duration::days(d)),
                limit: Some(limit),
                ..Default::default()
            };
            let entries = app.store.entries(&f)?;
            emit(json, &entries, |e| print::entries(e, all_projects))?;
        }
        Cmd::Search { query, all_projects, limit } => {
            let project = app.scope(all_projects)?;
            let hits = app.store.search(&query.join(" "), project.as_deref(), limit)?;
            emit(json, &hits, |h| print::hits(h, all_projects))?;
        }
        Cmd::Triage { days } => {
            let since = Some(now() - chrono::Duration::days(days));
            let res = app.store.resolve(&app.caller)?;
            let key = res.project.as_ref().map(|p| p.meta.key.clone());
            if let Some(k) = &key {
                app.store.sync_project(k)?;
            }
            app.store.sync_project(INBOX)?;
            let unfiled = match &key {
                Some(k) => app.store.entries(&EntryFilter {
                    project: Some(k.clone()),
                    unfiled: true,
                    exclude_kinds: vec![EntryKind::Handoff],
                    since,
                    ..Default::default()
                })?,
                None => vec![],
            };
            let inbox = app.store.entries(&EntryFilter { project: Some(INBOX.into()), since, ..Default::default() })?;
            #[derive(Serialize)]
            struct T {
                unfiled: Vec<Entry>,
                inbox: Vec<Entry>,
            }
            emit(json, &T { unfiled, inbox }, |t| print::triage(&t.unfiled, &t.inbox))?;
        }
        Cmd::Attach { entry, item } => {
            let e = app.store.attach(&entry, &item)?;
            emit(json, &e, |e| {
                println!("filed {} under {}", short_id(&e.meta.id), e.meta.item.as_deref().unwrap_or(""))
            })?;
        }
        Cmd::Edit { id } => {
            let item = app.store.item(&id)?;
            let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "vi".into());
            let status = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!("{editor} \"$1\""))
                .arg("sh")
                .arg(&item.path)
                .status()
                .context("starting editor")?;
            if !status.success() {
                bail!("editor exited with {status}");
            }
            let item = app.store.item(&id)?;
            emit(json, &item, |i| println!("{} saved (rev {})", i.meta.id, i.meta.rev))?;
        }
        Cmd::Reindex => {
            let t = std::time::Instant::now();
            let stats = app.store.reindex()?;
            emit(json, &stats, |s| {
                println!(
                    "indexed {} projects, {} items, {} entries, {} sessions in {:?}",
                    s.projects,
                    s.items,
                    s.entries,
                    s.sessions,
                    t.elapsed()
                );
                for w in &s.warnings {
                    eprintln!("warning: {w}");
                }
            })?;
        }
        Cmd::Doctor => print::doctor(&mut app.store, json)?,
        Cmd::Hook(h) => hook::run(&mut app.store, &app.caller, h)?,
        Cmd::Install(i) => install::run(&app.store.env, i)?,
        Cmd::Vault(_) | Cmd::Tui => unreachable!("handled before the store is opened"),
    }
    if trace {
        eprintln!("trace: total {:?}", t0.elapsed());
    }
    Ok(())
}

fn project_cmd(app: &mut App, cmd: ProjectCmd) -> Result<()> {
    let json = app.json;
    match cmd {
        ProjectCmd::Init { name, key, description, no_root } => {
            let cwd = app.caller.cwd.clone();
            let name = match name {
                Some(n) => n,
                None => {
                    let dir = t9_core::git::discover(&cwd).map(|g| g.root).unwrap_or(cwd.clone());
                    dir.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .context("cannot derive a name; pass --name")?
                }
            };
            let p = app.store.init_project(
                &name,
                key.as_deref(),
                description.as_deref().unwrap_or(""),
                (!no_root).then_some(cwd.as_path()),
            )?;
            let adopted = app.store.adopt_inbox(&p.meta.key)?;
            emit(json, &p, |p| {
                println!(
                    "created project {} ({}) — items will be numbered {}-1, {}-2, …",
                    p.meta.name, p.meta.key, p.meta.key, p.meta.key
                );
                print::project(p);
                print::adopted(adopted);
            })?;
        }
        ProjectCmd::Link { key } => {
            let p = app.store.link_project(&key, &app.caller.cwd)?;
            let adopted = app.store.adopt_inbox(&p.meta.key)?;
            emit(json, &p, |p| {
                println!("linked to {} ({})", p.meta.name, p.meta.key);
                print::project(p);
                print::adopted(adopted);
            })?;
        }
        ProjectCmd::Ls => {
            let ps = app.store.projects()?;
            emit(json, &ps, |ps| {
                if ps.is_empty() {
                    println!("no projects yet; run `t9 project init` in a project directory");
                }
                for p in ps {
                    let roots = p
                        .meta
                        .roots
                        .iter()
                        .map(|r| t9_core::env::display_path(std::path::Path::new(r)))
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!("{:<8} {:<28} {}", p.meta.key, p.meta.name, roots);
                }
            })?;
        }
        ProjectCmd::Show { key } => {
            let p = match key {
                Some(k) => app.store.project(&k)?,
                None => {
                    let k = app.project_key()?;
                    app.store.project(&k)?
                }
            };
            emit(json, &p, print::project)?;
        }
    }
    Ok(())
}

fn session_cmd(app: &mut App, cmd: SessionCmd) -> Result<()> {
    let json = app.json;
    match cmd {
        SessionCmd::Start { harness_id, context: with_context, budget } => {
            let s = app.store.start_session(&app.caller, harness_id.as_deref())?;
            if with_context {
                let caller = Caller { session: Some(s.meta.id.clone()), ..app.caller.clone() };
                let brief = context::build(&mut app.store, &caller)?;
                return emit(json, &brief, |b| print!("{}", context::render(b, budget)));
            }
            emit(json, &s, |s| println!("session {} ({}) in {}", s.meta.id, s.meta.agent, s.meta.project))?;
        }
        SessionCmd::End { handoff } => {
            let handoff = handoff.as_deref().map(text).transpose()?;
            let s = app.store.end_session(&app.caller, handoff.as_deref())?;
            emit(json, &s, |s| match s {
                Some(s) => println!("ended session {}", s.short_id()),
                None => println!("no open session here"),
            })?;
        }
        SessionCmd::Ls { open, all_projects, limit } => {
            let project = app.scope(all_projects)?;
            let ss = app.store.sessions(project.as_deref(), open, limit)?;
            emit(json, &ss, |ss| print::sessions(ss))?;
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.json;
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if json && !std::io::stderr().is_terminal() {
                eprintln!("{}", serde_json::json!({ "error": format!("{e:#}") }));
            } else {
                eprintln!("t9: {e:#}");
            }
            ExitCode::FAILURE
        }
    }
}
