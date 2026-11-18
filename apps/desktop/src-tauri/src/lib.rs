//! Resume AI desktop app: Tauri commands called by the web UI (`ui/api.js`), on top of resume-ai-core.

mod env;
mod pty;

use base64::Engine;
use resume_ai_core::{self as core, AgentConfig, ImageMessage, LaunchMode, Session, aliases, i18n::I18n};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;

pub struct AppState {
    config: Mutex<Value>,
    config_path: PathBuf,
    /// key → session of the last listing (trusted source for rename/delete; never paths from the UI)
    sessions: Mutex<HashMap<String, Session>>,
    pub ptys: pty::Ptys,
    /// Environment of new terminals; refreshed by `env_reload`
    env: Mutex<HashMap<String, String>>,
    /// session file → (mtime, size, extracted, messages)
    images: Mutex<HashMap<PathBuf, CachedImages>>,
}

type Res<T> = Result<T, String>;

/// Images of a session file: (mtime, size, extracted to disk, messages).
type CachedImages = (f64, u64, bool, Vec<ImageMessage>);

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ---------- Configuration ----------

fn default_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into())
    }
}

fn default_config() -> Value {
    let font = if cfg!(target_os = "macos") {
        "Menlo, Monaco, 'Courier New', monospace"
    } else {
        "'Cascadia Mono', 'Cascadia Code', Consolas, 'DejaVu Sans Mono', 'Courier New', monospace"
    };
    json!({
        "sidebarRatio": 0.3,
        "onboarded": false,
        "enabledAgents": [],
        "filters": {},
        "language": "auto",
        "shell": default_shell(),
        "fontSize": 14,
        "fontFamily": font,
        "agents": core::default_agent_config(),
        "shortcuts": {},
    })
}

/// Defaults + saved values (agents are merged one by one so new fields get their default).
fn merge_config(saved: &Value) -> Value {
    let mut cfg = default_config();
    let (Some(target), Some(source)) = (cfg.as_object_mut(), saved.as_object()) else { return cfg };
    for (k, v) in source {
        if k == "agents" {
            let agents = target.entry("agents").or_insert_with(|| json!({}));
            for (id, a) in v.as_object().into_iter().flatten() {
                let slot = agents.as_object_mut().unwrap().entry(id.clone()).or_insert_with(|| json!({}));
                for (f, x) in a.as_object().into_iter().flatten() {
                    slot.as_object_mut().unwrap().insert(f.clone(), x.clone());
                }
            }
        } else if k != "aliases" {
            target.insert(k.clone(), v.clone());
        }
    }
    cfg
}

/// The Electron version stored its settings in `%APPDATA%\Agent Sessions\config.json`.
fn legacy_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("Agent Sessions").join("config.json"))
}

fn load_config(path: &Path) -> Value {
    if let Ok(text) = fs::read_to_string(path) {
        return merge_config(&serde_json::from_str(&text).unwrap_or(Value::Null));
    }
    // First run: import the settings of the previous app (aliases go to the shared aliases file)
    if let Some(old) =
        legacy_config_path().and_then(|p| fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str::<Value>(&t).ok())
    {
        for (key, title) in old["aliases"].as_object().into_iter().flatten() {
            if let Some(t) = title.as_str() {
                let _ = aliases::set(key, Some(t));
            }
        }
        let mut cfg = merge_config(&old);
        if let Some(agent) = old["agentFilter"].as_str().filter(|a| *a != "all") {
            cfg["filters"] = json!({ "agent": [agent] });
        }
        return cfg;
    }
    default_config()
}

fn save_config(path: &Path, cfg: &Value) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(path, serde_json::to_string_pretty(cfg).unwrap_or_default());
}

fn agent_configs(cfg: &Value) -> BTreeMap<String, AgentConfig> {
    let mut all = core::default_agent_config();
    if let Some(user) = cfg.get("agents").and_then(|a| serde_json::from_value::<BTreeMap<String, AgentConfig>>(a.clone()).ok()) {
        all.extend(user);
    }
    all
}

fn i18n(state: &AppState) -> I18n {
    I18n::new(state.config.lock().unwrap()["language"].as_str().unwrap_or("auto"))
}

/// Config for the UI, with the shared aliases (the UI reads `config.aliases`).
fn config_view(state: &AppState) -> Value {
    let mut cfg = state.config.lock().unwrap().clone();
    cfg["aliases"] = json!(aliases::load());
    cfg
}

#[tauri::command]
fn config_get(state: State<AppState>) -> Value {
    config_view(&state)
}

