# Resume AI — Roadmap

Resume AI lists every AI coding-agent session on your machine (Claude Code, Codex, Gemini, opencode,
Copilot, Cursor) and resumes any of them in one keystroke. It ships as:

- **CLI / TUI** — `resume-ai`, installable with npm/pnpm, Homebrew, a shell script or a plain binary.
- **Desktop app** — `.exe`/`.msi` (Windows), `.dmg`/`.app` (macOS), `.AppImage`/`.deb`/`.rpm` (Linux).

Everything is English-first. User-facing text goes through i18n (`en`, `es`): the CLI picks the
language from its config file, the desktop app from a selector in Settings.

## Repository layout (monorepo)

```
resume-ai/
├── Cargo.toml                 Rust workspace
├── crates/
│   ├── resume-ai-core/          Session discovery for every agent, commands, rename/delete,
│   │                          compaction, Claude image extraction, i18n (locales/en.json, es.json).
│   │                          No UI, no Electron/Tauri.
│   └── resume-ai/               The CLI/TUI binary (ratatui). Package name on npm: resume-ai-cli
├── apps/
│   └── desktop/               Tauri v2 app: Rust backend on resume-ai-core + the web UI
├── legacy/electron/           Current Electron app, kept working until the Tauri app replaces it
└── .github/workflows/         CI and releases
```

## Phases

### 0. Repository setup
- [x] Install Rust toolchain (MSVC) on the Windows dev machine
- [x] Initialise git, Cargo workspace, move the Electron app to `legacy/electron/`
- [x] Choose license (MIT)
- [ ] CONTRIBUTING, code of conduct

### 1. `resume-ai-core` (Rust port of `legacy/electron/core`)
- [x] Shared utilities: jsonl head/tail reading, prompt cleaning, path hashing (sha256/md5), SQLite (`rusqlite`)
- [x] Providers: Claude Code, Codex, Gemini, opencode, Copilot, Cursor
- [x] Unified listing (parallel), hashed-cwd resolution, agent detection (`where`/`which`)
- [x] Command building: resume, new, fork (Claude), compact command per agent
- [x] Rename (native for Claude, alias otherwise), delete paths (trash), origin (`cli`/`claude-vscode`/`claude-desktop`)
- [x] Claude session images grouped by message (`imagePasteIds` ↔ `[Image #N]`)
- [x] Parity test: Rust output == legacy JS output on real session data (32 sessions, 0 differences; images byte-identical)

### 2. Internationalisation
- [x] `crates/resume-ai-core/locales/{en,es}.json` (inside the crate so it can be published), shared with the desktop app
- [x] Core returns codes (e.g. `origin: "claude-vscode"`, empty title = untitled), never display text
- [x] Fallback to English for missing keys; plurals (`.one`/`.other`); `{placeholders}`; system language detection
- [x] Test: every language has every key with the same placeholders
- [x] CLI: `language` in `config.toml`, `RESUME_AI_LANG`, `--lang`; `resume-ai config path|init`
- [ ] Translate `--help` texts (clap) — currently English only

### 3. CLI / TUI — `resume-ai`
- [x] Interactive list grouped by project (case-insensitive folders on Windows), details panel, search, filter popup (agent, origin)
- [x] Tabs with embedded agent terminals (portable-pty + vt100), like Neovim's `:terminal`; answers DSR/DA/OSC colour queries
- [x] New session, terminal tab in the project, resume in a new system terminal window (Windows Terminal, macOS Terminal, `$TERMINAL`)
- [x] Rename (native for Claude, shared aliases file otherwise), delete (to trash), continue compacted (auto-sends the compact command)
- [x] Claude images grouped by message (`[Image #N]`), opened with the system viewer
- [x] Config file with `language`, enabled agents, agent commands and `[keys]` (global shortcuts)
- [x] Scriptable subcommands: `list --json`, `agents`, `images`, `config path|init`
- [ ] `resume-ai resume <id>` (resume directly, no TUI)
- [ ] Mouse support (click tabs/rows, wheel scroll in the history)
- [ ] Inline image previews in terminals that support them (Kitty/iTerm2/Sixel)
- [ ] Reload environment (PATH) for a tab, like the desktop app's Ctrl+Shift+R
- [ ] Test the TUI by hand on macOS and Linux terminals

### 4. Desktop app (Tauri v2)
- [x] Port `main.js` to Tauri commands backed by `resume-ai-core` (`apps/desktop/src-tauri`)
- [x] Terminals with `portable-pty` (agent runs inside the user's shell), clipboard images with `arboard`, trash via core, native delete confirmation
- [x] Reuse the UI (xterm.js, tabs, filters, gallery, shortcuts); `window.api` bridge over Tauri `invoke`/events (`ui/api.js`)
- [x] Whole UI in English with i18n (`ui.*` keys); language selector in Settings, applied live without closing tabs
- [x] Imports the Electron app's settings and aliases on first run; aliases shared with the CLI
- [x] Environment reload from the Windows registry (Rust port), browser shortcuts that would reload the page are blocked
- [x] Debug smoke check (`RESUME_AI_SMOKE=1`) that reports what the UI rendered
- [ ] Remove `legacy/electron/` once the first release is out
- [ ] Test on macOS and Linux

### 5. Packaging & releases (GitHub Actions)
- [ ] CLI with **cargo-dist**: binaries for Windows/macOS/Linux (x64 + arm64), shell & PowerShell
      installers, npm package `resume-ai-cli` (`npm i -g resume-ai-cli`, `pnpm add -g resume-ai-cli`), Homebrew tap
- [ ] Desktop with **tauri-action**: NSIS `.exe` + `.msi`, universal `.dmg`, `.AppImage`/`.deb`/`.rpm`
- [ ] Auto-update for the desktop app (Tauri updater)
- [ ] Code signing: Windows (avoid SmartScreen warnings), macOS notarization (Apple Developer account)

### 6. macOS / Linux parity
- [ ] Environment reload via the login shell (`$SHELL -ilc env`)
- [ ] `Cmd` as the primary modifier on macOS
- [ ] Test every provider's data paths on each OS

### 7. Public launch
- [ ] README with screenshots/GIFs, install instructions per platform
- [ ] CI: `cargo fmt`, `clippy`, tests on Windows/macOS/Linux
- [ ] Issue/PR templates, changelog, first tagged release

## Open decisions
- npm package: `resume-ai-cli` (free) vs scoped `@resume-ai/cli` (needs the `resume-ai` npm org)
- GitHub repository owner/name
- Code-signing certificates (cost and who holds them)
