//! Terminals of the tabs: one process per key in a pseudo-terminal; output goes to xterm.js as events.

use anyhow::{Context, Result};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

struct Pty {
    generation: u64,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    exited: Arc<(Mutex<bool>, Condvar)>,
}

#[derive(Default)]
pub struct Ptys {
    map: Mutex<HashMap<String, Pty>>,
    next: AtomicU64,
}

#[derive(Clone, Serialize)]
struct PtyData<'a> {
    key: &'a str,
    data: String,
}

#[derive(Clone, Serialize)]
struct PtyExit<'a> {
    key: &'a str,
    code: u32,
}

/// UTF-8 decoding that keeps a character split between two reads for the next one.
fn decode(pending: &mut Vec<u8>, chunk: &[u8]) -> String {
    pending.extend_from_slice(chunk);
    match std::str::from_utf8(pending) {
        Ok(s) => {
            let s = s.to_string();
            pending.clear();
            s
        }
        Err(e) if e.error_len().is_none() => {
            let valid = e.valid_up_to();
            let s = String::from_utf8_lossy(&pending[..valid]).into_owned();
            pending.drain(..valid);
            s
        }
        Err(_) => {
            let s = String::from_utf8_lossy(pending).into_owned();
            pending.clear();
            s
        }
    }
}

pub struct Spawn<'a> {
    pub key: &'a str,
    pub program: &'a str,
    pub args: &'a [String],
    pub cwd: &'a Path,
    pub env: &'a HashMap<String, String>,
    pub cols: u16,
    pub rows: u16,
}

impl Ptys {
    pub fn has(&self, key: &str) -> bool {
        self.map.lock().unwrap().contains_key(key)
    }

    fn is_current(app: &AppHandle, key: &str, generation: u64) -> bool {
        let state = app.state::<crate::AppState>();
        state.ptys.map.lock().unwrap().get(key).is_some_and(|p| p.generation == generation)
    }

    pub fn open(&self, app: &AppHandle, s: Spawn) -> Result<()> {
        let size = PtySize { rows: s.rows.max(2), cols: s.cols.max(10), pixel_width: 0, pixel_height: 0 };
        let pair = native_pty_system().openpty(size)?;
        let mut cmd = CommandBuilder::new(s.program);
        cmd.args(s.args);
        cmd.cwd(s.cwd);
        cmd.env_clear();
        for (k, v) in s.env {
            cmd.env(k, v);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("FORCE_COLOR", "1");
        let mut child = pair.slave.spawn_command(cmd).with_context(|| format!("could not start {}", s.program))?;
        drop(pair.slave);

        let generation = self.next.fetch_add(1, Ordering::Relaxed);
        let exited = Arc::new((Mutex::new(false), Condvar::new()));
        let key = s.key.to_string();

        // Output → "pty-data" events (only while this process is still the tab's current one)
        let mut reader = pair.master.try_clone_reader()?;
        {
            let (app, key) = (app.clone(), key.clone());
            std::thread::spawn(move || {
                let mut buf = [0u8; 16 * 1024];
                let mut pending = Vec::new();
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let data = decode(&mut pending, &buf[..n]);
                    if !data.is_empty() && Self::is_current(&app, &key, generation) {
                        let _ = app.emit("pty-data", PtyData { key: &key, data });
                    }
                }
            });
        }
        let killer = child.clone_killer();
        // Exit → "pty-exit" event
        {
            let (app, key, exited) = (app.clone(), key.clone(), exited.clone());
            std::thread::spawn(move || {
                let code = child.wait().map(|s| s.exit_code()).unwrap_or(1);
                let (lock, cv) = &*exited;
                *lock.lock().unwrap() = true;
                cv.notify_all();
                let state = app.state::<crate::AppState>();
                let mut map = state.ptys.map.lock().unwrap();
                if map.get(&key).is_some_and(|p| p.generation == generation) {
                    map.remove(&key);
                    drop(map);
                    let _ = app.emit("pty-exit", PtyExit { key: &key, code });
                }
            });
        }
        let writer = pair.master.take_writer()?;
        self.map.lock().unwrap().insert(key, Pty { generation, writer, master: pair.master, killer, exited });
        Ok(())
    }

    pub fn write(&self, key: &str, data: &str) {
        if let Some(p) = self.map.lock().unwrap().get_mut(key) {
            let _ = p.writer.write_all(data.as_bytes());
            let _ = p.writer.flush();
        }
    }

    pub fn resize(&self, key: &str, cols: u16, rows: u16) {
        if cols == 0 || rows == 0 {
            return;
        }
        if let Some(p) = self.map.lock().unwrap().get(key) {
            let _ = p.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        }
    }

    /// Ends the process and waits (up to 1.5 s) so the agent releases its session files.
    pub fn kill(&self, key: &str) {
        let Some(mut p) = self.map.lock().unwrap().remove(key) else { return };
        let _ = p.killer.kill();
        let (lock, cv) = &*p.exited;
        let guard = lock.lock().unwrap();
        let _ = cv.wait_timeout_while(guard, Duration::from_millis(1500), |done| !*done);
    }

    pub fn kill_all(&self) {
        for (_, mut p) in self.map.lock().unwrap().drain() {
            let _ = p.killer.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn keeps_split_characters_for_the_next_read() {
        let mut pending = Vec::new();
        let bytes = "añ€".as_bytes();
        assert_eq!(decode(&mut pending, &bytes[..2]), "a"); // "ñ" cut in half
        assert_eq!(decode(&mut pending, &bytes[2..4]), "ñ"); // "€" cut too
        assert_eq!(decode(&mut pending, &bytes[4..]), "€");
        assert!(pending.is_empty());
    }
}