#[tauri::command]
fn config_set(state: State<AppState>, patch: Map<String, Value>) -> Value {
    {
        let mut cfg = state.config.lock().unwrap();
        for (k, v) in patch {
            if k != "aliases" {
                cfg[k] = v;
            }
        }
        save_config(&state.config_path, &cfg);
    }
    config_view(&state)
}

#[tauri::command]
fn config_open(app: AppHandle, state: State<AppState>) -> Res<()> {
    save_config(&state.config_path, &state.config.lock().unwrap());
    app.opener().open_path(state.config_path.display().to_string(), None::<&str>).map_err(err)
}

/// Translations for the UI: English merged with the chosen language (missing keys fall back to English).
#[tauri::command]
fn i18n_bundle(lang: String) -> Value {
    let code = core::i18n::resolve_language(&lang);
    let mut messages: Map<String, Value> = serde_json::from_str(core::i18n::bundle_json("en").unwrap()).unwrap_or_default();
    if let Some(other) = core::i18n::bundle_json(code).and_then(|j| serde_json::from_str::<Map<String, Value>>(j).ok()) {
        messages.extend(other);
    }
    let languages: Vec<Value> = core::i18n::LANGUAGES.iter().map(|(c, n)| json!({ "code": c, "name": n })).collect();
    json!({ "lang": code, "messages": messages, "languages": languages })
}

// ---------- Agents and sessions ----------

#[tauri::command]
fn agents_info() -> Vec<core::AgentInfo> {
    core::agent_info()
}

#[tauri::command]
async fn agents_detect(state: State<'_, AppState>) -> Res<Vec<core::Detection>> {
    let cfg = agent_configs(&state.config.lock().unwrap());
    Ok(core::detect_agents(&cfg))
}

#[tauri::command]
async fn sessions_list(state: State<'_, AppState>) -> Res<core::ListResult> {
    let (enabled, cfg) = {
        let c = state.config.lock().unwrap();
        let enabled: Vec<String> = serde_json::from_value(c["enabledAgents"].clone()).unwrap_or_default();
        (enabled, agent_configs(&c))
    };
    let res = core::list_all(&enabled, &cfg);
    *state.sessions.lock().unwrap() = res.sessions.iter().map(|s| (s.key.clone(), s.clone())).collect();
    Ok(res)
}

fn session(state: &AppState, key: &str) -> Res<Session> {
    state.sessions.lock().unwrap().get(key).cloned().ok_or_else(|| "session not found; reload the list (F5)".to_string())
}

#[tauri::command]
fn sessions_rename(state: State<AppState>, key: String, title: String) -> Value {
    match session(&state, &key).and_then(|s| aliases::rename(&s, &title).map_err(err)) {
        Ok(native) => json!({ "ok": true, "native": native, "aliases": aliases::load() }),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

#[tauri::command]
async fn sessions_delete(app: AppHandle, state: State<'_, AppState>, key: String) -> Res<Value> {
    let s = match session(&state, &key) {
        Ok(s) => s,
        Err(e) => return Ok(json!({ "ok": false, "error": e })),
    };
    let t = i18n(&state);
    let agent_cfg = agent_configs(&state.config.lock().unwrap());
    let agent = core::by_id(&s.agent).map(|a| a.name()).unwrap_or(&s.agent);
    let paths = match core::session_paths(&s, agent_cfg.get(&s.agent)) {
        Ok(Some(p)) if !p.is_empty() => p,
        Ok(Some(_)) => return Ok(json!({ "ok": false, "error": t.t("ui.delete.gone") })),
        Ok(None) => return Ok(json!({ "ok": false, "error": t.tf("delete.notAllowed", &[("agent", &agent)]) })),
        Err(e) => return Ok(json!({ "ok": false, "error": err(e) })),
    };
    let title = aliases::display_title(&s, &aliases::load()).to_string();
    let list: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    let confirmed = app
        .dialog()
        .message(format!("{}\n\n{}", t.t("delete.note"), list.join("\n")))
        .title(t.tf("ui.delete.question", &[("title", &title), ("agent", &agent)]))
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(t.t("ui.delete.confirm"), t.t("ui.cancel")))
        .blocking_show();
    if !confirmed {
        return Ok(json!({ "ok": false, "cancelled": true }));
    }
    state.ptys.kill(&key);
    let mut last = None;
    for _ in 0..3 {
        match core::trash_session(&s, agent_cfg.get(&s.agent)) {
            Ok(_) => {
                last = None;
                break;
            }
            Err(e) => {
                last = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(400)); // the agent may still hold the file
            }
        }
    }
    if let Some(e) = last {
        return Ok(json!({ "ok": false, "error": t.tf("delete.error", &[("error", &format!("{e:#}"))]) }));
    }
    let _ = aliases::set(&key, None);
    state.sessions.lock().unwrap().remove(&key);
    Ok(json!({ "ok": true }))
}

