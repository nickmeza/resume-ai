//! OpenAI Codex CLI — `~/.codex/sessions/YYYY/MM/DD/rollout-<date>-<uuid>.jsonl`

use super::{Provider, env_path};
use crate::model::Session;
use crate::util::{self, clean_prompt, content_text, first_str, str_of};
use anyhow::Result;
use rayon::prelude::*;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static UUID_TAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\.jsonl$").unwrap());

pub struct Codex;

fn parse(file: &Path) -> Option<Session> {
    let edges = util::read_jsonl_edges(file, 512 * 1024, 64 * 1024).ok()?;
    if edges.meta.len() == 0 {
        return None;
    }
    let mut id = UUID_TAIL.captures(&file.to_string_lossy()).map(|m| m[1].to_string());
    let (mut cwd, mut branch, mut custom_title) = (None, None, None);
    let (mut prompt, mut fallback) = (String::new(), String::new());
    for o in &edges.lines {
        let p = &o["payload"];
        let ty = o["type"].as_str().unwrap_or("");
        let pty = p["type"].as_str().unwrap_or("");
        if ty == "session_meta" {
            id = first_str(&[&p["id"], &p["session_id"]]).or(id);
            cwd = cwd.or_else(|| str_of(&p["cwd"]));
            branch = branch.or_else(|| str_of(&p["git"]["branch"]));
        }
        if cwd.is_none() && ty == "turn_context" {
            cwd = str_of(&p["cwd"]);
        }
        if ty == "event_msg" && pty == "thread_name_updated" {
            if let Some(name) = first_str(&[&p["name"], &p["thread_name"]]) {
                custom_title = Some(name);
            }
        }
        if prompt.is_empty() && ty == "event_msg" && pty == "user_message" {
            prompt = clean_prompt(p["message"].as_str().unwrap_or(""));
        }
        if fallback.is_empty() && ty == "response_item" && pty == "message" && p["role"] == "user" {
            fallback = clean_prompt(&content_text(&p["content"]));
        }
    }
    let title = custom_title.or((!prompt.is_empty()).then_some(prompt)).or((!fallback.is_empty()).then_some(fallback))?;
    let mut s = Session::new("codex", id?);
    s.title = title;
    s.cwd = cwd;
    s.git_branch = branch;
    s.mtime = util::mtime_ms(&edges.meta);
    s.src = Some(file.to_path_buf());
    Some(s)
}

impl Provider for Codex {
    fn id(&self) -> &'static str {
        "codex"
    }
    fn name(&self) -> &'static str {
        "Codex CLI"
    }
    fn badge(&self) -> &'static str {
        "CX"
    }
    fn color(&self) -> &'static str {
        "#10a37f"
    }
    fn default_command(&self) -> &'static str {
        "codex"
    }
    fn default_data_dir(&self) -> PathBuf {
        env_path("CODEX_HOME").unwrap_or_else(|| util::home().join(".codex"))
    }
    fn list(&self, data_dir: &Path) -> Result<Vec<Session>> {
        let mut files = Vec::new();
        util::walk(&data_dir.join("sessions"), &|n| n.starts_with("rollout-") && n.ends_with(".jsonl"), &mut files);
        Ok(files.par_iter().filter_map(|f| parse(f)).collect())
    }
    fn resume_args(&self, id: &str) -> Vec<String> {
        vec!["resume".into(), id.into()]
    }
    fn compact_command(&self) -> Option<&'static str> {
        Some("/compact")
    }
    fn trash_paths(&self, s: &Session) -> Option<Vec<PathBuf>> {
        Some(s.src.iter().cloned().collect())
    }
}
