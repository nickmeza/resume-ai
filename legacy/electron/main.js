const { app, BrowserWindow, ipcMain, shell, clipboard, dialog, Menu } = require('electron');
const path = require('path');
const fs = require('fs');
const os = require('os');
const { execFile } = require('child_process');
const pty = require('@lydell/node-pty');
const core = require('./core');

const isWin = process.platform === 'win32';

// ---------- Configuración (%APPDATA%\Agent Sessions\config.json) ----------
const DEFAULT_CONFIG = {
  sidebarRatio: 0.3,                 // proporción del panel izquierdo (0.3 = 30/70)
  onboarded: false,                  // false → muestra el selector de agentes al abrir
  enabledAgents: [],                 // ids de core/agents habilitados
  filters: null,                     // grupo de filtro -> valores marcados ({ agent: ['claude'], origin: ['claude-vscode'] })
  shell: isWin ? (process.env.COMSPEC || 'cmd.exe') : (process.env.SHELL || '/bin/bash'),
  fontSize: 14,
  fontFamily: "'Cascadia Mono', 'Cascadia Code', Consolas, 'Courier New', monospace",
  agents: core.defaultAgentConfig(), // por agente: { command, extraArgs, dataDir }
  aliases: {},                       // key de sesión -> nombre puesto en la app (agentes sin renombrado nativo)
  shortcuts: {},                     // id de acción -> atajo ("Ctrl+Shift+R"); vacío = valores por defecto
};

const configPath = () => path.join(app.getPath('userData'), 'config.json');

function loadConfig() {
  let saved = {};
  try { saved = JSON.parse(fs.readFileSync(configPath(), 'utf8')); } catch { /* primera vez */ }
  const agents = { ...DEFAULT_CONFIG.agents };
  for (const [id, cfg] of Object.entries(saved.agents || {})) agents[id] = { ...agents[id], ...cfg };
  return { ...DEFAULT_CONFIG, ...saved, agents };
}

function saveConfig(cfg) {
  fs.mkdirSync(path.dirname(configPath()), { recursive: true });
  fs.writeFileSync(configPath(), JSON.stringify(cfg, null, 2));
}

let config;
let win;
const ptys = new Map();      // key -> proceso pty
let sessionIndex = new Map(); // key -> sesión del último listado (fuente confiable para renombrar/borrar)
let baseEnv = { ...process.env }; // entorno de las terminales; se actualiza con "Recargar terminal"

function createWindow() {
  win = new BrowserWindow({
    width: 1400,
    height: 860,
    minWidth: 800,
    minHeight: 500,
    backgroundColor: '#1a1a1a',
    title: 'Agent Sessions',
    autoHideMenuBar: true,
    webPreferences: { preload: path.join(__dirname, 'preload.js'), contextIsolation: true, nodeIntegration: false },
  });
  win.loadFile(path.join(__dirname, 'renderer', 'index.html'));
  win.on('closed', () => { win = null; });
}

// Menú propio: el de Electron trae Ctrl+R / Ctrl+Shift+R (recargar la página), que chocan con los atajos.
function setupMenu() {
  Menu.setApplicationMenu(Menu.buildFromTemplate([
    { role: 'editMenu' },
    {
      label: 'Ver',
      submenu: [
        { role: 'toggleDevTools', accelerator: 'F12' },
        { type: 'separator' },
        { role: 'resetZoom' }, { role: 'zoomIn' }, { role: 'zoomOut' },
        { type: 'separator' },
        { role: 'togglefullscreen' },
      ],
    },
  ]));
}

// ---------- Entorno (equivalente a "source ~/.zshrc") ----------
function run(file, args) {
  return new Promise((resolve) => {
    execFile(file, args, { windowsHide: true, timeout: 10000, maxBuffer: 8 * 1024 * 1024 }, (err, out) => resolve(err ? '' : String(out)));
  });
}

/** Busca una variable sin distinguir mayúsculas (en Windows es "Path", no "PATH"). */
const envKey = (env, name) => Object.keys(env).find((k) => k.toLowerCase() === name.toLowerCase()) || name;

