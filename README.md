# Resume AI

List every AI coding-agent session on your machine and resume any of them in one keystroke.

Supported agents: **Claude Code**, **Codex CLI**, **Gemini CLI**, **opencode**, **GitHub Copilot CLI**
and **Cursor CLI** (experimental).

> Status: early development. The Rust core, the interactive CLI and the desktop app (Tauri) work;
> packaging and releases are next. See [ROADMAP.md](ROADMAP.md).

## Install

**Command-line app** (`resume-ai`):

```sh
# npm / pnpm (any OS with Node.js)
npm install -g resume-ai-cli
pnpm add -g resume-ai-cli

# macOS / Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/nickmeza/resume-ai/releases/latest/download/resume-ai-installer.sh | sh

# Windows (PowerShell)
powershell -ExecutionPolicy Bypass -c "irm https://github.com/nickmeza/resume-ai/releases/latest/download/resume-ai-installer.ps1 | iex"
```

Then run `resume-ai` (press `?` for the shortcuts).

**Desktop app**: download the installer for your system from the
[latest release](https://github.com/nickmeza/resume-ai/releases/latest) —
`.exe` / `.msi` (Windows), `.dmg` (macOS), `.AppImage` / `.deb` / `.rpm` (Linux).

> The installers are not code-signed yet: Windows may show a SmartScreen warning ("More info" → "Run anyway")
> and macOS may ask you to open the app from System Settings → Privacy & Security.

## Repository layout

| Path | What it is |
|---|---|
| `crates/resume-ai-core` | Rust library: finds sessions of every agent, builds resume commands, rename/delete, Claude images |
| `crates/resume-ai` | The `resume-ai` command-line tool |
| `apps/desktop` | Desktop app (Tauri v2): Rust backend in `src-tauri`, web UI in `ui` |
| `legacy/electron` | Previous desktop app (Electron), kept until the Tauri app is released |

## Development

Requirements: [Rust](https://rustup.rs) (stable). On Windows you also need the Visual Studio C++ Build Tools.

```sh
cargo run                       # interactive mode: sessions list + agent tabs (press ? for help)
cargo build                     # build everything
cargo test                      # unit tests
cargo run -- list               # list sessions of all agents
cargo run -- list --json        # same, as JSON
cargo run -- agents             # which agents are installed
cargo run -- images <session>   # images of a Claude Code session, grouped by message
cargo run -- config init        # create the config file (language, agents, shortcuts)
```

Desktop app (needs Node.js for the Tauri CLI and xterm.js):

```sh
cd apps/desktop
npm install
npm run dev      # run in development
npm run build    # installers in target/release/bundle/
```

The legacy Electron app still runs from its folder:

```sh
cd legacy/electron
npm install
npm start
```

## License

[MIT](LICENSE)
