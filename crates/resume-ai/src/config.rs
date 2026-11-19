//! CLI configuration file (`config.toml`).
//!
//! Location: `$XDG_CONFIG_HOME/resume-ai/config.toml`, `~/.config/resume-ai/config.toml` on
//! macOS/Linux, `%APPDATA%\resume-ai\config.toml` on Windows — or the file named in `RESUME_AI_CONFIG`.

use anyhow::{Context, Result};
use resume_ai_core::{AgentConfig, default_agent_config};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Written by `resume-ai config init`. Comments stay in English (like the rest of the project).
pub const TEMPLATE: &str = r#"# resume-ai configuration

# Interface language: "auto" (follow the system), "en" or "es"
language = "auto"

# Width of the sessions list in the interactive mode, in % of the screen (10–60)
sidebar_percent = 25

# Agents to show (empty = all). Ids: claude, codex, gemini, opencode, copilot, cursor
# enabled_agents = ["claude", "codex"]

# Per-agent overrides (remove the leading # to use them)
# [agents.claude]
# command = "claude"             # executable name or full path
# extra_args = "--model opus"    # appended to every launch
# data_dir = ""                  # where the agent stores sessions (empty = default)
# compact_command = "/compact"   # "" disables "continue compacted"

# Global shortcuts of the interactive mode (work inside agent tabs too). "" disables one.
# Use goto_tab_1 through goto_tab_9 to customize direct tab navigation.
# [keys]
"#;

/// [`TEMPLATE`] plus the default shortcuts, commented out.
pub fn template() -> String {
    format!("{TEMPLATE}{}", crate::tui::keys::template())
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    pub language: String,
    /// Width of the sessions list, in % of the screen
    pub sidebar_percent: u16,
    pub enabled_agents: Vec<String>,
    pub agents: BTreeMap<String, AgentConfig>,
    /// Shortcut overrides, see `tui::keys`.
    pub keys: BTreeMap<String, String>,
}

pub const DEFAULT_SIDEBAR_PERCENT: u16 = 25;

impl Default for Config {
    fn default() -> Self {
        Config {
            language: "auto".into(),
            sidebar_percent: DEFAULT_SIDEBAR_PERCENT,
            enabled_agents: Vec::new(),
            agents: BTreeMap::new(),
            keys: BTreeMap::new(),
        }
    }
}

impl Config {
    /// Agents to list: the configured ones, or every supported agent.
    pub fn enabled(&self) -> Vec<String> {
        if self.enabled_agents.is_empty() {
            resume_ai_core::AGENTS.iter().map(|a| a.id().to_string()).collect()
        } else {
            self.enabled_agents.clone()
        }
    }

    /// Defaults merged with the user's per-agent overrides (empty fields keep the default).
    pub fn agent_config(&self) -> BTreeMap<String, AgentConfig> {
        let mut all = default_agent_config();
        for (id, user) in &self.agents {
            let entry = all.entry(id.clone()).or_default();
            if !user.command.trim().is_empty() {
                entry.command = user.command.clone();
            }
            entry.extra_args = user.extra_args.clone();
            entry.data_dir = user.data_dir.clone();
            entry.compact_command = user.compact_command.clone();
        }
        all
    }
}

/// The `RESUME_AI_CONFIG` environment variable points to another file (handy for tests and dotfiles).
pub fn path() -> PathBuf {
    if let Some(p) = std::env::var_os("RESUME_AI_CONFIG").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    let base = if cfg!(windows) {
        dirs::config_dir()
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| dirs::home_dir().map(|h| h.join(".config")))
    };
    base.unwrap_or_else(|| PathBuf::from(".")).join("resume-ai").join("config.toml")
}

/// The config file, or the defaults when it does not exist yet.
pub fn load() -> Result<Config> {
    let p = path();
    match std::fs::read_to_string(&p) {
        Ok(text) => toml::from_str(&text).with_context(|| p.display().to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(e).with_context(|| p.display().to_string()),
    }
}

/// Creates the file from [`TEMPLATE`]. Returns false if it already existed.
pub fn init() -> Result<bool> {
    let p = path();
    if p.exists() {
        return Ok(false);
    }
    std::fs::create_dir_all(p.parent().context("config path without parent")?)?;
    std::fs::write(&p, template())?;
    Ok(true)
}

/// Sets `key = value` among the top-level settings of a TOML text, keeping comments and everything else.
fn set_top_level(text: &str, key: &str, value: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let end = lines.iter().position(|l| l.trim_start().starts_with('[')).unwrap_or(lines.len());
    let line = format!("{key} = {value}");
    match lines[..end].iter().position(|l| l.split('=').next().is_some_and(|k| k.trim() == key)) {
        Some(i) => lines[i] = line,
        None => lines.insert(end, line),
    }
    lines.join("\n") + "\n"
}

/// Saves the answers of the setup (language and list width), creating the file if needed.
pub fn save_basics(language: &str, sidebar_percent: u16) -> Result<()> {
    let p = path();
    let text = std::fs::read_to_string(&p).unwrap_or_else(|_| template());
    let text = set_top_level(&text, "language", &format!("\"{language}\""));
    let text = set_top_level(&text, "sidebar_percent", &sidebar_percent.to_string());
    std::fs::create_dir_all(p.parent().context("config path without parent")?)?;
    std::fs::write(&p, text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_answers_replace_the_values_and_keep_the_rest() {
        let text = set_top_level(&template(), "language", "\"es\"");
        let text = set_top_level(&text, "sidebar_percent", "30");
        let c: Config = toml::from_str(&text).unwrap();
        assert_eq!((c.language.as_str(), c.sidebar_percent), ("es", 30));
        assert!(text.contains("# Per-agent overrides")); // comments kept
        // A file without the keys gets them before the first [section]
        let text = set_top_level("[agents.claude]\ncommand = \"claude\"\n", "sidebar_percent", "40");
        assert!(text.starts_with("sidebar_percent = 40\n[agents.claude]"));
    }

    #[test]
    fn template_parses_to_defaults() {
        let c: Config = toml::from_str(&template()).unwrap();
        assert_eq!(c.language, "auto");
        assert!(c.enabled_agents.is_empty() && c.agents.is_empty() && c.keys.is_empty());
        // Uncommenting the [keys] section must give a valid file
        let uncommented = template().replace("# [keys]", "[keys]").replace("# next_tab", "next_tab");
        let c: Config = toml::from_str(&uncommented).unwrap();
        assert_eq!(c.keys["next_tab"], "Shift+Right");
    }

    #[test]
    fn overrides_merge_with_defaults() {
        let c: Config = toml::from_str("language = \"es\"\n[agents.claude]\nextra_args = \"--model opus\"\n").unwrap();
        let cfg = c.agent_config();
        assert_eq!(cfg["claude"].command, "claude"); // kept the default
        assert_eq!(cfg["claude"].extra_args, "--model opus");
        assert_eq!(cfg["codex"].command, "codex");
    }
}
