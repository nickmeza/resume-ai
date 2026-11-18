const { contextBridge, ipcRenderer, webUtils } = require('electron');

contextBridge.exposeInMainWorld('api', {
  getConfig: () => ipcRenderer.invoke('config:get'),
  setConfig: (patch) => ipcRenderer.invoke('config:set', patch),
  openConfig: () => ipcRenderer.invoke('config:open'),
  agentInfo: () => ipcRenderer.invoke('agents:info'),
  detectAgents: () => ipcRenderer.invoke('agents:detect'),
  listSessions: () => ipcRenderer.invoke('sessions:list'),
  renameSession: (key, title) => ipcRenderer.invoke('sessions:rename', key, title),
  deleteSession: (key) => ipcRenderer.invoke('sessions:delete', key),
  findFork: (key, since) => ipcRenderer.invoke('sessions:findFork', key, since),
  reloadEnv: () => ipcRenderer.invoke('env:reload'),
  openPty: (opts) => ipcRenderer.invoke('pty:open', opts),
  writePty: (key, data) => ipcRenderer.send('pty:write', key, data),
  resizePty: (key, cols, rows) => ipcRenderer.send('pty:resize', key, cols, rows),
  killPty: (key) => ipcRenderer.invoke('pty:kill', key),
  readClipboard: () => ipcRenderer.invoke('clipboard:read'),
  writeClipboard: (t) => ipcRenderer.invoke('clipboard:write', t),
  openFolder: (p) => ipcRenderer.invoke('folder:open', p),
  openPath: (p) => ipcRenderer.invoke('folder:open', p),   // abre con la app predeterminada (visor de imágenes, etc.)
  revealPath: (p) => ipcRenderer.invoke('path:reveal', p),
  saveClipboardImage: () => ipcRenderer.invoke('clipboard:saveImage'),
  resolveImages: (paths, cwd) => ipcRenderer.invoke('images:resolve', paths, cwd),
  readImage: (p) => ipcRenderer.invoke('image:read', p),
  claudeImages: (query) => ipcRenderer.invoke('claude:images', query),
  pathForFile: (file) => webUtils.getPathForFile(file), // ruta de un archivo arrastrado
  onPtyData: (cb) => ipcRenderer.on('pty:data', (_e, key, data) => cb(key, data)),
  onPtyExit: (cb) => ipcRenderer.on('pty:exit', (_e, key, code) => cb(key, code)),
});
