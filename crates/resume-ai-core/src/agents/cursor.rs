//! Cursor CLI (cursor-agent) — `~/.cursor/chats/<md5(folder)>/<id>/store.db` (EXPERIMENTAL)
//! Undocumented format: `meta(key, value)` holding JSON, sometimes hex-encoded.

use super::Provider;
use crate::model::{CwdHash, HashAlgo, Session};
use crate::util::{self, str_of, value_bytes};
use anyhow::Result;
use rayon::prelude::*;
use regex::Regex;
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static WORKSPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"Workspace Path:\s*([^\n\r<"\\]+)"#).unwrap());
static QUERY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<user_query>(.*?)</user_query>").unwrap());

pub struct Cursor;

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

/// A meta value: plain JSON or hex-encoded JSON.
fn decode_meta(v: &Value) -> Option<Map<String, Value>> {
    let text = String::from_utf8_lossy(&value_bytes(v)).into_owned();
    let candidates = [Some(text.clone()), decode_hex(&text).map(|b| String::from_utf8_lossy(&b).into_owned())];
    candidates.into_iter().flatten().find_map(|c| serde_json::from_str::<Value>(&c).ok()?.as_object().cloned())
}

fn parse(db_file: &Path) -> Option<Session> {
    let chat_dir = db_file.parent()?;
    let hash_dir = chat_dir.parent()?.file_name()?.to_string_lossy().into_owned();
    let mut mtime = util::mtime_ms(&fs::metadata(db_file).ok()?);
    if let Ok(wal) = fs::metadata(format!("{}-wal", db_file.display())) {
        mtime = mtime.max(util::mtime_ms(&wal));
    }
    let conn = util::open_sqlite(db_file).ok()?;
    let mut meta = Map::new();
    for row in util::sqlite_rows(&conn, "SELECT * FROM meta").ok()? {
        if let Some(m) = row.get("value").and_then(decode_meta) {
            meta.extend(m);
        }
    }
    let (mut cwd, mut prompt) = (None, String::new());
    if let Ok(blobs) = util::sqlite_rows(&conn, "SELECT data FROM blobs LIMIT 40") {
        for row in blobs {
            let text = String::from_utf8_lossy(&value_bytes(row.get("data").unwrap_or(&Value::Null))).into_owned();
            if cwd.is_none() {
                cwd = WORKSPACE.captures(&text).map(|m| m[1].trim().to_string());
            }
            if prompt.is_empty() {
                if let Some(q) = QUERY.captures(&text) {
                    prompt = util::collapse_ws(q[1].trim());
                }
            }
        }
    }
    let id = meta
        .get("agentId")
        .and_then(str_of)
        .unwrap_or_else(|| chat_dir.file_name().unwrap_or_default().to_string_lossy().into_owned());
    let mut s = Session::new("cursor", id);
    s.title = meta.get("name").and_then(str_of).or((!prompt.is_empty()).then_some(prompt)).unwrap_or_default(); // empty = untitled (translated by the UI)
    s.mtime = mtime;
    s.src = Some(chat_dir.to_path_buf());
    if cwd.is_none() {
        s.cwd_hash = Some(CwdHash { algo: HashAlgo::Md5, value: hash_dir });
    }
    s.cwd = cwd;
    Some(s)
}

impl Provider for Cursor {
    fn id(&self) -> &'static str {
        "cursor"
    }
    fn name(&self) -> &'static str {
        "Cursor CLI"
    }
    fn badge(&self) -> &'static str {
        "CU"
    }
    fn color(&self) -> &'static str {
        "#8a8a8a"
    }
    fn experimental(&self) -> bool {
        true
    }
    fn default_command(&self) -> &'static str {
        "cursor-agent"
    }
    fn default_data_dir(&self) -> PathBuf {
        util::home().join(".cursor")
    }
    fn extra_data_dirs(&self) -> Vec<PathBuf> {
        if cfg!(windows) { super::env_path("APPDATA").map(|a| a.join("Cursor")).into_iter().collect() } else { Vec::new() }
    }
    fn list(&self, data_dir: &Path) -> Result<Vec<Session>> {
        let mut files = Vec::new();
        for dir in std::iter::once(data_dir.to_path_buf()).chain(self.extra_data_dirs()) {
            util::walk(&dir.join("chats"), &|n| n == "store.db", &mut files);
        }
        Ok(files.par_iter().filter_map(|f| parse(f)).collect())
    }
    fn resume_args(&self, id: &str) -> Vec<String> {
        vec!["--resume".into(), id.into()]
    }
    fn trash_paths(&self, s: &Session) -> Option<Vec<PathBuf>> {
        Some(s.src.iter().cloned().collect())
    }
}
