//! Claude Code — `~/.claude/projects/<encoded-folder>/<uuid>.jsonl`

use super::{Provider, env_path};
use crate::model::{ImageMessage, ImageRef, Session};
use crate::util::{self, clean_prompt, content_text, str_of, truthy};
use anyhow::{Context, Result};
use base64::Engine;
use rayon::prelude::*;
use regex::Regex;
use serde_json::{Value, json};
use std::fs::{self, File, FileTimes, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static UUID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$").unwrap());

pub struct Claude;

fn parse(file: &Path) -> Option<Session> {
    let id = file.file_stem()?.to_str()?;
    if !UUID.is_match(id) {
        return None; // agent-*.jsonl are subagents
    }
    let edges = util::read_jsonl_edges(file, 256 * 1024, 64 * 1024).ok()?;
    if edges.meta.len() == 0 {
        return None;
    }
    let mut s = Session::new("claude", id);
    s.mtime = util::mtime_ms(&edges.meta);
    s.src = Some(file.to_path_buf());
    let (mut summary, mut custom_title, mut prompt) = (None, None, String::new());
    for o in &edges.lines {
        let ty = o["type"].as_str().unwrap_or("");
        if ty == "summary" && summary.is_none() {
            summary = str_of(&o["summary"]);
        }
        if ty == "custom-title" {
            if let Some(t) = str_of(&o["customTitle"]) {
                custom_title = Some(t); // the last one wins (like /rename)
            }
        }
        if s.cwd.is_none() {
            s.cwd = str_of(&o["cwd"]);
        }
        if s.git_branch.is_none() {
            s.git_branch = str_of(&o["gitBranch"]);
        }
        if let Some(ep) = str_of(&o["entrypoint"]) {
            s.origin.get_or_insert_with(|| ep.clone());
            s.last_origin = Some(ep);
        }
        if prompt.is_empty() && ty == "user" && !truthy(&o["isMeta"]) && !truthy(&o["isSidechain"]) && truthy(&o["message"]) {
            prompt = clean_prompt(&content_text(&o["message"]["content"]));
        }
    }
    s.title = custom_title.or(summary).or((!prompt.is_empty()).then_some(prompt))?;
    Some(s)
}

/// Folder where Claude keeps a project's sessions: the path with every non-alphanumeric char replaced by `-`.
pub fn project_dir(data_dir: &Path, cwd: &str) -> PathBuf {
    let encoded: String = cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    data_dir.join("projects").join(encoded)
}

/// Id of the newest session in `dir` created after `since_ms` (new tabs / forks whose id is still unknown).
pub fn find_newest(dir: &Path, since_ms: f64, exclude: &[String]) -> Option<String> {
    let mut best: Option<(String, f64)> = None;
    for e in fs::read_dir(dir).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(".jsonl") else { continue };
        if !UUID.is_match(id) || exclude.iter().any(|x| x == id) {
            continue;
        }
        let Ok(created) = e.metadata().and_then(|m| m.created()) else { continue };
        let t = util::to_ms(created);
        if t >= since_ms - 2000.0 && best.as_ref().is_none_or(|(_, bt)| t > *bt) {
            best = Some((id.to_string(), t));
        }
    }
    best.map(|(id, _)| id)
}

fn image_ext(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "png",
    }
}

/// Images the user sent, grouped by message in chronological order.
/// Each message stores `imagePasteIds` (the N of `[Image #N]`) in the same order as its image blocks.
/// With `out_dir`, every image is written there as a file; without it they are only counted.
pub fn images(s: &Session, out_dir: Option<&Path>) -> Result<Vec<ImageMessage>> {
    let src = s.src.as_ref().context("session without a file")?;
    if let Some(dir) = out_dir {
        fs::create_dir_all(dir)?;
    }
    let mut out = Vec::new();
    for line in BufReader::new(File::open(src)?).lines() {
        let line = line?;
        if !line.contains("\"image\"") {
            continue; // cheap filter: most lines have no images
        }
        let Ok(o) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(content) = o["message"]["content"].as_array() else { continue };
        if o["type"] != "user" || truthy(&o["isSidechain"]) {
            continue;
        }
        let blocks: Vec<&Value> = content.iter().filter(|x| x["type"] == "image" && x["source"]["type"] == "base64").collect();
        if blocks.is_empty() {
            continue;
        }
        let ids: Vec<Option<u64>> =
            o["imagePasteIds"].as_array().map(|a| a.iter().map(Value::as_u64).collect()).unwrap_or_default();
        let text = content
            .iter()
            .filter(|x| x["type"] == "text")
            .map(|x| clean_prompt(x["text"].as_str().unwrap_or("")))
            .find(|t| !t.is_empty())
            .unwrap_or_default();
        let uuid = o["uuid"].as_str().unwrap_or("").to_string();
        let mut msg = ImageMessage {
            timestamp: o["timestamp"].as_str().and_then(util::parse_date_ms).unwrap_or(0.0),
            text,
            origin: str_of(&o["entrypoint"]),
            images: Vec::new(),
            uuid: uuid.clone(),
        };
        for (i, b) in blocks.iter().enumerate() {
            let n = ids.get(i).copied().flatten();
            let mime = b["source"]["media_type"].as_str().unwrap_or("image/png").to_string();
            let data = b["source"]["data"].as_str().unwrap_or("");
            let mut img = ImageRef { n, size: (data.len() as u64 * 3) / 4, path: None, mime };
            if let Some(dir) = out_dir {
                let short: String = uuid.chars().take(8).collect();
                let suffix = n.map(|n| format!("-img{n}")).unwrap_or_default();
                let path = dir.join(format!("{short}-{}{suffix}.{}", i + 1, image_ext(&img.mime)));
                if !path.exists() {
                    fs::write(&path, base64::engine::general_purpose::STANDARD.decode(data)?)?;
                }
                img.path = Some(path);
            }
            msg.images.push(img);
        }
        out.push(msg);
    }
    Ok(out)
}

