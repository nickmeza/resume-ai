// Bridge between the UI and the Rust backend (Tauri commands and events).
// Exposes the same `window.api` the UI used with Electron, so renderer.js stays backend-agnostic.
(() => {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;

  // UI errors go to the app's console output (run it from a terminal to see them)
  const report = (level, msg) => invoke('ui_log', { level, message: String(msg) }).catch(() => {});
  window.addEventListener('error', (e) => report('error', `${e.message} (${e.filename}:${e.lineno})`));
  window.addEventListener('unhandledrejection', (e) => report('error', e.reason && e.reason.stack ? e.reason.stack : e.reason));
  if (new URLSearchParams(location.search).has('smoke')) window.__smoke = report;

  // Files dropped on the window: Tauri gives real paths (a web page cannot)
  const dropHandlers = [];
  listen('tauri://drag-drop', (e) => {
    const paths = (e.payload && e.payload.paths) || [];
    if (paths.length) dropHandlers.forEach((cb) => cb(paths, e.payload.position));
  });
  const dragHandlers = [];
  listen('tauri://drag-enter', () => dragHandlers.forEach((cb) => cb(true)));
  listen('tauri://drag-leave', () => dragHandlers.forEach((cb) => cb(false)));
  listen('tauri://drag-drop', () => dragHandlers.forEach((cb) => cb(false)));

  window.api = {
    getConfig: () => invoke('config_get'),
    setConfig: (patch) => invoke('config_set', { patch }),
    openConfig: () => invoke('config_open'),
    i18nBundle: (lang) => invoke('i18n_bundle', { lang }),
    agentInfo: () => invoke('agents_info'),
    detectAgents: () => invoke('agents_detect'),
    listSessions: () => invoke('sessions_list'),
    loadOrganization: () => invoke('organization_load'),
    changeOrganization: (key, action) => invoke('organization_change', { key, action }),
    renameSession: (key, title) => invoke('sessions_rename', { key, title }),
    deleteSession: (key) => invoke('sessions_delete', { key }),
    findFork: (key, since) => invoke('sessions_find_fork', { key, since }),
    reloadEnv: () => invoke('env_reload'),
    openPty: (opts) => invoke('pty_open', { opts }),
    writePty: (key, data) => { invoke('pty_write', { key, data }); },
    resizePty: (key, cols, rows) => { invoke('pty_resize', { key, cols, rows }); },
    killPty: (key) => invoke('pty_kill', { key }),
    readClipboard: () => invoke('clipboard_read'),
    writeClipboard: (text) => invoke('clipboard_write', { text }),
    saveClipboardImage: () => invoke('clipboard_save_image'),
    openFolder: (path) => invoke('folder_open', { path }),
    openPath: (path) => invoke('folder_open', { path }), // default app (image viewer, etc.)
    revealPath: (path) => invoke('path_reveal', { path }),
    resolveImages: (paths, cwd) => invoke('images_resolve', { paths, cwd: cwd || null }),
    readImage: (path) => invoke('image_read', { path }),
    claudeImages: (query) => invoke('claude_images', { query }),
    onPtyData: (cb) => listen('pty-data', (e) => cb(e.payload.key, e.payload.data)),
    onPtyExit: (cb) => listen('pty-exit', (e) => cb(e.payload.key, e.payload.code)),
    onFileDrop: (cb) => dropHandlers.push(cb),
    onFileDrag: (cb) => dragHandlers.push(cb),
  };

  // ---------- i18n ----------
  // t('key', { name: value }) with {name} placeholders; plural keys end in .one / .other.
  let messages = {};
  window.i18n = {
    lang: 'en',
    languages: [],
    async load(pref) {
      const b = await window.api.i18nBundle(pref || 'auto');
      messages = b.messages;
      this.lang = b.lang;
      this.languages = b.languages;
      document.documentElement.lang = b.lang;
    },
    t(key, vars) {
      let s = messages[key] != null ? messages[key] : key;
      if (vars) for (const [k, v] of Object.entries(vars)) s = s.split(`{${k}}`).join(String(v));
      return s;
    },
    plural(key, count, vars) {
      return this.t(`${key}.${count === 1 ? 'one' : 'other'}`, { count, ...(vars || {}) });
    },
    /** Fills elements marked with data-i18n / data-i18n-title / data-i18n-placeholder. */
    apply(root = document) {
      root.querySelectorAll('[data-i18n]').forEach((el) => { el.textContent = this.t(el.dataset.i18n); });
      root.querySelectorAll('[data-i18n-html]').forEach((el) => { el.innerHTML = this.t(el.dataset.i18nHtml); });
      root.querySelectorAll('[data-i18n-title]').forEach((el) => { el.title = this.t(el.dataset.i18nTitle); });
      root.querySelectorAll('[data-i18n-placeholder]').forEach((el) => { el.placeholder = this.t(el.dataset.i18nPlaceholder); });
    },
  };
})();
