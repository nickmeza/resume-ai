//! State and actions of the interactive mode.

use super::keys::{Action, KeyMap};
use super::term::{Launch, PendingInput, TermTab};
use crate::config::Config;
use core::organization::{self, Change, Organization, RECENT_LIMIT, View};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::widgets::ListState;
use resume_ai_core::{self as core, AgentConfig, ImageMessage, LaunchMode, Session, aliases, i18n::I18n};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

pub enum Row {
    Group { name: String, cwd: Option<String>, count: usize },
    Session(usize),
}

pub enum Mode {
    Normal,
    Search,
    Filter { cursor: usize },
    Rename { input: String },
    ConfirmDelete,
    ConfirmQuit,
    ConfirmClose(usize),
    Help,
    PickAgent { cwd: String, project: String, cursor: usize },
    Images { title: String, session_id: String, messages: Vec<ImageMessage>, cursor: usize },
}

/// One entry of the filter popup.
pub enum FilterRow {
    Header(String),
    Option { group: &'static str, value: String, label: String, color: Option<String>, count: usize },
}

pub struct Tab {
    pub title: String,
    pub agent: Option<String>,
    /// Session key (`claude:<id>`), or `new:…` / `compact:…` / `shell:…`.
    pub key: String,
    pub term: Option<TermTab>,
    pub error: Option<anyhow::Error>,
    pub launch: Launch,
    /// Project folder and start time: to find the Claude session of a new tab (its id is not known yet)
    pub cwd: Option<String>,
    pub started_ms: f64,
}

/// Where each pane was drawn (for mouse clicks and the wheel).
#[derive(Default)]
pub struct LayoutAreas {
    pub list: Rect,
    pub views: [Rect; 4],
    pub tabs: Rect,
    pub terminal: Rect,
    /// (first column, last column + 1, tab number) of each tab in the tab bar
    pub tab_spans: Vec<(u16, u16, usize)>,
}

pub struct App {
    pub t: I18n,
    pub keys: KeyMap,
    pub cfg: Config,
    agent_cfg: BTreeMap<String, AgentConfig>,
    pub sessions: Vec<Session>,
    pub organization: Organization,
    pub view: View,
    pub aliases: BTreeMap<String, String>,
    pub load_errors: Vec<String>,
    pub rows: Vec<Row>,
    pub list: ListState,
    pub search: String,
    pub filters: BTreeMap<&'static str, BTreeSet<String>>,
    pub mode: Mode,
    pub tabs: Vec<Tab>,
    /// Tab shown on the right: n = tabs[n - 1], 0 = none open
    pub active: usize,
    /// Where the keys go: the sessions list (left) or the active tab's terminal (right)
    pub focus_list: bool,
    pub status: Option<(String, Instant)>,
    pub term_size: (u16, u16),
    pub layout: LayoutAreas,
    last_click: Option<(Instant, usize)>,
    notify: Sender<()>,
    pub quit: bool,
    counter: usize,
}

impl App {
    pub fn new(cfg: Config, t: I18n, notify: Sender<()>) -> Self {
        let mut app = App {
            keys: KeyMap::new(&cfg.keys),
            agent_cfg: cfg.agent_config(),
            cfg,
            t,
            sessions: Vec::new(),
            organization: Organization::default(),
            view: View::default(),
            aliases: BTreeMap::new(),
            load_errors: Vec::new(),
            rows: Vec::new(),
            list: ListState::default(),
            search: String::new(),
            filters: BTreeMap::new(),
            mode: Mode::Normal,
            tabs: Vec::new(),
            active: 0,
            status: None,
            focus_list: true,
            term_size: (24, 80),
            layout: LayoutAreas::default(),
            last_click: None,
            notify,
            quit: false,
            counter: 0,
        };
        app.reload();
        app
    }

    // ---------- Data ----------

    pub fn reload(&mut self) {
        let res = core::list_all(&self.cfg.enabled(), &self.agent_cfg);
        self.load_errors =
            res.errors.iter().map(|e| self.t.tf("list.agentError", &[("agent", &e.agent), ("error", &e.error)])).collect();
        self.sessions = res.sessions;
        self.aliases = aliases::load();
        match organization::load() {
            Ok(organization) => self.organization = organization,
            Err(e) => self.load_errors.push(format!("{e:#}")),
        }
        self.rebuild();
    }

    pub fn title_of(&self, s: &Session) -> String {
        let t = aliases::display_title(s, &self.aliases);
        if t.is_empty() { self.t.t("session.untitled") } else { t.to_string() }
    }

    fn matches_filters(&self, s: &Session, skip: Option<&str>) -> bool {
        self.filters
            .iter()
            .all(|(group, values)| Some(*group) == skip || values.is_empty() || values.contains(&filter_value(group, s)))
    }

