//! Drawing of the interactive mode.

use super::app::{App, FilterRow, Mode, Row};
use super::keys::Action;
use super::term::CommandNotFound;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use resume_ai_core::{self as core, i18n::I18n};
use unicode_width::UnicodeWidthStr;

const ACCENT: Color = Color::Rgb(217, 119, 87);
const MUTED: Color = Color::Rgb(140, 140, 140);
const SELECTED_BG: Color = Color::Rgb(62, 42, 34);
const RUNNING: Color = Color::Rgb(92, 184, 92);
const WARN: Color = Color::Rgb(217, 165, 87);

fn hex(c: &str) -> Color {
    let v = u32::from_str_radix(c.trim_start_matches('#'), 16).unwrap_or(0x888888);
    Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn badge(agent: &str) -> Span<'static> {
    let (text, color) = core::by_id(agent).map(|a| (a.badge(), hex(a.color()))).unwrap_or(("??", MUTED));
    Span::styled(format!(" {text} "), Style::default().bg(color).fg(Color::Black).add_modifier(Modifier::BOLD))
}

/// Cuts a string to `width` columns, adding "…".
fn fit(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars() {
        if out.width() + c.to_string().width() + 1 > width {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

pub fn time_ago(t: &I18n, ms: f64) -> String {
    let now = chrono::Utc::now().timestamp_millis() as f64;
    let mins = ((now - ms) / 60_000.0).max(0.0) as u64;
    match mins {
        0 => t.t("time.now"),
        m if m < 60 => t.tf("time.minutes", &[("n", &m)]),
        m if m < 60 * 24 => t.tf("time.hours", &[("n", &(m / 60))]),
        m if m < 60 * 24 * 30 => t.tf("time.days", &[("n", &(m / 60 / 24))]),
        _ => format_date(ms),
    }
}

fn format_date(ms: f64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// Layout: sessions list on the left (always visible, `sidebar_percent` wide), tabs + terminal on the right.
pub fn draw(f: &mut Frame, app: &mut App) {
    let [main, status_area] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(f.area());
    let pct = app.cfg.sidebar_percent.clamp(10, 60);
    let [left, right] = Layout::horizontal([Constraint::Percentage(pct), Constraint::Percentage(100 - pct)]).areas(main);
    draw_sidebar(f, app, left);
    let [tabs_area, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(right);
    app.term_size = (body.height, body.width);
    app.layout.tabs = tabs_area;
    app.layout.terminal = if app.active > 0 { body } else { Rect::default() };
    draw_tabs(f, app, tabs_area);
    if app.active == 0 {
        draw_welcome(f, app, body);
    } else {
        draw_tab(f, app, body);
    }
    draw_status(f, app, status_area);
    draw_popup(f, app);
}

fn draw_tabs(f: &mut Frame, app: &mut App, area: Rect) {
    let mut spans = Vec::new();
    let mut tab_spans = Vec::new(); // clickable ranges
    // The active tab is orange while the keyboard is in the terminal, grey while it is in the list
    let active = if app.in_terminal() {
        Style::default().bg(ACCENT).fg(Color::Black).add_modifier(Modifier::BOLD)
    } else {
        Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD)
    };
    for (i, tab) in app.tabs.iter().enumerate() {
        spans.push(Span::raw(" "));
        let start = area.x + spans.iter().map(|s| s.width() as u16).sum::<u16>();
        let alive = tab.term.as_ref().is_some_and(|t| t.alive());
        let style = if app.active == i + 1 {
            active
        } else if alive {
            Style::default()
        } else {
            Style::default().fg(MUTED).add_modifier(Modifier::CROSSED_OUT)
        };
        spans.push(Span::styled(format!(" {} ", i + 1), style));
        match &tab.agent {
            Some(a) => spans.push(badge(a)),
            None => spans.push(Span::styled(" >_ ", Style::default().bg(Color::DarkGray).fg(Color::White))),
        }
        spans.push(Span::styled(format!(" {} ", fit(&tab.title, 22)), style));
        let end = area.x + spans.iter().map(|s| s.width() as u16).sum::<u16>();
        tab_spans.push((start, end, i + 1));
    }
    app.layout.tab_spans = tab_spans;
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Left pane: title, search, active filters and the sessions list. Orange border = keyboard here.
fn draw_sidebar(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = !app.in_terminal();
    let border = if focused { ACCENT } else { Color::DarkGray };
    let visible = app.rows.iter().filter(|r| matches!(r, Row::Session(_))).count();
    let title_style = if focused { Style::default().fg(ACCENT).add_modifier(Modifier::BOLD) } else { Style::default().fg(MUTED) };
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(border))
        .title(Span::styled(format!(" ≡ {} ({visible}) ", app.t.t("tabs.sessions")), title_style));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let filters = app.active_filter_labels();
    let view_rows = if inner.width < 40 { 2 } else { 1 };
    let [_, search_area, views_area, chips_area, list_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(view_rows),
        Constraint::Length(if filters.is_empty() { 0 } else { 1 }),
        Constraint::Min(1),
    ])
    .areas(inner);

    // Search line
    let searching = matches!(app.mode, Mode::Search);
    let width = search_area.width.saturating_sub(4) as usize;
    let mut line = vec![Span::styled(" / ", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))];
    if app.search.is_empty() && !searching {
        line.push(Span::styled(fit(&app.t.t("search.placeholder"), width), Style::default().fg(MUTED)));
    } else {
        line.push(Span::raw(app.search.clone()));
    }
    f.render_widget(Paragraph::new(Line::from(line)), search_area);
    let tabs = if view_rows == 1 {
        Layout::horizontal([Constraint::Ratio(1, 4); 4]).areas(views_area)
    } else {
        let [top, bottom] = Layout::vertical([Constraint::Length(1); 2]).areas(views_area);
        let [favorites, recent] = Layout::horizontal([Constraint::Ratio(1, 2); 2]).areas(top);
        let [all, hidden] = Layout::horizontal([Constraint::Ratio(1, 2); 2]).areas(bottom);
        [favorites, recent, all, hidden]
    };
    app.layout.views = tabs;
    for (view, area) in core::organization::View::ALL.into_iter().zip(tabs.iter().copied()) {
        let mut style = Style::default().fg(MUTED);
        if view == app.view {
            style = style.fg(ACCENT).add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
        }
        f.render_widget(Paragraph::new(fit(&app.t.t(view.label()), area.width as usize)).centered().style(style), area);
    }
    if searching {
        f.set_cursor_position(Position::new(search_area.x + 3 + app.search.width() as u16, search_area.y));
    }
    if !filters.is_empty() {
        let text = app.t.tf("filter.active", &[("list", &filters.join(", "))]);
        f.render_widget(Paragraph::new(Span::styled(format!(" {}", fit(&text, width)), Style::default().fg(ACCENT))), chips_area);
    }
    app.layout.list = list_area;
    draw_list(f, app, list_area);
}

/// Right pane when no session is open: how to use the app + details of the selected session.
fn draw_welcome(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.t;
    let focus = app.keys.label(Action::ToggleFocus);
    let lines = vec![
        Line::raw(""),
        Line::styled(format!("  {}", t.t("pane.empty.title")), Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Line::raw(""),
        Line::raw(format!("  {}", t.t("pane.empty.open"))),
        Line::raw(format!("  {}", t.tf("pane.empty.focus", &[("focus", &focus)]))),
        Line::raw(format!("  {}", t.t("pane.empty.help"))),
        Line::raw(""),
    ];
    let [top, details] = Layout::vertical([Constraint::Length(lines.len() as u16), Constraint::Min(1)]).areas(area);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), top);
    draw_details(f, app, Rect::new(details.x + 1, details.y, details.width.saturating_sub(2), details.height));
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect) {
    let width = area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = app
        .rows
        .iter()
        .map(|row| match row {
            Row::Group { name, cwd, count } => {
                let mut spans = vec![
                    Span::styled(format!(" ▾ {}", name.to_uppercase()), Style::default().fg(MUTED).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("  {count}"), Style::default().fg(MUTED)),
                ];
                if let Some(c) = cwd {
                    let rest = width.saturating_sub(name.width() + count.to_string().width() + 6);
                    if rest > 10 {
                        spans.push(Span::styled(format!("  {}", fit(c, rest)), Style::default().fg(Color::DarkGray)));
                    }
                }
                ListItem::new(Line::from(spans))
            }
            Row::Session(i) => {
                let s = &app.sessions[*i];
                let open = app.tab_open_for(&s.key).map(|t| app.tabs[t].term.as_ref().is_some_and(|x| x.alive()));
                let dot = match open {
                    Some(true) => Span::styled(" ● ", Style::default().fg(RUNNING)),
                    Some(false) => Span::styled(" ○ ", Style::default().fg(MUTED)),
                    None => Span::raw("   "),
                };
                // Extra info gives way to the title: the branch goes first, then the origin tag,
                // and in a narrow list even the time
                let mut parts = if width >= 34 { vec![time_ago(&app.t, s.mtime)] } else { Vec::new() };
                let origin = s.origin.as_deref().filter(|o| *o != "cli").map(|o| format!("[{}]", app.t.origin(o)));
                let branch = s.git_branch.as_deref().filter(|b| *b != "HEAD").map(|b| format!("⎇ {}", fit(b, 16)));
                for extra in [origin, branch].into_iter().flatten() {
                    let with = parts.iter().map(|p| p.width() + 2).sum::<usize>() + extra.width();
                    if !parts.is_empty() && width.saturating_sub(3 + 5 + 2 + with) >= width * 55 / 100 {
                        parts.insert(1, extra);
                    }
                }
                let meta = parts.join("  ");
                let favorite = app.organization.favorites.contains(&s.key);
                let marker_width = usize::from(favorite) * 2;
                let title_width = width.saturating_sub(3 + 5 + meta.width() + 2 + marker_width);
                let title = fit(&app.title_of(s), title_width);
                let pad = width.saturating_sub(3 + 5 + title.width() + meta.width() + marker_width);
                ListItem::new(Line::from(vec![
                    dot,
                    badge(&s.agent),
                    Span::raw(" "),
                    Span::styled(if favorite { "★ " } else { "" }, Style::default().fg(ACCENT)),
                    Span::raw(title),
                    Span::raw(" ".repeat(pad)),
                    Span::styled(meta, Style::default().fg(MUTED)),
                ]))
            }
        })
        .collect();
    let empty = items.is_empty();
    let list =
        List::new(items).highlight_style(Style::default().bg(SELECTED_BG).add_modifier(Modifier::BOLD)).highlight_symbol("▌");
    f.render_stateful_widget(list, area, &mut app.list);
    if empty {
        let key =
            if app.search.is_empty() && app.filters.values().all(|v| v.is_empty()) { "list.empty" } else { "list.noResults" };
        f.render_widget(Paragraph::new(Span::styled(format!("  {}", app.t.t(key)), Style::default().fg(MUTED))), area);
    }
}

fn draw_details(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default().title(Span::styled(
        format!("{} ", app.t.t("details.title").to_uppercase()),
        Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
    ));
    let Some(s) = app.selected_session() else {
        f.render_widget(block, area);
        return;
    };
    let label = |k: &str| Span::styled(format!("{}: ", app.t.t(k)), Style::default().fg(MUTED));
    let agent = core::by_id(&s.agent).map(|a| a.name()).unwrap_or(&s.agent);
    // The title can be a whole first prompt: at most 3 lines so the fields stay visible
    let title = fit(&app.title_of(s), area.width.saturating_sub(2) as usize * 3);
    let mut lines = vec![
        Line::from(Span::styled(title, Style::default().add_modifier(Modifier::BOLD))),
        Line::raw(""),
        Line::from(vec![label("details.agent"), badge(&s.agent), Span::raw(format!(" {agent}"))]),
        Line::from(vec![label("details.folder"), Span::raw(s.cwd.clone().unwrap_or_else(|| app.t.t("session.unknownFolder")))]),
    ];
    if let Some(b) = s.git_branch.as_ref().filter(|b| *b != "HEAD") {
        lines.push(Line::from(vec![label("details.branch"), Span::raw(b.clone())]));
    }
    if let Some(o) = &s.origin {
        lines.push(Line::from(vec![label("details.createdIn"), Span::raw(app.t.origin(o))]));
        if let Some(last) = s.last_origin.as_ref().filter(|l| *l != o) {
            lines.push(Line::from(vec![label("details.lastUsedIn"), Span::raw(app.t.origin(last))]));
        }
    }
    lines.push(Line::from(vec![
        label("details.lastActivity"),
        Span::raw(format!("{} ({})", format_date(s.mtime), time_ago(&app.t, s.mtime))),
    ]));
    lines.push(Line::from(vec![label("details.id"), Span::styled(s.id.clone(), Style::default().fg(Color::DarkGray))]));
    f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
}

fn draw_tab(f: &mut Frame, app: &mut App, area: Rect) {
    let idx = app.active - 1;
    let t = &app.t;
    let close = app.keys.label(Action::CloseTab);
    let tab = &mut app.tabs[idx];
    let Some(term) = tab.term.as_mut() else {
        let mut lines = vec![Line::raw("")];
        if let Some(error) = &tab.error {
            if let Some(CommandNotFound(program)) = error.downcast_ref::<CommandNotFound>() {
                let agent = tab.agent.as_deref().and_then(core::by_id);
                let message = match agent {
                    Some(agent) => t.tf("tab.agentMissing", &[("agent", &agent.name()), ("command", program)]),
                    None => t.tf("tab.commandMissing", &[("command", program)]),
                };
                lines.push(Line::styled(message, Style::default().fg(WARN)));
                if tab.agent.as_deref() == Some("claude") {
                    lines.push(Line::raw(t.t("tab.installGuide")));
                    lines.push(Line::styled("https://code.claude.com/docs/en/quickstart", Style::default().fg(ACCENT)));
                }
                lines.push(Line::raw(t.t("tab.pathHint")));
            } else {
                lines.push(Line::styled(t.tf("tab.error", &[("error", &format!("{error:#}"))]), Style::default().fg(WARN)));
            }
        }
        lines.push(Line::styled(t.t("tab.errorHint"), Style::default().fg(MUTED)));
        let p = Paragraph::new(lines).alignment(Alignment::Center).wrap(Wrap { trim: true });
        f.render_widget(p, area);
        return;
    };
    term.resize(area.height, area.width);
    if let Some((x, y)) = term.render(area, f.buffer_mut()) {
        f.set_cursor_position(Position::new(x, y));
    }
    if let Some(code) = term.exit_code {
        let msg = t.tf("tab.exited", &[("code", &code), ("close", &close)]);
        let bar = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
        f.render_widget(Clear, bar);
        f.render_widget(
            Paragraph::new(Span::styled(format!(" {msg}"), Style::default().bg(Color::DarkGray).fg(Color::White))),
            bar,
        );
    }
    if term.scroll > 0 {
        let msg = format!(" {} ", t.tf("tab.scrolled", &[("n", &term.scroll)]));
        let w = msg.width() as u16;
        let r = Rect::new(area.right().saturating_sub(w), area.y, w.min(area.width), 1);
        f.render_widget(Paragraph::new(Span::styled(msg, Style::default().bg(ACCENT).fg(Color::Black))), r);
    }
}

/// Bottom bar: where the keyboard is (SESSIONS / TERMINAL) and the keys that work there.
fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let k = |a| app.keys.label(a);
    let in_terminal = app.in_terminal();
    let (text, style) = if let Some((msg, _)) = &app.status {
        (msg.clone(), Style::default().fg(ACCENT))
    } else if let Some(err) = app.load_errors.first().filter(|_| !in_terminal) {
        (format!("⚠ {err}"), Style::default().fg(WARN))
    } else if !in_terminal {
        (app.t.tf("hints.list", &[("focus", &k(Action::ToggleFocus))]), Style::default().fg(MUTED))
    } else {
        let text = app.t.tf(
            "hints.term",
            &[
                ("focus", &k(Action::ToggleFocus)),
                ("images", &k(Action::Images)),
                ("prev", &k(Action::PrevTab)),
                ("next", &k(Action::NextTab)),
                ("close", &k(Action::CloseTab)),
                ("scroll", &k(Action::ScrollUp)),
            ],
        );
        (text, Style::default().fg(MUTED))
    };
    let (label, label_style) = if in_terminal {
        (app.t.t("focus.terminal"), Style::default().bg(ACCENT).fg(Color::Black).add_modifier(Modifier::BOLD))
    } else {
        (app.t.t("focus.list"), Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD))
    };
    let label = format!(" {label} ");
    let rest = area.width.saturating_sub(label.width() as u16 + 12) as usize;
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(label, label_style), Span::styled(format!(" {}", fit(&text, rest)), style)])),
        area,
    );
    f.render_widget(
        Paragraph::new(Span::styled("resume-ai ", Style::default().fg(Color::DarkGray))).alignment(Alignment::Right),
        area,
    );
}

