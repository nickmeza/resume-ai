/* global Terminal, FitAddon */
const $ = (s) => document.querySelector(s);
const t = (key, vars) => window.i18n.t(key, vars);
const plural = (key, count, vars) => window.i18n.plural(key, count, vars);

// ---------------- Keyboard shortcuts (changed in ⌨) ----------------
const SHORTCUTS = [
  { id: 'tab.next', def: 'Shift+Right' },
  { id: 'tab.prev', def: 'Shift+Left' },
  { id: 'tab.moveRight', def: 'Ctrl+Shift+Right' },
  { id: 'tab.moveLeft', def: 'Ctrl+Shift+Left' },
  ...Array.from({ length: 9 }, (_, i) => ({ id: `tab.goto${i + 1}`, def: `Ctrl+${i + 1}` })),
  { id: 'tab.newShell', def: 'Ctrl+Shift+T' },
  { id: 'tab.close', def: 'Ctrl+W' },
  { id: 'session.next', def: 'Shift+Down' },
  { id: 'session.prev', def: 'Shift+Up' },
  { id: 'session.rename', def: 'F2' },
  { id: 'terminal.reload', def: 'Ctrl+Shift+R' },
  { id: 'sessions.refresh', def: 'F5' },
  { id: 'search.focus', def: 'Ctrl+K' },
];
const DEFAULT_SHORTCUTS = Object.fromEntries(SHORTCUTS.map((s) => [s.id, s.def]));

function shortcutLabel(id) {
  const m = id.match(/^tab\.goto(\d)$/);
  return m ? t('ui.shortcut.tab.goto', { n: m[1] }) : t(`ui.shortcut.${id}`);
}

// Browser shortcuts of the web view that would reload or print the page (reloading would close every terminal)
const BLOCKED_BROWSER_KEYS = new Set(['ctrl+r', 'ctrl+shift+r', 'f5', 'ctrl+f5', 'ctrl+p', 'ctrl+f', 'f3', 'ctrl+g', 'ctrl+u', 'ctrl+j', 'ctrl+h', 'f7']);

const state = {
  config: null,
  agents: [],            // from agents_info
  agentById: {},
  sessions: [],
  terms: new Map(),      // key -> { term, fit, wrap, info, alive }
  order: [],             // tab keys, in order
  activeKey: null,
  cursorKey: null,       // session highlighted in the list (Shift+↑/↓)
  collapsed: new Set(JSON.parse(localStorage.getItem('collapsed') || '[]')),
  newCounter: 0,
  compactCounter: 0,
  shellCounter: 0,
  shortcuts: {},         // id -> shortcut
  keymap: new Map(),     // normalized lowercase shortcut -> id
  draft: null,           // shortcuts being edited in the settings modal
  recording: null,       // id of the shortcut being recorded
  renaming: false,
  galleryOpen: (() => { try { return localStorage.getItem('gallery') === '1'; } catch { return false; } })(),
  galleryTimer: null,
  navTimer: null,
};

// ---------------- Utilities ----------------
function basename(p) {
  if (!p) return '';
  const parts = p.replace(/[\\/]+$/, '').split(/[\\/]/);
  return parts[parts.length - 1] || p;
}

function timeAgo(ms) {
  const s = Math.floor((Date.now() - ms) / 1000);
  if (s < 60) return t('time.now');
  const m = Math.floor(s / 60); if (m < 60) return t('time.minutes', { n: m });
  const h = Math.floor(m / 60); if (h < 24) return t('time.hours', { n: h });
  const d = Math.floor(h / 24); if (d < 30) return t('time.days', { n: d });
  return new Date(ms).toLocaleDateString(window.i18n.lang);
}

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

function badge(agentId) {
  const a = state.agentById[agentId];
  const b = el('span', 'badge', a ? a.badge : '?');
  b.style.background = a ? a.color : '#888';
  b.title = a ? a.name : agentId;
  return b;
}

function setStatus(text) { $('#status').textContent = text; }

/** Display name of an origin code (`claude-vscode` → "VS Code"); unknown codes are shown as-is. */
function originLabel(code) {
  const key = `origin.${code}`;
  const label = t(key);
  return label === key ? code : label;
}

const sessionByKey = (key) => state.sessions.find((s) => s.key === key);
const displayTitle = (s) => (state.config.aliases && state.config.aliases[s.key]) || s.title || t('session.untitled');

/** Compact command of an agent ('' = not supported). Can be changed with agents.<id>.compactCommand. */
function compactCommandOf(agentId) {
  const cfg = state.config.agents[agentId] || {};
  return cfg.compactCommand != null ? String(cfg.compactCommand).trim() : (state.agentById[agentId]?.compactCommand || '');
}

// ---------------- Shortcuts: normalization ----------------
const CODE_KEYS = {
  ArrowLeft: 'Left', ArrowRight: 'Right', ArrowUp: 'Up', ArrowDown: 'Down', Space: 'Space',
  Backquote: '`', Minus: '-', Equal: '=', Comma: ',', Period: '.', Slash: '/', Semicolon: ';',
  Quote: "'", BracketLeft: '[', BracketRight: ']', Backslash: '\\',
};
const MOD_ORDER = ['Ctrl', 'Alt', 'Shift', 'Meta'];

/** Key name by physical position (Shift+1 is still "1", not "!"). */
function keyName(e) {
  const c = e.code || '';
  if (/^Key[A-Z]$/.test(c)) return c.slice(3);
  if (/^Digit\d$/.test(c)) return c.slice(5);
  if (/^Numpad\d$/.test(c)) return c.slice(6);
  if (CODE_KEYS[c]) return CODE_KEYS[c];
  if (['Control', 'Shift', 'Alt', 'Meta', 'AltGraph'].includes(e.key)) return null;
  return e.key.length === 1 ? e.key.toUpperCase() : e.key;
}

