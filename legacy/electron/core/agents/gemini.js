// Gemini CLI — ~/.gemini/tmp/<hash|nombre>/chats/session-*.json
// La carpeta del proyecto se obtiene de .project_root, de ~/.gemini/projects.json
// o comparando el SHA-256 de rutas conocidas (lo resuelve core/index.js).
const u = require('../util');

async function readProjectsJson(dataDir) {
  try {
    const j = JSON.parse(await u.fsp.readFile(u.path.join(dataDir, 'projects.json'), 'utf8'));
    const byName = {};
    for (const [p, name] of Object.entries(j.projects || {})) byName[name] = p;
    return byName;
  } catch { return {}; }
}

async function parse(file, slotCwd) {
  const stat = await u.fsp.stat(file);
  const j = JSON.parse(await u.fsp.readFile(file, 'utf8'));
  const msgs = Array.isArray(j.messages) ? j.messages : [];
  let prompt = '';
  for (const m of msgs) {
    if (m && m.type === 'user') { prompt = u.cleanPrompt(u.contentText(m.content)); if (prompt) break; }
  }
  const title = j.summary || j.title || prompt;
  if (!j.sessionId || !title) return null;
  const s = {
    id: j.sessionId,
    title,
    mtime: Date.parse(j.lastUpdated || '') || stat.mtimeMs,
    cwd: j.cwd || j.projectRoot || slotCwd,
    gitBranch: j.gitBranch,
    src: file,
  };
  if (!s.cwd && j.projectHash) s.cwdHash = { algo: 'sha256', value: j.projectHash };
  return s;
}

module.exports = {
  id: 'gemini',
  name: 'Gemini CLI',
  badge: 'GM',
  color: '#4b8bf5',
  defaultCommand: 'gemini',
  defaultDataDir: () => (process.env.GEMINI_CLI_HOME ? u.path.join(process.env.GEMINI_CLI_HOME, '.gemini') : u.path.join(u.home(), '.gemini')),
  resumeArgs: (id) => ['--resume', id],
  compactCommand: '/compress',
  newArgs: () => [],
  trashPaths: (s) => [s.src],
  async list(dataDir) {
    const tmp = u.path.join(dataDir, 'tmp');
    const byName = await readProjectsJson(dataDir);
    let slots = [];
    try { slots = await u.fsp.readdir(tmp, { withFileTypes: true }); } catch { return []; }
    const jobs = [];
    for (const d of slots) {
      if (!d.isDirectory()) continue;
      const slot = u.path.join(tmp, d.name);
      let slotCwd = byName[d.name];
      try { slotCwd = (await u.fsp.readFile(u.path.join(slot, '.project_root'), 'utf8')).trim() || slotCwd; } catch { /* sin marcador */ }
      let files = [];
      try { files = await u.fsp.readdir(u.path.join(slot, 'chats')); } catch { continue; }
      for (const f of files) {
        if (f.startsWith('session-') && f.endsWith('.json')) jobs.push([u.path.join(slot, 'chats', f), slotCwd]);
      }
    }
    return (await u.mapLimit(jobs, 16, ([f, c]) => parse(f, c))).filter(Boolean);
  },
};