    fn matches_search(&self, s: &Session) -> bool {
        let q = self.search.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        let agent = core::by_id(&s.agent).map(|a| a.name()).unwrap_or("");
        let origin = s.origin.as_deref().map(|o| self.t.origin(o)).unwrap_or_default();
        let hay = format!(
            "{} {} {} {} {} {agent} {} {origin}",
            self.title_of(s),
            s.title,
            s.cwd.as_deref().unwrap_or(""),
            s.git_branch.as_deref().unwrap_or(""),
            s.id,
            s.origin.as_deref().unwrap_or("")
        );
        hay.to_lowercase().contains(&q)
    }

    /// Recomputes the visible rows (grouped by project) keeping the selected session.
    pub fn rebuild(&mut self) {
        let keep = self.selected_session().map(|s| s.key.clone());
        let searching = !self.search.trim().is_empty();
        let visible = self.sessions.iter().enumerate().filter(|(_, s)| {
            self.organization.includes(&s.key, self.view, searching) && self.matches_filters(s, None) && self.matches_search(s)
        });
        let mut rows = Vec::new();
        if self.view == View::Recent && !searching {
            rows.extend(visible.take(RECENT_LIMIT).map(|(i, _)| Row::Session(i)));
        } else {
            let mut groups: Vec<(Option<String>, Option<String>, Vec<usize>)> = Vec::new();
            for (i, s) in visible {
                let key = s.cwd.as_deref().map(organization::project_key);
                match groups.iter_mut().find(|(k, ..)| *k == key) {
                    Some((.., items)) => items.push(i),
                    None => groups.push((key, s.cwd.clone(), vec![i])),
                }
            }
            for (_, cwd, items) in groups {
                let name = cwd.as_deref().map(basename).unwrap_or_else(|| self.t.t("session.unknownFolder"));
                rows.push(Row::Group { name, cwd, count: items.len() });
                rows.extend(items.into_iter().map(Row::Session));
            }
        }
        self.rows = rows;
        let idx = keep
            .and_then(|k| self.rows.iter().position(|r| matches!(r, Row::Session(i) if self.sessions[*i].key == k)))
            .or_else(|| self.rows.iter().position(|r| matches!(r, Row::Session(_))));
        self.list.select(idx);
    }

    pub fn selected_index(&self) -> Option<usize> {
        match self.rows.get(self.list.selected()?)? {
            Row::Session(i) => Some(*i),
            Row::Group { .. } => None,
        }
    }

    pub fn selected_session(&self) -> Option<&Session> {
        self.selected_index().map(|i| &self.sessions[i])
    }

    fn move_cursor(&mut self, delta: isize) {
        let sessions: Vec<usize> =
            self.rows.iter().enumerate().filter(|(_, r)| matches!(r, Row::Session(_))).map(|(i, _)| i).collect();
        if sessions.is_empty() {
            return;
        }
        let cur = self.list.selected().and_then(|s| sessions.iter().position(|&r| r == s)).unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, sessions.len() as isize - 1) as usize;
        self.list.select(Some(sessions[next]));
    }

    pub fn filter_rows(&self) -> Vec<FilterRow> {
        let mut out = vec![FilterRow::Header(self.t.t("filter.agents"))];
        let base: Vec<&Session> = self.sessions.iter().filter(|s| self.matches_filters(s, Some("agent"))).collect();
        for id in self.cfg.enabled() {
            let Some(a) = core::by_id(&id) else { continue };
            let count = base.iter().filter(|s| s.agent == id).count();
            out.push(FilterRow::Option {
                group: "agent",
                value: id.clone(),
                label: a.name().into(),
                color: Some(a.color().into()),
                count,
            });
        }
        let base: Vec<&Session> = self.sessions.iter().filter(|s| self.matches_filters(s, Some("origin"))).collect();
        let origins: BTreeSet<String> = self.sessions.iter().filter_map(|s| s.origin.clone()).collect();
        if !origins.is_empty() {
            out.push(FilterRow::Header(self.t.t("filter.origin")));
            for o in origins {
                let count = base.iter().filter(|s| s.origin.as_deref() == Some(o.as_str())).count();
                out.push(FilterRow::Option { group: "origin", label: self.t.origin(&o), value: o, color: None, count });
            }
            if self.sessions.iter().any(|s| s.origin.is_none()) {
                let count = base.iter().filter(|s| s.origin.is_none()).count();
                out.push(FilterRow::Option {
                    group: "origin",
                    value: String::new(),
                    label: self.t.t("filter.noOrigin"),
                    color: None,
                    count,
                });
            }
        }
        out
    }

