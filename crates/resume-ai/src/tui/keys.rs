//! Global shortcuts (work everywhere, also inside agent tabs). Configurable in `config.toml` → `[keys]`.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use resume_ai_core::organization::View;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    SessionView(View),
    NextTab,
    PrevTab,
    /// Switch the keyboard between the sessions list and the terminal
    ToggleFocus,
    /// Go to the sessions list
    Home,
    /// Images of the session open in the active tab
    Images,
    GotoTab(u8),
    CloseTab,
    ScrollUp,
    ScrollDown,
}

/// (config name, action, default key).
fn defaults(macos: bool) -> Vec<(String, Action, &'static str)> {
    let mut v = vec![
        ("view_favorites".to_string(), Action::SessionView(View::Favorites), "Alt+F"),
        ("view_recent".to_string(), Action::SessionView(View::Recent), "Alt+R"),
        ("view_all".to_string(), Action::SessionView(View::All), "Alt+A"),
        ("view_hidden".to_string(), Action::SessionView(View::Hidden), "Alt+H"),
        ("toggle_focus".to_string(), Action::ToggleFocus, "Alt+S"),
        ("images".to_string(), Action::Images, "Alt+I"),
        ("next_tab".to_string(), Action::NextTab, "Shift+Right"),
        ("prev_tab".to_string(), Action::PrevTab, "Shift+Left"),
        ("home".to_string(), Action::Home, "Alt+0"),
        ("close_tab".to_string(), Action::CloseTab, "Ctrl+W"),
        ("scroll_up".to_string(), Action::ScrollUp, "Shift+PageUp"),
        ("scroll_down".to_string(), Action::ScrollDown, "Shift+PageDown"),
    ];
    const ALT: [&str; 9] = ["Alt+1", "Alt+2", "Alt+3", "Alt+4", "Alt+5", "Alt+6", "Alt+7", "Alt+8", "Alt+9"];
    for (i, key) in ALT.iter().enumerate() {
        v.push((format!("goto_tab_{}", i + 1), Action::GotoTab(i as u8 + 1), key));
    }
    if macos {
        for (_, action, key) in &mut v {
            *key = match action {
                Action::SessionView(View::Favorites) => "F3",
                Action::SessionView(View::Recent) => "F4",
                Action::SessionView(View::All) => "F6",
                Action::SessionView(View::Hidden) => "F7",
                Action::ToggleFocus => "F8",
                Action::Images => "F9",
                Action::Home => "F10",
                Action::GotoTab(_) => "",
                _ => *key,
            };
        }
    }
    v
}

/// Template lines for the config file.
pub fn template() -> String {
    defaults(cfg!(target_os = "macos"))
        .iter()
        .filter(|(n, ..)| !n.starts_with("goto_tab_") || n == "goto_tab_1")
        .map(|(n, _, k)| format!("# {n} = \"{k}\"\n"))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub mods: KeyModifiers,
    pub code: KeyCode,
    pub label: String,
}

/// "Ctrl+Shift+Right" → binding. Returns None for empty/invalid specs.
pub fn parse(spec: &str) -> Option<Binding> {
    let mut mods = KeyModifiers::NONE;
    let mut code = None;
    for part in spec.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_lowercase().as_str() {
            "ctrl" | "control" => mods |= KeyModifiers::CONTROL,
            "alt" | "option" | "meta" => mods |= KeyModifiers::ALT,
            "shift" => mods |= KeyModifiers::SHIFT,
            "left" => code = Some(KeyCode::Left),
            "right" => code = Some(KeyCode::Right),
            "up" => code = Some(KeyCode::Up),
            "down" => code = Some(KeyCode::Down),
            "pageup" | "pgup" => code = Some(KeyCode::PageUp),
            "pagedown" | "pgdn" => code = Some(KeyCode::PageDown),
            "home" => code = Some(KeyCode::Home),
            "end" => code = Some(KeyCode::End),
            "tab" => code = Some(KeyCode::Tab),
            "enter" => code = Some(KeyCode::Enter),
            "esc" | "escape" => code = Some(KeyCode::Esc),
            "space" => code = Some(KeyCode::Char(' ')),
            "backspace" => code = Some(KeyCode::Backspace),
            "delete" | "del" => code = Some(KeyCode::Delete),
            f if f.len() > 1 && f.starts_with('f') && f[1..].parse::<u8>().is_ok() => {
                code = Some(KeyCode::F(f[1..].parse().ok()?))
            }
            other if other.chars().count() == 1 => code = Some(KeyCode::Char(other.chars().next()?)),
            _ => return None,
        }
    }
    Some(Binding { mods, code: code?, label: spec.trim().to_string() })
}