fn popup_area(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width.saturating_sub(4));
    let h = height.min(area.height.saturating_sub(2));
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

fn popup_block(title: String) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(format!(" {title} "), Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)))
}

fn hint(text: String) -> Line<'static> {
    Line::styled(text, Style::default().fg(MUTED))
}

fn draw_popup(f: &mut Frame, app: &App) {
    let screen = f.area();
    let t = &app.t;
    match &app.mode {
        Mode::Normal | Mode::Search => {}
        Mode::Filter { cursor } => {
            let rows = app.filter_rows();
            let items: Vec<ListItem> = rows
                .iter()
                .map(|r| match r {
                    FilterRow::Header(h) => {
                        ListItem::new(Line::styled(h.to_uppercase(), Style::default().fg(MUTED).add_modifier(Modifier::BOLD)))
                    }
                    FilterRow::Option { group, value, label, color, count } => {
                        let on = app.filters.get(group).is_some_and(|v| v.contains(value));
                        let mut spans = vec![Span::styled(
                            if on { " [x] " } else { " [ ] " },
                            Style::default().fg(if on { ACCENT } else { MUTED }),
                        )];
                        if let Some(c) = color {
                            spans.push(Span::styled("● ", Style::default().fg(hex(c))));
                        }
                        spans.push(Span::raw(label.clone()));
                        spans.push(Span::styled(format!("  {count}"), Style::default().fg(MUTED)));
                        ListItem::new(Line::from(spans))
                    }
                })
                .collect();
            let area = popup_area(screen, 44, rows.len() as u16 + 4);
            f.render_widget(Clear, area);
            let block = popup_block(t.t("filter.title"));
            let inner = block.inner(area);
            f.render_widget(block, area);
            let [list_area, hint_area] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
            let mut state = ListState::default().with_selected(Some(*cursor));
            f.render_stateful_widget(List::new(items).highlight_style(Style::default().bg(SELECTED_BG)), list_area, &mut state);
            f.render_widget(Paragraph::new(hint(t.t("filter.hint"))), hint_area);
        }
        Mode::Rename { input } => {
            let Some(s) = app.selected_session() else { return };
            let agent = core::by_id(&s.agent);
            let note = if agent.is_some_and(|a| a.native_rename()) {
                t.tf("rename.hintNative", &[("agent", &agent.map(|a| a.name()).unwrap_or(""))])
            } else {
                t.tf("rename.hintAlias", &[("agent", &agent.map(|a| a.name()).unwrap_or(""))])
            };
            let area = popup_area(screen, 70, 6);
            f.render_widget(Clear, area);
            let block = popup_block(t.t("rename.title"));
            let inner = block.inner(area);
            f.render_widget(block, area);
            let shown = fit(input, inner.width.saturating_sub(2) as usize);
            f.render_widget(
                Paragraph::new(vec![
                    Line::raw(format!(" {shown}")),
                    Line::raw(""),
                    hint(format!(" {note}")),
                    hint(format!(" {}", t.t("rename.keys"))),
                ]),
                inner,
            );
            f.set_cursor_position(Position::new(inner.x + 1 + shown.width() as u16, inner.y));
        }
        Mode::ConfirmDelete => {
            let Some(s) = app.selected_session() else { return };
            confirm(f, t.t("delete.title"), t.tf("delete.confirm", &[("title", &fit(&app.title_of(s), 50))]), t.t("delete.note"));
        }
        Mode::ConfirmQuit => {
            confirm(f, t.t("quit.title"), t.plural("quit.confirm", app.running_tabs() as u64, &[]), String::new())
        }
        Mode::ConfirmClose(_) => confirm(f, t.t("close.title"), t.t("close.confirm"), String::new()),
        Mode::Help => draw_help(f, app),
        Mode::PickAgent { project, cursor, .. } => {
            let agents = app.cfg.enabled();
            let items: Vec<ListItem> = agents
                .iter()
                .filter_map(|id| core::by_id(id))
                .map(|a| ListItem::new(Line::from(vec![Span::raw(" "), badge(a.id()), Span::raw(format!(" {}", a.name()))])))
                .collect();
            let area = popup_area(screen, 44, agents.len() as u16 + 4);
            f.render_widget(Clear, area);
            let block = popup_block(t.tf("new.title", &[("project", project)]));
            let inner = block.inner(area);
            f.render_widget(block, area);
            let [list_area, hint_area] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
            let mut state = ListState::default().with_selected(Some(*cursor));
            f.render_stateful_widget(List::new(items).highlight_style(Style::default().bg(SELECTED_BG)), list_area, &mut state);
            f.render_widget(Paragraph::new(hint(t.t("new.hint"))), hint_area);
        }
        Mode::Images { title, messages, cursor, .. } => {
            let mut items = Vec::new();
            let mut selected = None;
            let mut flat = 0;
            for (mi, m) in messages.iter().enumerate() {
                let when = if mi == 0 {
                    format!("{} · {}", t.t("images.latest"), time_ago(t, m.timestamp))
                } else {
                    time_ago(t, m.timestamp)
                };
                let origin = m.origin.as_deref().map(|o| format!(" · {}", t.origin(o))).unwrap_or_default();
                items.push(ListItem::new(Line::styled(
                    format!(" {when}{origin}"),
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                )));
                if !m.text.is_empty() {
                    items.push(ListItem::new(Line::styled(format!("   {}", fit(&m.text, 70)), Style::default().fg(MUTED))));
                }
                for img in &m.images {
                    if flat == *cursor {
                        selected = Some(items.len());
                    }
                    flat += 1;
                    let n = img.n.map(|n| format!("#{n}")).unwrap_or_else(|| "  ".into());
                    let name = img
                        .path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .map(|f| f.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    items.push(ListItem::new(Line::from(vec![
                        Span::styled(format!("   {n:<4}"), Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw(name),
                        Span::styled(format!("  {} KB", img.size / 1024), Style::default().fg(MUTED)),
                    ])));
                }
            }
            let area = popup_area(screen, 80, items.len() as u16 + 4);
            f.render_widget(Clear, area);
            let block = popup_block(t.tf("images.title", &[("title", &fit(title, 50))]));
            let inner = block.inner(area);
            f.render_widget(block, area);
            let [list_area, hint_area] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
            let mut state = ListState::default().with_selected(selected);
            f.render_stateful_widget(List::new(items).highlight_style(Style::default().bg(SELECTED_BG)), list_area, &mut state);
            f.render_widget(Paragraph::new(hint(t.t("images.hint"))), hint_area);
        }
    }
}

fn confirm(f: &mut Frame, title: String, question: String, note: String) {
    let area = popup_area(f.area(), 64, if note.is_empty() { 5 } else { 7 });
    f.render_widget(Clear, area);
    let block = popup_block(title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut lines = vec![Line::raw(""), Line::raw(format!(" {question}"))];
    if !note.is_empty() {
        lines.push(Line::raw(""));
        lines.push(hint(format!(" {note}")));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn draw_help(f: &mut Frame, app: &App) {
    let t = &app.t;
    let k = |a| app.keys.label(a);
    let tab_keys = (1..=9).filter_map(|tab| app.keys.bound_label(Action::GotoTab(tab))).collect::<Vec<_>>();
    let mut tab_rows = Vec::new();
    if tab_keys.iter().copied().eq(["Alt+1", "Alt+2", "Alt+3", "Alt+4", "Alt+5", "Alt+6", "Alt+7", "Alt+8", "Alt+9"]) {
        tab_rows.push(("Alt+1…9".into(), t.t("help.gotoTab")));
    } else {
        for tab in 1..=9 {
            if let Some(key) = app.keys.bound_label(Action::GotoTab(tab)) {
                tab_rows.push((key.into(), t.tf("help.gotoTabNumber", &[("tab", &tab.to_string())])));
            }
        }
    }
    if let Some(key) = app.keys.bound_label(Action::Home) {
        tab_rows.push((key.into(), t.t("help.home")));
    }
    let sections: Vec<(String, Vec<(String, String)>)> = vec![
        (
            t.t("help.list"),
            vec![
                ("↑ ↓ / j k".into(), t.t("help.move")),
                ("Enter".into(), t.t("help.open")),
                ("/".into(), t.t("help.search")),
                ("f".into(), t.t("help.filters")),
                ("*".into(), t.t("organize.favoriteToggle")),
                ("h".into(), t.t("organize.hideToggle")),
                ("H".into(), t.t("organize.hideProject")),
                ("n".into(), t.t("help.new")),
                ("c".into(), t.t("help.compact")),
                ("r / F2".into(), t.t("help.rename")),
                ("d / Del".into(), t.t("help.delete")),
                ("i".into(), t.t("help.images")),
                ("o".into(), t.t("help.folder")),
                ("t".into(), t.t("help.shell")),
                ("w".into(), t.t("help.window")),
                ("R / F5".into(), t.t("help.reload")),
                ("q".into(), t.t("help.quit")),
            ],
        ),
        (
            t.t("help.everywhere"),
            vec![
                (format!("{} · Tab", k(Action::ToggleFocus)), t.t("help.focus")),
                (k(Action::Images), t.t("help.tabImages")),
                (core::organization::View::ALL.map(|view| k(Action::SessionView(view))).join(" "), t.t("organize.views")),
                (format!("{} / {}", k(Action::PrevTab), k(Action::NextTab)), t.t("help.tabs")),
                (k(Action::CloseTab), t.t("help.closeTab")),
                (format!("{} / {}", k(Action::ScrollUp), k(Action::ScrollDown)), t.t("help.scroll")),
            ]
            .into_iter()
            .chain(tab_rows)
            .collect(),
        ),
    ];
    let mut lines = Vec::new();
    for (title, rows) in sections {
        lines.push(Line::styled(format!(" {}", title.to_uppercase()), Style::default().fg(MUTED).add_modifier(Modifier::BOLD)));
        for (keys, desc) in rows {
            lines.push(Line::from(vec![Span::styled(format!("  {keys:<31}"), Style::default().fg(ACCENT)), Span::raw(desc)]));
        }
        lines.push(Line::raw(""));
    }
    lines.push(hint(format!(" {}", t.t("help.mouse"))));
    lines.push(hint(format!(" {}", t.t("help.config"))));
    let area = popup_area(f.area(), 90, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(Paragraph::new(lines).block(popup_block(t.t("help.title"))), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use ratatui::{Terminal, backend::TestBackend};

    fn dump(term: &Terminal<TestBackend>) -> String {
        let buf = term.backend().buffer();
        (0..buf.area.height)
            .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The mouse wheel over a terminal must scroll its history, not send ↑↓ to the program
    /// (agents use ↑↓ for their prompt history). A click on a tab switches to it.
    #[test]
    fn wheel_scrolls_the_history_and_clicks_switch_tabs() {
        use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = App::new(Config::default(), I18n::new("en"), tx.clone());
        let mut term = Terminal::new(TestBackend::new(100, 20)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap(); // sets the terminal size of new tabs
        let launch = if cfg!(windows) {
            super::super::term::Launch {
                program: "cmd.exe".into(),
                args: vec!["/d".into(), "/c".into(), "for /L %i in (1,1,60) do @echo line %i".into()],
                cwd: None,
            }
        } else {
            super::super::term::Launch {
                program: "sh".into(),
                args: vec!["-c".into(), "for i in $(seq 1 60); do echo line $i; done".into()],
                cwd: None,
            }
        };
        for title in ["one", "two"] {
            app.spawn_tab(title.into(), None, title.into(), launch.clone());
        }
        app.active = 1;
        app.focus_list = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline && !app.tabs[0].term.as_ref().unwrap().row_text(16).contains("line") {
            let _ = rx.recv_timeout(std::time::Duration::from_millis(200));
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        term.draw(|f| draw(f, &mut app)).unwrap();

        let area = app.layout.terminal;
        let at = |kind, column, row| MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE };
        app.on_mouse(at(MouseEventKind::ScrollUp, area.x + 5, area.y + 5));
        assert!(app.tabs[0].term.as_ref().unwrap().scroll > 0, "the wheel should scroll the history");

        let (start, _, n) = app.layout.tab_spans[1];
        app.on_mouse(at(MouseEventKind::Down(MouseButton::Left), start + 1, app.layout.tabs.y));
        assert_eq!((app.active, n), (2, 2), "clicking the second tab should switch to it");
    }

    #[test]
    fn missing_claude_shows_install_guide_and_keeps_tab_on_retry() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let (tx, _) = std::sync::mpsc::channel();
        let mut app = App::new(Config::default(), I18n::new("es"), tx);
        let mut terminal = Terminal::new(TestBackend::new(120, 20)).unwrap();
        let program = "resume-ai-test-missing-claude".to_string();
        app.spawn_tab(
            "Claude".into(),
            Some("claude".into()),
            "test".into(),
            super::super::term::Launch { program: program.clone(), args: vec!["--resume".into(), "test".into()], cwd: None },
        );
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let output = dump(&terminal);
        assert!(output.contains("Puede que Claude Code"));
        assert!(output.contains("https://code.claude.com/docs/en/quickstart"));

        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.tabs[0].launch.program, program);
        assert_eq!(app.tabs[0].launch.args, ["--resume", "test"]);
        assert!(app.tabs[0].error.as_ref().unwrap().is::<CommandNotFound>());
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.tabs.is_empty());
    }

    /// Prints the UI with this machine's real sessions: `cargo test -p resume-ai preview -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn preview() {
        let (tx, rx) = std::sync::mpsc::channel();
        let lang = std::env::var("PREVIEW_LANG").unwrap_or_else(|_| "es".into());
        let mut app = App::new(Config::default(), I18n::new(&lang), tx);
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        println!("===== {lang} · list =====\n{}", dump(&term));
        app.mode = Mode::Filter { cursor: 1 };
        term.draw(|f| draw(f, &mut app)).unwrap();
        println!("===== filters =====\n{}", dump(&term));
        app.mode = Mode::Help;
        term.draw(|f| draw(f, &mut app)).unwrap();
        println!("===== help =====\n{}", dump(&term));

        // A tab running a real program
        app.mode = Mode::Normal;
        let launch = if cfg!(windows) {
            super::super::term::Launch {
                program: "cmd.exe".into(),
                args: vec!["/d".into(), "/k".into(), "echo agent output here".into()],
                cwd: None,
            }
        } else {
            super::super::term::Launch {
                program: "sh".into(),
                args: vec!["-c".into(), "echo agent output here; sleep 5".into()],
                cwd: None,
            }
        };
        app.spawn_tab("demo".into(), Some("claude".into()), "demo".into(), launch);
        app.active = 1;
        app.focus_list = false;
        let _ = rx.recv_timeout(std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(800));
        term.draw(|f| draw(f, &mut app)).unwrap();
        println!("===== tab =====\n{}", dump(&term).lines().take(8).collect::<Vec<_>>().join("\n"));
        println!("…\n{}", dump(&term).lines().last().unwrap_or(""));
    }
}
