//! Helpers shared by the providers: fast jsonl reading, prompt cleaning, hashing, paths.

use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::SystemTime;

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

pub fn to_ms(t: SystemTime) -> f64 {
    t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
}

pub fn mtime_ms(meta: &fs::Metadata) -> f64 {
    meta.modified().map(to_ms).unwrap_or(0.0)
}

/// Parses ISO-8601 / RFC 3339 dates (`2025-01-31T10:00:00.000Z`) to milliseconds.
pub fn parse_date_ms(s: &str) -> Option<f64> {
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(d.timestamp_millis() as f64);
    }
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f").ok().map(|d| d.and_utc().timestamp_millis() as f64)
}

fn read_range(f: &mut File, start: u64, len: u64) -> Result<Vec<u8>> {
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity(len as usize);
    f.take(len).read_to_end(&mut buf)?;
    Ok(buf)
}

pub struct Edges {
    pub meta: fs::Metadata,
    pub lines: Vec<Value>,
}

/// JSON lines from the start (and optionally the end) of a `.jsonl` without reading it all.
/// Lines cut by the boundaries are skipped.
pub fn read_jsonl_edges(file: &Path, head: u64, tail: u64) -> Result<Edges> {
    let meta = fs::metadata(file)?;
    let size = meta.len();
    let mut f = File::open(file)?;
    let mut chunks = vec![read_range(&mut f, 0, size.min(head))?];
    if tail > 0 && size > head {
        let start = head.max(size - tail);
        chunks.push(read_range(&mut f, start, size - start)?);
    }
    let mut lines = Vec::new();
    for chunk in chunks {
        for line in String::from_utf8_lossy(&chunk).split('\n') {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                lines.push(v);
            }
        }
    }
    Ok(Edges { meta, lines })
}

/// JavaScript-style truthiness, used to mirror the agents' own loose formats.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// The value as a non-empty string.
pub fn str_of(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

/// First non-empty string among several fields (`a || b || c`).
pub fn first_str(values: &[&Value]) -> Option<String> {
    values.iter().find_map(|v| str_of(v))
}

/// Text of a message "content": a string, `[{type:'text', text}]` or `[{text}]`.
pub fn content_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(items) => {
            for c in items {
                match c {
                    Value::String(s) => return s.clone(),
                    Value::Object(_) => {
                        if let Some(text) = c["text"].as_str() {
                            let ty = c["type"].as_str().unwrap_or("");
                            if ty.is_empty() || ty.contains("text") {
                                return text.to_string();
                            }
                        }
                    }
                    _ => {}
                }
            }
            String::new()
        }
        Value::Object(_) => content["text"].as_str().unwrap_or("").to_string(),
        _ => String::new(),
    }
}

static RE_CMD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<command-name>([^<]+)</command-name>").unwrap());
static RE_ARGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<command-args>([^<]*)</command-args>").unwrap());
static RE_QUERY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<user_query>(.*?)</user_query>").unwrap());
static RE_AGENTS_MD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^# AGENTS\.md instructions").unwrap());
static RE_WS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

pub fn collapse_ws(s: &str) -> String {
    RE_WS.replace_all(s, " ").into_owned()
}

/// Turns a prompt into a title. Returns "" for internal/system messages.
pub fn clean_prompt(text: &str) -> String {
    let t = text.trim();
    if t.is_empty() {
        return String::new();
    }
    if t.starts_with('<') {
        if let (Some(cmd), Some(args)) = (RE_CMD.captures(t), RE_ARGS.captures(t)) {
            let args = args[1].trim();
            if !args.is_empty() {
                return format!("{} {}", cmd[1].trim(), args);
            }
        }
        if let Some(q) = RE_QUERY.captures(t) {
            return collapse_ws(q[1].trim());
        }
        return String::new();
    }
    if RE_AGENTS_MD.is_match(t) {
        return String::new();
    }
    collapse_ws(t)
}

/// Recursively collects files whose name passes `filter` (max depth 6, like the agents' layouts need).
pub fn walk(dir: &Path, filter: &dyn Fn(&str) -> bool, out: &mut Vec<PathBuf>) {
    fn go(dir: &Path, filter: &dyn Fn(&str) -> bool, out: &mut Vec<PathBuf>, depth: u32) {
        if depth > 6 {
            return;
        }
        let Ok(entries) = fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let path = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                go(&path, filter, out, depth + 1);
            } else if filter(&e.file_name().to_string_lossy()) {
                out.push(path);
            }
        }
    }
    go(dir, filter, out, 0);
}

