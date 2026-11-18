//! Data types shared by every provider and by the CLI / desktop front-ends.
//! Nothing here contains display text: front-ends translate codes (e.g. `origin`) themselves.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One resumable conversation of an agent.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// Agent-specific session id (what the resume command expects).
    pub id: String,
    /// Provider id: `claude`, `codex`, `gemini`, `opencode`, `copilot`, `cursor`.
    pub agent: String,
    /// Unique key across agents: `<agent>:<id>`.
    pub key: String,
    /// May be empty: front-ends show their translated "(untitled)".
    pub title: String,
    /// Project folder the session belongs to (the agent must be started there to find it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    /// Last activity, milliseconds since the Unix epoch.
    pub mtime: f64,
    /// File or folder holding the session (used for rename/delete/images).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src: Option<PathBuf>,
    /// Where the session was created (Claude only): `cli`, `claude-vscode`, `claude-desktop`…
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// Where the session was used last (Claude only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_origin: Option<String>,
    /// Project folder stored as a hash (Gemini/Cursor); resolved against known folders after listing.
    #[serde(skip)]
    pub cwd_hash: Option<CwdHash>,
}

impl Session {
    pub fn new(agent: &str, id: impl Into<String>) -> Self {
        let id = id.into();
        Session { key: format!("{agent}:{id}"), agent: agent.to_string(), id, ..Default::default() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgo {
    Sha256,
    Md5,
}

#[derive(Debug, Clone)]
pub struct CwdHash {
    pub algo: HashAlgo,
    pub value: String,
}

/// Static description of a provider.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub badge: &'static str,
    pub color: &'static str,
    pub experimental: bool,
    pub default_command: &'static str,
    pub default_data_dir: PathBuf,
    pub can_delete: bool,
    pub native_rename: bool,
    /// Slash command that compacts the conversation (`/compact`, `/compress`), if any.
    pub compact_command: Option<&'static str>,
    /// Whether the agent can resume into a copy (`--fork-session`).
    pub forks: bool,
}

/// User overrides for one agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct AgentConfig {
    /// Executable name or full path (empty = default command).
    pub command: String,
    /// Extra arguments appended to every launch.
    #[serde(alias = "extra_args")]
    pub extra_args: String,
    /// Where the agent stores its sessions (empty = default location).
    #[serde(alias = "data_dir")]
    pub data_dir: String,
    /// Overrides the compact slash command (`Some("")` disables it).
    #[serde(alias = "compact_command", skip_serializing_if = "Option::is_none")]
    pub compact_command: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListError {
    pub agent: String,
    pub error: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ListResult {
    pub sessions: Vec<Session>,
    pub errors: Vec<ListError>,
}

/// Result of probing whether an agent is installed / has data.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detection {
    pub id: &'static str,
    pub data_dir: PathBuf,
    pub data_exists: bool,
    pub command_path: Option<PathBuf>,
    pub count: usize,
    pub error: Option<String>,
}

/// How to start an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LaunchMode {
    New,
    Resume,
    /// Resume into a copy when the agent supports it (the front-end then sends the compact command).
    Compact,
}

/// A program plus its arguments, ready to spawn (no shell involved).
#[derive(Debug, Clone, Serialize)]
pub struct CommandLine {
    pub program: String,
    pub args: Vec<String>,
}

impl CommandLine {
    /// Single-line form for shells and for showing to the user.
    pub fn to_shell_string(&self) -> String {
        std::iter::once(&self.program).chain(&self.args).map(|a| quote_arg(a)).collect::<Vec<_>>().join(" ")
    }
}

fn quote_arg(a: &str) -> String {
    if a.is_empty() || a.contains(char::is_whitespace) { format!("\"{}\"", a.replace('"', "\\\"")) } else { a.to_string() }
}

/// One user message of a Claude session that carried images.
#[derive(Debug, Clone, Serialize)]
pub struct ImageMessage {
    pub uuid: String,
    /// Milliseconds since the Unix epoch.
    pub timestamp: f64,
    /// First meaningful text of the message.
    pub text: String,
    /// `entrypoint` code of the message (`cli`, `claude-vscode`…).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    pub images: Vec<ImageRef>,
}

/// One image inside a message. `n` is the number shown as `[Image #N]`.
#[derive(Debug, Clone, Serialize)]
pub struct ImageRef {
    pub n: Option<u64>,
    pub mime: String,
    pub size: u64,
    /// Extracted file (only when extraction was requested).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}