async function readRegEnv(hive) {
  const vars = [];
  for (const line of (await run('reg', ['query', hive])).split(/\r?\n/)) {
    const m = line.match(/^ {4}(.+?) {4}(REG_(?:EXPAND_)?SZ) {4}(.*)$/);
    if (m) vars.push({ name: m[1], expand: m[2] === 'REG_EXPAND_SZ', value: m[3] });
  }
  return vars;
}

/** Relee las variables de entorno como las vería una terminal recién abierta. */
async function refreshEnv() {
  if (isWin) {
    const env = { ...process.env };
    const expand = (v) => v.replace(/%([^%]+)%/g, (all, n) => env[envKey(env, n)] ?? all);
    const machine = await readRegEnv('HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment');
    const user = await readRegEnv('HKCU\\Environment');
    if (!machine.length && !user.length) throw new Error('No se pudieron leer las variables del registro');
    // Las de usuario se aplican después, como hace Windows. Path se arma al final porque puede
    // usar variables que aparecen más abajo en el registro (%PNPM_HOME%, %NVM_HOME%…).
    const vars = [...machine, ...user];
    const isPath = (v) => v.name.toLowerCase() === 'path';
    for (const v of vars) if (!isPath(v)) env[envKey(env, v.name)] = v.value;
    for (const v of vars) if (!isPath(v) && v.expand) env[envKey(env, v.name)] = expand(v.value);
    const paths = vars.filter(isPath).map((v) => (v.expand ? expand(v.value) : v.value).replace(/;+$/, ''));
    if (paths.length) env[envKey(env, 'Path')] = paths.join(';');
    baseEnv = env;
  } else {
    const out = await run(config.shell, ['-ilc', 'env']);
    if (!out) throw new Error(`No se pudo leer el entorno de ${config.shell}`);
    const env = {};
    for (const line of out.split('\n')) {
      const i = line.indexOf('=');
      if (i > 0) env[line.slice(0, i)] = line.slice(i + 1);
    }
    baseEnv = { ...process.env, ...env };
  }
}

// ---------- Terminales ----------
function spawnTerminal({ key, agent, mode, sessionId, cwd, cols, rows }) {
  if (ptys.has(key)) return { ok: true, reused: true };

  let workDir = cwd;
  let warning = null;
  if (!workDir || !fs.existsSync(workDir)) {
    if (cwd) warning = `La carpeta original ya no existe: ${cwd}. Se abrió en tu carpeta de usuario; puede que el agente no encuentre la sesión.`;
    else if (mode !== 'shell') warning = 'No se pudo determinar la carpeta de esta sesión. Se abrió en tu carpeta de usuario.';
    workDir = os.homedir();
  }

  // mode 'shell' = terminal sola, sin agente
  const command = mode === 'shell' ? null : core.buildCommand(agent, config.agents[agent], mode, sessionId);
  const file = config.shell;
  const isCmd = /cmd(\.exe)?$/i.test(file);
  const isPwsh = /(powershell|pwsh)(\.exe)?$/i.test(file);
  let args;
  if (isWin && isCmd) args = command ? `/d /k ${command}` : '/d'; // al salir del agente, queda la consola abierta
  else if (isPwsh) args = command ? ['-NoLogo', '-NoExit', '-Command', command] : ['-NoLogo'];
  else args = command ? ['-lc', `${command}; exec ${file}`] : ['-l'];

  const proc = pty.spawn(file, args, {
    name: 'xterm-256color',
    cols: cols || 120,
    rows: rows || 30,
    cwd: workDir,
    env: { ...baseEnv, FORCE_COLOR: '1', COLORTERM: 'truecolor' },
    useConpty: true,
  });
  // Al reiniciar, el proceso viejo puede emitir datos o salir después de crear el nuevo con la misma key
  const current = () => ptys.get(key) === proc;
  proc.onData((data) => current() && win && win.webContents.send('pty:data', key, data));
  proc.onExit(({ exitCode }) => {
    if (!current()) return;
    ptys.delete(key);
    win && win.webContents.send('pty:exit', key, exitCode);
  });
  ptys.set(key, proc);
  return { ok: true, warning, command: command || file, cwd: workDir };
}

