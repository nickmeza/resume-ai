//! Names given to sessions inside resume-ai, for agents that cannot be renamed natively.
//! Shared by the CLI and the desktop app: `<data dir>/resume-ai/aliases.json` (`{ "<key>": "title" }`).

use crate::model::Session;
use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

pub fn path() -> PathBuf {
    dirs::data_dir().unwrap_or_else(crate::util::home).join("resume-ai").join("aliases.json")
}

pub fn load() -> BTreeMap<String, String> {
    fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save(map: &BTreeMap<String, String>) -> Result<()> {
    let p = path();
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(p, serde_json::to_string_pretty(map)?)?;
    Ok(())
}

/// Sets (`Some`) or removes (`None`) the alias of a session key.
pub fn set(key: &str, title: Option<&str>) -> Result<BTreeMap<String, String>> {
    let mut map = load();
    match title {
        Some(t) => map.insert(key.to_string(), t.to_string()),
        None => map.remove(key),
    };
    save(&map)?;
    Ok(map)
}

/// Renames natively when the agent supports it (Claude Code), otherwise stores an alias.
/// An empty title removes the alias. Returns `true` when the agent's own files were changed.
pub fn rename(s: &Session, title: &str) -> Result<bool> {
    let title = crate::util::collapse_ws(title.trim());
    let title: String = title.chars().take(200).collect();
    if title.is_empty() {
        set(&s.key, None)?;
        return Ok(false);
    }
    let native = crate::by_id(&s.agent).is_some_and(|a| a.native_rename()) && crate::rename_session(s, &title).is_ok();
    if native {
        set(&s.key, None)?;
    } else {
        set(&s.key, Some(&title))?;
    }
    Ok(native)
}

/// Title to show: alias, then the session's own title (may still be empty = untitled).
pub fn display_title<'a>(s: &'a Session, aliases: &'a BTreeMap<String, String>) -> &'a str {
    aliases.get(&s.key).map(String::as_str).unwrap_or(&s.title)
}