function eventToAccel(e) {
  const k = keyName(e);
  if (!k) return null;
  return [e.ctrlKey && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift', e.metaKey && 'Meta', k].filter(Boolean).join('+');
}

/** "ctrl+shift+r" → "Ctrl+Shift+R" (accepts what people type by hand in config.json). */
function normalizeAccel(s) {
  const mods = new Set();
  let key = '';
  for (const p of String(s || '').split('+').map((x) => x.trim()).filter(Boolean)) {
    const l = p.toLowerCase();
    if (['ctrl', 'control', 'cmdorctrl'].includes(l)) mods.add('Ctrl');
    else if (['alt', 'option'].includes(l)) mods.add('Alt');
    else if (l === 'shift') mods.add('Shift');
    else if (['meta', 'cmd', 'command', 'win', 'super'].includes(l)) mods.add('Meta');
    else key = p.length === 1 ? p.toUpperCase() : p[0].toUpperCase() + p.slice(1);
  }
  return key ? [...MOD_ORDER.filter((m) => mods.has(m)), key].join('+') : '';
}

function applyShortcuts() {
  state.shortcuts = { ...DEFAULT_SHORTCUTS, ...(state.config.shortcuts || {}) };
  state.keymap = new Map();
  for (const [id, accel] of Object.entries(state.shortcuts)) {
    const n = normalizeAccel(accel);
    if (n) state.keymap.set(n.toLowerCase(), id);
  }
}

const shortcutOrNone = (id) => state.shortcuts[id] || t('ui.noShortcut');

// ---------------- Resizable 30/70 splitter ----------------
function applyRatio(r) {
  $('#sidebar').style.flexBasis = `${(r * 100).toFixed(2)}%`;
  fitActive();
}

function setupSplitter() {
  const splitter = $('#splitter');
  let dragging = false;
  splitter.addEventListener('mousedown', (e) => {
    dragging = true; splitter.classList.add('dragging');
    document.body.style.cursor = 'col-resize'; document.body.style.userSelect = 'none';
    e.preventDefault();
  });
  window.addEventListener('mousemove', (e) => {
    if (!dragging) return;
    const r = Math.min(0.7, Math.max(0.12, e.clientX / window.innerWidth));
    state.config.sidebarRatio = r;
    applyRatio(r);
  });
  window.addEventListener('mouseup', () => {
    if (!dragging) return;
    dragging = false; splitter.classList.remove('dragging');
    document.body.style.cursor = ''; document.body.style.userSelect = '';
    window.api.setConfig({ sidebarRatio: Number(state.config.sidebarRatio.toFixed(3)) });
  });
  splitter.addEventListener('dblclick', () => {
    state.config.sidebarRatio = 0.3; applyRatio(0.3);
    window.api.setConfig({ sidebarRatio: 0.3 });
  });
}

// ---------------- Agent picker ----------------
async function openAgentsModal(firstRun = false) {
  $('#modal').classList.remove('hidden');
  $('#btn-cancel').classList.toggle('hidden', firstRun);
  const rows = $('#agent-rows');
  rows.replaceChildren(el('p', 'muted', t('ui.agents.detecting')));
  const detected = await window.api.detectAgents();
  const det = Object.fromEntries(detected.map((d) => [d.id, d]));
  rows.replaceChildren();

  for (const a of state.agents) {
    const d = det[a.id] || {};
    const cfg = state.config.agents[a.id] || {};
    const installed = !!(d.commandPath || d.count);
    const checked = firstRun ? installed : state.config.enabledAgents.includes(a.id);

    const row = el('label', 'agent-row');
    const cb = el('input'); cb.type = 'checkbox'; cb.checked = checked; cb.dataset.agent = a.id;

    const info = el('div');
    const name = el('div', 'name');
    name.append(badge(a.id), document.createTextNode(a.name));
    if (a.experimental) name.append(el('span', 'exp', t('ui.agents.experimental')));
    const st = el('div', 'st');
    st.append(el('span', d.commandPath ? 'ok' : 'bad', d.commandPath ? `✓ ${d.commandPath}` : `✗ ${t('agents.commandNotFound')}`));
    st.append(document.createTextNode(' · '));
    const n = d.count || 0;
    st.append(el('span', n ? 'ok' : '', d.dataExists ? plural('ui.agents.sessionsIn', n, { dir: d.dataDir }) : t('ui.agents.noData', { dir: d.dataDir })));
    if (d.error) st.append(el('span', 'bad', ` · ${d.error}`));
    info.append(name, st);

    const cmd = el('input'); cmd.type = 'text'; cmd.value = cfg.command || a.defaultCommand;
    cmd.placeholder = a.defaultCommand; cmd.title = t('ui.agents.commandTitle');
    cmd.dataset.cmd = a.id;
    cmd.addEventListener('focus', () => cmd.select());

    row.append(cb, info, cmd);
    rows.append(row);
  }
}

async function saveAgentsModal() {
  const enabled = [...document.querySelectorAll('#agent-rows input[type=checkbox]')]
    .filter((c) => c.checked).map((c) => c.dataset.agent);
  const agents = { ...state.config.agents };
  document.querySelectorAll('#agent-rows input[data-cmd]').forEach((i) => {
    agents[i.dataset.cmd] = { ...agents[i.dataset.cmd], command: i.value.trim() || state.agentById[i.dataset.cmd].defaultCommand };
  });
  const filters = { ...state.config.filters, agent: (state.config.filters.agent || []).filter((id) => enabled.includes(id)) };
  state.config = await window.api.setConfig({ enabledAgents: enabled, agents, onboarded: true, filters });
  $('#modal').classList.add('hidden');
  await loadSessions();
}

// ---------------- Filters (funnel button next to the search box) ----------------
// Each group says which value a session has and which options to show. To add a filter, add a group here.
// Options inside a group combine with OR; groups combine with AND. Nothing checked = no filtering.
const FILTER_GROUPS = [
  {
    id: 'agent',
    label: () => t('filter.agents'),
    valueOf: (s) => s.agent,
    options: () => state.config.enabledAgents.filter((id) => state.agentById[id])
      .map((id) => ({ value: id, label: state.agentById[id].name, color: state.agentById[id].color })),
  },
  {
    id: 'origin',
    label: () => t('ui.filter.originGroup'),
    valueOf: (s) => s.origin || '',
    options: (sessions) => {
      const seen = new Set(sessions.filter((s) => s.origin).map((s) => s.origin));
      const opts = [...seen].map((value) => ({ value, label: originLabel(value) }));
      if (opts.length && sessions.some((s) => !s.origin)) opts.push({ value: '', label: t('filter.noOrigin') });
      return opts;
    },
  },
];

const selectedOf = (groupId) => (state.config.filters && state.config.filters[groupId]) || [];

/** Does the session pass the filters? (skip = group to ignore, to count that group's options) */
function matchFilters(s, skip) {
  return FILTER_GROUPS.every((g) => {
    const sel = selectedOf(g.id);
    return g.id === skip || !sel.length || sel.includes(g.valueOf(s));
  });
}

function setFilters(filters) {
  state.config.filters = filters;
  window.api.setConfig({ filters });
  renderFilters();
  if (!$('#filter-pop').classList.contains('hidden')) renderFilterPop();
  renderList();
}

function toggleFilter(groupId, value, on) {
  const sel = new Set(selectedOf(groupId));
  if (on) sel.add(value); else sel.delete(value);
  setFilters({ ...state.config.filters, [groupId]: [...sel] });
}

function optionLabel(g, value) {
  const o = g.options(state.sessions).find((x) => x.value === value);
  return o ? o.label : value || t('ui.filter.noData');
}

/** Filter button (with the number of active filters) and the row of active filters under the search box. */
function renderFilters() {
  const active = FILTER_GROUPS.flatMap((g) => selectedOf(g.id).map((value) => ({ g, value })));
  const btn = $('#btn-filter');
  btn.classList.toggle('on', active.length > 0);
  btn.querySelector('.count').textContent = active.length || '';
  btn.title = active.length ? plural('ui.filter.buttonActive', active.length) : t('filter.title');

  const box = $('#filter-summary');
  box.replaceChildren();
  box.classList.toggle('hidden', !active.length);
  for (const { g, value } of active) {
    const c = el('button', 'chip on');
    const a = g.id === 'agent' ? state.agentById[value] : null;
    if (a) { const sw = el('span', 'sw'); sw.style.background = a.color; c.append(sw); }
    c.append(document.createTextNode(`${optionLabel(g, value)} ×`));
    c.title = t('ui.filter.remove');
    c.addEventListener('click', () => toggleFilter(g.id, value, false));
    box.append(c);
  }
}

function renderFilterPop() {
  const pop = $('#filter-pop');
  pop.replaceChildren();
  for (const g of FILTER_GROUPS) {
    const opts = g.options(state.sessions);
    if (!opts.length) continue;
    const sel = new Set(selectedOf(g.id));
    const base = state.sessions.filter((s) => matchFilters(s, g.id));
    const sec = el('div', 'fp-group');
    sec.append(el('div', 'fp-title', g.label()));
    for (const o of opts) {
      const row = el('label', 'fp-opt');
      const cb = el('input');
      cb.type = 'checkbox';
      cb.checked = sel.has(o.value);
      cb.addEventListener('change', () => toggleFilter(g.id, o.value, cb.checked));
      row.append(cb);
      if (o.color) { const sw = el('span', 'sw'); sw.style.background = o.color; row.append(sw); }
      row.append(el('span', 'fp-label', o.label), el('span', 'fp-count', String(base.filter((s) => g.valueOf(s) === o.value).length)));
      sec.append(row);
    }
    pop.append(sec);
  }
  const foot = el('div', 'fp-foot');
  const clear = el('button', null, t('ui.filter.clear'));
  clear.disabled = !FILTER_GROUPS.some((g) => selectedOf(g.id).length);
  clear.addEventListener('click', () => setFilters({}));
  foot.append(clear);
  pop.append(foot);
}

function hideFilterPop() { $('#filter-pop').classList.add('hidden'); }

function toggleFilterPop() {
  const pop = $('#filter-pop');
  if (!pop.classList.contains('hidden')) { hideFilterPop(); return; }
  renderFilterPop();
  pop.classList.remove('hidden');
  const r = $('#btn-filter').getBoundingClientRect();
  pop.style.top = `${r.bottom + 4}px`;
  pop.style.left = `${Math.max(4, r.right - pop.offsetWidth)}px`;
}

// ---------------- Sessions list ----------------
async function loadSessions() {
  if (!state.config.enabledAgents.length) {
    state.sessions = [];
    setStatus(t('ui.status.noAgents'));
    renderFilters(); renderList();
    return;
  }
  setStatus(t('ui.status.loading'));
  const res = await window.api.listSessions();
  state.sessions = res.sessions;
  const err = res.errors.map((e) => `${state.agentById[e.agent]?.name || e.agent}: ${e.error}`).join(' · ');
  setStatus(plural('agents.sessions', res.sessions.length) + (err ? ` · ⚠ ${err}` : ''));
  renderFilters();
  renderList();
}

/** Groups by folder; on Windows "C:\x" and "c:\x\" are the same folder. */
function groupSessions(list) {
  const groups = new Map();
  for (const s of list) {
    const key = s.cwd ? s.cwd.replace(/[\\/]+$/, '').replace(/\//g, '\\').toLowerCase() : '__none__';
    if (!groups.has(key)) groups.set(key, { key, cwd: s.cwd, name: s.cwd ? basename(s.cwd) : t('ui.group.unknownFolder'), items: [] });
    groups.get(key).items.push(s);
  }
  return [...groups.values()];
}

function renderList() {
  if (state.renaming) return; // don't destroy the text field while typing
  const q = $('#search').value.trim().toLowerCase();
  const filtered = state.sessions.filter((s) => matchFilters(s) && (!q ||
    `${displayTitle(s)} ${s.title} ${s.cwd || ''} ${s.gitBranch || ''} ${s.id} ${state.agentById[s.agent]?.name || ''} ${s.origin || ''} ${s.origin ? originLabel(s.origin) : ''}`.toLowerCase().includes(q)));

  const list = $('#list');
  list.replaceChildren();

  for (const g of groupSessions(filtered)) {
    const group = el('div', 'group' + (state.collapsed.has(g.key) && !q ? ' collapsed' : ''));
    group.dataset.key = g.key;
    const header = el('div', 'group-header');
    header.title = g.cwd || '';
    header.append(el('span', 'chev', '▾'), el('span', 'name', g.name), el('span', 'count', g.items.length));
    if (g.cwd) {
      const btnNew = el('button', null, '＋');
      btnNew.title = t('ui.group.newSession', { path: g.cwd });
      btnNew.addEventListener('click', (e) => { e.stopPropagation(); newSessionMenu(e.currentTarget, g.cwd, g.name); });
      header.append(btnNew);
      header.addEventListener('contextmenu', (e) => { e.preventDefault(); groupMenu(g, e.clientX, e.clientY); });
    }
    header.addEventListener('click', () => {
      group.classList.toggle('collapsed');
      if (group.classList.contains('collapsed')) state.collapsed.add(g.key); else state.collapsed.delete(g.key);
      localStorage.setItem('collapsed', JSON.stringify([...state.collapsed]));
    });

    const items = el('div', 'items');
    for (const s of g.items) {
      const item = el('div', 'item' + (state.activeKey === s.key ? ' active' : '')
        + (state.cursorKey === s.key ? ' cursor' : '') + (s.cwd ? '' : ' no-cwd'));
      item.dataset.key = s.key;
      const title = displayTitle(s);
      let origin = '';
      if (s.origin) {
        origin = `\n${t('ui.item.createdIn', { origin: originLabel(s.origin) })}`;
        if (s.lastOrigin && s.lastOrigin !== s.origin) origin += ` · ${t('ui.item.lastUsedIn', { origin: originLabel(s.lastOrigin) })}`;
      }
      item.title = `${title}\n\n${state.agentById[s.agent]?.name || s.agent} · ID: ${s.id}${origin}\n${s.cwd || ''}`;
      const row = el('div', 'title');
      const term = state.terms.get(s.key);
      if (term && term.alive) row.append(el('span', 'dot'));
      row.append(badge(s.agent), el('span', 't', title));
      const meta = el('div', 'meta');
      meta.append(el('span', null, timeAgo(s.mtime)));
      // Only non-terminal origins are tagged (VS Code, desktop app…) to keep the list clean
      if (s.origin && s.origin !== 'cli') meta.append(el('span', `origin origin-${s.origin.replace(/[^a-z0-9-]/gi, '')}`, originLabel(s.origin)));
      if (s.gitBranch && s.gitBranch !== 'HEAD') meta.append(el('span', null, `⎇ ${s.gitBranch}`));
      if (!s.cwd) meta.append(el('span', 'warn', t('ui.item.unknownFolder')));
      item.append(row, meta);
      item.addEventListener('click', () => openSession(s));
      item.addEventListener('contextmenu', (e) => { e.preventDefault(); sessionMenu(s, e.clientX, e.clientY); });
      items.append(item);
    }
    group.append(header, items);
    list.append(group);
  }

  const filtering = q || FILTER_GROUPS.some((g) => selectedOf(g.id).length);
  if (!filtered.length) list.append(el('div', 'item', filtering ? t('list.noResults') : t('list.empty')));
}

function markListSelection() {
  document.querySelectorAll('#list .item[data-key]').forEach((i) => {
    i.classList.toggle('active', i.dataset.key === state.activeKey);
    i.classList.toggle('cursor', i.dataset.key === state.cursorKey && i.dataset.key !== state.activeKey);
  });
}

/** Shift+↑/↓: walks the visible list. A session that already has a terminal is shown at once;
 *  otherwise it opens when you stop pressing (so no agent starts for every session you pass). */
function moveSessionCursor(dir) {
  const items = [...document.querySelectorAll('#list .item[data-key]')].filter((i) => i.offsetParent !== null);
  if (!items.length) return;
  let idx = items.findIndex((i) => i.dataset.key === state.cursorKey);
  if (idx === -1) idx = items.findIndex((i) => i.dataset.key === state.activeKey);
  idx = idx === -1 ? (dir > 0 ? 0 : items.length - 1) : Math.max(0, Math.min(items.length - 1, idx + dir));
  const key = items[idx].dataset.key;
  state.cursorKey = key;
  markListSelection();
  items[idx].scrollIntoView({ block: 'nearest' });
  clearTimeout(state.navTimer);
  const s = sessionByKey(key);
  if (!s) return;
  if (state.terms.has(key)) openSession(s);
  else state.navTimer = setTimeout(() => { if (state.cursorKey === key) openSession(s); }, 450);
}

// ---------------- Floating menus ----------------
function hideMenu() { $('#menu').classList.add('hidden'); }

/** entries: [{ label, sub?, icon?, hint?, tip?, danger?, disabled?, id?, action } | '-'] (id lets us update it later) */
function showMenu(x, y, entries) {
  const menu = $('#menu');
  menu.replaceChildren();
  menu.dataset.for = '';
  for (const it of entries) {
    if (it === '-') { menu.append(el('div', 'sep')); continue; }
    const b = el('button', it.danger ? 'danger' : null);
    if (it.id) b.dataset.id = it.id;
    if (it.icon) b.append(it.icon);
    const label = el('span', 'label');
    label.append(el('span', null, it.label));
    if (it.sub) label.append(el('span', 'sub', it.sub)); // grey note under the label
    b.append(label);
    if (it.hint) b.append(el('span', 'kbd-hint', it.hint));
    if (it.tip) b.title = it.tip;
    if (it.disabled) { b.disabled = true; b.title = it.disabled; }
    b.addEventListener('click', () => { hideMenu(); it.action(); });
    menu.append(b);
  }
  menu.style.left = '0px'; menu.style.top = '0px';
  menu.classList.remove('hidden');
  const r = menu.getBoundingClientRect();
  menu.style.left = `${Math.max(4, Math.min(x, window.innerWidth - r.width - 4))}px`;
  menu.style.top = `${Math.max(4, Math.min(y, window.innerHeight - r.height - 4))}px`;
}

function newSessionMenu(anchor, cwd, name) {
  const enabled = state.config.enabledAgents;
  if (enabled.length === 1) return openNew(enabled[0], cwd, name);
  const r = anchor.getBoundingClientRect();
  showMenu(r.left, r.bottom + 4, enabled.filter((id) => state.agentById[id]).map((id) => ({
    icon: badge(id), label: state.agentById[id].name, action: () => openNew(id, cwd, name),
  })));
}

function sessionMenu(s, x, y) {
  const a = state.agentById[s.agent];
  const compact = compactCommandOf(s.agent);
  showMenu(x, y, [
    { label: t('ui.menu.open'), action: () => openSession(s) },
    {
      label: a && a.forks ? t('ui.menu.continueFork') : t('ui.menu.continueCompact'),
      sub: a && a.forks ? t('ui.menu.continueForkSub') : t('ui.menu.continueCompactSub'),
      tip: compact ? t('ui.menu.continueTip', { cmd: compact }) : null,
      disabled: compact ? null : t('compact.notSupported', { agent: a ? a.name : s.agent }),
      action: () => continueCompacted(s),
    },
    ...(s.agent === 'claude' ? [{ id: 'images', label: t('ui.menu.images'), sub: t('ui.menu.imagesSearching'), action: () => openGalleryTab(s) }] : []),
    '-',
    {
      label: t('ui.menu.rename'), hint: state.shortcuts['session.rename'], action: () => startRename(s.key),
      tip: a && a.nativeRename ? t('rename.hintNative', { agent: a.name }) : t('rename.hintAlias', { agent: a ? a.name : s.agent }),
    },
    { label: t('ui.menu.copyId'), action: () => window.api.writeClipboard(s.id) },
    ...(s.cwd ? [
      { label: t('ui.menu.openFolder'), action: () => window.api.openFolder(s.cwd) },
      { label: t('ui.menu.openShellHere'), action: () => openShell(s.cwd) },
    ] : []),
    '-',
    {
      label: t('ui.menu.delete'), danger: true, action: () => deleteSession(s.key),
      disabled: a && !a.canDelete ? t('delete.notAllowed', { agent: a.name }) : null,
    },
  ]);
  if (s.agent !== 'claude') return;
  // Counts the images without starting the agent (and without extracting them) and updates the entry
  $('#menu').dataset.for = s.key;
  window.api.claudeImages({ key: s.key, extract: false }).then((res) => {
    const menu = $('#menu');
    const b = menu.querySelector('button[data-id="images"]');
    if (!b || menu.dataset.for !== s.key || menu.classList.contains('hidden')) return;
    const total = res.ok ? res.messages.reduce((n, m) => n + m.images.length, 0) : 0;
    const msgs = res.ok ? res.messages.length : 0;
    b.querySelector('.sub').textContent = !res.ok ? t('ui.menu.imagesError', { error: res.error })
      : total ? plural('images.count', total, { messages: msgs }) : t('images.none');
    b.disabled = !total;
  });
}

function groupMenu(g, x, y) {
  showMenu(x, y, [
    ...state.config.enabledAgents.filter((id) => state.agentById[id]).map((id) => ({
      icon: badge(id), label: t('ui.menu.newWith', { agent: state.agentById[id].name }), action: () => openNew(id, g.cwd, g.name),
    })),
    '-',
    { label: t('ui.menu.openShellHere'), action: () => openShell(g.cwd) },
    { label: t('ui.menu.openFolder'), action: () => window.api.openFolder(g.cwd) },
  ]);
}

function tabMenu(key, x, y) {
  const idx = state.order.indexOf(key);
  const s = sessionByKey(key);
  showMenu(x, y, [
    {
      label: state.terms.get(key).kind ? t('ui.menu.reload') : t('ui.menu.reloadTerminal'),
      hint: state.shortcuts['terminal.reload'], action: () => reloadTerminal(key),
    },
    ...(s ? [{ label: t('ui.menu.rename'), action: () => startRename(key) }] : []),
    '-',
    { label: t('ui.menu.moveLeft'), hint: state.shortcuts['tab.moveLeft'], disabled: idx === 0 ? t('ui.menu.alreadyFirst') : null, action: () => moveTab(key, idx - 1) },
    { label: t('ui.menu.moveRight'), hint: state.shortcuts['tab.moveRight'], disabled: idx === state.order.length - 1 ? t('ui.menu.alreadyLast') : null, action: () => moveTab(key, idx + 1) },
    '-',
    { label: t('ui.menu.close'), hint: state.shortcuts['tab.close'], action: () => closeTab(key) },
    { label: t('ui.menu.closeOthers'), disabled: state.order.length < 2 ? t('ui.menu.noOthers') : null, action: () => closeOtherTabs(key) },
  ]);
}

// ---------------- Rename / delete ----------------
function startRename(key) {
  const s = sessionByKey(key);
  if (!s || state.renaming) return;
  const item = document.querySelector(`#list .item[data-key="${CSS.escape(key)}"]`);
  if (!item) { setStatus(t('ui.rename.notVisible')); return; }
  item.closest('.group')?.classList.remove('collapsed');
  const span = item.querySelector('.t');
  const input = el('input', 'rename');
  input.value = displayTitle(s);
  span.replaceWith(input);
  state.renaming = true;
  input.focus(); input.select();

  let done = false;
  const finish = async (save) => {
    if (done) return;
    done = true;
    state.renaming = false;
    const value = input.value.trim();
    if (!save || value === displayTitle(s)) { renderList(); return; }
    const res = await window.api.renameSession(key, value);
    if (!res.ok) { setStatus(`⚠ ${res.error}`); renderList(); return; }
    state.config.aliases = res.aliases;
    await loadSessions();
    setStatus(t('rename.done'));
    const fresh = sessionByKey(key);
    const entry = state.terms.get(key);
    if (entry && fresh) { entry.info.title = displayTitle(fresh); renderTabs(); updateHeader(); }
  };
  input.addEventListener('keydown', (e) => {
    e.stopPropagation();
    if (e.key === 'Enter') finish(true);
    if (e.key === 'Escape') finish(false);
  });
  input.addEventListener('blur', () => finish(true));
  input.addEventListener('click', (e) => e.stopPropagation());
}

async function deleteSession(key) {
  const res = await window.api.deleteSession(key);
  if (res.cancelled) return;
  if (!res.ok) { setStatus(`⚠ ${res.error}`); return; }
  if (state.terms.has(key)) await closeTab(key);
  if (state.cursorKey === key) state.cursorKey = null;
  await loadSessions();
  setStatus(t('delete.done'));
}

// ---------------- Terminals ----------------
function makeTerminal(key, info) {
  const wrap = el('div', 'term-wrap hidden');
  $('#terminals').append(wrap);

  const term = new Terminal({
    fontFamily: state.config.fontFamily,
    fontSize: state.config.fontSize,
    cursorBlink: true,
    allowProposedApi: true,
    scrollback: 10000,
    theme: { background: '#1a1a1a', foreground: '#e6e6e6', cursor: '#d97757', selectionBackground: '#d9775755' },
  });
  const fit = new FitAddon.FitAddon();
  term.loadAddon(fit);
  term.open(wrap);

  // Copy / paste like Windows Terminal
  term.attachCustomKeyEventHandler((e) => {
    if (e.type !== 'keydown') return true;
    const k = e.key.toLowerCase();
    if (e.ctrlKey && k === 'c' && term.hasSelection()) {
      window.api.writeClipboard(term.getSelection()); term.clearSelection(); return false;
    }
    if (e.ctrlKey && e.shiftKey && k === 'c') {
      if (term.hasSelection()) window.api.writeClipboard(term.getSelection()); return false;
    }
    if (e.ctrlKey && k === 'v') {
      pasteClipboard(key); e.preventDefault(); return false;
    }
    return true;
  });
  wrap.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    if (term.hasSelection()) { window.api.writeClipboard(term.getSelection()); term.clearSelection(); }
    else pasteClipboard(key);
  });
  registerImageLinks(term, key);
  registerImageRefLinks(term, key);

  term.onData((d) => {
    const e = state.terms.get(key);
    if (e && e.pendingInput) {
      e.pendingInput.cancelled = true;
      e.pendingInput = null;
      setStatus(t('ui.status.autoSendCancelled'));
    }
    window.api.writePty(key, d);
  });
  term.onResize(({ cols, rows }) => window.api.resizePty(key, cols, rows));

  info.startedAt = info.startedAt || Date.now(); // to find the id of a new session later
  const entry = { term, fit, wrap, info, alive: false, outBytes: 0, lastOutputAt: 0, pendingInput: null, images: [] };
  state.terms.set(key, entry);
  state.order.push(key);
  new ResizeObserver(() => { if (state.activeKey === key) safeFit(entry); }).observe(wrap);
  return entry;
}

