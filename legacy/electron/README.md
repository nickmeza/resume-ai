# Agent Sessions

UI mínima para Windows que lista las sesiones de tus agentes de IA para programar y las retoma con un clic.

```
┌──────────── 30% ─────────────┬──────────────────── 70% ────────────────────┐
│ [Todos 8][Claude 3][Codex 2]… │  Codex CLI · Agrega tests al endpoint        │
│ 🔍 Buscar…                     │  C:\Users\ana\api                            │
│ ▾ API                  3   ＋  │ ──────────────────────────────────────────── │
│   ● CX Agrega tests…           │  > codex resume 0199aaaa-…                   │
│     OC Refactor de pagos       │                                              │
│ ▾ MI-APP               3   ＋  │   (sesión de Codex retomada)                 │
│     CC Arregla el login        │                                              │
│     GM Explica el router       │                                              │
└────────────────────────────────┴──────────────────────────────────────────────┘
```

## Agentes soportados

| Agente | Dónde lee las sesiones | Comando para retomar |
|---|---|---|
| **Claude Code** | `%USERPROFILE%\.claude\projects\*\*.jsonl` | `claude --resume <id>` |
| **Codex CLI** | `%USERPROFILE%\.codex\sessions\AAAA\MM\DD\rollout-*.jsonl` | `codex resume <id>` |
| **Gemini CLI** | `%USERPROFILE%\.gemini\tmp\<proyecto>\chats\session-*.json` | `gemini --resume <id>` |
| **opencode** | `%USERPROFILE%\.local\share\opencode\opencode.db` (y `storage\` antiguo) | `opencode --session <id>` |
| **GitHub Copilot CLI** | `%USERPROFILE%\.copilot\session-state\<id>\` | `copilot --resume=<id>` |
| **Cursor CLI** *(experimental)* | `%USERPROFILE%\.cursor\chats\<md5>\<id>\store.db` | `cursor-agent --resume <id>` |

También se respetan las variables `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `GEMINI_CLI_HOME`, `XDG_DATA_HOME` y `COPILOT_HOME`.

Todos los agentes guardan las sesiones según la carpeta del proyecto. Por eso cada terminal se abre
**en la carpeta original** de la sesión. Gemini y Cursor guardan esa carpeta como hash; la app la recupera
comparándola con las rutas que conoce de los demás agentes. Si no puede, la marca como "carpeta desconocida".

## Qué hace

- **Primer arranque:** detecta qué agentes tienes (si el comando está en el PATH o si hay sesiones guardadas) y te deja elegirlos.
  Después puedes volver a esta pantalla con ⚙.
- **Lista unificada:** las sesiones se agrupan por proyecto y cada una lleva un distintivo de color según su agente.
  Puedes buscar con `Ctrl+K`. Con el botón de filtro (embudo) eliges agentes y tags de origen. En Claude Code el origen puede ser terminal, VS Code o la app de escritorio.
  Los filtros activos aparecen debajo del buscador y se quitan con ×. Para agregar otro tipo de filtro, suma un grupo en `FILTER_GROUPS` (`renderer/renderer.js`).
- **Clic en una sesión:** abre a la derecha una terminal real (ConPTY) que retoma la sesión.
- **＋ en un proyecto:** inicia una sesión nueva en esa carpeta con el agente que elijas.
- Las terminales siguen vivas aunque cambies de sesión. Las abiertas se marcan con ●.
- **Divisor:** se arrastra para cambiar el tamaño y la app lo recuerda. Con doble clic vuelve a 30/70.
- **Copiar y pegar** al estilo Windows Terminal: `Ctrl+C` con selección copia, `Ctrl+V` pega y el clic derecho copia o pega.
- **Recarga:** `F5`. La lista también se refresca al volver a la ventana.
- **Pestañas:** cada terminal abierta es una pestaña. Puedes arrastrarlas para reordenarlas, cerrarlas con clic central y abrir una terminal sin agente con **＋**.
- **Clic derecho en una sesión:** renombrar, copiar ID, abrir la carpeta o una terminal allí, y eliminar.
  - *Renombrar:* en Claude Code se guarda en la propia sesión (igual que `/rename`). En los demás agentes, el nombre solo se guarda en esta app.
  - *Continuar desde una nueva sesión:* retoma la sesión y envía el comando para compactarla (`/compact` en Claude, Codex y opencode; `/compress` en Gemini).
    En Claude se hace sobre una copia (`--fork-session`), así que la original queda intacta. Compactar cuesta tokens una vez, pero después cada mensaje es más barato.
    En Copilot y Cursor la opción aparece desactivada. Si tu versión tiene ese comando, ponlo en `agents.<id>.compactCommand` de `config.json`.
  - *Eliminar:* mueve los archivos de la sesión a la **papelera de reciclaje**. opencode no lo permite, porque guarda todas sus sesiones en una sola base SQLite.
- **Imágenes:**
  - `Ctrl+V` con una captura en el portapapeles la guarda como PNG temporal y pega su ruta. Los agentes la adjuntan al ver la ruta.
    Las capturas se guardan en `%TEMP%agent-sessions` y se borran a los 7 días.
  - Al arrastrar archivos a la terminal se escriben sus rutas.
  - `Ctrl+clic` en una ruta de imagen la abre en una **pestaña de imagen**. `Ctrl+Shift+clic` la abre con el visor del sistema.
  - El botón 🖼 del encabezado lista las imágenes que adjuntaste en esa pestaña, aunque el agente haya cambiado la ruta por `[Image #1]`.
- **Imágenes de la sesión (solo Claude Code):** Claude guarda cada `[Image #N]` junto al mensaje en que se envió (`imagePasteIds`).
  - Al pasar el ratón por `[Image #N]` en la terminal se ve una vista previa con el mensaje y la hora. `Ctrl+clic` la abre en una pestaña.
  - El botón 🖼 abre un panel lateral con las imágenes agrupadas por mensaje (el más reciente arriba) y las pegadas que aún no enviaste.
  - Clic derecho en una sesión → **Ver imágenes**: abre la galería sin iniciar el agente.
  - Las imágenes se extraen en `%TEMP%agent-sessionssesiones<id>` y se borran a los 7 días.
- **Recargar terminal (Opciones ▾ / `Ctrl+Shift+R`):** vuelve a leer el PATH y las variables de entorno de Windows y reinicia la terminal.
  Equivale a `source ~/.zshrc` y sirve después de instalar algo nuevo.

### Atajos (se cambian en ⌨)

| Acción | Por defecto |
|---|---|
| Pestaña anterior / siguiente | `Shift+←` / `Shift+→` |
| Ir a la pestaña N | `Ctrl+1` … `Ctrl+9` |
| Mover pestaña a la izquierda / derecha | `Ctrl+Shift+←` / `Ctrl+Shift+→` |
| Nueva terminal / cerrar pestaña | `Ctrl+Shift+T` / `Ctrl+W` |
| Sesión anterior / siguiente de la lista | `Shift+↑` / `Shift+↓` |
| Renombrar sesión | `F2` |
| Recargar terminal | `Ctrl+Shift+R` |
| Recargar lista / buscar | `F5` / `Ctrl+K` |

Los atajos tienen prioridad sobre la terminal. Si un atajo choca con algo que usas dentro de un agente (por ejemplo, `Shift+←` para seleccionar
texto en PowerShell), cámbialo o déjalo sin asignar en ⌨. Se guardan en `config.json`, en la clave `shortcuts`.

## Requisitos

- Windows 10/11
- [Node.js 22.13+](https://nodejs.org). opencode y Cursor necesitan el módulo `node:sqlite` de Node.
- Al menos uno de los agentes instalado

No necesitas Visual Studio Build Tools, porque la terminal usa `@lydell/node-pty`, que trae binarios precompilados.

## Uso

```bat
npm install
npm start
```

Para crear un `.exe` (portable e instalador en `dist\`):

```bat
npm run dist
```

## Configuración

El archivo es `%APPDATA%\Agent Sessions\config.json`. Lo abres desde ⚙ con el botón "Editar config.json".

```jsonc
{
  "sidebarRatio": 0.3,                // ancho del panel izquierdo
  "enabledAgents": ["claude", "codex"],
  "shell": "C:\\Windows\\system32\\cmd.exe",   // o powershell.exe / pwsh.exe
  "fontSize": 14,
  "agents": {
    "claude": { "command": "claude", "extraArgs": "--model opus", "dataDir": "" },
    "codex":  { "command": "C:\\ruta\\a\\codex.cmd", "extraArgs": "", "dataDir": "" }
    // dataDir vacío = ubicación por defecto
  }
}
```

## Estructura

```
core/              ← lógica compartida, Node puro (la usará también la versión TUI)
  index.js           registro de agentes, detección, listado unificado y comandos
  util.js            lectura eficiente de .jsonl, hashes, sqlite
  agents/*.js        un archivo por agente: dónde leer, cómo parsear y cómo retomar
main.js            Electron: ventana, config y terminales (node-pty)
preload.js         puente seguro entre la UI y el proceso principal
renderer/          UI (HTML/CSS/JS + xterm.js)
```

### Agregar otro agente

Crea `core/agents/miagente.js` exportando
`{ id, name, badge, color, defaultCommand, defaultDataDir(), list(dataDir), resumeArgs(id), newArgs() }`
y regístralo en `core/index.js`. La UI lo muestra automáticamente.

## Problemas comunes

- **"no se reconoce como comando"**: busca la ruta con `where codex` (o el agente que sea) y ponla en el campo de comando en ⚙.
- **No aparecen las sesiones de un agente**: revisa que la ruta del estado de ⚙ sea correcta. Si no lo es, ajusta `dataDir`.
- **Error con node-pty**: borra `node_modules` y ejecuta `npm install` otra vez.
