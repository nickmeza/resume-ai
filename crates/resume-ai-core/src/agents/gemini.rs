//! Gemini CLI — `~/.gemini/tmp/<hash|name>/chats/session-*.json`
//! The project folder comes from `.project_root`, from `~/.gemini/projects.json`, or by matching
//! the SHA-256 of known folders (resolved in `lib.rs`).

use super::{Provider, env_path};
use crate::model::{CwdHash, HashAlgo, Session};
use crate::util::{self, clean_prompt, content_text, first_str, str_of};
use anyhow::Result;
use rayon::prelude::*;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub struct Gemini;

/// `projects.json` maps folder → slot name; we need slot name → folder.
fn read_projects_json(data_dir: &Path) -> HashMap<String, String> {
    let Ok(text) = fs::read_to_string(data_dir.join("projects.json")) else { return HashMap::new() };
    let Ok(j) = serde_json::from_str::<Value>(&text) else { return HashMap::new() };
    j["projects"]
        .as_object()
        .map(|m| m.iter().filter_map(|(path, name)| Some((name.as_str()?.to_string(), path.clone()))).collect())
        .unwrap_or_default()
}

fn parse(file: &Path, slot_cwd: Option<&str>) -> Option<Session> {
    let meta = fs::metadata(file).ok()?;
    let j: Value = serde_json::from_str(&fs::read_to_string(file).ok()?).ok()?;
    let prompt = j["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["type"] == "user")
        .map(|m| clean_prompt(&content_text(&m["content"])))
        .find(|p| !p.is_empty())
        .unwrap_or_default();
    let title = first_str(&[&j["summary"], &j["title"]]).or((!prompt.is_empty()).then_some(prompt))?;
    let mut s = Session::new("gemini", str_of(&j["sessionId"])?);
    s.title = title;
    s.mtime = j["lastUpdated"].as_str().and_then(util::parse_date_ms).unwrap_or_else(|| util::mtime_ms(&meta));
    s.cwd = first_str(&[&j["cwd"], &j["projectRoot"]]).or_else(|| slot_cwd.map(str::to_string));
    s.git_branch = str_of(&j["gitBranch"]);
    s.src = Some(file.to_path_buf());
    if s.cwd.is_none() {
        s.cwd_hash = str_of(&j["projectHash"]).map(|value| CwdHash { algo: HashAlgo::Sha256, value });
    }
    Some(s)
}

impl Provider for Gemini {
    fn id(&self) -> &'static str {
        "gemini"
    }
    fn name(&self) -> &'static str {
        "Gemini CLI"
    }
    fn badge(&self) -> &'static str {
        "GM"
    }
    fn color(&self) -> &'static str {
        "#4b8bf5"
    }
    fn default_command(&self) -> &'static str {
        "gemini"
    }
    fn default_data_dir(&self) -> PathBuf {
        env_path("GEMINI_CLI_HOME").map(|h| h.join(".gemini")).unwrap_or_else(|| util::home().join(".gemini"))
    }
    fn list(&self, data_dir: &Path) -> Result<Vec<Session>> {
        let by_name = read_projects_json(data_dir);
        let Ok(slots) = fs::read_dir(data_dir.join("tmp")) else { return Ok(Vec::new()) };
        let mut jobs: Vec<(PathBuf, Option<String>)> = Vec::new();
        for slot in slots.flatten() {
            if !slot.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let dir = slot.path();
            let name = slot.file_name().to_string_lossy().into_owned();
            let marker =
                fs::read_to_string(dir.join(".project_root")).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            let slot_cwd = marker.or_else(|| by_name.get(&name).cloned());
            let Ok(chats) = fs::read_dir(dir.join("chats")) else { continue };
            for f in chats.flatten() {
                let n = f.file_name().to_string_lossy().into_owned();
                if n.starts_with("session-") && n.ends_with(".json") {
                    jobs.push((f.path(), slot_cwd.clone()));
                }
            }
        }
        Ok(jobs.par_iter().filter_map(|(f, c)| parse(f, c.as_deref())).collect())
    }
    fn resume_args(&self, id: &str) -> Vec<String> {
        vec!["--resume".into(), id.into()]
    }
    fn compact_command(&self) -> Option<&'static str> {
        Some("/compress")
    }
    fn trash_paths(&self, s: &Session) -> Option<Vec<PathBuf>> {
        Some(s.src.iter().cloned().collect())
    }
}
