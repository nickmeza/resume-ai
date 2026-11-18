// Utilidades compartidas por todos los proveedores (Node puro, sin Electron:
// el mismo código lo usará la futura versión TUI).
const fs = require('fs');
const fsp = fs.promises;
const os = require('os');
const path = require('path');
const crypto = require('crypto');

const home = () => os.homedir();
const isWin = process.platform === 'win32';

async function exists(p) {
  try { await fsp.access(p); return true; } catch { return false; }
}

async function readSlice(file, start, length) {
  const fh = await fsp.open(file, 'r');
  try {
    const buf = Buffer.alloc(length);
    const { bytesRead } = await fh.read(buf, 0, length, start);
    return buf.subarray(0, bytesRead).toString('utf8');
  } finally {
    await fh.close();
  }
}

/** Itera las líneas JSON del inicio (y opcionalmente el final) de un .jsonl sin leerlo entero. */
async function readJsonlEdges(file, { head = 256 * 1024, tail = 64 * 1024 } = {}) {
  const stat = await fsp.stat(file);
  const chunks = [await readSlice(file, 0, Math.min(stat.size, head))];
  if (tail && stat.size > head) {
    const start = Math.max(head, stat.size - tail);
    chunks.push(await readSlice(file, start, stat.size - start));
  }
  const out = [];
  for (const chunk of chunks) {
    for (const line of chunk.split('\n')) {
      if (!line.trim()) continue;
      try { out.push(JSON.parse(line)); } catch { /* línea cortada */ }
    }
  }
  return { stat, lines: out };
}

/** Texto de un "content" que puede ser string, [{type:'text',text}] o [{text}] */
function contentText(content) {
  if (typeof content === 'string') return content;
  if (Array.isArray(content)) {
    for (const c of content) {
      if (!c) continue;
      if (typeof c === 'string') return c;
      if (typeof c.text === 'string' && (!c.type || /text/.test(c.type))) return c.text;
    }
  }
  if (content && typeof content.text === 'string') return content.text;
  return '';
}

/** Limpia un prompt para usarlo como título. Devuelve '' si es un mensaje interno. */
function cleanPrompt(text) {
  const t = (text || '').trim();
  if (!t) return '';
  if (t.startsWith('<')) {
    const cmd = t.match(/<command-name>([^<]+)<\/command-name>/);
    const args = t.match(/<command-args>([^<]*)<\/command-args>/);
    if (cmd && args && args[1].trim()) return `${cmd[1].trim()} ${args[1].trim()}`;
    const q = t.match(/<user_query>([\s\S]*?)<\/user_query>/);
    if (q) return q[1].trim().replace(/\s+/g, ' ');
    return '';
  }
  if (/^# AGENTS\.md instructions/i.test(t)) return '';
  return t.replace(/\s+/g, ' ');
}

async function walk(dir, filter, out = [], depth = 0) {
  if (depth > 6) return out;
  let entries;
  try { entries = await fsp.readdir(dir, { withFileTypes: true }); } catch { return out; }
  for (const e of entries) {
    const full = path.join(dir, e.name);
    if (e.isDirectory()) await walk(full, filter, out, depth + 1);
    else if (filter(e.name)) out.push(full);
  }
  return out;
}

const sha256 = (s) => crypto.createHash('sha256').update(s).digest('hex');
const md5 = (s) => crypto.createHash('md5').update(s).digest('hex');

/** Variantes de una ruta para comparar hashes (Windows es inconsistente con mayúsculas y barras). */
function pathVariants(p) {
  const set = new Set([p, p.replace(/[\\/]+$/, '')]);
  for (const v of [...set]) {
    if (/^[A-Za-z]:/.test(v)) {
      set.add(v[0].toLowerCase() + v.slice(1));
      set.add(v[0].toUpperCase() + v.slice(1));
    }
  }
  for (const v of [...set]) { set.add(v.replace(/\\/g, '/')); }
  return [...set];
}

let sqliteMod;
function openSqlite(file) {
  if (sqliteMod === undefined) {
    try {
      // Silencia el ExperimentalWarning de node:sqlite
      const orig = process.emitWarning;
      process.emitWarning = (w, ...a) => (String(w).includes('SQLite') ? undefined : orig.call(process, w, ...a));
      sqliteMod = require('node:sqlite');
      process.emitWarning = orig;
    } catch { sqliteMod = null; }
  }
  if (!sqliteMod) throw new Error('Se necesita Node 22.13+ (node:sqlite) para leer este agente');
  return new sqliteMod.DatabaseSync(file, { readOnly: true });
}

/** Ejecuta en paralelo con límite (para no abrir miles de archivos a la vez). */
async function mapLimit(items, limit, fn) {
  const out = new Array(items.length);
  let i = 0;
  const workers = Array.from({ length: Math.min(limit, items.length) }, async () => {
    while (i < items.length) {
      const idx = i++;
      try { out[idx] = await fn(items[idx]); } catch { out[idx] = null; }
    }
  });
  await Promise.all(workers);
  return out;
}

const SAFE_ID = /^[A-Za-z0-9_.:-]{1,128}$/;

module.exports = {
  home, isWin, exists, readSlice, readJsonlEdges, contentText, cleanPrompt, walk,
  sha256, md5, pathVariants, openSqlite, mapLimit, SAFE_ID, fsp, path,
};
