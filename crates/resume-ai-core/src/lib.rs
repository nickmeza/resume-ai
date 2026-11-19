//! Core of Resume AI: finds the sessions of every supported AI coding agent and builds the
//! commands to resume them. Shared by the CLI and the desktop app; contains no UI and no
//! user-facing text (front-ends translate codes such as `origin`).

pub mod agents;
pub mod aliases;
pub mod i18n;
pub mod model;
pub mod organization;
pub mod util;

pub use agents::{ALL as AGENTS, Provider, by_id};
pub use model::*;

use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Static info of every supported agent.
pub fn agent_info() -> Vec<AgentInfo> {
    AGENTS.iter().map(|a| a.info()).collect()
}

/// Default per-agent configuration (users override it in their config file).
pub fn default_agent_config() -> BTreeMap<String, AgentConfig> {
    AGENTS
        .iter()
        .map(|a| (a.id().to_string(), AgentConfig { command: a.default_command().to_string(), ..Default::default() }))
        .collect()
}

pub fn data_dir_of(agent: &dyn Provider, cfg: Option<&AgentConfig>) -> PathBuf {
    cfg.map(|c| c.data_dir.trim()).filter(|d| !d.is_empty()).map(PathBuf::from).unwrap_or_else(|| agent.default_data_dir())
}

fn roots_of(agent: &dyn Provider, cfg: Option<&AgentConfig>) -> Vec<PathBuf> {
    std::iter::once(data_dir_of(agent, cfg)).chain(agent.extra_data_dirs()).collect()
}

/// Effective compact command for an agent (`None` = not supported or disabled by the user).
pub fn compact_command_of(agent: &dyn Provider, cfg: Option<&AgentConfig>) -> Option<String> {
    match cfg.and_then(|c| c.compact_command.as_deref()) {
        Some(custom) => Some(custom.trim().to_string()).filter(|c| !c.is_empty()),
        None => agent.compact_command().map(str::to_string),
    }
}

/// Fills in project folders stored as hashes (Gemini = SHA-256, Cursor = MD5) using folders
/// already known from other sessions.
fn resolve_hashed_cwds(sessions: &mut [Session]) {
    let mut sha = HashMap::new();
    let mut md5 = HashMap::new();
    for p in sessions.iter().filter_map(|s| s.cwd.clone()) {
        for v in util::path_variants(&p) {
            sha.entry(util::sha256_hex(&v)).or_insert_with(|| p.clone());
            md5.entry(util::md5_hex(&v)).or_insert_with(|| p.clone());
        }
    }
    for s in sessions.iter_mut() {
        if let Some(h) = s.cwd_hash.take() {
            if s.cwd.is_none() {
                let table = if h.algo == HashAlgo::Sha256 { &sha } else { &md5 };
                s.cwd = table.get(&h.value.to_lowercase()).cloned();
            }
        }
    }
}

/// Sessions of the enabled agents, newest first. Errors of one agent do not stop the others.
pub fn list_all(enabled: &[String], cfg: &BTreeMap<String, AgentConfig>) -> ListResult {
    let results: Vec<(String, Result<Vec<Session>>)> = enabled
        .par_iter()
        .filter_map(|id| by_id(id))
        .map(|a| (a.id().to_string(), a.list(&data_dir_of(a, cfg.get(a.id())))))
        .collect();
    let mut out = ListResult::default();
    for (agent, res) in results {
        match res {
            Ok(list) => out.sessions.extend(list),
            Err(e) => out.errors.push(ListError { agent, error: format!("{e:#}") }),
        }
    }
    resolve_hashed_cwds(&mut out.sessions);
    out.sessions.sort_by(|a, b| b.mtime.total_cmp(&a.mtime));
    out
}