    pub fn active_filter_labels(&self) -> Vec<String> {
        self.filter_rows()
            .into_iter()
            .filter_map(|r| match r {
                FilterRow::Option { group, value, label, .. } if self.filters.get(group).is_some_and(|v| v.contains(&value)) => {
                    Some(label)
                }
                _ => None,
            })
            .collect()
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    pub fn tab_open_for(&self, key: &str) -> Option<usize> {
        self.tabs.iter().position(|t| t.key == key)
    }

    pub fn running_tabs(&self) -> usize {
        self.tabs.iter().filter(|t| t.term.as_ref().is_some_and(|x| x.alive())).count()
    }

    // ---------- Tabs ----------

    pub(super) fn spawn_tab(&mut self, title: String, agent: Option<String>, key: String, launch: Launch) -> usize {
        let (rows, cols) = self.term_size;
        let cwd = launch.cwd.as_ref().map(|c| c.display().to_string());
        let started_ms = chrono::Utc::now().timestamp_millis() as f64;
        let (term, error) = match TermTab::spawn(&launch, rows, cols, self.notify.clone()) {
            Ok(t) => (Some(t), None),
            Err(e) => (None, Some(e)),
        };
        self.tabs.push(Tab { title, agent, key, term, error, launch, cwd, started_ms });
        self.active = self.tabs.len();
        self.focus_list = false; // a new tab is ready to type in
        self.active
    }

    fn launch_for(&self, agent: &str, mode: LaunchMode, id: Option<&str>, cwd: Option<&str>) -> anyhow::Result<Launch> {
        let cmd = core::build_command(agent, self.agent_cfg.get(agent), mode, id)?;
        Ok(Launch { program: cmd.program, args: cmd.args, cwd: cwd.map(PathBuf::from) })
    }

    pub fn open_selected(&mut self) {
        let Some(i) = self.selected_index() else { return };
        self.open_session(i);
    }

    fn open_session(&mut self, i: usize) {
        let s = self.sessions[i].clone();
        if let Some(t) = self.tab_open_for(&s.key) {
            self.active = t + 1;
            self.focus_list = false;
            return;
        }
        match self.launch_for(&s.agent, LaunchMode::Resume, Some(&s.id), s.cwd.as_deref()) {
            Ok(launch) => {
                self.spawn_tab(self.title_of(&s), Some(s.agent.clone()), s.key.clone(), launch);
            }
            Err(e) => self.set_status(format!("{e:#}")),
        }
    }

    /// "Continue compacted": resumes into a copy when the agent can fork, else the same session,
    /// then types the agent's compact command once it finished loading.
    pub fn continue_compacted(&mut self) {
        let Some(i) = self.selected_index() else { return };
        let s = self.sessions[i].clone();
        let Some(a) = core::by_id(&s.agent) else { return };
        let Some(command) = core::compact_command_of(a, self.agent_cfg.get(&s.agent)) else {
            self.set_status(self.t.tf("compact.notSupported", &[("agent", &a.name())]));
            return;
        };
        let tab = if a.fork_args("x").is_some() {
            self.counter += 1;
            match self.launch_for(&s.agent, LaunchMode::Compact, Some(&s.id), s.cwd.as_deref()) {
                Ok(l) => self.spawn_tab(
                    format!("↳ {}", self.title_of(&s)),
                    Some(s.agent.clone()),
                    format!("compact:{}:{}", s.key, self.counter),
                    l,
                ),
                Err(e) => return self.set_status(format!("{e:#}")),
            }
        } else {
            self.open_session(i);
            self.active
        };
        if let Some(term) = self.tabs.get_mut(tab - 1).and_then(|t| t.term.as_mut()) {
            term.pending = Some(PendingInput { text: command.clone(), started: Instant::now(), sent_text_at: None });
            self.set_status(self.t.tf("compact.waiting", &[("cmd", &command)]));
        }
    }

    pub fn new_session(&mut self, agent: &str, cwd: &str, project: &str) {
        self.counter += 1;
        match self.launch_for(agent, LaunchMode::New, None, Some(cwd)) {
            Ok(l) => {
                let title = self.t.tf("new.tabTitle", &[("project", &project)]);
                self.spawn_tab(title, Some(agent.to_string()), format!("new:{agent}:{}", self.counter), l);
            }
            Err(e) => self.set_status(format!("{e:#}")),
        }
    }

    pub fn open_shell(&mut self) {
        let cwd = self.selected_session().and_then(|s| s.cwd.clone());
        let program = if cfg!(windows) {
            std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
        } else {
            std::env::var("SHELL")
                .ok()
                .filter(|shell| !shell.trim().is_empty())
                .unwrap_or_else(|| if cfg!(target_os = "macos") { "/bin/zsh".into() } else { "/bin/sh".into() })
        };
        self.counter += 1;
        let title = format!("{} · {}", self.t.t("shell.title"), cwd.as_deref().map(basename).unwrap_or_else(|| "~".into()));
        self.spawn_tab(
            title,
            None,
            format!("shell:{}", self.counter),
            Launch { program, args: Vec::new(), cwd: cwd.map(PathBuf::from) },
        );
    }

    pub fn restart_tab(&mut self, n: usize) {
        let notify = self.notify.clone();
        let (rows, cols) = self.term_size;
        let Some(tab) = n.checked_sub(1).and_then(|index| self.tabs.get_mut(index)) else { return };
        tab.term = None; // drops (kills) the old process first
        match TermTab::spawn(&tab.launch, rows, cols, notify) {
            Ok(t) => {
                tab.term = Some(t);
                tab.error = None;
            }
            Err(e) => tab.error = Some(e),
        }
    }

    pub fn close_tab(&mut self, n: usize) {
        if n == 0 || n > self.tabs.len() {
            return;
        }
        self.tabs.remove(n - 1); // Drop kills the process
        if self.active > n {
            self.active -= 1;
        } else if self.active == n {
            self.active = n.min(self.tabs.len()); // the tab that took its place, or the previous one
        }
        if self.active == 0 {
            self.focus_list = true; // nothing left on the right
        }
    }

    /// Whether the keys go to a terminal (focus on the right and a tab open).
    pub fn in_terminal(&self) -> bool {
        !self.focus_list && self.active > 0
    }

    /// Called on every loop turn: exit detection and the pending compact command.
    pub fn tick(&mut self) {
        let mut messages = Vec::new();
        for tab in &mut self.tabs {
            let Some(term) = tab.term.as_mut() else { continue };
            term.poll_exit();
            let Some(mut p) = term.pending.take() else { continue };
            if !term.alive() {
                continue;
            }
            match p.sent_text_at {
                None if term.output_bytes() > 200 && term.quiet_for_ms() > 1500 => {
                    term.write(p.text.as_bytes());
                    p.sent_text_at = Some(Instant::now());
                }
                // Enter separately: sent together, some agents treat it as a paste and don't run it
                Some(at) if at.elapsed() >= Duration::from_millis(400) => {
                    term.write(b"\r");
                    messages.push(self.t.tf("compact.sent", &[("cmd", &p.text)]));
                    continue; // done
                }
                None if p.started.elapsed() > Duration::from_secs(45) => {
                    messages.push(self.t.tf("compact.timeout", &[("cmd", &p.text)]));
                    continue; // give up
                }
                _ => {}
            }
            term.pending = Some(p);
        }
        for m in messages {
            self.set_status(m);
        }
        if self.status.as_ref().is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(8)) {
            self.status = None;
        }
    }

