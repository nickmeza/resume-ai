//! Environment of the terminals, and its reload (the equivalent of `source ~/.zshrc`):
//! picks up PATH and variables of programs installed after the app started.

use anyhow::{Result, bail};
use std::collections::HashMap;

/// Variable lookup ignoring case (on Windows it is "Path", not "PATH").
#[cfg(windows)]
fn key_of(env: &HashMap<String, String>, name: &str) -> String {
    env.keys().find(|k| k.eq_ignore_ascii_case(name)).cloned().unwrap_or_else(|| name.to_string())
}

#[cfg(windows)]
pub fn refreshed() -> Result<HashMap<String, String>> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    struct Var {
        name: String,
        value: String,
        expand: bool,
    }
    fn read(root: RegKey, path: &str) -> Vec<Var> {
        let Ok(key) = root.open_subkey(path) else { return Vec::new() };
        key.enum_values()
            .filter_map(|v| v.ok())
            .filter_map(|(name, raw)| {
                use winreg::enums::RegType::{REG_EXPAND_SZ, REG_SZ};
                let expand = match raw.vtype {
                    REG_SZ => false,
                    REG_EXPAND_SZ => true,
                    _ => return None,
                };
                let value: String = winreg::types::FromRegValue::from_reg_value(&raw).ok()?;
                Some(Var { name, value, expand })
            })
            .collect()
    }

    let machine = read(RegKey::predef(HKEY_LOCAL_MACHINE), r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment");
    let user = read(RegKey::predef(HKEY_CURRENT_USER), "Environment");
    if machine.is_empty() && user.is_empty() {
        bail!("could not read the environment variables from the registry");
    }
    let mut env: HashMap<String, String> = std::env::vars().collect();
    let expand = |v: &str, env: &HashMap<String, String>| -> String {
        let mut out = String::new();
        let mut rest = v;
        while let Some(start) = rest.find('%') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            match after.find('%') {
                Some(end) => {
                    let name = &after[..end];
                    match env.get(&key_of(env, name)) {
                        Some(val) => out.push_str(val),
                        None => out.push_str(&format!("%{name}%")),
                    }
                    rest = &after[end + 1..];
                }
                None => {
                    out.push_str(&rest[start..]);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        out
    };
    // User variables apply after machine ones (like Windows). Path goes last because it may use
    // variables defined further down (%PNPM_HOME%, %NVM_HOME%…).
    let vars: Vec<Var> = machine.into_iter().chain(user).collect();
    let is_path = |v: &Var| v.name.eq_ignore_ascii_case("path");
    for v in vars.iter().filter(|v| !is_path(v)) {
        env.insert(key_of(&env, &v.name), v.value.clone());
    }
    for v in vars.iter().filter(|v| !is_path(v) && v.expand) {
        let value = expand(&v.value, &env);
        env.insert(key_of(&env, &v.name), value);
    }
    let paths: Vec<String> = vars
        .iter()
        .filter(|v| is_path(v))
        .map(|v| if v.expand { expand(&v.value, &env) } else { v.value.clone() })
        .map(|p| p.trim_end_matches(';').to_string())
        .collect();
    if !paths.is_empty() {
        env.insert(key_of(&env, "Path"), paths.join(";"));
    }
    Ok(env)
}

#[cfg(not(windows))]
pub fn refreshed() -> Result<HashMap<String, String>> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let out = std::process::Command::new(&shell).args(["-ilc", "env"]).output()?;
    if !out.status.success() {
        bail!("could not read the environment of {shell}");
    }
    let mut env: HashMap<String, String> = std::env::vars().collect();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some((k, v)) = line.split_once('=') {
            env.insert(k.to_string(), v.to_string());
        }
    }
    Ok(env)
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn reads_the_registry_and_expands_path() {
        let env = super::refreshed().unwrap();
        let path = env.get(&super::key_of(&env, "Path")).unwrap();
        assert!(path.to_lowercase().contains("system32"));
        assert!(!regex_like_unexpanded(path), "unexpanded variable left in PATH: {path}");
    }

    fn regex_like_unexpanded(p: &str) -> bool {
        p.split(';').any(|part| part.starts_with('%') && part[1..].contains('%'))
    }
}
