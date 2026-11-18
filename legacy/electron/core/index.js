// Núcleo compartido: registro de agentes, listado unificado y construcción de comandos.
// No depende de Electron → lo reutilizará la versión TUI.
const { execFile } = require('child_process');
const u = require('./util');

const AGENTS = [
  require('./agents/claude'),
  require('./agents/codex'),
  require('./agents/gemini'),
  require('./agents/opencode'),
  require('./agents/copilot'),
  require('./agents/cursor'),
];
const byId = Object.fromEntries(AGENTS.map((a) => [a.id, a]));

function agentInfo() {
  return AGENTS.map((a) => ({
    id: a.id, name: a.name, badge: a.badge, color: a.color,
    experimental: !!a.experimental, defaultCommand: a.defaultCommand, defaultDataDir: a.defaultDataDir(),
    canDelete: !!a.trashPaths, nativeRename: !!a.rename, compactCommand: a.compactCommand || '', forks: !!a.forkArgs,
  }));
}

/** Configuración por defecto de cada agente (el usuario la puede sobreescribir). */
function defaultAgentConfig() {
  const out = {};
  for (const a of AGENTS) out[a.id] = { command: a.defaultCommand, extraArgs: '', dataDir: '' };
  return out;
}

const dataDirOf = (a, cfg) => (cfg && cfg.dataDir) || a.defaultDataDir();
const rootsOf = (a, cfg) => [dataDirOf(a, cfg), ...(a.extraDataDirs ? a.extraDataDirs() : [])];

function whichCommand(cmd) {
  return new Promise((resolve) => {
    if (!cmd) return resolve(null);
    if (/[\\/]/.test(cmd)) return u.exists(cmd.replace(/^"|"$/g, '')).then((ok) => resolve(ok ? cmd : null));
    const tool = u.isWin ? 'where' : 'which';
    execFile(tool, [cmd], { timeout: 4000, windowsHide: true }, (err, stdout) => {
      resolve(err ? null : String(stdout).split(/\r?\n/)[0].trim() || null);
    });
  });
}

/** Detecta qué agentes están instalados / tienen sesiones. */
async function detectAgents(agentsCfg = {}) {
  return Promise.all(AGENTS.map(async (a) => {
    const cfg = agentsCfg[a.id] || {};
    const dataDir = dataDirOf(a, cfg);
    const dirs = [dataDir, ...(a.extraDataDirs ? a.extraDataDirs() : [])];
    const dataExists = (await Promise.all(dirs.map(u.exists))).some(Boolean);
    const commandPath = await whichCommand(cfg.command || a.defaultCommand);
    let count = 0;
    let error = null;
    if (dataExists) {
      try { count = (await a.list(dataDir)).length; } catch (e) { error = String(e.message || e); }
    }
    return { id: a.id, dataDir, dataExists, commandPath, count, error };
  }));
}

/** Resuelve carpetas guardadas como hash (Gemini = sha256, Cursor = md5) con rutas ya conocidas. */
function resolveHashedCwds(sessions) {
  const known = new Set(sessions.map((s) => s.cwd).filter(Boolean));
  const table = { sha256: new Map(), md5: new Map() };
  for (const p of known) {
    for (const v of u.pathVariants(p)) {
      table.sha256.set(u.sha256(v), p);
      table.md5.set(u.md5(v), p);
    }
  }
  for (const s of sessions) {
    if (!s.cwd && s.cwdHash) s.cwd = table[s.cwdHash.algo].get(s.cwdHash.value.toLowerCase());
    delete s.cwdHash;
  }
}

/** Lista las sesiones de todos los agentes habilitados, ordenadas por fecha. */
async function listAll(enabled, agentsCfg = {}) {
  const errors = [];
  const results = await Promise.all(enabled.filter((id) => byId[id]).map(async (id) => {
    const a = byId[id];
    const dataDir = dataDirOf(a, agentsCfg[id]);
    try {
      const list = await a.list(dataDir);
      return list.map((s) => ({ ...s, agent: id, key: `${id}:${s.id}` }));
    } catch (e) {
      errors.push({ agent: id, error: String(e.message || e) });
      return [];
    }
  }));
  const sessions = results.flat();
  resolveHashedCwds(sessions);
  sessions.sort((x, y) => y.mtime - x.mtime);
  return { sessions, errors };
}

/** Construye la línea de comando para retomar o iniciar una sesión. */
function buildCommand(agentId, agentCfg = {}, mode = 'new', sessionId = null) {
  const a = byId[agentId];
  if (!a) throw new Error(`Agente desconocido: ${agentId}`);
  let cmd = agentCfg.command || a.defaultCommand;
  if (/\s/.test(cmd) && !cmd.startsWith('"')) cmd = `"${cmd}"`;
  let args;
  if (mode === 'resume' || mode === 'compact') {
    if (!u.SAFE_ID.test(String(sessionId))) throw new Error('ID de sesión inválido');
    // compact = retomar en una copia (si el agente lo soporta); el comando de compactar lo envía la UI
    args = mode === 'compact' && a.forkArgs ? a.forkArgs(sessionId) : a.resumeArgs(sessionId);
  } else {
    args = a.newArgs();
  }
  const extra = (agentCfg.extraArgs || '').trim();
  return [cmd, ...args, extra].filter(Boolean).join(' ');
}

/** Rutas a borrar para eliminar una sesión, o null si el agente no lo permite. Nunca sale de la carpeta del agente. */
function sessionPaths(s, agentCfg = {}) {
  const a = byId[s.agent];
  if (!a || !a.trashPaths || !s.src) return null;
  const roots = rootsOf(a, agentCfg).map((r) => u.path.resolve(r));
  const paths = a.trashPaths(s).map((p) => u.path.resolve(p));
  for (const p of paths) {
    const inside = roots.some((r) => {
      const rel = u.path.relative(r, p);
      return rel && !rel.startsWith('..') && !u.path.isAbsolute(rel);
    });
    if (!inside) throw new Error(`Ruta fuera de la carpeta del agente: ${p}`);
  }
  return paths;
}

/** Renombra la sesión en los archivos del propio agente (solo si el agente lo soporta). */
async function renameSession(s, title) {
  const a = byId[s.agent];
  if (!a || !a.rename || !s.src) throw new Error('Este agente no permite renombrar sesiones');
  await a.rename(s, title);
}

/** ID de la copia creada al continuar en una nueva sesión, o null. */
async function findFork(s, since) {
  const a = byId[s.agent];
  if (!a || !a.findFork || !s.src) return null;
  try { return await a.findFork(s, since); } catch { return null; }
}

module.exports = {
  AGENTS, agentInfo, defaultAgentConfig, detectAgents, listAll, buildCommand, whichCommand, sessionPaths, renameSession, findFork,
};