    // ---------- Session actions ----------

    fn change_organization(&mut self, action: Change) {
        let Some(s) = self.selected_session() else { return };
        if matches!(action, Change::HideProject) && s.cwd.is_none() {
            self.set_status(self.t.t("open.folderMissing"));
            return;
        }
        match organization::change(s, &self.sessions, action) {
            Ok(organization) => {
                self.organization = organization;
                self.rebuild();
            }
            Err(e) => self.set_status(format!("{e:#}")),
        }
    }

    pub fn start_rename(&mut self) {
        if let Some(s) = self.selected_session() {
            self.mode = Mode::Rename { input: self.title_of(s) };
        }
    }

    fn finish_rename(&mut self, input: String) {
        let Some(s) = self.selected_session().cloned() else { return };
        match aliases::rename(&s, &input) {
            Ok(_) => {
                self.reload();
                let title = self.sessions.iter().find(|x| x.key == s.key).map(|x| self.title_of(x));
                if let (Some(title), Some(t)) = (title, self.tab_open_for(&s.key)) {
                    self.tabs[t].title = title;
                }
                self.set_status(self.t.t("rename.done"));
            }
            Err(e) => self.set_status(format!("{e:#}")),
        }
    }

    pub fn start_delete(&mut self) {
        let Some(s) = self.selected_session() else { return };
        let agent = core::by_id(&s.agent);
        if agent.is_some_and(|a| a.trash_paths(s).is_none()) {
            let name = agent.map(|a| a.name()).unwrap_or("");
            self.set_status(self.t.tf("delete.notAllowed", &[("agent", &name)]));
            return;
        }
        self.mode = Mode::ConfirmDelete;
    }

    fn finish_delete(&mut self) {
        let Some(s) = self.selected_session().cloned() else { return };
        if let Some(t) = self.tab_open_for(&s.key) {
            self.close_tab(t + 1);
            std::thread::sleep(Duration::from_millis(300)); // let the agent release its files
        }
        match core::trash_session(&s, self.agent_cfg.get(&s.agent)) {
            Ok(_) => {
                let _ = aliases::set(&s.key, None);
                self.set_status(self.t.t("delete.done"));
                self.reload();
            }
            Err(e) => self.set_status(self.t.tf("delete.error", &[("error", &format!("{e:#}"))])),
        }
    }

    pub fn start_new(&mut self) {
        let Some(cwd) = self.selected_session().and_then(|s| s.cwd.clone()) else {
            self.set_status(self.t.t("open.folderMissing"));
            return;
        };
        let project = basename(&cwd);
        let agents = self.cfg.enabled();
        if agents.len() == 1 {
            self.new_session(&agents[0], &cwd, &project);
        } else {
            self.mode = Mode::PickAgent { cwd, project, cursor: 0 };
        }
    }

    /// Images of the session selected in the list (`i`).
    pub fn show_images(&mut self) {
        let Some(s) = self.selected_session().cloned() else { return };
        let title = self.title_of(&s);
        self.open_images_popup(&s, title);
    }

