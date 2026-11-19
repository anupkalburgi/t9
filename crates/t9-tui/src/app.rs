//! TUI state and key handling. Rendering is in `ui.rs`; data access in `data.rs`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use t9_core::index::Hit;
use t9_core::*;

use crate::data::{self, ItemDetail, ProjectRow};

const REFRESH: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Projects,
    Project,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Brief,
    Items,
    Decisions,
    Journal,
    Triage,
}

impl Tab {
    pub const ALL: [Tab; 5] = [Tab::Brief, Tab::Items, Tab::Decisions, Tab::Journal, Tab::Triage];
    pub fn title(self) -> &'static str {
        match self {
            Tab::Brief => "1 Overview",
            Tab::Items => "2 Items",
            Tab::Decisions => "3 Decisions",
            Tab::Journal => "4 Journal",
            Tab::Triage => "5 Triage",
        }
    }
    fn index(self) -> usize {
        Tab::ALL.iter().position(|t| *t == self).unwrap()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum InputPurpose {
    Add { parent: Option<String> },
    Log { item: Option<String> },
    Decide { status: Status },
    Attach { entry: String },
    CreateProject { folder: String },
}

#[derive(Clone, Debug)]
pub struct Input {
    pub prompt: String,
    pub hint: String,
    pub buffer: String,
    pub purpose: InputPurpose,
}

#[derive(Clone, Debug)]
pub struct Pick {
    pub item: String,
    pub options: Vec<Status>,
    pub sel: usize,
}

#[derive(Clone, Debug, Default)]
pub struct SearchState {
    pub query: String,
    pub hits: Vec<Hit>,
    pub sel: usize,
    pub all_projects: bool,
}

#[derive(Clone, Debug)]
pub enum Mode {
    Normal,
    Help,
    Input(Input),
    Pick(Pick),
    Search(SearchState),
}

/// What the event loop must do after a key.
#[derive(Debug, PartialEq)]
pub enum Action {
    None,
    Quit,
    /// Suspend the TUI and open this file in `$EDITOR`.
    Edit(PathBuf),
}

#[derive(Clone, Debug)]
pub struct Latest {
    pub at: Timestamp,
    pub who: String,
    pub what: String,
}

pub enum Detail {
    Item(Box<ItemDetail>),
    Entry(Entry),
}

pub struct App {
    pub store: Store,
    pub screen: Screen,
    pub tab: Tab,
    pub mode: Mode,
    pub projects: Vec<ProjectRow>,
    /// Folders where agents worked without a project; listed after the projects.
    pub unlinked: Vec<t9_core::unlinked::UnlinkedFolder>,
    /// Selection over projects followed by unlinked folders.
    pub proj_sel: usize,
    pub key: Option<String>,
    /// The agent's brief as text (shown with `b`) and as data (the Overview).
    pub brief: String,
    pub overview: Option<t9_core::context::Brief>,
    pub raw_brief: bool,
    pub brief_scroll: u16,
    pub items: Vec<(usize, Item)>,
    pub item_sel: usize,
    pub show_closed: bool,
    pub decisions: Vec<Item>,
    pub dec_sel: usize,
    pub journal: Vec<Entry>,
    pub j_sel: usize,
    pub triage: Vec<Entry>,
    pub t_sel: usize,
    pub detail: Option<Detail>,
    detail_for: Option<String>,
    pub detail_scroll: u16,
    pub message: Option<String>,
    last_refresh: Instant,
    /// Wall-clock time of the last reload, shown in the status line.
    pub refreshed_at: chrono::DateTime<chrono::Local>,
    /// Newest change in the project: when, who, what.
    pub latest: Option<Latest>,
    /// Latest journal entry time per item id (from the loaded journal).
    pub touched: std::collections::HashMap<String, Timestamp>,
}

fn clamp(sel: usize, len: usize) -> usize {
    if len == 0 { 0 } else { sel.min(len - 1) }
}

fn step(sel: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (sel as isize + delta).clamp(0, len as isize - 1) as usize
}

impl App {
    /// Opens on `initial` (a project key) if given, else on the project list.
    pub fn new(store: Store, initial: Option<String>) -> Result<App> {
        let mut app = App {
            store,
            screen: Screen::Projects,
            tab: Tab::Brief,
            mode: Mode::Normal,
            projects: vec![],
            unlinked: vec![],
            proj_sel: 0,
            key: None,
            brief: String::new(),
            overview: None,
            raw_brief: false,
            brief_scroll: 0,
            items: vec![],
            item_sel: 0,
            show_closed: false,
            decisions: vec![],
            dec_sel: 0,
            journal: vec![],
            j_sel: 0,
            triage: vec![],
            t_sel: 0,
            detail: None,
            detail_for: None,
            detail_scroll: 0,
            message: None,
            last_refresh: Instant::now(),
            refreshed_at: chrono::Local::now(),
            latest: None,
            touched: Default::default(),
        };
        app.projects = data::projects(&mut app.store)?;
        app.unlinked = data::unlinked(&mut app.store)?;
        if let Some(k) = initial {
            app.open_project(&k)?;
        }
        Ok(app)
    }

    /// Writes from the TUI are made as `human`, in a session of their own: the cwd is the vault, so agents
    /// working in the project directory never pick this session up.
    pub fn caller(&self) -> Caller {
        Caller {
            cwd: self.store.env.vault.clone(),
            agent: Some("human".into()),
            session: None,
            project: self.key.clone(),
        }
    }

    /// Reads the brief as if from the project's first root, so it shows the focus of an agent working there.
    fn brief_caller(&self, key: &str) -> Caller {
        let root = self.store.project(key).ok().and_then(|p| p.meta.roots.first().cloned());
        Caller {
            cwd: root.map(PathBuf::from).unwrap_or_else(|| self.store.env.vault.clone()),
            agent: None,
            session: None,
            project: Some(key.into()),
        }
    }

    pub fn project_name(&self) -> String {
        match &self.key {
            Some(k) => self.store.project(k).map(|p| format!("{} ({k})", p.meta.name)).unwrap_or_else(|_| k.clone()),
            None => "projects".into(),
        }
    }

    pub fn open_project(&mut self, key: &str) -> Result<()> {
        let p = self.store.project(key)?;
        self.key = Some(p.meta.key.clone());
        self.screen = Screen::Project;
        self.item_sel = 0;
        self.dec_sel = 0;
        self.j_sel = 0;
        self.t_sel = 0;
        self.brief_scroll = 0;
        self.detail_for = None;
        self.reload()
    }

    /// Reloads the current view from the vault, keeping selections on the same ids.
    pub fn reload(&mut self) -> Result<()> {
        self.last_refresh = Instant::now();
        self.refreshed_at = chrono::Local::now();
        let Some(key) = self.key.clone() else {
            self.projects = data::projects(&mut self.store)?;
            self.unlinked = data::unlinked(&mut self.store)?;
            self.proj_sel = clamp(self.proj_sel, self.projects.len() + self.unlinked.len());
            return Ok(());
        };
        let keep_item = self.items.get(self.item_sel).map(|(_, i)| i.meta.id.clone());
        let keep_dec = self.decisions.get(self.dec_sel).map(|i| i.meta.id.clone());
        let keep_j = self.journal.get(self.j_sel).map(|e| e.meta.id.clone());
        let keep_t = self.triage.get(self.t_sel).map(|e| e.meta.id.clone());
        let brief_caller = self.brief_caller(&key);
        let (overview, text) = data::brief(&mut self.store, &brief_caller)?;
        self.overview = Some(overview);
        self.brief = text;
        self.items = data::tree(&mut self.store, &key, self.show_closed)?;
        self.decisions = data::decisions(&mut self.store, &key)?;
        self.journal = data::journal(&mut self.store, &key)?;
        self.touched.clear();
        for e in &self.journal {
            if let Some(id) = &e.meta.item {
                let t = self.touched.entry(id.clone()).or_insert(e.meta.at);
                if e.meta.at > *t {
                    *t = e.meta.at;
                }
            }
        }
        self.latest = self.compute_latest();
        self.triage = data::triage(&mut self.store, &key)?;
        let find = |id: &Option<String>, ids: Vec<&str>, old: usize| match id {
            Some(id) => ids.iter().position(|x| x == id).unwrap_or(old),
            None => old,
        };
        self.item_sel = clamp(
            find(&keep_item, self.items.iter().map(|(_, i)| i.meta.id.as_str()).collect(), self.item_sel),
            self.items.len(),
        );
        self.dec_sel = clamp(
            find(&keep_dec, self.decisions.iter().map(|i| i.meta.id.as_str()).collect(), self.dec_sel),
            self.decisions.len(),
        );
        self.j_sel = clamp(
            find(&keep_j, self.journal.iter().map(|e| e.meta.id.as_str()).collect(), self.j_sel),
            self.journal.len(),
        );
        self.t_sel = clamp(
            find(&keep_t, self.triage.iter().map(|e| e.meta.id.as_str()).collect(), self.t_sel),
            self.triage.len(),
        );
        self.detail_for = None;
        self.load_detail()
    }

    /// When an item was last touched: its own update or its newest journal entry.
    pub fn touched_at(&self, i: &Item) -> Timestamp {
        self.touched.get(&i.meta.id).copied().map_or(i.meta.updated, |t| t.max(i.meta.updated))
    }

    /// The newest change: the latest journal entry, or a later item edit (e.g. made in an editor).
    fn compute_latest(&self) -> Option<Latest> {
        let entry = self.journal.first().map(|e| Latest {
            at: e.meta.at,
            who: if e.meta.agent.is_empty() { "unknown".into() } else { e.meta.agent.clone() },
            what: match &e.meta.item {
                Some(i) => format!("{} {i}: {}", e.meta.kind, t9_core::context::first_line(&e.body, 200)),
                None => format!("{}: {}", e.meta.kind, t9_core::context::first_line(&e.body, 200)),
            },
        });
        let edited =
            self.items.iter().map(|(_, i)| i).chain(self.decisions.iter()).max_by_key(|i| i.meta.updated).map(|i| {
                Latest {
                    at: i.meta.updated,
                    who: if i.meta.created_by.is_empty() { "unknown".into() } else { i.meta.created_by.clone() },
                    what: format!("updated {}: {}", i.meta.id, i.meta.title),
                }
            });
        match (entry, edited) {
            (Some(e), Some(i)) => Some(if i.at > e.at { i } else { e }),
            (e, i) => e.or(i),
        }
    }

    /// Id of the thing selected in the current tab.
    pub fn selected_id(&self) -> Option<String> {
        match self.tab {
            Tab::Brief => None,
            Tab::Items => self.items.get(self.item_sel).map(|(_, i)| i.meta.id.clone()),
            Tab::Decisions => self.decisions.get(self.dec_sel).map(|i| i.meta.id.clone()),
            Tab::Journal => self.journal.get(self.j_sel).map(|e| e.meta.id.clone()),
            Tab::Triage => self.triage.get(self.t_sel).map(|e| e.meta.id.clone()),
        }
    }

    fn selected_item_id(&self) -> Option<String> {
        match self.tab {
            Tab::Items | Tab::Decisions => self.selected_id(),
            Tab::Journal => self.journal.get(self.j_sel).and_then(|e| e.meta.item.clone()),
            _ => None,
        }
    }

    fn load_detail(&mut self) -> Result<()> {
        let id = self.selected_id();
        if id == self.detail_for {
            return Ok(());
        }
        self.detail_scroll = 0;
        self.detail_for = id.clone();
        self.detail = match (self.tab, id) {
            (Tab::Items | Tab::Decisions, Some(id)) => {
                Some(Detail::Item(Box::new(data::item_detail(&mut self.store, &id)?)))
            }
            (Tab::Journal | Tab::Triage, Some(id)) => Some(Detail::Entry(data::entry(&self.store, &id)?)),
            _ => None,
        };
        Ok(())
    }

    /// Periodic refresh so agents' writes show up while the TUI is open.
    pub fn tick(&mut self) -> Result<()> {
        if matches!(self.mode, Mode::Normal) && self.last_refresh.elapsed() >= REFRESH {
            self.reload()?;
        }
        Ok(())
    }

    /// After t9 itself or an editor changed vault files: commit them in the background.
    pub fn committed_writes(&mut self) {
        if self.store.env.git {
            t9_core::vcs::spawn_background(&self.store.env);
        }
        self.store.wrote.set(false);
    }

    fn after_write(&mut self, msg: String) -> Result<()> {
        self.message = Some(msg);
        if self.store.wrote.get() {
            self.committed_writes();
        }
        self.reload()
    }

    pub fn on_key(&mut self, k: KeyEvent) -> Action {
        match self.handle(k) {
            Ok(a) => a,
            Err(e) => {
                self.message = Some(format!("error: {e:#}"));
                Action::None
            }
        }
    }

    fn handle(&mut self, k: KeyEvent) -> Result<Action> {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            return Ok(Action::Quit);
        }
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Normal => self.normal(k),
            Mode::Help => Ok(Action::None),
            Mode::Input(input) => self.input(k, input),
            Mode::Pick(pick) => self.pick(k, pick),
            Mode::Search(s) => self.search(k, s),
        }
    }

    fn move_sel(&mut self, delta: isize) -> Result<()> {
        match (self.screen, self.tab) {
            (Screen::Projects, _) => {
                self.proj_sel = step(self.proj_sel, self.projects.len() + self.unlinked.len(), delta)
            }
            (_, Tab::Brief) => self.brief_scroll = (self.brief_scroll as isize + delta).max(0) as u16,
            (_, Tab::Items) => self.item_sel = step(self.item_sel, self.items.len(), delta),
            (_, Tab::Decisions) => self.dec_sel = step(self.dec_sel, self.decisions.len(), delta),
            (_, Tab::Journal) => self.j_sel = step(self.j_sel, self.journal.len(), delta),
            (_, Tab::Triage) => self.t_sel = step(self.t_sel, self.triage.len(), delta),
        }
        self.load_detail()
    }

    fn set_tab(&mut self, t: Tab) -> Result<()> {
        self.tab = t;
        self.detail_for = None;
        self.load_detail()
    }

    fn input_mode(&mut self, prompt: &str, hint: &str, purpose: InputPurpose) {
        self.mode = Mode::Input(Input { prompt: prompt.into(), hint: hint.into(), buffer: String::new(), purpose });
    }

    fn normal(&mut self, k: KeyEvent) -> Result<Action> {
        self.message = None;
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        match k.code {
            KeyCode::Char('q') => return Ok(Action::Quit),
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('r') => {
                self.reload()?;
                self.message = Some("refreshed".into());
            }
            KeyCode::Char('/') => {
                self.mode = Mode::Search(SearchState { all_projects: self.key.is_none(), ..Default::default() })
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_sel(1)?,
            KeyCode::Up | KeyCode::Char('k') => self.move_sel(-1)?,
            KeyCode::PageDown => self.move_sel(10)?,
            KeyCode::PageUp => self.move_sel(-10)?,
            KeyCode::Home | KeyCode::Char('g') => self.move_sel(-100_000)?,
            KeyCode::End | KeyCode::Char('G') => self.move_sel(100_000)?,
            KeyCode::Char('J') => self.detail_scroll = self.detail_scroll.saturating_add(3),
            KeyCode::Char('K') => self.detail_scroll = self.detail_scroll.saturating_sub(3),
            _ if self.screen == Screen::Projects => return self.projects_key(k),
            _ => return self.project_key(k, shift),
        }
        Ok(Action::None)
    }

    /// The unlinked folder selected on the Projects screen, if the selection is past the projects.
    pub fn selected_folder(&self) -> Option<&t9_core::unlinked::UnlinkedFolder> {
        self.proj_sel.checked_sub(self.projects.len()).and_then(|i| self.unlinked.get(i))
    }

    /// Selects an unlinked folder on the Projects screen (used when the TUI is opened inside one).
    pub fn select_folder(&mut self, folder: &str) {
        if let Some(i) = self.unlinked.iter().position(|u| u.cwd == folder) {
            self.proj_sel = self.projects.len() + i;
        }
    }

    fn projects_key(&mut self, k: KeyEvent) -> Result<Action> {
        match k.code {
            KeyCode::Esc => return Ok(Action::Quit),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('i') => {
                if let Some(u) = self.selected_folder().cloned() {
                    self.mode = Mode::Input(Input {
                        prompt: format!(
                            "Create a project for {}",
                            t9_core::env::display_path(std::path::Path::new(&u.cwd))
                        ),
                        hint: "NAME | KEY (edit if you like). Notes agents left in this folder move into it.".into(),
                        buffer: format!("{} | {}", u.name, u.key),
                        purpose: InputPurpose::CreateProject { folder: u.cwd },
                    });
                } else if k.code != KeyCode::Char('i')
                    && let Some(row) = self.projects.get(self.proj_sel)
                {
                    let key = row.project.meta.key.clone();
                    self.open_project(&key)?;
                }
            }
            _ => {}
        }
        Ok(Action::None)
    }

    fn project_key(&mut self, k: KeyEvent, _shift: bool) -> Result<Action> {
        match k.code {
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Left => {
                self.screen = Screen::Projects;
                self.key = None;
                self.reload()?;
            }
            KeyCode::Tab => self.set_tab(Tab::ALL[(self.tab.index() + 1) % 5])?,
            KeyCode::BackTab => self.set_tab(Tab::ALL[(self.tab.index() + 4) % 5])?,
            KeyCode::Char(c @ '1'..='5') => self.set_tab(Tab::ALL[c as usize - '1' as usize])?,
            KeyCode::Char('b') if self.tab == Tab::Brief => {
                self.raw_brief = !self.raw_brief;
                self.brief_scroll = 0;
            }
            KeyCode::Char('c') if self.tab == Tab::Items => {
                self.show_closed = !self.show_closed;
                self.reload()?;
                self.message = Some(if self.show_closed { "showing closed items" } else { "hiding closed items" }.into());
            }
            KeyCode::Char('a') => self.input_mode(
                "Add item",
                "KIND TITLE, e.g. `bug Crash on save` (kinds: task bug regression revisit idea question feature phase plan note)",
                InputPurpose::Add { parent: None },
            ),
            KeyCode::Char('A') if self.tab == Tab::Items => {
                let parent = self.selected_id();
                self.input_mode(
                    &format!("Add child of {}", parent.clone().unwrap_or_default()),
                    "KIND TITLE, e.g. `task Write the parser`",
                    InputPurpose::Add { parent },
                );
            }
            KeyCode::Char('d') => self.input_mode(
                "Record a decision",
                "TITLE | WHY, e.g. `Use FTS5 for search | no extra process`",
                InputPurpose::Decide { status: Status::Accepted },
            ),
            KeyCode::Char('x') => self.input_mode(
                "Record a rejected option",
                "TITLE | WHY, e.g. `Use a daemon for writes | CLI must work alone`",
                InputPurpose::Decide { status: Status::Rejected },
            ),
            KeyCode::Char('n') => {
                let item = self.selected_item_id();
                let target = item.clone().unwrap_or_else(|| "the project (unfiled)".into());
                self.input_mode(&format!("Note on {target}"), "text of the journal entry", InputPurpose::Log { item });
            }
            KeyCode::Char('s') => {
                if let Some(id) = self.selected_item_id() {
                    let item = self.store.item(&id)?;
                    let options = if item.meta.kind == Kind::Decision {
                        vec![Status::Accepted, Status::Proposed, Status::Rejected, Status::Superseded]
                    } else if item.meta.kind == Kind::Idea {
                        vec![Status::Open, Status::Accepted, Status::Rejected, Status::Done]
                    } else {
                        vec![Status::Open, Status::Active, Status::Blocked, Status::Done, Status::Dropped]
                    };
                    let sel = options.iter().position(|s| *s == item.meta.status).unwrap_or(0);
                    self.mode = Mode::Pick(Pick { item: id, options, sel });
                }
            }
            KeyCode::Char('e') if matches!(self.tab, Tab::Items | Tab::Decisions) => {
                if let Some(id) = self.selected_id() {
                    return Ok(Action::Edit(PathBuf::from(self.store.item(&id)?.path)));
                }
            }
            KeyCode::Char('f') if self.tab == Tab::Triage => {
                if let Some(id) = self.selected_id() {
                    self.input_mode(
                        &format!("File entry {} under", short_id(&id)),
                        "item id, e.g. BIL-12",
                        InputPurpose::Attach { entry: id },
                    );
                }
            }
            _ => {}
        }
        Ok(Action::None)
    }

    fn input(&mut self, k: KeyEvent, mut input: Input) -> Result<Action> {
        match k.code {
            KeyCode::Esc => return Ok(Action::None),
            KeyCode::Enter => {
                let text = input.buffer.trim().to_string();
                if text.is_empty() {
                    return Ok(Action::None);
                }
                let caller = self.caller();
                let msg = match input.purpose {
                    InputPurpose::Add { parent } => {
                        let i = data::add(&mut self.store, &caller, &text, parent)?;
                        format!("added {} {} {}", i.meta.id, i.meta.kind, i.meta.title)
                    }
                    InputPurpose::Log { item } => {
                        let e = data::log(&mut self.store, &caller, &text, item)?;
                        format!("noted {} on {}", short_id(&e.meta.id), e.meta.item.as_deref().unwrap_or("project"))
                    }
                    InputPurpose::Decide { status } => {
                        let d = data::decide(&mut self.store, &caller, &text, status)?;
                        format!("recorded {} [{}] {}", d.meta.id, d.meta.status, d.meta.title)
                    }
                    InputPurpose::Attach { entry } => {
                        let e = data::attach(&mut self.store, &entry, &text)?;
                        format!("filed {} under {}", short_id(&e.meta.id), e.meta.item.unwrap_or_default())
                    }
                    InputPurpose::CreateProject { folder } => {
                        let (p, moved) = data::create_project(&mut self.store, &folder, &text)?;
                        let key = p.meta.key.clone();
                        self.after_write(format!(
                            "created {} ({key}) for {}; {moved} inbox notes moved in",
                            p.meta.name,
                            t9_core::env::display_path(std::path::Path::new(&folder))
                        ))?;
                        self.open_project(&key)?;
                        return Ok(Action::None);
                    }
                };
                self.after_write(msg)?;
            }
            KeyCode::Backspace => {
                input.buffer.pop();
                self.mode = Mode::Input(input);
            }
            KeyCode::Char(c) => {
                input.buffer.push(c);
                self.mode = Mode::Input(input);
            }
            _ => self.mode = Mode::Input(input),
        }
        Ok(Action::None)
    }

    fn pick(&mut self, k: KeyEvent, mut pick: Pick) -> Result<Action> {
        match k.code {
            KeyCode::Esc => {}
            KeyCode::Down | KeyCode::Char('j') => {
                pick.sel = step(pick.sel, pick.options.len(), 1);
                self.mode = Mode::Pick(pick);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                pick.sel = step(pick.sel, pick.options.len(), -1);
                self.mode = Mode::Pick(pick);
            }
            KeyCode::Enter => {
                let status = pick.options[pick.sel];
                let caller = self.caller();
                let i = data::set_status(&mut self.store, &caller, &pick.item, status)?;
                self.after_write(format!("{} is now {}", i.meta.id, i.meta.status))?;
            }
            _ => self.mode = Mode::Pick(pick),
        }
        Ok(Action::None)
    }

    fn run_search(&mut self, s: &mut SearchState) -> Result<()> {
        let key = if s.all_projects { None } else { self.key.clone() };
        s.hits =
            if s.query.trim().is_empty() { vec![] } else { data::search(&mut self.store, &s.query, key.as_deref())? };
        s.sel = clamp(s.sel, s.hits.len());
        Ok(())
    }

    fn search(&mut self, k: KeyEvent, mut s: SearchState) -> Result<Action> {
        match k.code {
            KeyCode::Esc => return Ok(Action::None),
            KeyCode::Down => s.sel = step(s.sel, s.hits.len(), 1),
            KeyCode::Up => s.sel = step(s.sel, s.hits.len(), -1),
            KeyCode::Tab => {
                s.all_projects = !s.all_projects || self.key.is_none();
                self.run_search(&mut s)?;
            }
            KeyCode::Backspace => {
                s.query.pop();
                self.run_search(&mut s)?;
            }
            KeyCode::Char(c) => {
                s.query.push(c);
                self.run_search(&mut s)?;
            }
            KeyCode::Enter => {
                if let Some(hit) = s.hits.get(s.sel).cloned() {
                    self.jump_to(&hit)?;
                    return Ok(Action::None);
                }
            }
            _ => {}
        }
        self.mode = Mode::Search(s);
        Ok(Action::None)
    }

    /// Opens a search hit: an item in the Items or Decisions tab, an entry in the Journal tab.
    fn jump_to(&mut self, hit: &Hit) -> Result<()> {
        if self.key.as_deref() != Some(hit.project.as_str()) && hit.project != INBOX {
            self.open_project(&hit.project)?;
        }
        if hit.kind == "item" {
            let item = self.store.item(&hit.id)?;
            if item.meta.kind == Kind::Decision {
                self.set_tab(Tab::Decisions)?;
                self.dec_sel = self.decisions.iter().position(|d| d.meta.id == hit.id).unwrap_or(0);
            } else {
                if item.meta.status.is_closed() && !self.show_closed {
                    self.show_closed = true;
                    self.reload()?;
                }
                self.set_tab(Tab::Items)?;
                self.item_sel = self.items.iter().position(|(_, i)| i.meta.id == hit.id).unwrap_or(0);
            }
        } else {
            self.set_tab(Tab::Journal)?;
            self.j_sel = self.journal.iter().position(|e| e.meta.id == hit.id).unwrap_or(0);
        }
        self.detail_for = None;
        self.load_detail()
    }
}