function safeFit(entry) {
  if (!entry.fit) return; // image/gallery tab
  try { entry.fit.fit(); } catch { /* not laid out yet */ }
}

// Dropping files on the window types their paths into the active terminal (agents attach images from the path)
function onFilesDropped(paths) {
  const key = state.activeKey;
  const entry = state.terms.get(key);
  if (!entry || !entry.term) return;
  entry.term.paste(`${paths.map(quotePath).join(' ')} `);
  for (const p of paths) if (IMAGE_RE.test(p)) addAttachment(key, p);
  entry.term.focus();
}

// ---------------- Images ----------------
const IMAGE_RE = /\.(png|jpe?g|gif|webp|bmp|svg|ico|avif)$/i;
const IMG_EXT = '(?:png|jpe?g|gif|webp|bmp|svg|ico|avif)';
// Image paths in a terminal line: quoted (may contain spaces) or bare (absolute, ~, relative)
const IMAGE_PATH_PATTERNS = [
  new RegExp(`"([^"\\r\\n]+?\\.${IMG_EXT})"`, 'gi'),
  new RegExp(`'([^'\\r\\n]+?\\.${IMG_EXT})'`, 'gi'),
  new RegExp(`((?:[A-Za-z]:[\\\\/]|\\\\\\\\|~[\\\\/])?[^\\s"'<>|*?()\\[\\]]+\\.${IMG_EXT})(?![\\w])`, 'gi'),
];