/// Id of the copy created by "continue in a new session" (Claude `--fork-session`).
#[tauri::command]
fn sessions_find_fork(state: State<AppState>, key: String, since: f64) -> Option<String> {
    let s = session(&state, &key).ok()?;
    core::claude::find_newest(s.src.as_ref()?.parent()?, since, &[s.id])
}

// ---------- Terminals ----------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PtyOpen {
    key: String,
    agent: Option<String>,
    mode: String,
    session_id: Option<String>,
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
}

fn is_shell(shell: &str, names: &[&str]) -> bool {
    let file = Path::new(shell).file_stem().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
    names.contains(&file.as_str())
}

#[tauri::command]
async fn pty_open(app: AppHandle, state: State<'_, AppState>, opts: PtyOpen) -> Res<Value> {
    if state.ptys.has(&opts.key) {
        return Ok(json!({ "ok": true, "reused": true }));
    }
    let (shell, agent_cfg) = {
        let c = state.config.lock().unwrap();
        (c["shell"].as_str().map(str::to_string).unwrap_or_else(default_shell), agent_configs(&c))
    };
    // The UI translates the warning codes
    let mut warning = Value::Null;
    let cwd = match opts.cwd.as_deref().filter(|c| !c.is_empty()) {
        Some(c) if Path::new(c).is_dir() => PathBuf::from(c),
        Some(c) => {
            warning = json!({ "code": "folderMissing", "path": c });
            dirs::home_dir().unwrap_or_default()
        }
        None => {
            if opts.mode != "shell" {
                warning = json!({ "code": "folderUnknown" });
            }
            dirs::home_dir().unwrap_or_default()
        }
    };

    // The agent runs inside the user's shell, so the shell stays open when the agent exits
    let command = if opts.mode == "shell" {
        None
    } else {
        let mode = match opts.mode.as_str() {
            "new" => LaunchMode::New,
            "compact" => LaunchMode::Compact,
            _ => LaunchMode::Resume,
        };
        let agent = opts.agent.as_deref().unwrap_or("");
        Some(core::build_command(agent, agent_cfg.get(agent), mode, opts.session_id.as_deref()).map_err(err)?)
    };
    let args: Vec<String> = match &command {
        None if is_shell(&shell, &["cmd"]) => vec!["/d".into()],
        None if is_shell(&shell, &["powershell", "pwsh"]) => vec!["-NoLogo".into()],
        None => vec!["-l".into()],
        Some(c) if is_shell(&shell, &["cmd"]) => {
            // Each part as its own argument: portable-pty only quotes the ones with spaces
            let mut a = vec!["/d".to_string(), "/k".to_string(), c.program.clone()];
            a.extend(c.args.iter().cloned());
            a
        }
        Some(c) if is_shell(&shell, &["powershell", "pwsh"]) => {
            let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
            let line = std::iter::once(format!("& {}", quote(&c.program)))
                .chain(c.args.iter().map(|a| quote(a)))
                .collect::<Vec<_>>()
                .join(" ");
            vec!["-NoLogo".into(), "-NoExit".into(), "-Command".into(), line]
        }
        Some(c) => vec!["-lc".into(), format!("{}; exec {}", c.to_shell_string(), shell)],
    };
    let env = state.env.lock().unwrap().clone();
    let spawn = pty::Spawn {
        key: &opts.key,
        program: &shell,
        args: &args,
        cwd: &cwd,
        env: &env,
        cols: opts.cols.unwrap_or(120),
        rows: opts.rows.unwrap_or(30),
    };
    match state.ptys.open(&app, spawn) {
        Ok(()) => Ok(json!({
            "ok": true,
            "warning": warning,
            "command": command.map(|c| c.to_shell_string()).unwrap_or(shell),
            "cwd": cwd.display().to_string(),
        })),
        Err(e) => Ok(json!({ "ok": false, "error": format!("{e:#}") })),
    }
}

#[tauri::command]
fn pty_write(state: State<AppState>, key: String, data: String) {
    state.ptys.write(&key, &data);
}

#[tauri::command]
fn pty_resize(state: State<AppState>, key: String, cols: u16, rows: u16) {
    state.ptys.resize(&key, cols, rows);
}

