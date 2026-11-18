// GitHub Copilot CLI — ~/.copilot/session-state/<uuid>/{workspace.yaml, events.jsonl}
const u = require('../util');

function parseYaml(text) {
  // YAML simple "clave: valor" (suficiente para workspace.yaml)
  const o = {};
  for (const line of text.split(/\r?\n/)) {
    const m = line.match(/^([A-Za-z_][\w-]*):\s*(.*)$/);
    if (m) o[m[1]] = m[2].replace(/^['"]|['"]$/g, '').trim();
  }
  return o;
}

async function parse(dir) {
  const id = u.path.basename(dir);
  const ws = u.path.join(dir, 'workspace.yaml');
  const ev = u.path.join(dir, 'events.jsonl');
  let y = {};
  let mtime = 0;
  try { y = parseYaml(await u.fsp.readFile(ws, 'utf8')); mtime = (await u.fsp.stat(ws)).mtimeMs; } catch { /* sin yaml */ }
  let prompt = '';
  if (await u.exists(ev)) {
    const { stat, lines } = await u.readJsonlEdges(ev, { head: 256 * 1024, tail: 0 });
    mtime = Math.max(mtime, stat.mtimeMs);
    for (const o of lines) {
      if (!/user/i.test(o.type || '')) continue;
      const d = o.data || o.payload || o;
      prompt = u.cleanPrompt(d.content || d.text || d.message || u.contentText(d.content));
      if (prompt) break;
    }
  }
  const title = y.name || y.title || y.summary || prompt;
  if (!title) return null;
  const updated = Date.parse(y.updated_at || y.updatedAt || '');
  return {
    id: y.id || id,
    title,
    cwd: y.cwd || y.git_root || y.gitRoot,
    gitBranch: y.branch || y.git_branch,
    mtime: Math.max(mtime, Number.isNaN(updated) ? 0 : updated),
    src: dir,
  };
}

module.exports = {
  id: 'copilot',
  name: 'GitHub Copilot CLI',
  badge: 'GH',
  color: '#a371f7',
  defaultCommand: 'copilot',
  defaultDataDir: () => process.env.COPILOT_HOME || u.path.join(u.home(), '.copilot'),
  resumeArgs: (id) => [`--resume=${id}`],
  // Sin compactCommand confirmado; se puede activar con agents.copilot.compactCommand en config.json
  newArgs: () => [],
  trashPaths: (s) => [s.src],
  async list(dataDir) {
    const root = u.path.join(dataDir, 'session-state');
    let dirs = [];
    try { dirs = await u.fsp.readdir(root, { withFileTypes: true }); } catch { return []; }
    const paths = dirs.filter((d) => d.isDirectory()).map((d) => u.path.join(root, d.name));
    return (await u.mapLimit(paths, 32, parse)).filter(Boolean);
  },
};