    /// Images of the session open in the active tab, also for new tabs once Claude saved them.
    pub fn show_tab_images(&mut self) {
        let Some(tab) = self.active.checked_sub(1).and_then(|i| self.tabs.get(i)) else { return };
        let title = tab.title.clone();
        match self.tab_session(self.active) {
            Some(s) => self.open_images_popup(&s, title),
            None => self.set_status(self.t.t("images.notClaude")),
        }
    }

    fn open_images_popup(&mut self, s: &Session, title: String) {
        if s.agent != "claude" {
            self.set_status(self.t.t("images.notClaude"));
            return;
        }
        match core::claude::images(s, Some(&images_dir(&s.id))) {
            Ok(messages) if messages.is_empty() => self.set_status(self.t.t("images.none")),
            Ok(mut messages) => {
                messages.reverse(); // newest first
                self.mode = Mode::Images { title, session_id: s.id.clone(), messages, cursor: 0 };
            }
            Err(e) => self.set_status(format!("{e:#}")),
        }
    }

    /// The Claude session of a tab: a listed session, or — for new tabs and compacted copies, whose
    /// id is not known yet — the newest session of the tab's folder created after the tab opened.
    fn tab_session(&self, n: usize) -> Option<Session> {
        let tab = self.tabs.get(n.checked_sub(1)?)?;
        if tab.agent.as_deref() != Some("claude") {
            return None;
        }
        if let Some(s) = self.sessions.iter().find(|s| s.key == tab.key) {
            return Some(s.clone());
        }
        let claude = core::by_id("claude")?;
        let dir = core::claude::project_dir(&core::data_dir_of(claude, self.agent_cfg.get("claude")), tab.cwd.as_deref()?);
        // Sessions already shown by other tabs are not this one
        let taken: Vec<String> = self
            .tabs
            .iter()
            .filter_map(|t| t.key.strip_prefix("claude:"))
            .map(|id| id.split(':').next().unwrap_or(id).to_string())
            .collect();
        let id = core::claude::find_newest(&dir, tab.started_ms, &taken)?;
        let mut s = Session::new("claude", id.clone());
        s.src = Some(dir.join(format!("{id}.jsonl")));
        Some(s)
    }

    /// Click on "[Image #N]" in a Claude tab: opens that image with the system viewer.
    fn open_image_ref(&mut self, n: u64) {
        let Some(s) = self.tab_session(self.active) else { return };
        let messages = core::claude::images(&s, Some(&images_dir(&s.id))).unwrap_or_default();
        // The newest message with that number (Claude may repeat numbers across messages)
        let hit = messages.iter().rev().flat_map(|m| &m.images).find(|i| i.n == Some(n)).and_then(|i| i.path.clone());
        match hit {
            Some(p) => {
                open_with_system(&p.display().to_string());
                self.set_status(self.t.tf("images.opened", &[("file", &format!("#{n}"))]));
            }
            None => self.set_status(self.t.tf("images.refNotSaved", &[("n", &n)])),
        }
    }

    // ---------- Mouse ----------

