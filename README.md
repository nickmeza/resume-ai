<div align="center">

<img src="apps/desktop/src-tauri/icons/logo.svg" width="96" alt="resume-ai logo" />

# resume-ai

**Every AI coding session on your machine, in one place. Pick one and you are back in it.**

Claude Code · Codex CLI · Gemini CLI · opencode · GitHub Copilot CLI · Cursor CLI

[Install](#install) · [Command line](#the-command-line-app) · [Desktop app](#the-desktop-app) · [How it works](#how-it-works) · [Configuration](#configuration) · [FAQ](#faq)

</div>

---

## Why

AI coding agents save every conversation, but each one hides them in its own folder, with its own
format and its own `--resume` flag. After a few days you have dozens of sessions spread over many
projects and no good way to find "the one where we fixed the login".

**resume-ai** finds the sessions of every agent you use, groups them by project, lets you search and
filter them, and resumes any of them with one key — in the right folder, with the right command.
Several sessions can run side by side in tabs.

```
 ≡ Sessions (32)              │  1  CC  Fix the login redirect   2  CX  Add tests to the API
 / search…                    │
  ▾ WEB-APP  12               │  ╭───────────────────────────────────────────────╮
▌ ●  CC  Fix the login red…   │  │ ✻ Welcome back! Resuming your session…        │
     CX  Add tests to the A…  │  ╰───────────────────────────────────────────────╯
     GM  Explain the router   │
  ▾ MOBILE  5                 │  > the redirect still loops after logout
     CC  Push notifications   │
     OC  Refactor payments    │  ⏺ Let me look at the auth middleware…
  ▾ API  9                    │
     CC  Rate limiting        │
 TERMINAL  Alt+S: back to the sessions list · Alt+I: images · Shift+←/→: switch tab · Ctrl+W: close tab
```

It comes in two flavours that share the same engine:

| | **Command line** (`resume-ai`) | **Desktop app** (Resume AI) |
|---|---|---|
| Where | Any terminal, also over SSH | Windows, macOS, Linux window |
| Agents run in | Tabs inside the terminal (like Neovim's `:terminal`) | Tabs with a full terminal emulator |
| Extras | Mouse support, scriptable subcommands (`list --json`) | Right-click menus, drag & drop, image previews, gallery |
| Size | ~5 MB single binary | ~4 MB installer |

## Features

- **One list for every agent** — grouped by project folder, newest first, with a coloured tag per agent.
- **Search and filters** — by title, folder, branch or id; filter by agent or by where the session was
  created (terminal, VS Code, Claude desktop app).
- **Resume in tabs** — every session opens in its own tab, in its original folder. Keep several agents
  running at once and jump between them with the keyboard.
- **Continue compacted** — resume a long session and send the agent's compact command (`/compact`,
  `/compress`) automatically, so the next messages do not resend the whole context. For Claude Code it
  works on a copy (`--fork-session`), leaving the original untouched.
- **Rename and delete** — Claude Code sessions are renamed natively (like its `/rename`); for the other
  agents the name is kept by resume-ai. Deleting moves the session's files to the system trash.
- **Images** — see the images you sent in a Claude Code session, grouped by message, matching the
  `[Image #N]` references; paste screenshots straight into an agent (they are saved as files and pasted
  as a path).
- **English and Spanish** — the language follows your system, or pick it in the settings.
- **Private** — it only reads files on your machine. No account, no network calls, no telemetry.

## Install

### Command line

```sh
# npm or pnpm (any OS with Node.js)
npm install -g resume-ai-cli
pnpm add -g resume-ai-cli

# macOS / Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/nickmeza/resume-ai/releases/latest/download/resume-ai-installer.sh | sh

# Windows (PowerShell)
powershell -ExecutionPolicy Bypass -c "irm https://github.com/nickmeza/resume-ai/releases/latest/download/resume-ai-installer.ps1 | iex"
```

Then run `resume-ai`. The first time it asks two questions: the language and how wide the sessions
list should be.

> The npm package is called `resume-ai-cli` (the name `resume-ai` was taken on npm); the command it
> installs is `resume-ai`.

### Desktop app

Download the installer for your system from the
**[latest release](https://github.com/nickmeza/resume-ai/releases/latest)**:

| System | File |
|---|---|
| Windows | `resume-ai_x.y.z_x64-setup.exe` or `.msi` |
| macOS (Intel and Apple Silicon) | `resume-ai_x.y.z_universal.dmg` |
| Linux | `.AppImage`, `.deb` or `.rpm` |

> The installers are not code-signed yet. Windows may show "Windows protected your PC" →
> **More info** → **Run anyway**. On macOS, if it says the app cannot be opened, go to
> **System Settings → Privacy & Security** and click **Open anyway**.

## The command line app

Run `resume-ai`. The sessions list is always on the left; the agent you open runs on the right.
The bottom bar tells you where the keyboard is (**SESSIONS** or **TERMINAL**) and which keys work there.

**In the sessions list**

| Key | Action |
|---|---|
| `↑` `↓` | Move |
| `Enter` | Open the session in a tab (or switch to it) |
| `Tab` | Go to the terminal without opening anything |
| `/` | Search |
| `f` | Filters (agents, origin) |
| `n` | New session in the selected project |
| `c` | Continue compacted |
| `r` / `F2` | Rename |
| `d` / `Del` | Delete (to the trash) |
| `i` | Images of the session (Claude Code) |
| `o` | Open the project folder |
| `t` | Plain terminal in the project |
| `w` | Resume in a new window of your terminal app |
| `R` / `F5` | Reload the list |
| `?` | All the shortcuts |
| `q` | Quit |

**Everywhere, also while typing in an agent**

| Key | Action |
|---|---|
| `Alt+S` | Switch between the sessions list and the terminal |
| `Alt+I` | Images of the session in the active tab |
| `Shift+←` / `Shift+→` | Previous / next tab |
| `Alt+1` … `Alt+9` | Go to tab N (`Alt+0`: the sessions list) |
| `Ctrl+W` | Close the tab |
| `Shift+PgUp` / `Shift+PgDn` | Scroll the terminal history |

**Mouse**: the wheel scrolls the pane under the pointer (it never reaches the agent as ↑↓, which would
change its prompt), a click selects or focuses, a double-click opens, and clicking an `[Image #N]`
reference opens that image. Hold **Shift** while dragging to select text.

**Scripting**

```sh
resume-ai list                 # table of sessions
resume-ai list --json          # everything, as JSON
resume-ai agents               # which agents are installed and how many sessions each has
resume-ai images <session-id>  # images of a Claude Code session, grouped by message
resume-ai config setup         # ask the language and list width again
resume-ai config path          # where the config file is
```

## The desktop app

The same list on the left and real terminals on the right, plus what a window makes easy:

- **Right-click a session**: open, continue compacted, view images, rename, copy id, open the folder,
  open a terminal there, delete.
- **Tabs** you can drag to reorder; `Ctrl+1…9` to jump, `Shift+←/→` to cycle.
- **Images**: hover an `[Image #N]` in the terminal to preview it, `Ctrl+click` to open it in a tab;
  the 🖼 panel shows every image of the session grouped by message.
- **Paste screenshots** with `Ctrl+V` and **drop files** on a terminal: their paths are typed for you,
  and agents attach the images.
- **Reload terminal** (`Ctrl+Shift+R`): picks up PATH changes after installing something, without
  closing the app — like `source ~/.zshrc`.
- **Settings** (⌨): language, shell, font size and every keyboard shortcut.

## How it works

resume-ai reads the files each agent already writes. It does not change how the agents work and does
not need them to be running.

| Agent | Where its sessions live | How it is resumed |
|---|---|---|
| **Claude Code** | `~/.claude/projects/<folder>/<id>.jsonl` | `claude --resume <id>` |
| **Codex CLI** | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` | `codex resume <id>` |
| **Gemini CLI** | `~/.gemini/tmp/<project>/chats/session-*.json` | `gemini --resume <id>` |
| **opencode** | `~/.local/share/opencode/opencode.db` | `opencode --session <id>` |
| **GitHub Copilot CLI** | `~/.copilot/session-state/<id>/` | `copilot --resume=<id>` |
| **Cursor CLI** *(experimental)* | `~/.cursor/chats/<hash>/<id>/store.db` | `cursor-agent --resume <id>` |

`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `GEMINI_CLI_HOME`, `XDG_DATA_HOME` and `COPILOT_HOME` are respected.

- Sessions are tied to a project folder, so each one is resumed **in its original folder**. Gemini and
  Cursor store the folder as a hash; resume-ai recovers it by matching the folders it knows.
- Large session files are read only at the start and the end, so listing hundreds of sessions takes a
  fraction of a second.
- Renaming a Claude Code session appends a `custom-title` entry (what its `/rename` does). Deleting
  moves the files to the trash; opencode keeps every session in one database, so deleting is disabled
  for it.
- Claude Code stores the images you send inside the session file, next to the numbers shown as
  `[Image #N]`. resume-ai extracts them to a temporary folder only when you ask to see them.

## Configuration

**Command line**: `config.toml`, in `%APPDATA%\resume-ai\` (Windows) or `~/.config/resume-ai/`
(macOS/Linux). `resume-ai config init` creates it with every option commented.

```toml
language = "auto"        # "auto" (system), "en" or "es"
sidebar_percent = 25     # width of the sessions list (10–60)

# enabled_agents = ["claude", "codex"]   # empty = all

[agents.claude]
extra_args = "--model opus"   # appended to every launch
# command = "C:\\tools\\claude.exe"     # full path if it is not on PATH

[keys]
toggle_focus = "Alt+S"
close_tab = "Ctrl+W"
```

`RESUME_AI_LANG=es` overrides the language for one run, and `RESUME_AI_CONFIG=/path/to/file.toml` uses
another config file.

**Desktop app**: everything is in the ⌨ settings; the file behind it opens from ⚙ → *Edit config.json*.
Names given to sessions are shared between the command line and the desktop app.

## FAQ

**An agent does not show up.** Run `resume-ai agents`: it shows where each agent's data was looked for
and whether its command was found. Set `command` or `data_dir` for that agent in the config.

**"Command not found" when opening a session.** The agent is not on your `PATH`. Put its full path in
the config (`[agents.<id>] command = "…"`), or in the desktop app in ⚙.

**Does continuing compacted save tokens?** Compacting costs tokens once (the agent summarises the
history); the messages after it no longer resend the whole conversation, so it pays off when you keep
working in that session.

**Is my code or conversation sent anywhere?** No. resume-ai only reads local files and starts the
agents you already use.

## Development

```
crates/resume-ai-core   Rust library: discovery of every agent's sessions, resume commands,
                        rename/delete, compaction, Claude images, translations (locales/)
crates/resume-ai        The command line app (ratatui, embedded terminals with portable-pty + vt100)
apps/desktop            The desktop app (Tauri v2): Rust backend in src-tauri/, web UI in ui/
```

Requirements: [Rust](https://rustup.rs) (stable) and, for the desktop app, Node.js. On Windows you
also need the Visual Studio C++ Build Tools.

```sh
cargo run                    # command line app
cargo test --workspace       # tests

cd apps/desktop && npm install
npm run dev                  # desktop app in development
npm run build                # installers in target/release/bundle/
```

Releases are built by GitHub Actions when a `v*` tag is pushed (cargo-dist for the command line,
tauri-action for the desktop app). Adding an agent means one file in `crates/resume-ai-core/src/agents/`
implementing the `Provider` trait. Translations live in `crates/resume-ai-core/locales/`; a test makes
sure every language has every key. See [ROADMAP.md](ROADMAP.md) for what comes next.

## License

[MIT](LICENSE)
