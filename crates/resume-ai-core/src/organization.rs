//! Session organization shared by desktop and CLI.

use crate::Session;
use anyhow::Result;
use rusqlite::{Connection, params};
use serde::Serialize;
use std::{collections::BTreeSet, fs, time::Duration};

pub const RECENT_LIMIT: usize = 5;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    Favorites,
    #[default]
    Recent,
    All,
    Hidden,
}

impl View {
    pub const ALL: [Self; 4] = [Self::Favorites, Self::Recent, Self::All, Self::Hidden];

    pub fn label(self) -> &'static str {
        match self {
            Self::Favorites => "organize.favorites",
            Self::Recent => "organize.recent",
            Self::All => "organize.all",
            Self::Hidden => "organize.hidden",
        }
    }
}

#[derive(Default, Serialize)]
pub struct Organization {
    pub favorites: BTreeSet<String>,
    pub hidden: BTreeSet<String>,
}

#[derive(Clone, Copy)]
pub enum Change {
    Favorite,
    Hide,
    HideProject,
}

impl Organization {
    pub fn includes(&self, key: &str, view: View, searching: bool) -> bool {
        if view == View::Hidden {
            return self.hidden.contains(key);
        }
        !self.hidden.contains(key) && (searching || view != View::Favorites || self.favorites.contains(key))
    }
}

fn open() -> Result<Connection> {
    let dir = dirs::data_dir().unwrap_or_else(crate::util::home).join("resume-ai");
    fs::create_dir_all(&dir)?;
    let db = Connection::open(dir.join("organization.sqlite3"))?;
    db.busy_timeout(Duration::from_secs(3))?;
    initialize(&db)?;
    Ok(db)
}

fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_flags (
            key TEXT NOT NULL,
            flag TEXT NOT NULL,
            PRIMARY KEY (key, flag)
        )",
    )?;
    Ok(())
}

fn read(db: &Connection) -> Result<Organization> {
    let mut result = Organization::default();
    let mut stmt = db.prepare("SELECT key, flag FROM session_flags")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        match row.get_ref(1)?.as_str()? {
            "favorite" => result.favorites.insert(row.get(0)?),
            "hidden" => result.hidden.insert(row.get(0)?),
            _ => continue,
        };
    }
    Ok(result)
}

pub fn load() -> Result<Organization> {
    read(&open()?)
}

pub fn project_key(path: &str) -> String {
    let path = path.trim_end_matches(['\\', '/']);
    if cfg!(windows) { path.replace('/', "\\").to_lowercase() } else { path.to_string() }
}

pub fn change<'a>(session: &Session, sessions: impl IntoIterator<Item = &'a Session>, action: Change) -> Result<Organization> {
    update(&mut open()?, session, sessions, action)
}

fn update<'a>(
    db: &mut Connection,
    session: &Session,
    sessions: impl IntoIterator<Item = &'a Session>,
    action: Change,
) -> Result<Organization> {
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    match action {
        Change::Favorite | Change::Hide => {
            let flag = if matches!(action, Change::Favorite) { "favorite" } else { "hidden" };
            if tx.execute("DELETE FROM session_flags WHERE key = ?1 AND flag = ?2", params![session.key, flag])? == 0 {
                tx.execute("INSERT INTO session_flags VALUES (?1, ?2)", params![session.key, flag])?;
            }
        }
        Change::HideProject => {
            let folder = session.cwd.as_deref().map(project_key).ok_or_else(|| anyhow::anyhow!("Missing project folder"))?;
            let mut insert = tx.prepare(
                "INSERT OR IGNORE INTO session_flags
                 SELECT ?1, 'hidden' WHERE NOT EXISTS (
                    SELECT 1 FROM session_flags WHERE key = ?1 AND flag = 'favorite'
                 )",
            )?;
            for s in sessions.into_iter().filter(|s| s.cwd.as_deref().is_some_and(|p| project_key(p) == folder)) {
                insert.execute([&s.key])?;
            }
        }
    }
    let result = read(&tx)?;
    tx.commit()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_hide_preserves_favorites_and_allows_restoring_a_session() -> Result<()> {
        let mut db = Connection::open_in_memory()?;
        initialize(&db)?;
        let mut older = Session::new("claude", "1");
        older.cwd = Some("/project".into());
        let mut favorite = Session::new("codex", "1");
        favorite.cwd = Some("/project/".into());
        let mut other = Session::new("codex", "2");
        other.cwd = Some("/other".into());
        let sessions = [&older, &favorite, &other];

        update(&mut db, &favorite, sessions, Change::Favorite)?;
        let organized = update(&mut db, &older, sessions, Change::HideProject)?;
        assert!(organized.includes(&favorite.key, View::Favorites, false));
        assert!(organized.includes(&other.key, View::All, false));
        assert!(organized.includes(&older.key, View::Hidden, true));
        assert!(!organized.includes(&older.key, View::Recent, true));
        assert!(!organized.includes(&other.key, View::Favorites, false));
        assert!(organized.includes(&other.key, View::Favorites, true));

        let restored = update(&mut db, &older, sessions, Change::Hide)?;
        assert!(restored.includes(&older.key, View::All, false));
        assert!(!restored.includes(&older.key, View::Hidden, false));
        Ok(())
    }
}