    /// The mouse is captured so the wheel never reaches the agent as ↑↓ (which would change its prompt).
    /// Wheel: scrolls the pane under the pointer. Click: focus, select, switch tab, open "[Image #N]".
    pub fn on_mouse(&mut self, ev: MouseEvent) {
        let (col, row) = (ev.column, ev.row);
        let inside = |r: Rect| r.contains(Position::new(col, row));
        let popup = !matches!(self.mode, Mode::Normal | Mode::Search);
        match ev.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let up = ev.kind == MouseEventKind::ScrollUp;
                if popup {
                    if let Mode::Images { cursor, .. } | Mode::Filter { cursor } | Mode::PickAgent { cursor, .. } = &mut self.mode
                    {
                        *cursor = if up { cursor.saturating_sub(1) } else { *cursor + 1 };
                    }
                } else if inside(self.layout.terminal) && self.active > 0 {
                    let area = self.layout.terminal;
                    if let Some(term) = self.tabs[self.active - 1].term.as_mut() {
                        term.wheel(up, col - area.x, row - area.y);
                    }
                } else if inside(self.layout.list) {
                    self.move_cursor(if up { -3 } else { 3 });
                }
            }
            MouseEventKind::Down(MouseButton::Left) if !popup => {
                if let Some(index) = self.layout.views.iter().position(|&area| inside(area)) {
                    self.on_action(Action::SessionView(View::ALL[index]));
                } else if let Some(&(_, _, n)) =
                    self.layout.tab_spans.iter().find(|(x0, x1, _)| inside(self.layout.tabs) && col >= *x0 && col < *x1)
                {
                    self.active = n;
                    self.focus_list = false;
                } else if inside(self.layout.list) {
                    self.focus_list = true;
                    let idx = self.list.offset() + (row - self.layout.list.y) as usize;
                    if matches!(self.rows.get(idx), Some(Row::Session(_))) {
                        // Second click on the same row (within 0.5 s) opens it
                        let double = self.list.selected() == Some(idx)
                            && self.last_click.is_some_and(|(at, r)| r == idx && at.elapsed() < Duration::from_millis(500));
                        self.list.select(Some(idx));
                        self.last_click = Some((Instant::now(), idx));
                        if double {
                            self.open_selected();
                        }
                    }
                } else if inside(self.layout.terminal) && self.active > 0 {
                    self.focus_list = false;
                    let area = self.layout.terminal;
                    let text = self.tabs[self.active - 1].term.as_ref().map(|t| t.row_text(row - area.y)).unwrap_or_default();
                    if let Some(n) = image_ref_at(&text, (col - area.x) as usize) {
                        self.open_image_ref(n);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn open_folder(&mut self) {
        match self.selected_session().and_then(|s| s.cwd.clone()) {
            Some(cwd) => open_with_system(&cwd),
            None => self.set_status(self.t.t("open.folderMissing")),
        }
    }

    /// Resumes the selected session in a new window/tab of the system terminal.
    pub fn open_in_new_window(&mut self) {
        let Some(s) = self.selected_session().cloned() else { return };
        let Ok(launch) = self.launch_for(&s.agent, LaunchMode::Resume, Some(&s.id), s.cwd.as_deref()) else { return };
        let key = if new_terminal_window(&launch) { "window.opened" } else { "window.unsupported" };
        self.set_status(self.t.t(key));
    }

    // ---------- Keys ----------

    pub fn on_paste(&mut self, text: String) {
        match &mut self.mode {
            Mode::Search => {
                self.search.push_str(&text);
                self.rebuild();
            }
            Mode::Rename { input } => input.push_str(&text),
            _ if !self.focus_list && self.active > 0 => {
                if let Some(term) = self.tabs.get_mut(self.active - 1).and_then(|t| t.term.as_mut()) {
                    term.paste(&text);
                }
            }
            _ => {}
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if matches!(self.mode, Mode::Search) {
            if let Some(action @ Action::SessionView(_)) = self.keys.action(&key) {
                self.on_action(action);
                return;
            }
        }
        // Popups first (they belong to the sessions list, or to closing a tab)
        if !matches!(self.mode, Mode::Normal) {
            self.on_mode_key(key);
            return;
        }
        if let Some(action) = self.keys.action(&key) {
            self.on_action(action);
            return;
        }
        if self.in_terminal() {
            self.on_tab_key(key);
        } else {
            self.on_list_key(key);
        }
    }

    fn on_action(&mut self, action: Action) {
        let n = self.tabs.len();
        match action {
            Action::SessionView(view) => {
                self.view = view;
                self.search.clear();
                self.mode = Mode::Normal;
                self.focus_list = true;
                self.last_click = None;
                self.rebuild();
            }
            // Switching tabs always lands in the terminal
            Action::NextTab | Action::PrevTab if n > 0 => {
                let cur = self.active.max(1) - 1;
                let next = if action == Action::NextTab { (cur + 1) % n } else { (cur + n - 1) % n };
                self.active = next + 1;
                self.focus_list = false;
            }
            Action::NextTab | Action::PrevTab => {}
            Action::ToggleFocus => self.focus_list = !self.focus_list || self.active == 0,
            Action::Home => self.focus_list = true,
            Action::Images => self.show_tab_images(),
            Action::GotoTab(i) if (i as usize) <= n => {
                self.active = i as usize;
                self.focus_list = false;
            }
            Action::GotoTab(_) => {}
            Action::CloseTab if self.active > 0 => {
                let alive = self.tabs[self.active - 1].term.as_ref().is_some_and(|t| t.alive());
                if alive { self.mode = Mode::ConfirmClose(self.active) } else { self.close_tab(self.active) }
            }
            Action::CloseTab => {}
            Action::ScrollUp | Action::ScrollDown if self.active > 0 => {
                let page = (self.term_size.0 as isize / 2).max(1);
                if let Some(term) = self.tabs[self.active - 1].term.as_mut() {
                    term.scroll_by(if action == Action::ScrollUp { page } else { -page });
                }
            }
            _ => {}
        }
    }

    fn on_tab_key(&mut self, key: KeyEvent) {
        let n = self.active;
        let tab = &mut self.tabs[n - 1];
        let Some(term) = tab.term.as_mut() else {
            if key.code == KeyCode::Enter {
                self.restart_tab(n);
            } else if key.code == KeyCode::Esc {
                self.close_tab(n);
            }
            return;
        };
        if !term.alive() {
            if key.code == KeyCode::Enter {
                self.restart_tab(n);
            }
            return;
        }
        if let Some(p) = term.pending.take() {
            self.status = Some((self.t.tf("compact.cancelled", &[("cmd", &p.text)]), Instant::now()));
        }
        term.send_key(key);
    }

    fn on_list_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if ctrl => self.request_quit(),
            KeyCode::Char('q') => self.request_quit(),
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1),
            KeyCode::PageUp => self.move_cursor(-10),
            KeyCode::PageDown => self.move_cursor(10),
            KeyCode::Home | KeyCode::Char('g') => self.move_cursor(-100_000),
            KeyCode::End | KeyCode::Char('G') => self.move_cursor(100_000),
            KeyCode::Enter => self.open_selected(),
            // Tab = go to the terminal on the right (if one is open)
            KeyCode::Tab if self.active > 0 => self.focus_list = false,
            KeyCode::Char('/') => self.mode = Mode::Search,
            KeyCode::Esc if !self.search.is_empty() => {
                self.search.clear();
                self.rebuild();
            }
            KeyCode::Char('f') => self.mode = Mode::Filter { cursor: 1 },
            KeyCode::Char('*') => self.change_organization(Change::Favorite),
            KeyCode::Char('h') => self.change_organization(Change::Hide),
            KeyCode::Char('H') if self.view != View::Hidden => self.change_organization(Change::HideProject),
            KeyCode::Char('n') => self.start_new(),
            KeyCode::Char('c') => self.continue_compacted(),
            KeyCode::Char('r') | KeyCode::F(2) => self.start_rename(),
            KeyCode::Char('d') | KeyCode::Delete => self.start_delete(),
            KeyCode::Char('i') => self.show_images(),
            KeyCode::Char('o') => self.open_folder(),
            KeyCode::Char('t') => self.open_shell(),
            KeyCode::Char('w') => self.open_in_new_window(),
            KeyCode::Char('R') | KeyCode::F(5) => {
                self.reload();
                let n = self.sessions.len() as u64;
                self.set_status(self.t.plural("agents.sessions", n, &[]));
            }
            KeyCode::Char('?') | KeyCode::F(1) => self.mode = Mode::Help,
            _ => {}
        }
    }

    pub fn request_quit(&mut self) {
        if self.running_tabs() > 0 { self.mode = Mode::ConfirmQuit } else { self.quit = true }
    }

    fn on_mode_key(&mut self, key: KeyEvent) {
        let mode = std::mem::replace(&mut self.mode, Mode::Normal);
        self.mode = match mode {
            Mode::Search => match key.code {
                KeyCode::Esc => {
                    self.search.clear();
                    self.rebuild();
                    Mode::Normal
                }
                KeyCode::Enter => Mode::Normal,
                KeyCode::Up | KeyCode::Down => {
                    self.move_cursor(if key.code == KeyCode::Up { -1 } else { 1 });
                    Mode::Search
                }
                KeyCode::Backspace => {
                    self.search.pop();
                    self.rebuild();
                    Mode::Search
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.search.push(c);
                    self.rebuild();
                    Mode::Search
                }
                _ => Mode::Search,
            },
            Mode::Filter { mut cursor } => {
                let rows = self.filter_rows();
                let options: Vec<usize> =
                    rows.iter().enumerate().filter(|(_, r)| matches!(r, FilterRow::Option { .. })).map(|(i, _)| i).collect();
                let pos = options.iter().position(|&i| i == cursor).unwrap_or(0);
                match key.code {
                    KeyCode::Esc | KeyCode::Char('f') | KeyCode::Char('q') => return,
                    KeyCode::Up | KeyCode::Char('k') => cursor = options[pos.saturating_sub(1)],
                    KeyCode::Down | KeyCode::Char('j') => cursor = options[(pos + 1).min(options.len() - 1)],
                    KeyCode::Char('c') => {
                        self.filters.clear();
                        self.rebuild();
                    }
                    KeyCode::Char(' ') | KeyCode::Enter => {
                        if let Some(FilterRow::Option { group, value, .. }) = rows.get(cursor) {
                            let set = self.filters.entry(group).or_default();
                            if !set.remove(value) {
                                set.insert(value.clone());
                            }
                            self.rebuild();
                        }
                    }
                    _ => {}
                }
                Mode::Filter { cursor }
            }
            Mode::Rename { mut input } => match key.code {
                KeyCode::Esc => Mode::Normal,
                KeyCode::Enter => {
                    self.finish_rename(input);
                    Mode::Normal
                }
                KeyCode::Backspace => {
                    input.pop();
                    Mode::Rename { input }
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => Mode::Rename { input: String::new() },
                KeyCode::Char(c) => {
                    input.push(c);
                    Mode::Rename { input }
                }
                _ => Mode::Rename { input },
            },
            Mode::ConfirmDelete => {
                if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('s') | KeyCode::Char('Y') | KeyCode::Char('S')) {
                    self.finish_delete();
                }
                Mode::Normal
            }
            Mode::ConfirmQuit => {
                if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('s') | KeyCode::Char('Y') | KeyCode::Char('S')) {
                    self.quit = true;
                }
                Mode::Normal
            }
            Mode::ConfirmClose(n) => {
                if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('s') | KeyCode::Char('Y') | KeyCode::Char('S')) {
                    self.close_tab(n);
                }
                Mode::Normal
            }
            Mode::Help => Mode::Normal,
            Mode::PickAgent { cwd, project, mut cursor } => {
                let agents = self.cfg.enabled();
                match key.code {
                    KeyCode::Esc => return,
                    KeyCode::Up | KeyCode::Char('k') => cursor = cursor.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => cursor = (cursor + 1).min(agents.len().saturating_sub(1)),
                    KeyCode::Enter => {
                        if let Some(a) = agents.get(cursor) {
                            self.new_session(a, &cwd, &project);
                        }
                        return;
                    }
                    _ => {}
                }
                Mode::PickAgent { cwd, project, cursor }
            }
            Mode::Images { title, session_id, messages, mut cursor } => {
                let flat: Vec<&std::path::Path> =
                    messages.iter().flat_map(|m| m.images.iter().filter_map(|i| i.path.as_deref())).collect();
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('i') => return,
                    KeyCode::Up | KeyCode::Char('k') => cursor = cursor.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => cursor = (cursor + 1).min(flat.len().saturating_sub(1)),
                    KeyCode::Enter => {
                        if let Some(p) = flat.get(cursor) {
                            open_with_system(&p.display().to_string());
                            let name = p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                            self.set_status(self.t.tf("images.opened", &[("file", &name)]));
                        }
                    }
                    KeyCode::Char('a') => open_with_system(&images_dir(&session_id).display().to_string()),
                    _ => {}
                }
                Mode::Images { title, session_id, messages, cursor }
            }
            Mode::Normal => Mode::Normal,
        };
    }
}

