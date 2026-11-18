//! One module per agent: where it stores sessions, how to parse them and how to resume them.
//! To add an agent, implement [`Provider`] and register it in [`ALL`].

use crate::model::{AgentInfo, Session};
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

pub mod claude;
pub mod codex;
pub mod copilot;
pub mod cursor;
pub mod gemini;
pub mod opencode;

pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    /// Two-letter tag shown next to each session.
    fn badge(&self) -> &'static str;
    fn color(&self) -> &'static str;
    fn experimental(&self) -> bool {
        false
    }
    fn default_command(&self) -> &'static str;
    fn default_data_dir(&self) -> PathBuf;
    /// Other folders that may also hold sessions.
    fn extra_data_dirs(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    /// Every session found under `data_dir`.
    fn list(&self, data_dir: &Path) -> Result<Vec<Session>>;

    fn resume_args(&self, id: &str) -> Vec<String>;
    fn new_args(&self) -> Vec<String> {
        Vec::new()
    }
    /// Arguments to resume into a copy with a new id, if the agent supports it.
    fn fork_args(&self, _id: &str) -> Option<Vec<String>> {
        None
    }
    /// Slash command that compacts the conversation, if any.
    fn compact_command(&self) -> Option<&'static str> {
        None
    }

    /// Files/folders to move to the trash to delete a session. `None` = the agent does not allow it.
    fn trash_paths(&self, _s: &Session) -> Option<Vec<PathBuf>> {
        None
    }
    /// Whether [`Provider::rename`] writes the new title into the agent's own files.
    fn native_rename(&self) -> bool {
        false
    }
    fn rename(&self, _s: &Session, _title: &str) -> Result<()> {
        bail!("{} does not support renaming sessions", self.name())
    }

    fn info(&self) -> AgentInfo {
        AgentInfo {
            id: self.id(),
            name: self.name(),
            badge: self.badge(),
            color: self.color(),
            experimental: self.experimental(),
            default_command: self.default_command(),
            default_data_dir: self.default_data_dir(),
            can_delete: self.trash_paths(&Session::default()).is_some(),
            native_rename: self.native_rename(),
            compact_command: self.compact_command(),
            forks: self.fork_args("x").is_some(),
        }
    }
}

pub static ALL: &[&dyn Provider] =
    &[&claude::Claude, &codex::Codex, &gemini::Gemini, &opencode::Opencode, &copilot::Copilot, &cursor::Cursor];

pub fn by_id(id: &str) -> Option<&'static dyn Provider> {
    ALL.iter().copied().find(|p| p.id() == id)
}

/// Environment variable as a path, if set and not empty.
pub(crate) fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}