/** Cierra una terminal y espera a que el proceso termine (para liberar los archivos de la sesión). */
function killPty(key) {
  const p = ptys.get(key);
  if (!p) return Promise.resolve();
  ptys.delete(key);
  return new Promise((resolve) => {
    const t = setTimeout(resolve, 1500);
    p.onExit(() => { clearTimeout(t); resolve(); });
    try { p.kill(); } catch { clearTimeout(t); resolve(); }
  });
}

// ---------- Renombrar / eliminar sesiones ----------
async function renameSession(key, rawTitle) {
  const s = sessionIndex.get(key);
  if (!s) return { ok: false, error: 'No encontré la sesión; recarga la lista (F5).' };
  const title = String(rawTitle || '').replace(/\s+/g, ' ').trim().slice(0, 200);
  const aliases = { ...config.aliases };
  let native = false;
  if (title && core.AGENTS.find((a) => a.id === s.agent)?.rename) {
    try { await core.renameSession(s, title); native = true; } catch { /* se guarda como alias */ }
  }
  if (native || !title) delete aliases[key];
  else aliases[key] = title;
  config = { ...config, aliases };
  saveConfig(config);
  return { ok: true, native, aliases };
}

async function trash(p) {
  for (let i = 0; ; i++) {
    try { await shell.trashItem(p); return; } catch (e) {
      if (i >= 2) throw e;
      await new Promise((r) => setTimeout(r, 400)); // el agente puede tardar en soltar el archivo
    }
  }
}

async function deleteSession(key) {
  const s = sessionIndex.get(key);
  if (!s) return { ok: false, error: 'No encontré la sesión; recarga la lista (F5).' };
  const agentName = core.AGENTS.find((a) => a.id === s.agent)?.name || s.agent;
  let paths;
  try { paths = core.sessionPaths(s, config.agents[s.agent]); } catch (e) { return { ok: false, error: String(e.message || e) }; }
  if (!paths) return { ok: false, error: `${agentName} no permite eliminar sesiones desde fuera de su propia app.` };
  paths = paths.filter((p) => fs.existsSync(p));
  if (!paths.length) return { ok: false, error: 'Los archivos de la sesión ya no existen.' };

  const title = config.aliases[key] || s.title;
  const { response } = await dialog.showMessageBox(win, {
    type: 'warning',
    buttons: ['Mover a la papelera', 'Cancelar'],
    defaultId: 1,
    cancelId: 1,
    title: 'Eliminar sesión',
    message: `¿Eliminar la sesión "${title}" de ${agentName}?`,
    detail: `Se moverá a la papelera de reciclaje (puedes restaurarla desde ahí):\n\n${paths.join('\n')}`,
  });
  if (response !== 0) return { ok: false, cancelled: true };

  await killPty(key);
  try {
    for (const p of paths) await trash(p);
  } catch (e) {
    return { ok: false, error: `No se pudo mover a la papelera: ${e.message || e}` };
  }
  sessionIndex.delete(key);
  if (config.aliases[key]) {
    const aliases = { ...config.aliases };
    delete aliases[key];
    config = { ...config, aliases };
    saveConfig(config);
  }
  return { ok: true };
}

// ---------- Imágenes (pegar capturas, Ctrl+clic en rutas, pestañas de imagen) ----------
const IMAGE_EXT = /\.(png|jpe?g|gif|webp|bmp|svg|ico|avif)$/i;
const MIME = { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp',
  bmp: 'image/bmp', svg: 'image/svg+xml', ico: 'image/x-icon', avif: 'image/avif' };
const pastedDir = () => path.join(app.getPath('temp'), 'agent-sessions');

/** Guarda la imagen del portapapeles en un archivo temporal y devuelve su ruta (null si no hay imagen).
 *  Electron 45 usa la API asíncrona tipo W3C: read() → ClipboardItem[] con getType(mime) → Blob. */