/// Number of the "[Image #N]" under a column of a terminal line.
pub fn image_ref_at(line: &str, col: usize) -> Option<u64> {
    let chars: Vec<char> = line.chars().collect();
    let text = line;
    let mut from = 0;
    while let Some(pos) = text[from..].find("[Image #") {
        let start_byte = from + pos;
        let start = text[..start_byte].chars().count();
        let rest = &text[start_byte + 8..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        let end = start + 8 + digits.chars().count(); // index of the closing bracket
        if !digits.is_empty() && chars.get(end) == Some(&']') && (start..=end).contains(&col) {
            return digits.parse().ok();
        }
        from = start_byte + 8;
    }
    None
}

pub fn filter_value(group: &str, s: &Session) -> String {
    match group {
        "agent" => s.agent.clone(),
        "origin" => s.origin.clone().unwrap_or_default(),
        _ => String::new(),
    }
}

pub fn basename(p: &str) -> String {
    p.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next().filter(|s| !s.is_empty()).unwrap_or(p).to_string()
}

/// Where images of a Claude session are extracted (cache, safe to delete).
fn images_dir(session_id: &str) -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("resume-ai").join("images").join(session_id)
}

/// Opens a file or folder with the default application.
pub fn open_with_system(path: &str) {
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("explorer.exe");
        c.arg(path);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(path);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(path);
        c
    };
    let _ = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