#[tauri::command]
async fn pty_kill(state: State<'_, AppState>, key: String) -> Res<bool> {
    state.ptys.kill(&key);
    Ok(true)
}

#[tauri::command]
async fn env_reload(state: State<'_, AppState>) -> Res<Value> {
    match env::refreshed() {
        Ok(e) => {
            *state.env.lock().unwrap() = e;
            Ok(json!({ "ok": true }))
        }
        Err(e) => Ok(json!({ "ok": false, "error": format!("{e:#}") })),
    }
}

// ---------- Clipboard, files and images ----------

#[tauri::command]
fn clipboard_read() -> String {
    arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default()
}

#[tauri::command]
fn clipboard_write(text: String) {
    if let Ok(mut c) = arboard::Clipboard::new() {
        let _ = c.set_text(text);
    }
}

fn temp_dir() -> PathBuf {
    std::env::temp_dir().join("resume-ai")
}

/// Saves the clipboard image (e.g. a screenshot) as a PNG and returns its path.
#[tauri::command]
fn clipboard_save_image() -> Option<String> {
    let img = arboard::Clipboard::new().ok()?.get_image().ok()?;
    let dir = temp_dir();
    fs::create_dir_all(&dir).ok()?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    let path = dir.join(format!("pasted-{stamp}.png"));
    let file = fs::File::create(&path).ok()?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), img.width as u32, img.height as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().ok()?.write_image_data(&img.bytes).ok()?;
    Some(path.display().to_string())
}

/// Deletes pasted screenshots and extracted session images older than 7 days.
fn clean_temp() {
    let limit = std::time::SystemTime::now() - std::time::Duration::from_secs(7 * 24 * 3600);
    for dir in [temp_dir(), temp_dir().join("sessions")] {
        for e in fs::read_dir(dir).into_iter().flatten().flatten() {
            let old = e.metadata().and_then(|m| m.modified()).is_ok_and(|m| m < limit);
            if !old || e.file_name() == "sessions" {
                continue;
            }
            let p = e.path();
            let _ = if p.is_dir() { fs::remove_dir_all(p) } else { fs::remove_file(p) };
        }
    }
}

#[tauri::command]
fn folder_open(app: AppHandle, path: String) -> Res<()> {
    app.opener().open_path(path, None::<&str>).map_err(err)
}

#[tauri::command]
fn path_reveal(app: AppHandle, path: String) -> Res<()> {
    app.opener().reveal_item_in_dir(path).map_err(err)
}

const IMAGE_EXTS: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("bmp", "image/bmp"),
    ("svg", "image/svg+xml"),
    ("ico", "image/x-icon"),
    ("avif", "image/avif"),
];

fn image_mime(p: &Path) -> Option<&'static str> {
    let ext = p.extension()?.to_string_lossy().to_lowercase();
    IMAGE_EXTS.iter().find(|(e, _)| *e == ext).map(|(_, m)| *m)
}

/// Turns paths seen in a terminal (relative, `~`) into absolute paths of existing images.
#[tauri::command]
fn images_resolve(paths: Vec<String>, cwd: Option<String>) -> Vec<Option<String>> {
    let home = dirs::home_dir().unwrap_or_default();
    let base = cwd.map(PathBuf::from).filter(|c| c.is_dir()).unwrap_or_else(|| home.clone());
    paths
        .iter()
        .map(|p| {
            let p = match p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
                Some(rest) => home.join(rest),
                None => base.join(p),
            };
            (image_mime(&p).is_some() && p.is_file()).then(|| p.display().to_string())
        })
        .collect()
}

#[tauri::command]
async fn image_read(path: String) -> Res<Value> {
    let p = PathBuf::from(&path);
    let Some(mime) = image_mime(&p) else { return Ok(json!({ "ok": false, "error": "not an image file" })) };
    match fs::metadata(&p) {
        Ok(m) if m.len() > 64 * 1024 * 1024 => Ok(json!({ "ok": false, "error": "larger than 64 MB" })),
        Ok(m) => match fs::read(&p) {
            Ok(bytes) => Ok(json!({
                "ok": true,
                "size": m.len(),
                "dataUrl": format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)),
            })),
            Err(e) => Ok(json!({ "ok": false, "error": err(e) })),
        },
        Err(_) => Ok(json!({ "ok": false, "error": "the file no longer exists" })),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImagesQuery {
    key: Option<String>,
    session_id: Option<String>,
    cwd: Option<String>,
    since: Option<f64>,
    #[serde(default)]
    exclude: Vec<String>,
    extract: Option<bool>,
}