impl Provider for Claude {
    fn id(&self) -> &'static str {
        "claude"
    }
    fn name(&self) -> &'static str {
        "Claude Code"
    }
    fn badge(&self) -> &'static str {
        "CC"
    }
    fn color(&self) -> &'static str {
        "#d97757"
    }
    fn default_command(&self) -> &'static str {
        "claude"
    }
    fn default_data_dir(&self) -> PathBuf {
        env_path("CLAUDE_CONFIG_DIR").unwrap_or_else(|| util::home().join(".claude"))
    }

    fn list(&self, data_dir: &Path) -> Result<Vec<Session>> {
        let mut files = Vec::new();
        util::walk(&data_dir.join("projects"), &|n| n.ends_with(".jsonl"), &mut files);
        let subagents = format!("{0}subagents{0}", std::path::MAIN_SEPARATOR);
        Ok(files.par_iter().filter(|f| !f.to_string_lossy().contains(&subagents)).filter_map(|f| parse(f)).collect())
    }

    fn resume_args(&self, id: &str) -> Vec<String> {
        vec!["--resume".into(), id.into()]
    }
    fn fork_args(&self, id: &str) -> Option<Vec<String>> {
        Some(vec!["--resume".into(), id.into(), "--fork-session".into()])
    }
    fn compact_command(&self) -> Option<&'static str> {
        Some("/compact")
    }

    /// The `.jsonl` plus the `<id>/` folder with subagents and tool results.
    fn trash_paths(&self, s: &Session) -> Option<Vec<PathBuf>> {
        Some(s.src.iter().flat_map(|src| [src.clone(), src.with_extension("")]).collect())
    }

    fn native_rename(&self) -> bool {
        true
    }
    /// Same as Claude Code's `/rename`: appends a `custom-title` line. Keeps the file dates so the list order does not change.
    fn rename(&self, s: &Session, title: &str) -> Result<()> {
        let src = s.src.as_ref().context("session without a file")?;
        let meta = fs::metadata(src)?;
        let (accessed, modified) = (meta.accessed()?, meta.modified()?);
        let needs_newline = meta.len() > 0 && {
            let mut f = File::open(src)?;
            f.seek(SeekFrom::End(-1))?;
            let mut last = [0u8];
            f.read_exact(&mut last)?;
            last[0] != b'\n'
        };
        let line = serde_json::to_string(&json!({ "type": "custom-title", "customTitle": title, "sessionId": s.id }))?;
        {
            let mut f = OpenOptions::new().append(true).open(src)?;
            writeln!(f, "{}{line}", if needs_newline { "\n" } else { "" })?;
        }
        OpenOptions::new().write(true).open(src)?.set_times(FileTimes::new().set_accessed(accessed).set_modified(modified))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_appends_custom_title_and_keeps_dates() {
        let dir = std::env::temp_dir().join(format!("resume-ai-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("0b7d1aa3-1b26-439b-86c3-77afb5e1a916.jsonl");
        // No trailing newline on purpose: rename must add one before its line
        fs::write(&file, r#"{"type":"user","message":{"content":"fix the login"},"cwd":"/p","entrypoint":"cli"}"#).unwrap();
        let before = fs::metadata(&file).unwrap().modified().unwrap();

        let s = parse(&file).unwrap();
        assert_eq!((s.title.as_str(), s.origin.as_deref()), ("fix the login", Some("cli")));
        Claude.rename(&s, "Login fix ñ").unwrap();

        assert_eq!(parse(&file).unwrap().title, "Login fix ñ");
        assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), before);
        assert_eq!(fs::read_to_string(&file).unwrap().lines().count(), 2);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn encodes_project_dirs() {
        let d = project_dir(Path::new("/h/.claude"), r"D:\Proyectos\agent-sessions\agent-sessions");
        assert_eq!(d, Path::new("/h/.claude/projects/D--Proyectos-agent-sessions-agent-sessions"));
    }
}
