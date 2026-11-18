//! opencode — `~/.local/share/opencode/opencode.db` (SQLite, current versions)
//!            `~/.local/share/opencode/storage/session/<project>/ses_*.json` (older versions)

use super::{Provider, env_path};
use crate::model::Session;
use crate::util::{self, first_str, str_of, truthy};
use anyhow::Result;
use rayon::prelude::*;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub struct Opencode;

fn from_db(file: &Path) -> Result<Vec<Session>> {
    let conn = util::open_sqlite(file)?;
    let rows = util::sqlite_rows(&conn, "SELECT * FROM session")?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            // Subagent and archived sessions are not resumable on their own
            if truthy(r.get("parent_id").unwrap_or(&Value::Null))
                || truthy(r.get("parentID").unwrap_or(&Value::Null))
                || truthy(r.get("time_archived").unwrap_or(&Value::Null))
            {
                return None;
            }
            let num = |k: &str| r.get(k).and_then(Value::as_f64).filter(|n| *n != 0.0);
            let updated = num("time_updated").or_else(|| num("time_created")).unwrap_or(0.0);
            let mut s = Session::new("opencode", str_of(r.get("id")?)?);
            s.title = r.get("title").and_then(str_of).unwrap_or_default(); // empty = untitled (translated by the UI)
            s.cwd = r.get("directory").and_then(str_of);
            s.mtime = if updated < 1e12 { updated * 1000.0 } else { updated };
            Some(s)
        })
        .collect())
}

fn from_json(data_dir: &Path) -> Vec<Session> {
    let mut files = Vec::new();
    util::walk(&data_dir.join("storage").join("session"), &|n| n.starts_with("ses_") && n.ends_with(".json"), &mut files);
    files
        .par_iter()
        .filter_map(|f| {
            let j: Value = serde_json::from_str(&fs::read_to_string(f).ok()?).ok()?;
            if truthy(&j["parentID"]) {
                return None;
            }
            let t = &j["time"];
            let time = [&t["updated"], &t["created"]].into_iter().find_map(|v| v.as_f64().filter(|n| *n != 0.0));
            let mut s = Session::new("opencode", str_of(&j["id"])?);
            s.title = first_str(&[&j["title"]]).unwrap_or_default();
            s.cwd = str_of(&j["directory"]);
            s.mtime = time.unwrap_or_else(|| fs::metadata(f).map(|m| util::mtime_ms(&m)).unwrap_or(0.0));
            Some(s)
        })
        .collect()
}

impl Provider for Opencode {
    fn id(&self) -> &'static str {
        "opencode"
    }
    fn name(&self) -> &'static str {
        "opencode"
    }
    fn badge(&self) -> &'static str {
        "OC"
    }
    fn color(&self) -> &'static str {
        "#c9a227"
    }
    fn default_command(&self) -> &'static str {
        "opencode"
    }
    fn default_data_dir(&self) -> PathBuf {
        env_path("XDG_DATA_HOME").unwrap_or_else(|| util::home().join(".local").join("share")).join("opencode")
    }
    fn list(&self, data_dir: &Path) -> Result<Vec<Session>> {
        let mut by_id: HashMap<String, Session> = HashMap::new();
        for s in from_json(data_dir) {
            by_id.insert(s.id.clone(), s);
        }
        let db = data_dir.join("opencode.db");
        if db.exists() {
            for s in from_db(&db)? {
                by_id.insert(s.id.clone(), s); // the database wins over the old JSON storage
            }
        }
        Ok(by_id.into_values().collect())
    }
    fn resume_args(&self, id: &str) -> Vec<String> {
        vec!["--session".into(), id.into()]
    }
    fn compact_command(&self) -> Option<&'static str> {
        Some("/compact")
    }
    // No trash_paths: sessions live in a shared SQLite database; deleting from outside is unsafe.
}
