// opencode — ~/.local/share/opencode/opencode.db (SQLite, versiones actuales)
//            ~/.local/share/opencode/storage/session/<proyecto>/ses_*.json (versiones antiguas)
const u = require('../util');

function fromDb(file) {
  const db = u.openSqlite(file);
  try {
    const rows = db.prepare('SELECT * FROM session').all();
    return rows.map((r) => {
      if (r.parent_id || r.parentID || r.time_archived) return null; // subagentes / archivadas
      const updated = Number(r.time_updated || r.time_created || 0);
      return {
        id: r.id,
        title: r.title || '(sin título)',
        cwd: r.directory,
        mtime: updated < 1e12 ? updated * 1000 : updated,
      };
    }).filter(Boolean);
  } finally {
    db.close();
  }
}

async function fromJson(dataDir) {
  const files = await u.walk(u.path.join(dataDir, 'storage', 'session'), (n) => n.startsWith('ses_') && n.endsWith('.json'));
  return (await u.mapLimit(files, 32, async (f) => {
    const j = JSON.parse(await u.fsp.readFile(f, 'utf8'));
    if (j.parentID) return null;
    const t = j.time || {};
    return { id: j.id, title: j.title || '(sin título)', cwd: j.directory, mtime: t.updated || t.created || (await u.fsp.stat(f)).mtimeMs };
  })).filter(Boolean);
}

module.exports = {
  id: 'opencode',
  name: 'opencode',
  badge: 'OC',
  color: '#c9a227',
  defaultCommand: 'opencode',
  defaultDataDir: () => u.path.join(process.env.XDG_DATA_HOME || u.path.join(u.home(), '.local', 'share'), 'opencode'),
  resumeArgs: (id) => ['--session', id],
  compactCommand: '/compact',
  newArgs: () => [],
  // Sin trashPaths: las sesiones viven en una base SQLite compartida; borrarlas desde fuera es arriesgado.
  async list(dataDir) {
    const byId = new Map();
    for (const s of await fromJson(dataDir)) byId.set(s.id, s);
    const dbFile = u.path.join(dataDir, 'opencode.db');
    if (await u.exists(dbFile)) for (const s of fromDb(dbFile)) byId.set(s.id, s);
    return [...byId.values()];
  },
};