const quotePath = (p) => (/\s/.test(p) ? `"${p}"` : p);
const isImageTab = (entry) => entry && entry.kind === 'image';

/** Ctrl+V: text if there is any; if the clipboard holds an image (screenshot), saves it as PNG and pastes its path. */
async function pasteClipboard(key) {
  const entry = state.terms.get(key);
  if (!entry) return;
  const text = await window.api.readClipboard();
  if (text) { entry.term.paste(text); return; }
  const file = await window.api.saveClipboardImage();
  if (!file) return;
  entry.term.paste(`${quotePath(file)} `);
  addAttachment(key, file);
}

/** Remembers images attached in a tab (the agent may replace the path with "[Image #1]"). */
function addAttachment(key, file) {
  const entry = state.terms.get(key);
  if (!entry || entry.images.some((a) => a.path === file)) return;
  entry.images.push({ path: file, at: Date.now() });
  if (state.activeKey === key) { updateHeader(); refreshGallery(); }
}

/** Ctrl+click on an image path = image tab; Ctrl+Shift+click = system viewer. */
function registerImageLinks(term, key) {
  term.registerLinkProvider({
    provideLinks(y, callback) {
      const line = term.buffer.active.getLine(y - 1);
      if (!line) { callback(undefined); return; }
      const text = line.translateToString(true);
      const found = [];
      for (const re of IMAGE_PATH_PATTERNS) {
        re.lastIndex = 0;
        let m;
        while ((m = re.exec(text))) {
          const start = m.index + m[0].indexOf(m[1]);
          const end = start + m[1].length;
          if (!found.some((f) => start < f.end && end > f.start)) found.push({ raw: m[1], start, end });
        }
      }
      if (!found.length) { callback(undefined); return; }
      const entry = state.terms.get(key);
      window.api.resolveImages(found.map((f) => f.raw), entry && entry.info.cwd).then((resolved) => {
        const hint = t('ui.image.linkHint');
        const links = found.map((f, i) => resolved[i] && {
          range: { start: { x: f.start + 1, y }, end: { x: f.end, y } },
          text: resolved[i],
          decorations: { pointerCursor: true, underline: true },
          hover: () => setStatus(`🖼 ${resolved[i]} — ${hint}`),
          activate: (ev) => {
            if (!ev.ctrlKey && !ev.metaKey) { setStatus(hint); return; }
            if (ev.shiftKey) window.api.openPath(resolved[i]);
            else openImageTab(resolved[i]);
          },
        }).filter(Boolean);
        callback(links.length ? links : undefined);
      });
    },
  });
}

function formatSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/** Tab that shows an image. If it is already open, it is just focused. */
function openImageTab(file, title) {
  const key = `img:${file.toLowerCase()}`;
  if (state.terms.has(key)) { showTerminal(key); return; }

  const wrap = el('div', 'term-wrap image-view hidden');
  const bar = el('div', 'iv-bar');
  const meta = el('span', 'iv-info', t('ui.image.loadingShort'));
  const btnFit = el('button', null, t('ui.image.actualSize'));
  const btnCopy = el('button', null, t('ui.image.copyPath'));
  const btnDir = el('button', null, t('ui.image.reveal'));
  const btnSys = el('button', null, t('ui.image.openSystem'));
  bar.append(meta, el('span', 'spacer'), btnFit, btnCopy, btnDir, btnSys);
  const stage = el('div', 'iv-stage');
  const loading = el('div', 'iv-loading');
  loading.append(el('div', 'spinner'), el('span', 'msg', t('ui.image.loading')));
  const img = el('img', 'hidden');
  img.alt = basename(file);
  stage.append(loading, img);
  wrap.append(bar, stage);
  $('#terminals').append(wrap);

  const toggleFit = () => {
    stage.classList.toggle('actual');
    btnFit.textContent = stage.classList.contains('actual') ? t('ui.image.fit') : t('ui.image.actualSize');
  };
  btnFit.addEventListener('click', toggleFit);
  img.addEventListener('click', toggleFit);
  btnCopy.addEventListener('click', () => { window.api.writeClipboard(file); setStatus(t('ui.image.pathCopied')); });
  btnDir.addEventListener('click', () => window.api.revealPath(file));
  btnSys.addEventListener('click', () => window.api.openPath(file));

  const dir = file.replace(/[\\/][^\\/]+$/, '');
  const entry = {
    kind: 'image', wrap, alive: true, images: [],
    info: { title: title || basename(file), cwd: dir, path: file },
    view: { img, loading, meta, stage },
  };
  entry.reload = () => loadImage(entry);
  state.terms.set(key, entry);
  state.order.push(key);
  showTerminal(key);
  loadImage(entry);
}

async function loadImage(entry) {
  const { img, loading, meta } = entry.view;
  img.classList.add('hidden');
  loading.classList.remove('hidden', 'error');
  loading.querySelector('.msg').textContent = t('ui.image.loading');
  meta.textContent = t('ui.image.loadingShort');
  const res = await window.api.readImage(entry.info.path);
  if (!res.ok) {
    loading.classList.add('error');
    loading.querySelector('.msg').textContent = t('ui.image.error', { error: res.error });
    meta.textContent = '⚠';
    return;
  }
  img.onload = () => {
    loading.classList.add('hidden');
    img.classList.remove('hidden');
    meta.textContent = `${img.naturalWidth} × ${img.naturalHeight} px · ${formatSize(res.size)}`;
  };
  img.onerror = () => {
    loading.classList.add('error');
    loading.querySelector('.msg').textContent = t('ui.image.invalid');
  };
  img.src = res.dataUrl;
}

// ---------------- Claude session images ("[Image #N]") ----------------
const thumbCache = new Map(); // path -> dataUrl (extracted images never change)

async function thumbSrc(p) {
  if (thumbCache.has(p)) return thumbCache.get(p);
  const res = await window.api.readImage(p);
  if (!res.ok) return null;
  if (thumbCache.size > 200) thumbCache.delete(thumbCache.keys().next().value);
  thumbCache.set(p, res.dataUrl);
  return res.dataUrl;
}

/** Claude ids used by other tabs (so new sessions in the same folder are not confused). */
function claimedClaudeIds(key) {
  const ids = [];
  for (const [k, e] of state.terms) {
    if (e.kind || e.info.agent !== 'claude') continue;
    if (k !== key) ids.push(e.info.claudeId || e.info.sessionId);
    else if (e.info.forkOf) ids.push(e.info.sessionId); // the original of a copy
  }
  return ids.filter(Boolean);
}

/** Once the real id of a new session or a copy is known, restarting the tab resumes it. */
function adoptSessionId(entry, id) {
  entry.info.claudeId = id;
  if (entry.info.mode === 'new' || entry.info.forkOf) {
    entry.info.sessionId = id;
    if (entry.info.mode === 'new') entry.info.mode = 'resume';
    delete entry.info.forkOf;
  }
}

/** Images of a tab's Claude session, grouped by message (short cache). */
async function tabImages(key, maxAge = 3000) {
  const entry = state.terms.get(key);
  if (!entry || entry.kind || entry.info.agent !== 'claude') return null;
  if (entry.imgRes && Date.now() - entry.imgRes.at < maxAge) return entry.imgRes.res;
  const { info } = entry;
  const q = { cwd: info.cwd };
  const id = info.claudeId || (!info.forkOf && info.mode === 'resume' ? info.sessionId : null);
  if (id) q.sessionId = id;
  else { q.since = info.forkOf ? info.forkOf.since : info.startedAt; q.exclude = claimedClaudeIds(key); }
  const res = await window.api.claudeImages(q);
  if (!res.ok) return null;
  if (res.foundId) adoptSessionId(entry, res.foundId);
  entry.imgRes = { at: Date.now(), res };
  return res;
}

/** Images of the session + the ones pasted in this tab that were not sent yet. */
function imageCounts(entry) {
  const res = entry.imgRes && entry.imgRes.res;
  const msgs = res ? res.messages : [];
  const last = msgs.length ? msgs[msgs.length - 1].timestamp : 0;
  const isClaude = entry.info.agent === 'claude';
  const pending = entry.images.filter((a) => !isClaude || a.at > last).map((a) => a.path);
  const sent = msgs.reduce((n, m) => n + m.images.length, 0);
  return { res, pending, total: sent + pending.length };
}

/** The most recent image #n (Claude may repeat numbers across messages). */
function findImageRef(res, n) {
  let hit = null;
  let others = 0;
  for (let i = res.messages.length - 1; i >= 0; i--) {
    for (const img of res.messages[i].images) {
      if (img.n !== n) continue;
      if (!hit) hit = { msg: res.messages[i], img }; else others++;
    }
  }
  if (hit) hit.others = others;
  return hit;
}

const shortText = (s, n = 40) => (s && s.length > n ? `${s.slice(0, n - 1)}…` : s || '');
const imageTabTitle = (msg, img) => `${img.n != null ? `#${img.n}` : t('ui.image.word')} · ${shortText(msg.text) || timeAgo(msg.timestamp)}`;

// --- Floating preview when hovering "[Image #N]" ---
let previewSeq = 0;

function hidePreview() {
  previewSeq++;
  $('#img-preview').classList.add('hidden');
}

function placePreview(x, y) {
  const p = $('#img-preview');
  const r = p.getBoundingClientRect();
  let left = x + 16;
  let top = y + 18;
  if (left + r.width > window.innerWidth - 8) left = x - r.width - 16;
  if (top + r.height > window.innerHeight - 8) top = y - r.height - 12;
  p.style.left = `${Math.max(8, left)}px`;
  p.style.top = `${Math.max(8, top)}px`;
}

async function showRefPreview(key, n, ev) {
  const seq = ++previewSeq;
  const p = $('#img-preview');
  const { clientX: x, clientY: y } = ev;
  p.replaceChildren(el('div', 'pv-msg', t('ui.preview.searching', { n })));
  p.classList.remove('hidden');
  placePreview(x, y);
  const res = await tabImages(key);
  if (seq !== previewSeq) return;
  const hit = res && findImageRef(res, n);
  if (!hit) {
    p.replaceChildren(el('div', 'pv-msg', t('ui.preview.notSaved', { n })));
    placePreview(x, y);
    return;
  }
  const img = el('img');
  const cap = el('div', 'pv-cap');
  cap.append(el('b', null, t('ui.preview.image', { n })), document.createTextNode(` · ${timeAgo(hit.msg.timestamp)}`));
  const parts = [img, cap];
  if (hit.msg.text) parts.push(el('div', 'pv-text', hit.msg.text));
  if (hit.others) parts.push(el('div', 'pv-note', t('ui.preview.others', { count: hit.others, n })));
  parts.push(el('div', 'pv-hint', t('ui.preview.hint')));
  p.replaceChildren(...parts);
  img.onload = () => { if (seq === previewSeq) placePreview(x, y); };
  const src = await thumbSrc(hit.img.path);
  if (seq !== previewSeq) return;
  if (src) img.src = src;
  placePreview(x, y);
}

