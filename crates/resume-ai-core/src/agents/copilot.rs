//! GitHub Copilot CLI — `~/.copilot/session-state/<uuid>/{workspace.yaml, events.jsonl}`

use super::{Provider, env_path};
use crate::model::Session;
use crate::util::{self, clean_prompt, content_text};
use anyhow::Result;
use rayon::prelude::*;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static YAML_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([A-Za-z_][\w-]*):\s*(.*)$").unwrap());

pub struct Copilot;

/// Flat `key: value` YAML (enough for workspace.yaml).
fn parse_yaml(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let m = YAML_LINE.captures(line)?;
            let v = m[2].trim_start_matches(['\'', '"']).trim_end_matches(['\'', '"']).trim();
            Some((m[1].to_string(), v.to_string()))
        })
        .collect()
}

fn parse(dir: &Path) -> Option<Session> {
    let ws = dir.join("workspace.yaml");
    let ev = dir.join("events.jsonl");
    let mut mtime = 0.0;
    let y = match fs::read_to_string(&ws) {
        Ok(t) => {
            mtime = fs::metadata(&ws).map(|m| util::mtime_ms(&m)).unwrap_or(0.0);
            parse_yaml(&t)
        }
        Err(_) => HashMap::new(),
    };
    let get = |k: &str| y.get(k).filter(|v| !v.is_empty()).cloned();
    let mut prompt = String::new();
    if let Ok(edges) = util::read_jsonl_edges(&ev, 256 * 1024, 0) {
        mtime = f64::max(mtime, util::mtime_ms(&edges.meta));
        for o in &edges.lines {
            if !o["type"].as_str().unwrap_or("").to_lowercase().contains("user") {
                continue;
            }
            let d = if o["data"].is_object() {
                &o["data"]
            } else if o["payload"].is_object() {
                &o["payload"]
            } else {
                o
            };
            let raw: &Value =
                [&d["content"], &d["text"], &d["message"]].into_iter().find(|v| util::truthy(v)).unwrap_or(&Value::Null);
            prompt = clean_prompt(&raw.as_str().map(str::to_string).unwrap_or_else(|| content_text(raw)));
            if !prompt.is_empty() {
                break;
            }
        }
    }
    let title = get("name").or_else(|| get("title")).or_else(|| get("summary")).or((!prompt.is_empty()).then_some(prompt))?;
    let id = get("id").unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let updated = get("updated_at").or_else(|| get("updatedAt")).and_then(|d| util::parse_date_ms(&d)).unwrap_or(0.0);
    let mut s = Session::new("copilot", id);
    s.title = title;
    s.cwd = get("cwd").or_else(|| get("git_root")).or_else(|| get("gitRoot"));
    s.git_branch = get("branch").or_else(|| get("git_branch"));
    s.mtime = f64::max(mtime, updated);
    s.src = Some(dir.to_path_buf());
    Some(s)
}

impl Provider for Copilot {
    fn id(&self) -> &'static str {
        "copilot"
    }
    fn name(&self) -> &'static str {
        "GitHub Copilot CLI"
    }
    fn badge(&self) -> &'static str {
        "GH"
    }
    fn color(&self) -> &'static str {
        "#a371f7"
    }
    fn default_command(&self) -> &'static str {
        "copilot"
    }
    fn default_data_dir(&self) -> PathBuf {
        env_path("COPILOT_HOME").unwrap_or_else(|| util::home().join(".copilot"))
    }
    fn list(&self, data_dir: &Path) -> Result<Vec<Session>> {
        let Ok(entries) = fs::read_dir(data_dir.join("session-state")) else { return Ok(Vec::new()) };
        let dirs: Vec<PathBuf> =
            entries.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).collect();
        Ok(dirs.par_iter().filter_map(|d| parse(d)).collect())
    }
    fn resume_args(&self, id: &str) -> Vec<String> {
        vec![format!("--resume={id}")]
    }
    // No compact command confirmed yet; users can set agents.copilot.compactCommand in the config.
    fn trash_paths(&self, s: &Session) -> Option<Vec<PathBuf>> {
        Some(s.src.iter().cloned().collect())
    }
}
