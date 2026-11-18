//! Translations shared by the CLI and the desktop app.
//!
//! - Keys are flat (`"origin.claude-vscode"`), one JSON file per language in `locales/`.
//! - `{name}` placeholders are filled by [`I18n::tf`].
//! - Plurals use `.one` / `.other` key suffixes ([`I18n::plural`]).
//! - Missing keys fall back to English, then to the key itself.
//!
//! To add a language: create `locales/<code>.json` with every key of `en.json` and add it to [`LANGUAGES`].

use std::collections::HashMap;
use std::fmt::Display;

/// Supported languages: (code, native name).
pub const LANGUAGES: &[(&str, &str)] = &[("en", "English"), ("es", "Español")];

const EN: &str = include_str!("../locales/en.json");
const ES: &str = include_str!("../locales/es.json");

/// Raw JSON of a language bundle (the desktop app ships it to the web UI as-is).
pub fn bundle_json(lang: &str) -> Option<&'static str> {
    match lang {
        "en" => Some(EN),
        "es" => Some(ES),
        _ => None,
    }
}

fn parse(json: &str) -> HashMap<String, String> {
    let v: serde_json::Value = serde_json::from_str(json).expect("locale files are valid JSON (checked by tests)");
    v.as_object().map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()).unwrap_or_default()
}

/// Whether `code` is a supported language.
pub fn is_supported(code: &str) -> bool {
    LANGUAGES.iter().any(|(c, _)| *c == code)
}

/// Language of the operating system, if supported (`es-MX` → `es`).
pub fn system_language() -> Option<&'static str> {
    let locale = sys_locale::get_locale()?;
    let prefix = locale.split(['-', '_']).next()?.to_lowercase();
    LANGUAGES.iter().map(|(c, _)| *c).find(|c| *c == prefix)
}

/// Turns a user preference (`"auto"`, `""`, `"es"`, `"es-AR"`…) into a supported language code.
pub fn resolve_language(pref: &str) -> &'static str {
    let pref = pref.trim().to_lowercase();
    if pref.is_empty() || pref == "auto" {
        return system_language().unwrap_or("en");
    }
    let prefix = pref.split(['-', '_']).next().unwrap_or("");
    LANGUAGES.iter().map(|(c, _)| *c).find(|c| *c == prefix).unwrap_or("en")
}

pub struct I18n {
    lang: &'static str,
    messages: HashMap<String, String>,
    fallback: HashMap<String, String>,
}

impl I18n {
    /// `pref` is the user's setting (`"auto"` follows the system).
    pub fn new(pref: &str) -> Self {
        let lang = resolve_language(pref);
        I18n { lang, messages: parse(bundle_json(lang).unwrap_or(EN)), fallback: parse(EN) }
    }

    pub fn lang(&self) -> &'static str {
        self.lang
    }

    pub fn t(&self, key: &str) -> String {
        self.messages.get(key).or_else(|| self.fallback.get(key)).cloned().unwrap_or_else(|| key.to_string())
    }

    /// Translation with `{name}` placeholders.
    pub fn tf(&self, key: &str, args: &[(&str, &dyn Display)]) -> String {
        let mut s = self.t(key);
        for (name, value) in args {
            s = s.replace(&format!("{{{name}}}"), &value.to_string());
        }
        s
    }

    /// Plural form (`key.one` / `key.other`); `{count}` is filled automatically.
    pub fn plural(&self, key: &str, count: u64, args: &[(&str, &dyn Display)]) -> String {
        let form = if count == 1 { "one" } else { "other" };
        let mut all: Vec<(&str, &dyn Display)> = vec![("count", &count)];
        all.extend_from_slice(args);
        self.tf(&format!("{key}.{form}"), &all)
    }

    /// Display name of a session origin code (`claude-vscode` → "VS Code").
    pub fn origin(&self, code: &str) -> String {
        let key = format!("origin.{code}");
        let s = self.t(&key);
        if s == key { code.to_string() } else { s }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;
    use std::collections::BTreeSet;

    fn placeholders(s: &str) -> BTreeSet<String> {
        Regex::new(r"\{(\w+)\}").unwrap().captures_iter(s).map(|c| c[1].to_string()).collect()
    }

    #[test]
    fn every_language_has_every_key_with_the_same_placeholders() {
        let en = parse(EN);
        for (code, _) in LANGUAGES {
            let other = parse(bundle_json(code).unwrap());
            for (key, text) in &en {
                let t = other.get(key).unwrap_or_else(|| panic!("{code}.json is missing \"{key}\""));
                assert_eq!(placeholders(text), placeholders(t), "placeholders differ for \"{key}\" in {code}.json");
            }
            for key in other.keys() {
                assert!(en.contains_key(key), "{code}.json has \"{key}\", which en.json lacks");
            }
        }
    }

    #[test]
    fn translates_with_fallbacks() {
        let es = I18n::new("es-AR");
        assert_eq!(es.lang(), "es");
        assert_eq!(es.origin("claude-desktop"), "App de escritorio");
        assert_eq!(es.origin("some-new-ide"), "some-new-ide");
        assert_eq!(es.plural("agents.sessions", 1, &[]), "1 sesión");
        assert_eq!(es.plural("agents.sessions", 3, &[]), "3 sesiones");
        assert_eq!(es.t("no.such.key"), "no.such.key");
        assert_eq!(I18n::new("fr").lang(), "en");
    }
}