async function saveClipboardImage() {
  const items = await clipboard.read();
  for (const item of items) {
    const type = item.types.find((t) => /^image\//i.test(t));
    if (!type) continue;
    const blob = await item.getType(type);
    const buf = Buffer.from(await blob.arrayBuffer());
    if (!buf.length) continue;
    const ext = Object.keys(MIME).find((k) => MIME[k] === type.toLowerCase()) || 'png';
    fs.mkdirSync(pastedDir(), { recursive: true });
    const stamp = new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
    const file = path.join(pastedDir(), `pegado-${stamp}.${ext}`);
    await fs.promises.writeFile(file, buf);
    return file;
  }
  return null;
}

/** Borra capturas pegadas e imágenes extraídas de sesiones de hace más de 7 días. */
function cleanPastedImages() {
  const limit = Date.now() - 7 * 24 * 3600 * 1000;
  for (const dir of [pastedDir(), path.join(pastedDir(), 'sesiones')]) {
    let names = [];
    try { names = fs.readdirSync(dir); } catch { continue; }
    for (const n of names) {
      const p = path.join(dir, n);
      try {
        const st = fs.statSync(p);
        if (st.mtimeMs >= limit) continue;
        if (st.isDirectory()) { if (n !== 'sesiones') fs.rmSync(p, { recursive: true, force: true }); } else fs.unlinkSync(p);
      } catch { /* en uso */ }
    }
  }
}

// ---------- Imágenes de sesiones de Claude ("[Image #N]") ----------
const claudeAgent = core.AGENTS.find((a) => a.id === 'claude');
const claudeDataDir = () => (config.agents.claude && config.agents.claude.dataDir) || claudeAgent.defaultDataDir();
const imageCache = new Map(); // archivo de sesión -> { mtimeMs, size, extracted, messages }

/**
 * Imágenes de una sesión de Claude agrupadas por mensaje. La sesión se identifica por:
 *  - key / sessionId: sesión ya conocida
 *  - cwd + since (+ exclude): pestaña nueva o copia compactada cuyo ID aún no conocemos (la más nueva de su carpeta)
 * extract=false solo cuenta (no escribe archivos): para el menú contextual.
 */
async function claudeImages({ key, sessionId, cwd, since, exclude, extract = true }) {
  let s = key ? sessionIndex.get(key) : null;
  if (s && s.agent !== 'claude') return { ok: false, error: 'Solo disponible para Claude Code' };
  let foundId = null;
  if (!s && sessionId) {
    s = [...sessionIndex.values()].find((x) => x.agent === 'claude' && x.id === sessionId);
    if (!s && cwd) s = { id: sessionId, src: path.join(claudeAgent.projectDir(claudeDataDir(), cwd), `${sessionId}.jsonl`) };
  }
  if (!s && cwd && since) {
    const dir = claudeAgent.projectDir(claudeDataDir(), cwd);
    const id = await claudeAgent.findNewest(dir, since, exclude || []);
    if (id) { s = { id, src: path.join(dir, `${id}.jsonl`) }; foundId = id; }
  }
  if (!s || !s.src || !fs.existsSync(s.src)) return { ok: true, pending: true, messages: [] }; // aún no se envió nada

  const stat = await fs.promises.stat(s.src);
  const hit = imageCache.get(s.src);
  if (hit && hit.mtimeMs === stat.mtimeMs && hit.size === stat.size && (hit.extracted || !extract)) {
    return { ok: true, sessionId: s.id, foundId, messages: hit.messages };
  }
  const outDir = extract ? path.join(pastedDir(), 'sesiones', s.id) : null;
  try {
    const messages = await claudeAgent.images(s, outDir);
    imageCache.set(s.src, { mtimeMs: stat.mtimeMs, size: stat.size, extracted: extract, messages });
    if (outDir) { const now = new Date(); fs.utimesSync(outDir, now, now); } // la limpieza usa esta fecha
    return { ok: true, sessionId: s.id, foundId, messages };
  } catch (e) {
    return { ok: false, error: String(e.message || e) };
  }
}

/** Convierte rutas vistas en la terminal (relativas, con ~) en rutas absolutas de imágenes que existen. */
function resolveImages(paths, cwd) {
  const base = cwd && fs.existsSync(cwd) ? cwd : os.homedir();
  return (paths || []).map((p) => {
    const full = path.resolve(base, String(p).replace(/^~(?=[\\/])/, os.homedir()));
    try { return IMAGE_EXT.test(full) && fs.statSync(full).isFile() ? full : null; } catch { return null; }
  });
}

async function readImage(p) {
  if (!IMAGE_EXT.test(String(p))) return { ok: false, error: 'No es un archivo de imagen' };
  try {
    const stat = await fs.promises.stat(p);
    if (stat.size > 64 * 1024 * 1024) return { ok: false, error: 'La imagen pesa más de 64 MB; ábrela con el visor del sistema' };
    const buf = await fs.promises.readFile(p);
    const mime = MIME[path.extname(p).slice(1).toLowerCase()];
    return { ok: true, dataUrl: `data:${mime};base64,${buf.toString('base64')}`, size: stat.size };
  } catch (e) {
    return { ok: false, error: e.code === 'ENOENT' ? 'El archivo ya no existe' : String(e.message || e) };
  }
}

// ---------- IPC ----------
ipcMain.handle('config:get', () => config);
ipcMain.handle('config:set', (_e, patch) => {
  config = { ...config, ...patch };
  saveConfig(config);
  return config;
});
ipcMain.handle('config:open', () => { saveConfig(config); return shell.openPath(configPath()); });
ipcMain.handle('agents:info', () => core.agentInfo());
ipcMain.handle('agents:detect', () => core.detectAgents(config.agents));
ipcMain.handle('sessions:list', async () => {
  const res = await core.listAll(config.enabledAgents, config.agents);
  sessionIndex = new Map(res.sessions.map((s) => [s.key, s]));
  return res;
});
ipcMain.handle('sessions:rename', (_e, key, title) => renameSession(key, title));
ipcMain.handle('sessions:delete', (_e, key) => deleteSession(key));
ipcMain.handle('sessions:findFork', (_e, key, since) => {
  const s = sessionIndex.get(key);
  return s ? core.findFork(s, since) : null;
});
ipcMain.handle('env:reload', async () => {
  try { await refreshEnv(); return { ok: true }; } catch (err) { return { ok: false, error: String(err.message || err) }; }
});
ipcMain.handle('pty:open', (_e, opts) => {
  try { return spawnTerminal(opts); } catch (err) { return { ok: false, error: String(err.message || err) }; }
});
ipcMain.on('pty:write', (_e, key, data) => { const p = ptys.get(key); if (p) p.write(data); });
ipcMain.on('pty:resize', (_e, key, cols, rows) => {
  const p = ptys.get(key);
  if (p && cols > 0 && rows > 0) { try { p.resize(cols, rows); } catch { /* cerrándose */ } }
});
ipcMain.handle('pty:kill', async (_e, key) => { await killPty(key); return true; });
ipcMain.handle('clipboard:read', () => clipboard.readText());
ipcMain.handle('clipboard:write', (_e, text) => clipboard.writeText(text));
ipcMain.handle('folder:open', (_e, p) => shell.openPath(p));
ipcMain.handle('path:reveal', (_e, p) => shell.showItemInFolder(p));
ipcMain.handle('clipboard:saveImage', () => saveClipboardImage());
ipcMain.handle('images:resolve', (_e, paths, cwd) => resolveImages(paths, cwd));
ipcMain.handle('image:read', (_e, p) => readImage(p));
ipcMain.handle('claude:images', (_e, query) => claudeImages(query || {}));

app.whenReady().then(() => {
  config = loadConfig();
  saveConfig(config);
  setupMenu();
  createWindow();
  cleanPastedImages();
});

app.on('window-all-closed', () => {
  for (const p of ptys.values()) { try { p.kill(); } catch { /* ignore */ } }
  ptys.clear();
  app.quit();
});