/// Images of a Claude session grouped by message. The session is found by key/id, or — for new tabs
/// and copies whose id is still unknown — as the newest session of the folder created after `since`.
#[tauri::command]
async fn claude_images(state: State<'_, AppState>, query: ImagesQuery) -> Res<Value> {
    let extract = query.extract.unwrap_or(true);
    let data_dir = {
        let cfg = agent_configs(&state.config.lock().unwrap());
        core::data_dir_of(core::by_id("claude").unwrap(), cfg.get("claude"))
    };
    let mut found_id = None;
    let mut s = query.key.as_deref().and_then(|k| session(&state, k).ok());
    if s.as_ref().is_some_and(|s| s.agent != "claude") {
        return Ok(json!({ "ok": false, "error": "only available for Claude Code" }));
    }
    if s.is_none() {
        if let Some(id) = &query.session_id {
            s = state.sessions.lock().unwrap().values().find(|x| x.agent == "claude" && &x.id == id).cloned();
            if s.is_none() {
                if let Some(cwd) = &query.cwd {
                    let mut x = Session::new("claude", id.clone());
                    x.src = Some(core::claude::project_dir(&data_dir, cwd).join(format!("{id}.jsonl")));
                    s = Some(x);
                }
            }
        }
    }
    if s.is_none() {
        if let (Some(cwd), Some(since)) = (&query.cwd, query.since) {
            let dir = core::claude::project_dir(&data_dir, cwd);
            if let Some(id) = core::claude::find_newest(&dir, since, &query.exclude) {
                let mut x = Session::new("claude", id.clone());
                x.src = Some(dir.join(format!("{id}.jsonl")));
                found_id = Some(id);
                s = Some(x);
            }
        }
    }
    let Some(s) = s.filter(|s| s.src.as_ref().is_some_and(|p| p.is_file())) else {
        return Ok(json!({ "ok": true, "pending": true, "messages": [] })); // nothing sent yet
    };
    let src = s.src.clone().unwrap();
    let meta = fs::metadata(&src).map_err(err)?;
    let mtime = meta.modified().map(core::util::to_ms).unwrap_or(0.0);
    if let Some((m, size, extracted, msgs)) = state.images.lock().unwrap().get(&src) {
        if *m == mtime && *size == meta.len() && (*extracted || !extract) {
            return Ok(json!({ "ok": true, "sessionId": s.id, "foundId": found_id, "messages": msgs }));
        }
    }
    let out_dir = extract.then(|| temp_dir().join("sessions").join(&s.id));
    match core::claude::images(&s, out_dir.as_deref()) {
        Ok(msgs) => {
            state.images.lock().unwrap().insert(src, (mtime, meta.len(), extract, msgs.clone()));
            Ok(json!({ "ok": true, "sessionId": s.id, "foundId": found_id, "messages": msgs }))
        }
        Err(e) => Ok(json!({ "ok": false, "error": format!("{e:#}") })),
    }
}

/// Errors of the web UI, printed to the console (the web view's own console is not visible).
#[tauri::command]
fn ui_log(level: String, message: String) {
    eprintln!("[ui {level}] {message}");
}

// ---------- App ----------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let config_path = app.path().app_config_dir()?.join("config.json");
            let config = load_config(&config_path);
            save_config(&config_path, &config);
            app.manage(AppState {
                config: Mutex::new(config),
                config_path,
                sessions: Mutex::new(HashMap::new()),
                ptys: pty::Ptys::default(),
                env: Mutex::new(std::env::vars().collect()),
                images: Mutex::new(HashMap::new()),
            });
            std::thread::spawn(clean_temp);
            // Development check: RESUME_AI_SMOKE=1 prints what the UI rendered (sessions, texts, errors)
            #[cfg(debug_assertions)]
            if std::env::var_os("RESUME_AI_SMOKE").is_some() {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(6));
                    if let Some(w) = handle.get_webview_window("main") {
                        let _ = w.eval(include_str!("smoke.js"));
                    }
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                window.state::<AppState>().ptys.kill_all();
            }
        })
        .invoke_handler(tauri::generate_handler![
            config_get,
            config_set,
            config_open,
            i18n_bundle,
            agents_info,
            agents_detect,
            sessions_list,
            sessions_rename,
            sessions_delete,
            sessions_find_fork,
            pty_open,
            pty_write,
            pty_resize,
            pty_kill,
            env_reload,
            clipboard_read,
            clipboard_write,
            clipboard_save_image,
            folder_open,
            path_reveal,
            images_resolve,
            image_read,
            claude_images,
            ui_log,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Resume AI");
}
