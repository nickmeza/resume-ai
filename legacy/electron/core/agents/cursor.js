// Cursor CLI (cursor-agent) — ~/.cursor/chats/<md5(carpeta)>/<id>/store.db  (EXPERIMENTAL)
// Formato no documentado: meta(key, value) con JSON en hexadecimal.
const u = require('../util');

function decodeMeta(v) {
  const s = Buffer.isBuffer(v) || v instanceof Uint8Array ? Buffer.from(v).toString('utf8') : String(v);
  for (const cand of [s, /^[0-9a-f]+$/i.test(s) && s.length % 2 === 0 ? Buffer.from(s, 'hex').toString('utf8') : null]) {
    if (!cand) continue;
    try { return JSON.parse(cand); } catch { /* sigue */ }
  }
  return null;
}

async function parse(dbFile) {
  const hashDir = u.path.basename(u.path.dirname(u.path.dirname(dbFile)));
  const stat = await u.fsp.stat(dbFile);
  let mtime = stat.mtimeMs;
  try { mtime = Math.max(mtime, (await u.fsp.stat(`${dbFile}-wal`)).mtimeMs); } catch { /* sin wal */ }
  const db = u.openSqlite(dbFile);
  let meta = {};
  let cwd;
  let prompt = '';
  try {
    for (const r of db.prepare('SELECT * FROM meta').all()) {
      const m = decodeMeta(r.value);
      if (m && typeof m === 'object') meta = { ...meta, ...m };
    }
    try {
      for (const r of db.prepare('SELECT data FROM blobs LIMIT 40').all()) {
        const txt = Buffer.from(r.data || []).toString('utf8');
        if (!cwd) { const m = txt.match(/Workspace Path:\s*([^\n\r<"\\]+)/); if (m) cwd = m[1].trim(); }
        if (!prompt) { const q = txt.match(/<user_query>([\s\S]*?)<\/user_query>/); if (q) prompt = q[1].trim().replace(/\s+/g, ' '); }
      }
    } catch { /* tabla distinta */ }
  } finally {
    db.close();
  }
  const id = meta.agentId || u.path.basename(u.path.dirname(dbFile));
  const title = meta.name || prompt || '(chat de Cursor)';
  const s = { id, title, cwd, mtime, src: u.path.dirname(dbFile) };
  if (!cwd) s.cwdHash = { algo: 'md5', value: hashDir };
  return s;
}

module.exports = {
  id: 'cursor',
  name: 'Cursor CLI',
  badge: 'CU',
  color: '#8a8a8a',
  experimental: true,
  defaultCommand: 'cursor-agent',
  defaultDataDir: () => u.path.join(u.home(), '.cursor'),
  extraDataDirs: () => (u.isWin && process.env.APPDATA ? [u.path.join(process.env.APPDATA, 'Cursor')] : []),
  resumeArgs: (id) => ['--resume', id],
  newArgs: () => [],
  trashPaths: (s) => [s.src],
  async list(dataDir) {
    const dirs = [dataDir, ...this.extraDataDirs()];
    const files = [];
    for (const d of dirs) await u.walk(u.path.join(d, 'chats'), (n) => n === 'store.db', files);
    return (await u.mapLimit(files, 8, parse)).filter(Boolean);
  },
};
