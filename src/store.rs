use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
pub fn default_path() -> Result<PathBuf> {
    Ok(dirs::data_local_dir()
        .context("cannot locate local data directory")?
        .join("gdirect/state.sqlite3"))
}
pub struct Store {
    pub conn: Connection,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty())
            && !parent.exists()
        {
            std::fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
            }
        }
        // Create with restrictive permissions before SQLite ever writes credentials.
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS auth(id INTEGER PRIMARY KEY CHECK(id=1),account TEXT NOT NULL,token TEXT NOT NULL,expires_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS folders(id TEXT PRIMARY KEY,json TEXT NOT NULL,updated_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS downloads(id INTEGER PRIMARY KEY,file_id TEXT NOT NULL,name TEXT NOT NULL,path TEXT NOT NULL,status TEXT NOT NULL,created_at INTEGER NOT NULL);")?;
        Ok(Self { conn })
    }
    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key=?", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }
    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO settings VALUES(?,?)",
            params![key, value],
        )?;
        Ok(())
    }
    pub fn token(&self) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT token FROM auth WHERE id=1 AND expires_at>?",
                [now() + 60],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn account(&self) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT account FROM auth WHERE id=1", [], |r| r.get(0))
            .optional()?)
    }
    pub fn save_auth(&self, account: &str, token: &str) -> Result<()> {
        // gcloud issues ~1h access tokens; refresh early. Refresh credentials stay with gcloud.
        self.conn.execute(
            "INSERT OR REPLACE INTO auth VALUES(1,?,?,?)",
            params![account, token, now() + 2400],
        )?;
        Ok(())
    }
    pub fn cache(&self, folder: &str, json: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO folders VALUES(?,?,?)",
            params![folder, json, now()],
        )?;
        Ok(())
    }
    pub fn cached(&self, folder: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT json FROM folders WHERE id=?", [folder], |r| {
                r.get(0)
            })
            .optional()?)
    }
    pub fn start_download(&self, id: &str, name: &str, path: &Path) -> Result<i64> {
        self.conn.execute("INSERT INTO downloads(file_id,name,path,status,created_at) VALUES(?,?,?,'downloading',?)", params![id,name,path.to_string_lossy(),now()])?;
        Ok(self.conn.last_insert_rowid())
    }
    pub fn finish_download(&self, id: i64, status: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE downloads SET status=? WHERE id=?",
            params![status, id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expired_tokens_and_settings() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE auth(id INTEGER PRIMARY KEY, account TEXT, token TEXT, expires_at INTEGER); CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT);").unwrap();
        let db = Store { conn };
        db.save_auth("test@example.com", "secret").unwrap();
        assert_eq!(db.token().unwrap().as_deref(), Some("secret"));
        db.conn.execute("UPDATE auth SET expires_at=0", []).unwrap();
        assert!(db.token().unwrap().is_none());
        db.set("connections", "8").unwrap();
        assert_eq!(db.setting("connections").unwrap().as_deref(), Some("8"));
    }
}
