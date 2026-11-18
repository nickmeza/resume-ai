//! First-run setup: asks the interface language and the width of the sessions list.
//! Runs the first time `resume-ai` opens (no config file yet) and with `resume-ai config setup`.

use crate::config::{self, Config, DEFAULT_SIDEBAR_PERCENT};
use anyhow::Result;
use resume_ai_core::i18n::{I18n, LANGUAGES};
use std::io::{BufRead, Write};

fn ask(prompt: &str) -> Result<String> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line)? == 0 {
        anyhow::bail!("no input"); // stdin closed (Ctrl+Z / Ctrl+D)
    }
    Ok(line.trim().to_string())
}

pub fn run(current: &Config) -> Result<()> {
    // The language is not known yet: the first question is shown in every language
    let all: Vec<I18n> = LANGUAGES.iter().map(|(code, _)| I18n::new(code)).collect();
    let both = |key: &str| all.iter().map(|t| t.t(key)).collect::<Vec<_>>().join(" / ");

    println!("\n  resume-ai — {}\n", both("setup.title"));
    println!("  {}:", both("setup.language"));
    println!("    1) {}", both("setup.languageAuto"));
    for (i, (_, name)) in LANGUAGES.iter().enumerate() {
        println!("    {}) {name}", i + 2);
    }
    let options: Vec<&str> = std::iter::once("auto").chain(LANGUAGES.iter().map(|(c, _)| *c)).collect();
    let current_choice = options.iter().position(|o| *o == current.language).unwrap_or(0) + 1;
    let language = loop {
        let answer = ask(&format!("\n  {} [1-{}, Enter = {current_choice}]: ", both("setup.choose"), options.len()))?;
        if answer.is_empty() {
            break options[current_choice - 1];
        }
        match answer.parse::<usize>() {
            Ok(n) if (1..=options.len()).contains(&n) => break options[n - 1],
            _ => println!("  {}", both("setup.invalid")),
        }
    };

    let t = I18n::new(language);
    let default = if current.sidebar_percent == 0 { DEFAULT_SIDEBAR_PERCENT } else { current.sidebar_percent };
    println!("\n  {}", t.t("setup.width"));
    let percent = loop {
        let answer = ask(&format!("  {} ", t.tf("setup.widthPrompt", &[("default", &default)])))?;
        if answer.is_empty() {
            break default;
        }
        match answer.trim_end_matches('%').trim().parse::<u16>() {
            Ok(n) if (10..=60).contains(&n) => break n,
            _ => println!("  {}", t.t("setup.invalidWidth")),
        }
    };

    config::save_basics(language, percent)?;
    println!("\n  ✓ {}\n", t.tf("setup.saved", &[("path", &config::path().display())]));
    Ok(())
}