/** Links on "[Image #N]" in Claude tabs: preview on hover and Ctrl+click to open. */
function registerImageRefLinks(term, key) {
  term.registerLinkProvider({
    provideLinks(y, callback) {
      const entry = state.terms.get(key);
      const line = term.buffer.active.getLine(y - 1);
      if (!entry || entry.info.agent !== 'claude' || !line) { callback(undefined); return; }
      const text = line.translateToString(true);
      const links = [];
      const re = /\[Image #(\d+)\]/g;
      let m;
      while ((m = re.exec(text))) {
        const n = Number(m[1]);
        links.push({
          range: { start: { x: m.index + 1, y }, end: { x: m.index + m[0].length, y } },
          text: m[0],
          decorations: { pointerCursor: true, underline: true },
          hover: (ev) => showRefPreview(key, n, ev),
          leave: hidePreview,
          activate: async (ev) => {
            if (!ev.ctrlKey && !ev.metaKey) { setStatus(t('ui.image.linkHint')); return; }
            const res = await tabImages(key);
            const hit = res && findImageRef(res, n);
            if (!hit) { setStatus(`⚠ ${t('ui.preview.notSaved', { n })}`); return; }
            hidePreview();
            if (ev.shiftKey) window.api.openPath(hit.img.path);
            else openImageTab(hit.img.path, imageTabTitle(hit.msg, hit.img));
          },
        });
      }
      callback(links.length ? links : undefined);
    },
  });
}

// --- Gallery: images grouped by message (side panel and tab) ---
function galleryThumb(p, label, title) {
  const b = el('button', 'gal-thumb');
  const img = el('img');
  img.alt = label || basename(p);
  b.append(img);
  if (label) b.append(el('span', 'gal-n', label));
  b.title = `${label ? `${label} · ` : ''}${t('ui.gallery.thumbHint')}`;
  thumbSrc(p).then((src) => { if (src) img.src = src; else b.classList.add('broken'); });
  b.addEventListener('click', (e) => ((e.ctrlKey || e.metaKey) ? window.api.openPath(p) : openImageTab(p, title)));
  return b;
}

/** res = claude_images response (or null if the agent is not Claude); pending = pasted paths not sent yet. */
function renderGallery(body, res, { pending = [], pendingTitle = t('ui.gallery.pending'), note = '' } = {}) {
  body.replaceChildren();
  if (pending.length) {
    const sec = el('div', 'gal-msg pending');
    sec.append(el('div', 'gal-when', pendingTitle));
    const grid = el('div', 'gal-grid');
    for (const p of pending.slice().reverse()) grid.append(galleryThumb(p, null, basename(p)));
    sec.append(grid);
    body.append(sec);
  }
  const msgs = res ? res.messages.slice().reverse() : []; // newest first
  msgs.forEach((m, i) => {
    const sec = el('div', 'gal-msg');
    const origin = m.origin ? ` · ${originLabel(m.origin)}` : '';
    const when = el('div', 'gal-when', `${i === 0 ? `${t('ui.gallery.lastMessage')} · ` : ''}${timeAgo(m.timestamp)}${origin}`);
    when.title = new Date(m.timestamp).toLocaleString(window.i18n.lang);
    sec.append(when);
    if (m.text) { const tx = el('div', 'gal-text', m.text); tx.title = m.text; sec.append(tx); }
    const grid = el('div', 'gal-grid');
    for (const img of m.images) grid.append(galleryThumb(img.path, img.n != null ? `#${img.n}` : null, imageTabTitle(m, img)));
    sec.append(grid);
    body.append(sec);
  });
  if (!msgs.length && !pending.length) {
    body.append(el('div', 'gal-empty', res && res.pending ? t('ui.gallery.noneYet') : t('ui.gallery.none')));
  }
  if (note) body.append(el('div', 'gal-note', note));
}

function applyGallery() {
  const entry = state.terms.get(state.activeKey);
  const show = !!(state.galleryOpen && entry && !entry.kind && !$('#btn-images').classList.contains('hidden'));
  $('#gallery').classList.toggle('hidden', !show);
  $('#btn-images').classList.toggle('on', show);
  clearInterval(state.galleryTimer);
  if (show) {
    $('#gal-body').dataset.sig = '';
    refreshGallery();
    state.galleryTimer = setInterval(refreshGallery, 4000); // Claude saves images when each message is sent
  }
  requestAnimationFrame(fitActive);
}

function toggleGallery() {
  state.galleryOpen = !state.galleryOpen;
  try { localStorage.setItem('gallery', state.galleryOpen ? '1' : '0'); } catch { /* no storage */ }
  applyGallery();
}

async function refreshGallery() {
  const key = state.activeKey;
  const entry = state.terms.get(key);
  if (!entry || entry.kind || $('#gallery').classList.contains('hidden')) return;
  const isClaude = entry.info.agent === 'claude';
  if (isClaude) await tabImages(key, 2000);
  if (state.activeKey !== key) return;
  const { res, pending } = imageCounts(entry);
  $('#gal-title').textContent = isClaude ? t('ui.gallery.titleSession') : t('ui.gallery.titleAttached');
  const body = $('#gal-body');
  const sig = JSON.stringify([key, window.i18n.lang, res && res.messages.map((m) => m.uuid), pending, res && res.pending]);
  if (body.dataset.sig !== sig) {
    body.dataset.sig = sig;
    renderGallery(body, res, isClaude ? { pending } : {
      pending, pendingTitle: t('ui.gallery.attached'), note: t('ui.gallery.claudeOnly'),
    });
  }
  updateHeader();
}

/** Tab with every image of a Claude session, without starting the agent (right-click → View images). */
function openGalleryTab(s) {
  const key = `gal:${s.key}`;
  if (state.terms.has(key)) { showTerminal(key); state.terms.get(key).reload(); return; }
  const wrap = el('div', 'term-wrap gallery-view hidden');
  const body = el('div', 'gal-body gal-large');
  wrap.append(body);
  $('#terminals').append(wrap);
  const entry = { kind: 'gallery', wrap, alive: true, images: [], info: { title: t('ui.gallery.tabTitle', { title: displayTitle(s) }), cwd: s.cwd } };
  entry.reload = async () => {
    const loading = el('div', 'iv-loading');
    loading.append(el('div', 'spinner'), el('span', 'msg', t('ui.gallery.reading')));
    body.replaceChildren(loading);
    const res = await window.api.claudeImages({ key: s.key });
    if (!res.ok) { body.replaceChildren(el('div', 'gal-empty', t('ui.gallery.error', { error: res.error }))); return; }
    renderGallery(body, res);
  };
  state.terms.set(key, entry);
  state.order.push(key);
  showTerminal(key);
  entry.reload();
}

function fitActive() {
  const e = state.terms.get(state.activeKey);
  if (e) safeFit(e);
}

async function startPty(key, entry) {
  const { info } = entry;
  safeFit(entry);
  entry.outBytes = 0;
  const res = await window.api.openPty({
    key, agent: info.agent, mode: info.mode, sessionId: info.sessionId, cwd: info.cwd,
    cols: entry.term.cols, rows: entry.term.rows,
  });
  if (!res.ok) {
    entry.term.writeln(`\x1b[31m${t('ui.term.error', { error: res.error })}\x1b[0m`);
    return;
  }
  entry.alive = true;
  if (!res.reused) entry.term.writeln(`\x1b[90m> ${res.command}   (${res.cwd})\x1b[0m`);
  if (res.warning) entry.term.writeln(`\x1b[33m${t(`ui.term.${res.warning.code}`, { path: res.warning.path || '' })}\x1b[0m`);
  renderList();
  renderTabs();
}

function updateHeader() {
  const entry = state.terms.get(state.activeKey);
  if (!entry) return;
  const a = state.agentById[entry.info.agent];
  const image = isImageTab(entry);
  const name = image ? t('ui.image.title', { title: entry.info.title }) : `${a ? a.name + ' · ' : ''}${entry.info.title}`;
  const sub = image ? entry.info.path : entry.info.cwd || '';
  $('#term-name').textContent = name;
  $('#term-cwd').textContent = sub;
  $('.term-title').title = sub ? `${name}\n${sub}` : name; // the title is cut with "…"; the tooltip shows it whole
  // 🖼 opens the images panel: always in Claude tabs; in others only after attaching something
  const btnImages = $('#btn-images');
  const show = !entry.kind && (entry.info.agent === 'claude' || entry.images.length > 0);
  btnImages.classList.toggle('hidden', !show);
  const { total } = imageCounts(entry);
  btnImages.querySelector('span').textContent = total || '';
  btnImages.title = total ? `${t('ui.header.images')} (${total})` : t('ui.header.images');
}

/** "Options ▾" menu of the header: text only, depending on the kind of tab. */
function headerMenu() {
  const key = state.activeKey;
  const entry = state.terms.get(key);
  const menu = $('#menu');
  if (!menu.classList.contains('hidden') && menu.dataset.for === 'header') { hideMenu(); return; } // second click closes it
  if (!entry) return;
  const { info } = entry;
  const s = sessionByKey(key);
  let items;
  if (entry.kind === 'image') {
    items = [
      { label: t('ui.image.openSystem'), action: () => window.api.openPath(info.path) },
      { label: t('ui.image.reveal'), action: () => window.api.revealPath(info.path) },
      { label: t('ui.image.copyPath'), action: () => window.api.writeClipboard(info.path) },
      '-',
      { label: t('ui.image.reload'), hint: state.shortcuts['terminal.reload'], action: () => reloadTerminal(key) },
    ];
  } else {
    items = [
      ...(info.cwd ? [
        { label: t('ui.menu.openProjectFolder'), action: () => window.api.openFolder(info.cwd) },
        { label: t('ui.menu.openShellHere'), hint: state.shortcuts['tab.newShell'], action: () => openShell(info.cwd) },
        { label: t('ui.menu.copyFolderPath'), action: () => window.api.writeClipboard(info.cwd) },
        '-',
      ] : []),
      ...(s ? [{ label: t('ui.menu.renameSession'), hint: state.shortcuts['session.rename'], action: () => startRename(key) }] : []),
      entry.kind === 'gallery'
        ? { label: t('ui.gallery.reload'), hint: state.shortcuts['terminal.reload'], action: () => reloadTerminal(key) }
        : {
          label: t('ui.menu.reloadTerminal'), hint: state.shortcuts['terminal.reload'], action: () => reloadTerminal(key),
          sub: t('ui.menu.reloadTerminalSub'),
        },
    ];
  }
  const r = $('#btn-options').getBoundingClientRect();
  showMenu(r.left, r.bottom + 4, items);
  menu.dataset.for = 'header';
  menu.style.left = `${Math.max(4, r.right - menu.offsetWidth)}px`; // aligned with the button's right edge
}

function showTerminal(key) {
  state.activeKey = key;
  if (key) state.cursorKey = state.terms.has(key) && sessionByKey(key) ? key : state.cursorKey;
  for (const [k, e] of state.terms) e.wrap.classList.toggle('hidden', k !== key);
  const entry = state.terms.get(key);
  $('#empty').classList.toggle('hidden', !!entry);
  $('#term-header').classList.toggle('hidden', !entry);
  $('#tabbar').classList.toggle('hidden', !state.order.length);
  hidePreview();
  if (entry) {
    updateHeader();
    requestAnimationFrame(() => { safeFit(entry); if (entry.term) entry.term.focus(); });
    if (!entry.kind && entry.info.agent === 'claude') tabImages(key).then(() => { if (state.activeKey === key) updateHeader(); });
  }
  applyGallery();
  renderTabs();
  markListSelection();
}

async function openSession(s) {
  const key = s.key;
  let entry = state.terms.get(key);
  if (!entry) {
    entry = makeTerminal(key, { agent: s.agent, mode: 'resume', sessionId: s.id, cwd: s.cwd, title: displayTitle(s) });
    showTerminal(key);
    await startPty(key, entry);
  } else {
    showTerminal(key);
    if (!entry.alive) await startPty(key, entry);
  }
}

/** Resumes the session (into a copy when the agent can fork) and sends the compact command. */
async function continueCompacted(s) {
  const a = state.agentById[s.agent];
  const command = compactCommandOf(s.agent);
  if (!command) return;
  if (!a.forks) {
    // Compacts the same session: reuse its tab if open (never two processes on the same session)
    await openSession(s);
    sendWhenIdle(s.key, command);
    return;
  }
  const key = `compact:${s.key}:${++state.compactCounter}`;
  const entry = makeTerminal(key, {
    agent: s.agent, mode: 'compact', sessionId: s.id, cwd: s.cwd, title: `↳ ${displayTitle(s)}`,
    forkOf: { key: s.key, since: Date.now() },
  });
  showTerminal(key);
  await startPty(key, entry);
  sendWhenIdle(key, command);
}

/** Types a command into the agent once it finished loading (its output stays quiet for a moment).
 *  Cancelled if the user starts typing first, so it never mixes with what they type. */
function sendWhenIdle(key, text) {
  const entry = state.terms.get(key);
  if (!entry) return;
  const started = Date.now();
  const pending = { cancelled: false };
  entry.pendingInput = pending;
  setStatus(t('compact.waiting', { cmd: text }));
  const tick = () => {
    const e = state.terms.get(key);
    if (!e || !e.alive || pending.cancelled || e.pendingInput !== pending) return;
    if (e.outBytes > 200 && Date.now() - e.lastOutputAt > 1500) {
      e.pendingInput = null;
      window.api.writePty(key, text);
      // Enter separately: sent together, some agents treat it as a paste and don't run it
      setTimeout(() => window.api.writePty(key, '\r'), 400);
      setStatus(t('compact.sent', { cmd: text }));
      if (e.info.mode === 'compact') e.info.mode = 'resume';
      return;
    }
    if (Date.now() - started > 45000) {
      e.pendingInput = null;
      setStatus(`⚠ ${t('compact.timeout', { cmd: text })}`);
      return;
    }
    setTimeout(tick, 300);
  };
  tick();
}

async function openNew(agent, cwd, name) {
  const key = `new:${agent}:${++state.newCounter}`;
  const entry = makeTerminal(key, { agent, mode: 'new', cwd, title: t('ui.tab.new', { name }) });
  showTerminal(key);
  await startPty(key, entry);
}

/** Plain terminal (no agent). Defaults to the active tab's folder. */
async function openShell(cwd) {
  const active = state.terms.get(state.activeKey);
  const dir = cwd || (active && active.info.cwd) || ''; // in an image tab, cwd is its folder
  const key = `shell:${++state.shellCounter}`;
  const entry = makeTerminal(key, { mode: 'shell', cwd: dir, title: `${t('shell.title')} · ${dir ? basename(dir) : '~'}` });
  showTerminal(key);
  await startPty(key, entry);
}

async function closeTab(key) {
  const entry = state.terms.get(key);
  if (!entry) return;
  if (entry.term) {
    await window.api.killPty(key);
    entry.term.dispose();
  }
  entry.wrap.remove();
  state.terms.delete(key);
  const idx = state.order.indexOf(key);
  state.order.splice(idx, 1);
  if (state.activeKey === key) showTerminal(state.order[Math.min(idx, state.order.length - 1)] || null);
  else renderTabs();
  renderList();
}

async function closeOtherTabs(key) {
  for (const k of state.order.filter((x) => x !== key)) await closeTab(k);
  showTerminal(key);
}

async function restartTerminal(key, note) {
  const entry = state.terms.get(key);
  if (!entry) return;
  if (entry.reload) { entry.reload(); return; } // image or gallery tab
  // A compacted copy is resumed by its own id, not the original's
  const { forkOf } = entry.info;
  if (forkOf && entry.info.mode === 'resume') {
    const id = await window.api.findFork(forkOf.key, forkOf.since);
    if (id) { entry.info.sessionId = id; delete entry.info.forkOf; }
    else note = `${note || ''}\r\n${t('ui.term.forkNotFound')}`.trim();
  }
  await window.api.killPty(key);
  entry.alive = false;
  entry.term.reset();
  if (note) entry.term.writeln(`\x1b[90m${note}\x1b[0m`);
  await startPty(key, entry);
}

/** Like `source ~/.zshrc`: re-reads PATH and variables (of a newly installed program) and restarts the terminal. */
async function reloadTerminal(key = state.activeKey) {
  const entry = state.terms.get(key);
  if (!entry) return;
  if (entry.reload) { entry.reload(); return; }
  entry.term.writeln(`\r\n\x1b[90m${t('ui.term.reloadingEnv')}\x1b[0m`);
  const res = await window.api.reloadEnv();
  if (!res.ok) { entry.term.writeln(`\x1b[31m${t('ui.term.envError', { error: res.error })}\x1b[0m`); return; }
  await restartTerminal(key, t('ui.term.envReloaded'));
}

window.api.onPtyData((key, data) => {
  const e = state.terms.get(key);
  if (!e) return;
  e.outBytes += data.length;
  e.lastOutputAt = Date.now();
  e.term.write(data);
});

window.api.onPtyExit((key, code) => {
  const e = state.terms.get(key);
  if (!e) return;
  e.alive = false;
  e.term.writeln(`\r\n\x1b[90m${t('ui.term.exited', { code, key: shortcutOrNone('terminal.reload') })}\x1b[0m`);
  renderList();
  renderTabs();
});

// ---------------- Tabs ----------------
function renderTabs() {
  const bar = $('#tabbar');
  bar.classList.toggle('hidden', !state.order.length);
  bar.replaceChildren();
  state.order.forEach((key, i) => {
    const entry = state.terms.get(key);
    const tab = el('div', 'tab' + (key === state.activeKey ? ' active' : '') + (entry.alive ? '' : ' dead'));
    tab.draggable = true;
    tab.dataset.key = key;
    tab.title = `${entry.info.title}\n${entry.info.cwd || ''}`;
    if (i < 9) tab.append(el('span', 'num', String(i + 1)));
    if (entry.kind === 'gallery') tab.append(el('span', 'badge img', '▦'));
    else if (isImageTab(entry)) tab.append(el('span', 'badge img', 'IMG'));
    else tab.append(entry.info.agent ? badge(entry.info.agent) : el('span', 'badge shell', '>_'));
    tab.append(el('span', 't', entry.info.title));
    const x = el('button', 'x', '×');
    x.title = t('ui.menu.close');
    x.addEventListener('click', (e) => { e.stopPropagation(); closeTab(key); });
    tab.append(x);

    tab.addEventListener('click', () => showTerminal(key));
    tab.addEventListener('auxclick', (e) => { if (e.button === 1) { e.preventDefault(); closeTab(key); } });
    tab.addEventListener('contextmenu', (e) => { e.preventDefault(); tabMenu(key, e.clientX, e.clientY); });
    tab.addEventListener('dragstart', (e) => { e.dataTransfer.setData('text/x-tab', key); e.dataTransfer.effectAllowed = 'move'; });
    tab.addEventListener('dragover', (e) => {
      if (!e.dataTransfer.types.includes('text/x-tab')) return;
      e.preventDefault();
      tab.classList.add('drop');
    });
    tab.addEventListener('dragleave', () => tab.classList.remove('drop'));
    tab.addEventListener('drop', (e) => {
      e.preventDefault();
      tab.classList.remove('drop');
      const from = e.dataTransfer.getData('text/x-tab');
      if (from && from !== key) moveTab(from, state.order.indexOf(key));
    });
    bar.append(tab);
  });
  const plus = el('button', 'tab-new', '＋');
  plus.title = t('ui.tab.newShell', { key: shortcutOrNone('tab.newShell') });
  plus.addEventListener('click', () => openShell());
  bar.append(plus);
  bar.querySelector('.tab.active')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
}

function moveTab(key, toIdx) {
  const from = state.order.indexOf(key);
  if (from === -1) return;
  const to = Math.max(0, Math.min(state.order.length - 1, toIdx));
  if (to === from) return;
  state.order.splice(from, 1);
  state.order.splice(to, 0, key);
  renderTabs();
}

function cycleTab(dir) {
  const n = state.order.length;
  if (!n) return;
  const idx = state.order.indexOf(state.activeKey);
  showTerminal(state.order[(idx + dir + n) % n]);
}

// ---------------- Shortcut actions ----------------
const ACTIONS = {
  'tab.next': () => cycleTab(1),
  'tab.prev': () => cycleTab(-1),
  'tab.moveRight': () => state.activeKey && moveTab(state.activeKey, state.order.indexOf(state.activeKey) + 1),
  'tab.moveLeft': () => state.activeKey && moveTab(state.activeKey, state.order.indexOf(state.activeKey) - 1),
  'tab.newShell': () => openShell(),
  'tab.close': () => state.activeKey && closeTab(state.activeKey),
  'session.next': () => moveSessionCursor(1),
  'session.prev': () => moveSessionCursor(-1),
  'session.rename': () => startRename(state.cursorKey || state.activeKey),
  'terminal.reload': () => reloadTerminal(),
  'sessions.refresh': () => loadSessions(),
  'search.focus': () => { $('#search').focus(); $('#search').select(); },
};

function runAction(id) {
  const m = id.match(/^tab\.goto(\d)$/);
  if (m) {
    const key = state.order[Number(m[1]) - 1];
    if (key) showTerminal(key);
    return;
  }
  if (ACTIONS[id]) ACTIONS[id]();
}

// Listened to in the capture phase to get ahead of xterm (otherwise the terminal swallows the key).
function onKeyDown(e) {
  hidePreview();
  if (state.recording) { recordShortcut(e); return; }
  if (e.key === 'Escape') {
    hideMenu();
    hideFilterPop();
    if (!$('#settings').classList.contains('hidden')) closeSettings();
  }
  const accel = eventToAccel(e);
  if (!accel) return;
  const action = document.querySelector('.modal:not(.hidden)') ? null : state.keymap.get(accel.toLowerCase());
  if (!action) {
    if (BLOCKED_BROWSER_KEYS.has(accel.toLowerCase())) e.preventDefault(); // the terminal still gets the key
    return;
  }
  // In text fields, Shift+arrows and the like keep selecting text
  const target = e.target;
  const typing = target && /^(INPUT|TEXTAREA)$/.test(target.tagName) && !target.classList.contains('xterm-helper-textarea');
  if (typing && !e.ctrlKey && !e.altKey && !e.metaKey && !/^F\d+$/.test(accel)) return;
  e.preventDefault();
  e.stopPropagation();
  hideMenu();
  runAction(action);
}

// ---------------- Settings and shortcuts ----------------
function openSettings() {
  state.draft = { ...state.shortcuts };
  state.recording = null;
  const lang = $('#set-language');
  lang.replaceChildren(el('option', null, t('ui.settings.languageAuto')));
  lang.firstChild.value = 'auto';
  for (const l of window.i18n.languages) {
    const o = el('option', null, l.name);
    o.value = l.code;
    lang.append(o);
  }
  lang.value = state.config.language || 'auto';
  $('#set-shell').value = state.config.shell;
  $('#set-font').value = state.config.fontSize;
  $('#sc-note').textContent = '';
  renderShortcutRows();
  $('#settings').classList.remove('hidden');
}

function closeSettings() {
  state.recording = null;
  state.draft = null;
  $('#settings').classList.add('hidden');
}

function renderShortcutRows() {
  const box = $('#shortcut-rows');
  box.replaceChildren();
  for (const sc of SHORTCUTS) {
    const row = el('div', 'sc-row');
    const rec = state.recording === sc.id;
    const kbd = el('button', 'kbd' + (rec ? ' recording' : ''), rec ? t('ui.settings.recording') : (state.draft[sc.id] || '—'));
    kbd.addEventListener('click', () => { state.recording = sc.id; renderShortcutRows(); });
    const reset = el('button', 'sc-reset', '↺');
    reset.title = t('ui.settings.default', { key: sc.def || t('ui.settings.unassigned') });
    reset.disabled = normalizeAccel(state.draft[sc.id]) === normalizeAccel(sc.def);
    reset.addEventListener('click', () => assignShortcut(sc.id, sc.def));
    row.append(el('span', 'sc-label', shortcutLabel(sc.id)), kbd, reset);
    box.append(row);
  }
}

function assignShortcut(id, accel) {
  const norm = normalizeAccel(accel);
  const notes = [];
  if (norm) {
    for (const sc of SHORTCUTS) {
      if (sc.id !== id && normalizeAccel(state.draft[sc.id]).toLowerCase() === norm.toLowerCase()) {
        state.draft[sc.id] = '';
        notes.push(t('ui.settings.removedFrom', { key: norm, label: shortcutLabel(sc.id) }));
      }
    }
  }
  state.draft[id] = norm;
  state.recording = null;
  $('#sc-note').textContent = notes.join(' ');
  renderShortcutRows();
}

function recordShortcut(e) {
  e.preventDefault();
  e.stopPropagation();
  if (e.key === 'Escape') { state.recording = null; renderShortcutRows(); return; }
  const plain = !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey;
  if (plain && (e.key === 'Delete' || e.key === 'Backspace')) { assignShortcut(state.recording, ''); return; }
  const accel = eventToAccel(e);
  if (accel) assignShortcut(state.recording, accel); // only a modifier so far: keep waiting
}

/** Re-renders every text after a language change (terminals and tabs stay open). */
async function applyLanguage() {
  await window.i18n.load(state.config.language || 'auto');
  window.i18n.apply();
  $('#btn-refresh').title = t('ui.refresh.title', { key: shortcutOrNone('sessions.refresh') });
  $('#gal-body').dataset.sig = '';
  renderFilters();
  renderList();
  renderTabs();
  updateHeader();
  refreshGallery();
}

async function saveSettings() {
  const fontSize = Math.min(32, Math.max(8, Number($('#set-font').value) || 14));
  const shell = $('#set-shell').value.trim() || state.config.shell;
  const language = $('#set-language').value;
  const languageChanged = language !== (state.config.language || 'auto');
  state.config = await window.api.setConfig({ shortcuts: state.draft, fontSize, shell, language });
  applyShortcuts();
  for (const e of state.terms.values()) if (e.term) e.term.options.fontSize = fontSize;
  fitActive();
  closeSettings();
  if (languageChanged) await applyLanguage();
  else renderTabs();
}

// ---------------- Startup ----------------
async function init() {
  state.config = await window.api.getConfig();
  await window.i18n.load(state.config.language || 'auto');
  window.i18n.apply();
  state.agents = await window.api.agentInfo();
  state.agentById = Object.fromEntries(state.agents.map((a) => [a.id, a]));
  state.config.filters = state.config.filters || {};
  applyShortcuts();
  applyRatio(state.config.sidebarRatio || 0.3);
  setupSplitter();

  $('#search').addEventListener('input', renderList);
  $('#btn-filter').addEventListener('click', toggleFilterPop);
  $('#btn-refresh').addEventListener('click', loadSessions);
  $('#btn-refresh').title = t('ui.refresh.title', { key: shortcutOrNone('sessions.refresh') });
  $('#btn-agents').addEventListener('click', () => openAgentsModal(false));
  $('#btn-settings').addEventListener('click', openSettings);
  $('#btn-redetect').addEventListener('click', () => openAgentsModal(!state.config.onboarded));
  $('#btn-config').addEventListener('click', () => window.api.openConfig());
  $('#btn-cancel').addEventListener('click', () => $('#modal').classList.add('hidden'));
  $('#btn-save').addEventListener('click', saveAgentsModal);
  $('#btn-settings-cancel').addEventListener('click', closeSettings);
  $('#btn-settings-save').addEventListener('click', saveSettings);
  $('#btn-shortcuts-reset').addEventListener('click', () => {
    state.draft = { ...DEFAULT_SHORTCUTS }; state.recording = null;
    $('#sc-note').textContent = ''; renderShortcutRows();
  });
  $('#btn-close').addEventListener('click', () => closeTab(state.activeKey));
  $('#btn-options').addEventListener('click', headerMenu);
  $('#btn-images').addEventListener('click', toggleGallery);
  $('#gal-close').addEventListener('click', toggleGallery);
  $('#gal-refresh').addEventListener('click', () => {
    const e = state.terms.get(state.activeKey);
    if (e) e.imgRes = null;
    $('#gal-body').dataset.sig = '';
    refreshGallery();
  });
  window.api.onFileDrop(onFilesDropped);
  window.api.onFileDrag((over) => {
    const e = state.terms.get(state.activeKey);
    if (e && e.term) e.wrap.classList.toggle('drop-target', over);
  });

  document.addEventListener('mousedown', (e) => {
    if (!e.target.closest('#menu, #btn-options')) hideMenu(); // #btn-options opens/closes its own menu
    if (!e.target.closest('#filter-pop, #btn-filter')) hideFilterPop();
  });
  window.addEventListener('keydown', onKeyDown, true);
  window.addEventListener('resize', () => { hideMenu(); hideFilterPop(); fitActive(); });
  window.addEventListener('blur', () => { hideMenu(); hidePreview(); });
  window.addEventListener('focus', () => { if (!document.querySelector('.modal:not(.hidden)')) loadSessions(); });

  if (!state.config.onboarded) await openAgentsModal(true);
  else await loadSessions();
}

init();
