//! State and actions of the interactive mode.

use super::keys::{Action, KeyMap};
use super::term::{Launch, PendingInput, TermTab};
use crate::config::Config;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
    Images { session: usize, messages: Vec<ImageMessage>, cursor: usize },
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
    pub error: Option<String>,
}

pub struct App {
    pub t: I18n,
    pub keys: KeyMap,
    pub cfg: Config,
    agent_cfg: BTreeMap<String, AgentConfig>,
    pub sessions: Vec<Session>,
    pub aliases: BTreeMap<String, String>,
    pub load_errors: Vec<String>,
    pub rows: Vec<Row>,
    pub list: ListState,
    pub search: String,
    pub filters: BTreeMap<&'static str, BTreeSet<String>>,
    pub mode: Mode,
    pub tabs: Vec<Tab>,
    /// 0 = sessions list, n = tabs[n - 1]
    pub active: usize,
    pub status: Option<(String, Instant)>,
    pub term_size: (u16, u16),
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
            term_size: (24, 80),
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
        // Grouped by folder; on Windows "C:\x" and "c:\x\" are the same folder
        let norm = |p: &Option<String>| {
            p.as_deref().map(|p| {
                let p = p.trim_end_matches(['\\', '/']);
                if cfg!(windows) { p.replace('/', "\\").to_lowercase() } else { p.to_string() }
            })
        };
        let mut groups: Vec<(Option<String>, Option<String>, Vec<usize>)> = Vec::new();
        for (i, s) in self.sessions.iter().enumerate() {
            if !self.matches_filters(s, None) || !self.matches_search(s) {
                continue;
            }
            let key = norm(&s.cwd);
            match groups.iter_mut().find(|(k, ..)| *k == key) {
                Some((.., items)) => items.push(i),
                None => groups.push((key, s.cwd.clone(), vec![i])),
            }
        }
        self.rows.clear();
        for (_, cwd, items) in groups {
            let name = cwd.as_deref().map(basename).unwrap_or_else(|| self.t.t("session.unknownFolder"));
            self.rows.push(Row::Group { name, cwd, count: items.len() });
            self.rows.extend(items.into_iter().map(Row::Session));
        }
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

    fn spawn_tab(&mut self, title: String, agent: Option<String>, key: String, launch: Launch) -> usize {
        let (rows, cols) = self.term_size;
        let (term, error) = match TermTab::spawn(launch, rows, cols, self.notify.clone()) {
            Ok(t) => (Some(t), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        self.tabs.push(Tab { title, agent, key, term, error });
        self.active = self.tabs.len();
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
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
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
        let Some(tab) = self.tabs.get_mut(n - 1) else { return };
        let launch = match &tab.term {
            Some(t) => t.launch.clone(),
            None => return,
        };
        tab.term = None; // drops (kills) the old process first
        match TermTab::spawn(launch, rows, cols, notify) {
            Ok(t) => {
                tab.term = Some(t);
                tab.error = None;
            }
            Err(e) => tab.error = Some(format!("{e:#}")),
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

    pub fn show_images(&mut self) {
        let Some(i) = self.selected_index() else { return };
        let s = &self.sessions[i];
        if s.agent != "claude" {
            self.set_status(self.t.t("images.notClaude"));
            return;
        }
        let dir = images_dir(&s.id);
        match core::claude::images(s, Some(&dir)) {
            Ok(messages) if messages.is_empty() => self.set_status(self.t.t("images.none")),
            Ok(mut messages) => {
                messages.reverse(); // newest first
                self.mode = Mode::Images { session: i, messages, cursor: 0 };
            }
            Err(e) => self.set_status(format!("{e:#}")),
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
            _ if self.active > 0 => {
                if let Some(term) = self.tabs.get_mut(self.active - 1).and_then(|t| t.term.as_mut()) {
                    term.paste(&text);
                }
            }
            _ => {}
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        // Popups first (they belong to the sessions list, or to closing a tab)
        if !matches!(self.mode, Mode::Normal) {
            self.on_mode_key(key);
            return;
        }
        if let Some(action) = self.keys.action(&key) {
            self.on_action(action);
            return;
        }
        if self.active > 0 {
            self.on_tab_key(key);
        } else {
            self.on_list_key(key);
        }
    }

    fn on_action(&mut self, action: Action) {
        let n = self.tabs.len() + 1; // including the sessions list
        match action {
            Action::NextTab => self.active = (self.active + 1) % n,
            Action::PrevTab => self.active = (self.active + n - 1) % n,
            Action::Home => self.active = 0,
            Action::GotoTab(i) if (i as usize) <= self.tabs.len() => self.active = i as usize,
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
            if key.code == KeyCode::Enter && tab.error.is_some() {
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
            KeyCode::Char('/') => self.mode = Mode::Search,
            KeyCode::Esc if !self.search.is_empty() => {
                self.search.clear();
                self.rebuild();
            }
            KeyCode::Char('f') => self.mode = Mode::Filter { cursor: 1 },
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
            Mode::Images { session, messages, mut cursor } => {
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
                    KeyCode::Char('a') => open_with_system(&images_dir(&self.sessions[session].id).display().to_string()),
                    _ => {}
                }
                Mode::Images { session, messages, cursor }
            }
            Mode::Normal => Mode::Normal,
        };
    }
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
