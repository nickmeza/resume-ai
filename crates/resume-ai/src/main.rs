//! `resume-ai` — list and resume AI coding-agent sessions.
//! Without a subcommand it opens the interactive mode; subcommands are for scripts.

mod config;
mod setup;
mod tui;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use core::organization::{self, Change, RECENT_LIMIT, View};
use resume_ai_core::{self as core, i18n::I18n};

#[derive(Parser)]
#[command(name = "resume-ai", version, about = "List and resume your AI coding-agent sessions")]
struct Cli {
    /// Interface language (en, es, auto). Overrides the config file and RESUME_AI_LANG
    #[arg(long, global = true)]
    lang: Option<String>,
    /// Without a subcommand, the interactive mode opens
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// List sessions, newest first
    List {
        /// Only these agents (comma separated). Default: the enabled ones
        #[arg(long, value_delimiter = ',')]
        agent: Vec<String>,
        /// Print JSON instead of a table
        #[arg(long)]
        json: bool,
        /// Show only favorites
        #[arg(long, conflicts_with_all = ["recent", "hidden"])]
        favorites: bool,
        /// Show the five most recent visible sessions
        #[arg(long, conflicts_with = "hidden")]
        recent: bool,
        /// Show hidden sessions
        #[arg(long)]
        hidden: bool,
    },
    /// Toggle the favorite star for a session
    Favorite {
        /// Full session key (agent:id)
        key: String,
    },
    /// Toggle visibility for a session, or hide its project's non-favorites
    Hide {
        /// Full session key (agent:id)
        key: String,
        /// Hide sessions in the same project while keeping favorites visible
        #[arg(long)]
        project: bool,
    },
    /// Show which agents are installed and how many sessions each has
    Agents {
        #[arg(long)]
        json: bool,
    },
    /// Images sent in a Claude Code session, grouped by message
    Images {
        /// Session id or key (`claude:<id>`)
        session: String,
        /// Extract the images into this folder
        #[arg(long)]
        out: Option<std::path::PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Show or create the config file
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Print where the config file lives
    Path,
    /// Create the config file with commented defaults
    Init,
    /// Ask the setup questions again (language, width of the sessions list)
    Setup,
}

/// Language preference: --lang > RESUME_AI_LANG > config file > system.
fn language_pref(cli: &Cli, cfg: &config::Config) -> Result<String> {
    let pref = cli.lang.clone().or_else(|| std::env::var("RESUME_AI_LANG").ok()).unwrap_or_else(|| cfg.language.clone());
    let base = pref.trim().to_lowercase();
    let code = base.split(['-', '_']).next().unwrap_or("");
    if !(base.is_empty() || base == "auto" || core::i18n::is_supported(code)) {
        let available: Vec<&str> = core::i18n::LANGUAGES.iter().map(|(c, _)| *c).collect();
        let t = I18n::new("auto");
        bail!(t.tf("config.unknownLanguage", &[("lang", &pref), ("available", &available.join(", "))]));
    }
    Ok(pref)
}

fn main() -> Result<()> {
    use std::io::IsTerminal;
    let cli = Cli::parse();
    // First run of the interactive mode: ask the language and the list width before opening it
    let wants_setup = matches!(cli.command, Some(Command::Config { action: ConfigAction::Setup }));
    let first_run = cli.command.is_none() && !config::path().exists() && std::io::stdin().is_terminal();
    if wants_setup || first_run {
        setup::run(&config::load().unwrap_or_default())?;
        if wants_setup {
            return Ok(());
        }
    }
    let file_cfg = match config::load() {
        Ok(c) => c,
        Err(e) => {
            let t = I18n::new("auto");
            bail!(t.tf("config.invalid", &[("path", &config::path().display()), ("error", &format!("{e:#}"))]));
        }
    };
    let t = I18n::new(&language_pref(&cli, &file_cfg)?);
    let agent_cfg = file_cfg.agent_config();

    let Some(command) = cli.command else {
        return tui::run(file_cfg, t);
    };
    match command {
        Command::List { agent, json, favorites, recent, hidden } => {
            let enabled = if agent.is_empty() { file_cfg.enabled() } else { agent };
            let mut res = core::list_all(&enabled, &agent_cfg);
            let organization = organization::load()?;
            let view = if favorites {
                View::Favorites
            } else if hidden {
                View::Hidden
            } else if recent {
                View::Recent
            } else {
                View::All
            };
            res.sessions.retain(|s| organization.includes(&s.key, view, false));
            if view == View::Recent {
                res.sessions.truncate(RECENT_LIMIT);
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&res)?);
                return Ok(());
            }
            if res.sessions.is_empty() {
                println!("{}", t.t("list.empty"));
            }
            for s in &res.sessions {
                let badge = core::by_id(&s.agent).map(|a| a.badge()).unwrap_or("??");
                let title = if s.title.is_empty() { t.t("session.untitled") } else { s.title.clone() };
                let title: String = title.chars().take(70).collect();
                let folder = s.cwd.clone().unwrap_or_else(|| t.t("session.unknownFolder"));
                let origin =
                    s.origin.as_deref().filter(|o| *o != "cli").map(|o| format!("  [{}]", t.origin(o))).unwrap_or_default();
                println!("{badge}  {title:<70}  {folder}{origin}");
            }
            for e in &res.errors {
                eprintln!("{}", t.tf("list.agentError", &[("agent", &e.agent), ("error", &e.error)]));
            }
        }
        Command::Favorite { ref key } | Command::Hide { ref key, .. } => {
            let enabled = core::AGENTS.iter().map(|agent| agent.id().to_string()).collect::<Vec<_>>();
            let res = core::list_all(&enabled, &agent_cfg);
            let Some(session) = res.sessions.iter().find(|s| &s.key == key) else {
                bail!(t.t("organize.notFound"));
            };
            let action = match command {
                Command::Favorite { .. } => Change::Favorite,
                Command::Hide { project: true, .. } => Change::HideProject,
                _ => Change::Hide,
            };
            let updated = organization::change(session, &res.sessions, action)?;
            let label = match action {
                Change::Favorite if updated.favorites.contains(key) => "organize.markedFavorite",
                Change::Favorite => "organize.removedFavorite",
                Change::Hide if updated.hidden.contains(key) => "organize.sessionHidden",
                Change::Hide => "organize.sessionRestored",
                Change::HideProject => "organize.projectHidden",
            };
            println!("{}", t.t(label));
        }
        Command::Agents { json } => {
            let det = core::detect_agents(&agent_cfg);
            if json {
                println!("{}", serde_json::to_string_pretty(&det)?);
                return Ok(());
            }
            for d in det {
                let count = t.plural("agents.sessions", d.count as u64, &[]);
                let cmd = match d.command_path {
                    Some(p) => t.tf("agents.command", &[("path", &p.display())]),
                    None => t.t("agents.commandNotFound"),
                };
                println!("{:<9} {count:>14}  {cmd}", d.id);
            }
        }
        Command::Images { session, out, json } => {
            let id = session.strip_prefix("claude:").unwrap_or(&session);
            let res = core::list_all(&["claude".to_string()], &agent_cfg);
            let Some(s) = res.sessions.iter().find(|s| s.id == id) else {
                bail!(t.tf("images.sessionNotFound", &[("id", &id)]));
            };
            let msgs = core::claude::images(s, out.as_deref())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&msgs)?);
                return Ok(());
            }
            if msgs.is_empty() {
                println!("{}", t.t("images.none"));
                return Ok(());
            }
            let total: usize = msgs.iter().map(|m| m.images.len()).sum();
            println!("{}\n", t.plural("images.count", total as u64, &[("messages", &msgs.len())]));
            for m in &msgs {
                let refs: Vec<String> = m.images.iter().map(|i| i.n.map(|n| format!("#{n}")).unwrap_or("?".into())).collect();
                let text: String = m.text.chars().take(60).collect();
                println!("{}  {text}", refs.join(" "));
                for p in m.images.iter().filter_map(|i| i.path.as_ref()) {
                    println!("    {}", p.display());
                }
            }
        }
        Command::Config { action } => match action {
            ConfigAction::Path => println!("{}", t.tf("config.path", &[("path", &config::path().display())])),
            ConfigAction::Init => {
                let key = if config::init()? { "config.created" } else { "config.exists" };
                println!("{}", t.tf(key, &[("path", &config::path().display())]));
            }
            ConfigAction::Setup => {} // handled before loading the config
        },
    }
    Ok(())
}