/// Opens a new window/tab of the system terminal running `launch`. Returns false if none was found.
fn new_terminal_window(launch: &Launch) -> bool {
    use std::process::Command;
    let cwd = launch.cwd.clone().unwrap_or_else(|| dirs::home_dir().unwrap_or_default());
    let spawned = if cfg!(windows) {
        if core::which("wt").is_some() {
            Command::new("wt").args(["-w", "0", "new-tab", "-d"]).arg(&cwd).arg(&launch.program).args(&launch.args).spawn()
        } else {
            Command::new("cmd").args(["/c", "start", "", "/D"]).arg(&cwd).arg(&launch.program).args(&launch.args).spawn()
        }
    } else if cfg!(target_os = "macos") {
        let line = std::iter::once(&launch.program).chain(&launch.args).map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ");
        let script = format!(
            "tell application \"Terminal\" to do script \"cd {} && {}\"",
            shell_quote(&cwd.display().to_string()),
            line.replace('"', "\\\"")
        );
        Command::new("osascript").args(["-e", &script, "-e", "tell application \"Terminal\" to activate"]).spawn()
    } else {
        let line = std::iter::once(&launch.program).chain(&launch.args).map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ");
        let term = std::env::var("TERMINAL")
            .ok()
            .filter(|t| core::which(t).is_some())
            .or_else(|| core::which("x-terminal-emulator").map(|_| "x-terminal-emulator".into()));
        match term {
            Some(t) => Command::new(t).current_dir(&cwd).args(["-e", "sh", "-c", &line]).spawn(),
            None => return false,
        }
    };
    spawned.is_ok()
}

fn shell_quote(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=@".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::image_ref_at;

    #[test]
    fn finds_the_image_reference_under_the_click() {
        let line = "> mira [Image #4] y [Image #12] ñ";
        assert_eq!(image_ref_at(line, 7), Some(4)); // the opening bracket
        assert_eq!(image_ref_at(line, 16), Some(4)); // the closing bracket
        assert_eq!(image_ref_at(line, 17), None);
        assert_eq!(image_ref_at(line, 25), Some(12));
        assert_eq!(image_ref_at("[Image #] [Image 3]", 2), None);
    }
}
