// OpenAI Codex CLI — ~/.codex/sessions/AAAA/MM/DD/rollout-<fecha>-<uuid>.jsonl
const u = require('../util');

const UUID_TAIL = /([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\.jsonl$/i;

async function parse(file) {
  const { stat, lines } = await u.readJsonlEdges(file, { head: 512 * 1024, tail: 64 * 1024 });
  if (!stat.size) return null;
  const s = { mtime: stat.mtimeMs, src: file };
  const m = file.match(UUID_TAIL);
  if (m) s.id = m[1];
  for (const o of lines) {
    const p = o.payload || {};
    if (o.type === 'session_meta') {
      s.id = p.id || p.session_id || s.id;
      s.cwd = s.cwd || p.cwd;
      s.gitBranch = s.gitBranch || (p.git && p.git.branch);
    }
    if (!s.cwd && o.type === 'turn_context' && p.cwd) s.cwd = p.cwd;
    if (o.type === 'event_msg' && p.type === 'thread_name_updated' && (p.name || p.thread_name)) s.customTitle = p.name || p.thread_name;
    if (!s.prompt && o.type === 'event_msg' && p.type === 'user_message') s.prompt = u.cleanPrompt(p.message);
    if (!s.fallback && o.type === 'response_item' && p.type === 'message' && p.role === 'user') {
      s.fallback = u.cleanPrompt(u.contentText(p.content));
    }
  }
  s.title = s.customTitle || s.prompt || s.fallback;
  return s.id && s.title ? s : null;
}

module.exports = {
  id: 'codex',
  name: 'Codex CLI',
  badge: 'CX',
  color: '#10a37f',
  defaultCommand: 'codex',
  defaultDataDir: () => process.env.CODEX_HOME || u.path.join(u.home(), '.codex'),
  resumeArgs: (id) => ['resume', id],
  compactCommand: '/compact',
  newArgs: () => [],
  trashPaths: (s) => [s.src],
  async list(dataDir) {
    const files = await u.walk(u.path.join(dataDir, 'sessions'), (n) => n.startsWith('rollout-') && n.endsWith('.jsonl'));
    return (await u.mapLimit(files, 32, parse)).filter(Boolean);
  },
};