fn same(b: &Binding, key: &KeyEvent) -> bool {
    let relevant = KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT;
    let code = match key.code {
        KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
        c => c,
    };
    // Shift+letter arrives as an uppercase char, with or without the SHIFT flag depending on the terminal
    let mut mods = key.modifiers & relevant;
    if matches!(key.code, KeyCode::Char(c) if c.is_ascii_uppercase()) {
        mods |= KeyModifiers::SHIFT;
    }
    b.code == code && b.mods == mods
}

pub struct KeyMap {
    bindings: Vec<(Action, Binding)>,
}

impl KeyMap {
    /// Defaults overridden by the `[keys]` table (an empty string disables a shortcut).
    pub fn new(user: &BTreeMap<String, String>) -> Self {
        Self::for_platform(user, cfg!(target_os = "macos"))
    }

    fn for_platform(user: &BTreeMap<String, String>, macos: bool) -> Self {
        let bindings = defaults(macos)
            .into_iter()
            .filter_map(|(name, action, default)| {
                let spec = user.get(&name).map(String::as_str).unwrap_or(default);
                parse(spec).map(|b| (action, b))
            })
            .collect();
        KeyMap { bindings }
    }

    pub fn action(&self, key: &KeyEvent) -> Option<Action> {
        self.bindings.iter().find(|(_, b)| same(b, key)).map(|(a, _)| *a)
    }

    /// Label of the key bound to an action (for hints), or "—".
    pub fn label(&self, action: Action) -> String {
        self.bound_label(action).unwrap_or("—").to_string()
    }

    pub fn bound_label(&self, action: Action) -> Option<&str> {
        self.bindings.iter().find(|(a, _)| *a == action).map(|(_, b)| b.label.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_matches() {
        let km = KeyMap::for_platform(
            &BTreeMap::from([("close_tab".to_string(), "Ctrl+Q".to_string()), ("home".to_string(), String::new())]),
            false,
        );
        let ev = |c, m| KeyEvent::new(c, m);
        assert_eq!(km.action(&ev(KeyCode::Right, KeyModifiers::SHIFT)), Some(Action::NextTab));
        assert_eq!(km.action(&ev(KeyCode::Char('3'), KeyModifiers::ALT)), Some(Action::GotoTab(3)));
        assert_eq!(km.action(&ev(KeyCode::Char('q'), KeyModifiers::CONTROL)), Some(Action::CloseTab));
        assert_eq!(km.action(&ev(KeyCode::Char('w'), KeyModifiers::CONTROL)), None); // overridden
        assert_eq!(km.action(&ev(KeyCode::Char('0'), KeyModifiers::ALT)), None); // disabled
        assert_eq!(km.action(&ev(KeyCode::Right, KeyModifiers::NONE)), None);
        assert_eq!(parse("F12").unwrap().code, KeyCode::F(12));
        assert!(parse("Ctrl+Nope").is_none());
    }

    #[test]
    fn macos_shortcuts_work_without_option() {
        let overrides = BTreeMap::from([("goto_tab_3".to_string(), "Ctrl+3".to_string())]);
        let keys = KeyMap::for_platform(&overrides, true);
        let focus = KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE);
        assert_eq!(keys.action(&focus), Some(Action::ToggleFocus));
        assert_eq!(keys.bound_label(Action::ToggleFocus), Some("F8"));
        assert_eq!(keys.bound_label(Action::GotoTab(1)), None);
        assert_eq!(keys.bound_label(Action::GotoTab(3)), Some("Ctrl+3"));
    }
}
