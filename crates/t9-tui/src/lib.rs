//! `t9 tui`: a terminal UI over the vault.
//!
//! It reads and writes through `t9-core` in-process (see `data.rs`) and refreshes every 2 seconds, so work
//! recorded by agents appears while it is open. No daemon is needed.

mod app;
mod data;
mod ui;

use std::time::Duration;

use anyhow::{Context, Result};
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use t9_core::{Caller, Store};

pub use app::{Action, App, Mode, Screen, Tab};

/// Runs the TUI until the user quits. Opens on the project of `caller.cwd` if it resolves to one.
pub fn run(mut store: Store, caller: &Caller) -> Result<()> {
    let initial = store.resolve(caller)?.project.map(|p| p.meta.key);
    store.commit_on_drop = false;
    // Opened inside a git repository that has no project: offer to set it up.
    let unlinked_here = initial.is_none() && t9_core::git::discover(&caller.cwd).is_some();
    if unlinked_here {
        store.note_unlinked(&caller.cwd, "human")?;
    }
    let folder = store.folder_of(&caller.cwd);
    let mut app = App::new(store, initial)?;
    if unlinked_here {
        app.select_folder(&folder);
    }
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app);
    ratatui::restore();
    if app.store.wrote.get() {
        app.committed_writes();
    }
    // End the TUI's own session, if a write started one.
    let _ = app.store.end_session(&app.caller(), None);
    if app.store.wrote.get() {
        app.committed_writes();
    }
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, app))?;
        if event::poll(Duration::from_millis(500))?
            && let Event::Key(k) = event::read()?
            && k.kind == KeyEventKind::Press
        {
            match app.on_key(k) {
                Action::Quit => return Ok(()),
                Action::Edit(path) => {
                    ratatui::restore();
                    let editor =
                        std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "vi".into());
                    let status = std::process::Command::new("sh")
                        .arg("-c")
                        .arg(format!("{editor} \"$1\""))
                        .arg("sh")
                        .arg(&path)
                        .status()
                        .context("starting editor")?;
                    *terminal = ratatui::init();
                    app.message = Some(if status.success() {
                        format!("saved {}", path.file_name().unwrap_or_default().to_string_lossy())
                    } else {
                        format!("error: editor exited with {status}")
                    });
                    app.committed_writes();
                    app.reload()?;
                }
                Action::None => {}
            }
        }
        app.tick()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use t9_core::decisions::DecisionInput;
    use t9_core::*;

    struct Fixture {
        _tmp: tempfile::TempDir,
        app: App,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let work = root.join("work/demo");
        std::fs::create_dir_all(&work).unwrap();
        let mut store = Store::open(Env::new(root.join("state"), root.join("vault"))).unwrap();
        store.init_project("Demo", Some("DM"), "A demo project", Some(&work)).unwrap();
        let c = Caller { cwd: work.clone(), agent: Some("claude-code".into()), ..Default::default() };
        let add = |store: &mut Store, kind, title: &str, parent: Option<&str>| {
            store
                .add_item(
                    &c,
                    NewItem {
                        kind: Some(kind),
                        title: title.into(),
                        parent: parent.map(String::from),
                        ..Default::default()
                    },
                )
                .unwrap()
        };
        add(&mut store, Kind::Plan, "Ship v2", None);
        add(&mut store, Kind::Feature, "Search", Some("DM-1"));
        add(&mut store, Kind::Bug, "Snippets break on CJK", Some("DM-2"));
        store
            .decide(
                &c,
                DecisionInput {
                    title: "Use FTS5 for search".into(),
                    because: Some("no extra process".into()),
                    areas: vec!["search".into()],
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .log(&c, LogInput { kind: EntryKind::Handoff, body: "Next: DM-3".into(), item: None, refs: vec![] })
            .unwrap();
        let initial = store.resolve(&c).unwrap().project.map(|p| p.meta.key);
        Fixture { app: App::new(store, initial).unwrap(), _tmp: tmp }
    }

    fn screen(app: &mut App) -> String {
        let mut t = Terminal::new(TestBackend::new(140, 40)).unwrap();
        t.draw(|f| ui::draw(f, app)).unwrap();
        let buf = t.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn key(app: &mut App, code: KeyCode) -> Action {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn typ(app: &mut App, s: &str) {
        for c in s.chars() {
            key(app, KeyCode::Char(c));
        }
    }

    fn screen_at(app: &mut App, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| ui::draw(f, app)).unwrap();
        let buf = t.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn opens_on_an_overview_for_humans() {
        let mut f = fixture();
        assert_eq!(f.app.screen, Screen::Project);
        let s = screen(&mut f.app);
        for want in [
            "Demo (DM)",
            "1 Overview",
            "LAST HANDOFF",
            "Next: DM-3",
            "PLAN",
            "0/2 done",
            "OPEN ISSUES",
            "1 bugs",
            "DECISIONS",
        ] {
            assert!(s.contains(want), "missing {want:?}:\n{s}");
        }
        // How current it is: the newest change, a live status line, and green dots on fresh items.
        for want in [
            "LATEST",
            "claude-code handoff: Next: DM-3",
            "live · refreshed",
            "last update just now by claude-code",
            "● DM-2",
            "15 min",
            " NEW ",
        ] {
            assert!(s.contains(want), "missing {want:?}:\n{s}");
        }
        // `b` shows the agent's brief verbatim, and back.
        key(&mut f.app, KeyCode::Char('b'));
        let raw = screen(&mut f.app);
        assert!(raw.contains("Agent brief") && raw.contains("Last handoff (claude-code"), "{raw}");
        key(&mut f.app, KeyCode::Char('b'));
        assert!(screen(&mut f.app).contains("LAST HANDOFF"));
    }

    #[test]
    fn overview_fits_narrow_and_wide_terminals() {
        let mut f = fixture();
        let caller = f.app.caller();
        let long = "A very long question title that would never fit on one line of a narrow terminal pane at all";
        f.app
            .store
            .add_item(&caller, NewItem { kind: Some(Kind::Question), title: long.into(), ..Default::default() })
            .unwrap();
        f.app.reload().unwrap();
        let narrow = screen_at(&mut f.app, 80, 40);
        assert!(narrow.contains("WAITING ON YOU") && narrow.contains("A very long question title"), "{narrow}");
        assert!(narrow.contains('…'), "long titles are cut with an ellipsis:\n{narrow}");
        for line in narrow.lines() {
            assert!(line.chars().count() <= 80);
        }
        let wide = screen_at(&mut f.app, 180, 40);
        let row = wide.lines().find(|l| l.contains("this week")).unwrap();
        assert!(row.contains("OPEN ISSUES"), "two columns side by side:\n{wide}");
    }

    #[test]
    fn items_tree_detail_and_status_change() {
        let mut f = fixture();
        key(&mut f.app, KeyCode::Char('2'));
        let s = screen(&mut f.app);
        assert!(s.contains("DM-1") && s.contains("Ship v2") && s.contains("DM-3"), "{s}");
        // Select the bug (third row).
        key(&mut f.app, KeyCode::Down);
        key(&mut f.app, KeyCode::Down);
        let s = screen(&mut f.app);
        assert!(s.contains("Snippets break on CJK") && s.contains("parent   DM-2"), "{s}");
        key(&mut f.app, KeyCode::Char('s'));
        assert!(matches!(f.app.mode, Mode::Pick(_)));
        // open → active → blocked → done
        for _ in 0..3 {
            key(&mut f.app, KeyCode::Down);
        }
        key(&mut f.app, KeyCode::Enter);
        assert_eq!(f.app.store.item("DM-3").unwrap().meta.status, Status::Done);
        assert!(f.app.message.as_deref().unwrap().contains("DM-3 is now done"));
        // Closed items are hidden until `c`.
        let listed = |app: &App| app.items.iter().any(|(_, i)| i.meta.id == "DM-3");
        assert!(!listed(&f.app));
        key(&mut f.app, KeyCode::Char('c'));
        assert!(listed(&f.app));
    }

    #[test]
    fn record_decisions_rejections_items_and_notes() {
        let mut f = fixture();
        key(&mut f.app, KeyCode::Char('d'));
        typ(&mut f.app, "Use real temp vaults in tests | no mocks");
        key(&mut f.app, KeyCode::Enter);
        key(&mut f.app, KeyCode::Char('x'));
        typ(&mut f.app, "Use a daemon for writes | CLI must work alone");
        key(&mut f.app, KeyCode::Enter);
        key(&mut f.app, KeyCode::Char('a'));
        typ(&mut f.app, "idea Trigram tokenizer");
        key(&mut f.app, KeyCode::Enter);
        let d = f.app.store.item("DM-5").unwrap();
        assert_eq!((d.meta.kind, d.meta.status), (Kind::Decision, Status::Accepted));
        assert!(d.body.contains("no mocks"));
        assert_eq!(f.app.store.item("DM-6").unwrap().meta.status, Status::Rejected);
        assert_eq!(f.app.store.item("DM-7").unwrap().meta.kind, Kind::Idea);
        assert_eq!(d.meta.created_by, "human");

        key(&mut f.app, KeyCode::Char('3'));
        let s = screen(&mut f.app);
        assert!(s.contains("Decision log (3)") && s.contains("rejected") && s.contains("Use real temp vaults"), "{s}");

        key(&mut f.app, KeyCode::Char('4'));
        key(&mut f.app, KeyCode::Char('n'));
        typ(&mut f.app, "looked at the plan");
        key(&mut f.app, KeyCode::Enter);
        let s = screen(&mut f.app);
        assert!(s.contains("looked at the plan"), "{s}");
    }

    #[test]
    fn search_opens_the_hit() {
        let mut f = fixture();
        key(&mut f.app, KeyCode::Char('/'));
        typ(&mut f.app, "cjk");
        let s = screen(&mut f.app);
        assert!(s.contains("1 results") && s.contains("DM-3"), "{s}");
        key(&mut f.app, KeyCode::Enter);
        assert_eq!(f.app.tab, Tab::Items);
        assert_eq!(f.app.selected_id().as_deref(), Some("DM-3"));
    }

    #[test]
    fn an_unlinked_folder_can_be_set_up_from_the_project_list() {
        let mut f = fixture();
        // An agent worked in a repository with no project and left a note in the inbox.
        let root = f.app.store.env.vault.parent().unwrap().to_path_buf();
        let repo = root.join("work/payments");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let c = Caller { cwd: repo.clone(), agent: Some("claude-code".into()), ..Default::default() };
        f.app
            .store
            .log(&c, LogInput { kind: EntryKind::Note, body: "webhook retries".into(), item: None, refs: vec![] })
            .unwrap();

        key(&mut f.app, KeyCode::Esc);
        let s = screen(&mut f.app);
        assert!(s.contains("1 folder(s) not set up") && s.contains("⚠ not set up"), "{s}");
        let folder = f.app.unlinked[0].cwd.clone();
        f.app.select_folder(&folder);
        let s = screen(&mut f.app);
        assert!(s.contains("Not linked to a t9 project") && s.contains("--key PAY"), "{s}");

        key(&mut f.app, KeyCode::Char('i'));
        match &f.app.mode {
            Mode::Input(i) => assert_eq!(i.buffer, "payments | PAY"),
            other => panic!("expected the create-project prompt, got {other:?}"),
        }
        key(&mut f.app, KeyCode::Enter);
        assert_eq!(f.app.screen, Screen::Project);
        assert_eq!(f.app.key.as_deref(), Some("PAY"));
        assert!(f.app.message.as_deref().unwrap().contains("1 inbox notes moved in"), "{:?}", f.app.message);
        assert!(f.app.unlinked.is_empty() || f.app.unlinked.iter().all(|u| u.cwd != folder));
    }

    #[test]
    fn overview_shows_about_experiments_resources_and_a_timeline_tab() {
        let mut f = fixture();
        let caller = f.app.caller();
        f.app.store.about_set("DM", "what", "A demo of billing.").unwrap();
        let new = |kind, title: &str| NewItem { kind: Some(kind), title: title.into(), ..Default::default() };
        f.app.store.add_item(&caller, new(Kind::Experiment, "Batch webhooks")).unwrap();
        f.app
            .store
            .add_item(
                &caller,
                NewItem {
                    resource_type: Some("dataset".into()),
                    location: Some("s3://demo/orders".into()),
                    ..new(Kind::Resource, "Orders dataset")
                },
            )
            .unwrap();
        f.app.reload().unwrap();
        let s = screen(&mut f.app);
        for want in
            ["ABOUT", "A demo of billing.", "EXPERIMENTS", "Batch webhooks", "RESOURCES", "dataset · s3://demo/orders"]
        {
            assert!(s.contains(want), "missing {want:?}:\n{s}");
        }
        key(&mut f.app, KeyCode::Char('6'));
        assert_eq!(f.app.tab, Tab::Timeline);
        let s = screen(&mut f.app);
        assert!(s.contains("6 Timeline") && s.contains("new experiment") && s.contains("new resource"), "{s}");
    }

    #[test]
    fn project_list_help_edit_and_quit() {
        let mut f = fixture();
        key(&mut f.app, KeyCode::Esc);
        assert_eq!(f.app.screen, Screen::Projects);
        let s = screen(&mut f.app);
        assert!(s.contains("DM") && s.contains("Demo") && s.contains("1 issues"), "{s}");
        key(&mut f.app, KeyCode::Enter);
        assert_eq!(f.app.screen, Screen::Project);
        key(&mut f.app, KeyCode::Char('?'));
        assert!(screen(&mut f.app).contains("record a decision"));
        key(&mut f.app, KeyCode::Char('z'));
        assert!(matches!(f.app.mode, Mode::Normal));
        key(&mut f.app, KeyCode::Char('2'));
        assert!(matches!(key(&mut f.app, KeyCode::Char('e')), Action::Edit(p) if p.ends_with("DM-1-ship-v2.md")));
        assert_eq!(key(&mut f.app, KeyCode::Char('q')), Action::Quit);
    }
}