pub fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    hex(&Sha256::digest(s.as_bytes()))
}

pub fn md5_hex(s: &str) -> String {
    use md5::{Digest, Md5};
    hex(&Md5::digest(s.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Spellings of a path to compare hashes (Windows is inconsistent with drive-letter case and slashes).
pub fn path_variants(p: &str) -> Vec<String> {
    let mut set: Vec<String> = Vec::new();
    let add = |v: String, set: &mut Vec<String>| {
        if !set.contains(&v) {
            set.push(v);
        }
    };
    add(p.to_string(), &mut set);
    add(p.trim_end_matches(['\\', '/']).to_string(), &mut set);
    for v in set.clone() {
        let mut chars = v.chars();
        if let (Some(d), Some(':')) = (chars.next(), chars.next()) {
            if d.is_ascii_alphabetic() {
                add(format!("{}{}", d.to_ascii_lowercase(), &v[1..]), &mut set);
                add(format!("{}{}", d.to_ascii_uppercase(), &v[1..]), &mut set);
            }
        }
    }
    for v in set.clone() {
        add(v.replace('\\', "/"), &mut set);
    }
    set
}

/// Session ids allowed in a command line (prevents argument injection).
pub fn is_safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || "_.:-".contains(c))
}

/// Whether `path` is strictly inside `root` (case-insensitive on Windows).
pub fn is_inside(root: &Path, path: &Path) -> bool {
    let norm = |p: &Path| -> Vec<String> {
        p.components()
            .map(|c| {
                let s = c.as_os_str().to_string_lossy().into_owned();
                if cfg!(windows) { s.to_lowercase() } else { s }
            })
            .collect()
    };
    let (r, p) = (norm(root), norm(path));
    p.len() > r.len() && p[..r.len()] == r[..]
}

/// Opens a SQLite database read-only (the agents may be writing to it).
pub fn open_sqlite(file: &Path) -> Result<rusqlite::Connection> {
    use rusqlite::OpenFlags;
    Ok(rusqlite::Connection::open_with_flags(file, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?)
}

/// Every row of a query as JSON objects (column name → value), blobs as byte arrays.
pub fn sqlite_rows(conn: &rusqlite::Connection, sql: &str) -> Result<Vec<serde_json::Map<String, Value>>> {
    use rusqlite::types::ValueRef;
    let mut stmt = conn.prepare(sql)?;
    let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let rows = stmt.query_map([], |row| {
        let mut obj = serde_json::Map::new();
        for (i, name) in names.iter().enumerate() {
            let v = match row.get_ref(i)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(n) => Value::from(n),
                ValueRef::Real(f) => Value::from(f),
                ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
                ValueRef::Blob(b) => Value::Array(b.iter().map(|x| Value::from(*x)).collect()),
            };
            obj.insert(name.clone(), v);
        }
        Ok(obj)
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Bytes of a value that SQLite returned as text or blob.
pub fn value_bytes(v: &Value) -> Vec<u8> {
    match v {
        Value::String(s) => s.as_bytes().to_vec(),
        Value::Array(a) => a.iter().filter_map(|x| x.as_u64().map(|n| n as u8)).collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_prompts() {
        assert_eq!(clean_prompt("  hola\n  mundo "), "hola mundo");
        assert_eq!(clean_prompt("<command-name>/review</command-name><command-args> 12 </command-args>"), "/review 12");
        assert_eq!(clean_prompt("<ide_opened_file>x</ide_opened_file>"), "");
        assert_eq!(clean_prompt("<user_query>\n fix  it </user_query>"), "fix it");
        assert_eq!(clean_prompt("# AGENTS.md instructions for x"), "");
    }

    #[test]
    fn path_variants_cover_case_and_slashes() {
        let v = path_variants(r"c:\Users\me\");
        assert!(v.contains(&r"C:\Users\me".to_string()));
        assert!(v.contains(&"c:/Users/me".to_string()));
    }

    #[test]
    fn safe_ids() {
        assert!(is_safe_id("0199aaaa-1234_ab.c:d"));
        assert!(!is_safe_id("abc; rm -rf"));
        assert!(!is_safe_id(""));
    }

    #[test]
    fn inside_checks() {
        assert!(is_inside(Path::new("/a/b"), Path::new("/a/b/c.jsonl")));
        assert!(!is_inside(Path::new("/a/b"), Path::new("/a/b")));
        assert!(!is_inside(Path::new("/a/b"), Path::new("/a/bc/x")));
    }
}
