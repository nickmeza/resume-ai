//! `resume-ai` — list and resume AI coding-agent sessions.
//! Without a subcommand it opens the interactive mode; subcommands are for scripts.

mod config;
mod tui;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
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
    let cli = Cli::parse();
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
        Command::List { agent, json } => {
            let enabled = if agent.is_empty() { file_cfg.enabled() } else { agent };
            let res = core::list_all(&enabled, &agent_cfg);
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
        },
    }
    Ok(())
}
