// Claude Code — ~/.claude/projects/<carpeta-codificada>/<uuid>.jsonl
const u = require('../util');

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

// Campo "entrypoint" de cada línea: desde dónde se usó Claude Code
const ORIGINS = { cli: 'Terminal', 'claude-vscode': 'VS Code', 'claude-desktop': 'App de escritorio', 'claude-jetbrains': 'JetBrains' };
const originLabel = (e) => ORIGINS[e] || e;

async function parse(file) {
  const id = u.path.basename(file, '.jsonl');
  if (!UUID.test(id)) return null; // ignora agent-*.jsonl (subagentes)
  const { stat, lines } = await u.readJsonlEdges(file);
  if (!stat.size) return null;
  const s = { id, mtime: stat.mtimeMs, src: file };
  for (const o of lines) {
    if (o.type === 'summary' && o.summary && !s.summary) s.summary = o.summary;
    if (o.type === 'custom-title' && o.customTitle) s.customTitle = o.customTitle;
    if (!s.cwd && o.cwd) s.cwd = o.cwd;
    if (!s.gitBranch && o.gitBranch) s.gitBranch = o.gitBranch;
    if (o.entrypoint) { s.origin = s.origin || o.entrypoint; s.lastOrigin = o.entrypoint; } // dónde se creó / dónde se usó por última vez
    if (!s.prompt && o.type === 'user' && !o.isMeta && !o.isSidechain && o.message) {
      s.prompt = u.cleanPrompt(u.contentText(o.message.content));
    }
  }
  s.title = s.customTitle || s.summary || s.prompt;
  if (s.origin) {
    s.originLabel = originLabel(s.origin);
    s.lastOriginLabel = originLabel(s.lastOrigin);
  }
  delete s.lastOrigin;
  return s.title ? s : null;
}

/** Carpeta donde Claude guarda las sesiones de un proyecto: la ruta con todo lo que no es letra o número cambiado por "-". */
function projectDir(dataDir, cwd) {
  return u.path.join(dataDir, 'projects', cwd.replace(/[^a-zA-Z0-9]/g, '-'));
}

/** ID de la sesión más nueva de una carpeta creada después de "since" (para sesiones nuevas o copias cuyo ID aún no conocemos). */
async function findNewest(dir, since, excludeIds = []) {
  let names = [];
  try { names = await u.fsp.readdir(dir); } catch { return null; }
  let best = null;
  for (const n of names) {
    const id = n.replace(/\.jsonl$/, '');
    if (!n.endsWith('.jsonl') || excludeIds.includes(id) || !UUID.test(id)) continue;
    const t = (await u.fsp.stat(u.path.join(dir, n))).birthtimeMs;
    if (t >= since - 2000 && (!best || t > best.t)) best = { id, t };
  }
  return best && best.id;
}

const IMAGE_EXT = { 'image/png': 'png', 'image/jpeg': 'jpg', 'image/gif': 'gif', 'image/webp': 'webp' };

/**
 * Imágenes que enviaste en la sesión, agrupadas por mensaje (en orden cronológico).
 * Cada mensaje trae imagePasteIds = los N de "[Image #N]", en el mismo orden que sus bloques de imagen.
 * Con outDir, cada imagen se guarda como archivo (para verla, abrirla con el visor, etc.); sin outDir solo se cuentan.
 */
async function images(s, outDir) {
  const readline = require('readline');
  const out = [];
  // La carpeta se crea antes de abrir el archivo: si se espera después, readline pierde las primeras líneas
  if (outDir) await u.fsp.mkdir(outDir, { recursive: true });
  const rl = readline.createInterface({ input: require('fs').createReadStream(s.src, { encoding: 'utf8' }), crlfDelay: Infinity });
  for await (const line of rl) {
    if (!line.includes('"image"')) continue; // descarta rápido las líneas sin imágenes
    let o;
    try { o = JSON.parse(line); } catch { continue; }
    const c = o.message && o.message.content;
    if (o.type !== 'user' || o.isSidechain || !Array.isArray(c)) continue;
    const blocks = c.filter((x) => x && x.type === 'image' && x.source && x.source.type === 'base64');
    if (!blocks.length) continue;
    const ids = Array.isArray(o.imagePasteIds) ? o.imagePasteIds : [];
    let text = '';
    for (const x of c) if (x.type === 'text' && (text = u.cleanPrompt(x.text))) break;
    const msg = { uuid: o.uuid, timestamp: Date.parse(o.timestamp) || 0, text, origin: o.entrypoint && originLabel(o.entrypoint), images: [] };
    for (const [i, b] of blocks.entries()) {
      const n = ids[i] != null ? ids[i] : null;
      const img = { n, mime: b.source.media_type, size: Math.round((b.source.data.length * 3) / 4) };
      if (outDir) {
        const name = `${String(o.uuid).slice(0, 8)}-${i + 1}${n != null ? `-img${n}` : ''}.${IMAGE_EXT[b.source.media_type] || 'png'}`;
        img.path = u.path.join(outDir, name);
        if (!(await u.exists(img.path))) await u.fsp.writeFile(img.path, Buffer.from(b.source.data, 'base64'));
      }
      msg.images.push(img);
    }
    out.push(msg);
  }
  return out;
}

module.exports = {
  id: 'claude',
  name: 'Claude Code',
  badge: 'CC',
  color: '#d97757',
  defaultCommand: 'claude',
  defaultDataDir: () => process.env.CLAUDE_CONFIG_DIR || u.path.join(u.home(), '.claude'),
  resumeArgs: (id) => ['--resume', id],
  // "Continuar desde una nueva sesión": copia la sesión con otro ID y la compacta; la original queda intacta
  forkArgs: (id) => ['--resume', id, '--fork-session'],
  compactCommand: '/compact',
  /** ID de la copia creada con --fork-session: el .jsonl más nuevo de la misma carpeta creado después de "since". */
  findFork(s, since) {
    return findNewest(u.path.dirname(s.src), since, [s.id]);
  },
  projectDir,
  findNewest,
  images,
  newArgs: () => [],
  // El .jsonl y la carpeta <id>/ con subagentes y resultados de herramientas
  trashPaths: (s) => [s.src, s.src.replace(/\.jsonl$/i, '')],
  /** Igual que /rename de Claude Code: agrega una línea custom-title (conserva la fecha para no reordenar). */
  async rename(s, title) {
    const stat = await u.fsp.stat(s.src);
    const last = stat.size ? await u.readSlice(s.src, stat.size - 1, 1) : '\n';
    const line = JSON.stringify({ type: 'custom-title', customTitle: title, sessionId: s.id });
    await u.fsp.appendFile(s.src, `${last === '\n' ? '' : '\n'}${line}\n`);
    await u.fsp.utimes(s.src, stat.atimeMs / 1000, stat.mtimeMs / 1000);
  },
  async list(dataDir) {
    const root = u.path.join(dataDir, 'projects');
    const files = await u.walk(root, (n) => n.endsWith('.jsonl'));
    return (await u.mapLimit(files.filter((f) => !f.includes(`${u.path.sep}subagents${u.path.sep}`)), 32, parse)).filter(Boolean);
  },
};