/// Finds an executable like `where` / `which` (respects PATHEXT on Windows).
pub fn which(cmd: &str) -> Option<PathBuf> {
    let cmd = cmd.trim().trim_matches('"');
    if cmd.is_empty() {
        return None;
    }
    if cmd.contains(['/', '\\']) {
        let p = PathBuf::from(cmd);
        return p.is_file().then_some(p);
    }
    let exts: Vec<String> = if cfg!(windows) {
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        let mut v: Vec<String> = pathext.split(';').filter(|e| !e.is_empty()).map(|e| e.to_lowercase()).collect();
        if Path::new(cmd).extension().is_some() {
            v.insert(0, String::new());
        }
        v
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&std::env::var_os("PATH")?) {
        for ext in &exts {
            let p = dir.join(format!("{cmd}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Which agents are installed (command on PATH) and/or have sessions.
pub fn detect_agents(cfg: &BTreeMap<String, AgentConfig>) -> Vec<Detection> {
    AGENTS
        .par_iter()
        .map(|a| {
            let c = cfg.get(a.id());
            let data_dir = data_dir_of(*a, c);
            let data_exists = roots_of(*a, c).iter().any(|d| d.exists());
            let command = c.map(|c| c.command.trim()).filter(|s| !s.is_empty()).unwrap_or(a.default_command());
            let (count, error) = if data_exists {
                match a.list(&data_dir) {
                    Ok(l) => (l.len(), None),
                    Err(e) => (0, Some(format!("{e:#}"))),
                }
            } else {
                (0, None)
            };
            Detection { id: a.id(), data_dir, data_exists, command_path: which(command), count, error }
        })
        .collect()
}

/// Splits user-provided extra arguments, honouring double quotes.
fn split_args(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut quoted, mut any) = (Vec::new(), String::new(), false, false);
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        out.push(cur);
    }
    out
}

/// Command that starts or resumes a session.
pub fn build_command(
    agent_id: &str,
    cfg: Option<&AgentConfig>,
    mode: LaunchMode,
    session_id: Option<&str>,
) -> Result<CommandLine> {
    let a = by_id(agent_id).with_context(|| format!("unknown agent: {agent_id}"))?;
    let program = cfg.map(|c| c.command.trim().trim_matches('"')).filter(|s| !s.is_empty()).unwrap_or(a.default_command());
    let mut args = match mode {
        LaunchMode::New => a.new_args(),
        LaunchMode::Resume | LaunchMode::Compact => {
            let id = session_id.context("missing session id")?;
            if !util::is_safe_id(id) {
                bail!("invalid session id: {id}");
            }
            if mode == LaunchMode::Compact { a.fork_args(id).unwrap_or_else(|| a.resume_args(id)) } else { a.resume_args(id) }
        }
    };
    args.extend(split_args(cfg.map(|c| c.extra_args.as_str()).unwrap_or("")));
    Ok(CommandLine { program: program.to_string(), args })
}

/// Paths to move to the trash to delete a session, or `None` if the agent does not allow it.
/// Never returns anything outside the agent's own data folders.
pub fn session_paths(s: &Session, cfg: Option<&AgentConfig>) -> Result<Option<Vec<PathBuf>>> {
    let a = by_id(&s.agent).with_context(|| format!("unknown agent: {}", s.agent))?;
    let Some(paths) = a.trash_paths(s) else { return Ok(None) };
    if s.src.is_none() {
        return Ok(None);
    }
    let roots = roots_of(a, cfg);
    for p in &paths {
        if !roots.iter().any(|r| util::is_inside(r, p)) {
            bail!("path outside the agent's data folder: {}", p.display());
        }
    }
    Ok(Some(paths.into_iter().filter(|p| p.exists()).collect()))
}

/// Moves a session's files to the system trash (recoverable).
pub fn trash_session(s: &Session, cfg: Option<&AgentConfig>) -> Result<Vec<PathBuf>> {
    let paths = session_paths(s, cfg)?.with_context(|| format!("{} does not allow deleting sessions", s.agent))?;
    if paths.is_empty() {
        bail!("the session files no longer exist");
    }
    trash::delete_all(&paths)?;
    Ok(paths)
}

/// Renames a session inside the agent's own files (only agents with native rename).
pub fn rename_session(s: &Session, title: &str) -> Result<()> {
    let a = by_id(&s.agent).with_context(|| format!("unknown agent: {}", s.agent))?;
    a.rename(s, title)
}

/// Claude-specific helpers (project folders, new-session lookup, images).
pub mod claude {
    pub use crate::agents::claude::{find_newest, images, project_dir};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_extra_args() {
        assert_eq!(split_args(r#"--model opus  --add-dir "C:\My Dir""#), vec!["--model", "opus", "--add-dir", r"C:\My Dir"]);
        assert!(split_args("   ").is_empty());
    }

    #[test]
    fn builds_commands() {
        let c = build_command("claude", None, LaunchMode::Compact, Some("abc-1")).unwrap();
        assert_eq!(c.to_shell_string(), "claude --resume abc-1 --fork-session");
        let c = build_command("codex", None, LaunchMode::Resume, Some("abc-1")).unwrap();
        assert_eq!(c.to_shell_string(), "codex resume abc-1");
        assert!(build_command("codex", None, LaunchMode::Resume, Some("a; rm")).is_err());
    }

    #[test]
    fn agent_capabilities() {
        let info = agent_info();
        let get = |id: &str| info.iter().find(|a| a.id == id).unwrap();
        assert!(get("claude").can_delete && get("claude").native_rename && get("claude").forks);
        assert!(!get("opencode").can_delete);
        assert_eq!(get("gemini").compact_command, Some("/compress"));
        assert_eq!(get("copilot").compact_command, None);
    }
}
