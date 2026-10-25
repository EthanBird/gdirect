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
        conn.execute_batch("CREATE TABLE IF NOT EXISTS endpoints(ip TEXT PRIMARY KEY,latency_ms INTEGER,verified_at INTEGER NOT NULL,available INTEGER NOT NULL,error TEXT);
            CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS auth(id INTEGER PRIMARY KEY CHECK(id=1),account TEXT NOT NULL,token TEXT NOT NULL,expires_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS folders(id TEXT PRIMARY KEY,json TEXT NOT NULL,updated_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS downloads(id INTEGER PRIMARY KEY,file_id TEXT NOT NULL,name TEXT NOT NULL,path TEXT NOT NULL,status TEXT NOT NULL,created_at INTEGER NOT NULL);")?;
        Ok(Self { conn })
    }
    // verified_at is the last successful probe; failures keep it so stale IPs age out.
    pub fn endpoint(&self, ip: &str, latency: Option<i64>, error: Option<&str>) -> Result<()> {
        match latency {
            Some(ms) => self.conn.execute(
                "INSERT OR REPLACE INTO endpoints VALUES(?,?,?,1,NULL)",
                params![ip, ms, now()],
            )?,
            None => self.conn.execute(
                "INSERT INTO endpoints VALUES(?,NULL,0,0,?) ON CONFLICT(ip) DO UPDATE SET latency_ms=NULL,available=0,error=excluded.error",
                params![ip, error],
            )?,
        };
        Ok(())
    }
    // Prunes endpoints without a success in 7 days and returns the best `limit` remaining.
    pub fn rescan_candidates(&self, limit: usize) -> Result<Vec<String>> {
        self.conn.execute(
            "DELETE FROM endpoints WHERE verified_at<=?",
            [now() - 7 * 86400],
        )?;
        let mut query = self.conn.prepare(
            "SELECT ip FROM endpoints ORDER BY available DESC,latency_ms,verified_at DESC LIMIT ?",
        )?;
        Ok(query
            .query_map([limit as i64], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn best_ip(&self) -> Result<Option<std::net::IpAddr>> {
        let ip:Option<String>=self.conn.query_row("SELECT ip FROM endpoints WHERE available=1 AND verified_at>? ORDER BY latency_ms LIMIT 1",[now()-86400],|r|r.get(0)).optional()?;
        ip.map(|s| s.parse().map_err(Into::into)).transpose()
    }
    pub fn endpoints(&self) -> Result<Vec<(String, Option<i64>, bool)>> {
        let mut query = self.conn.prepare(
            "SELECT ip,latency_ms,available FROM endpoints ORDER BY available DESC,latency_ms",
        )?;
        Ok(query
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?)
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
    pub fn clear_auth(&self) -> Result<Option<String>> {
        let account = self.account()?;
        let removed = self.conn.execute("DELETE FROM auth", [])?
            + self.conn.execute(
                "DELETE FROM settings WHERE key IN ('oauth_credentials','oauth_client') OR key LIKE 'upload:%'",
                [],
            )?;
        Ok(account.or_else(|| (removed > 0).then(|| "saved credentials".into())))
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
    fn endpoints_require_recent_success() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE endpoints(ip TEXT PRIMARY KEY,latency_ms INTEGER,verified_at INTEGER,available INTEGER,error TEXT);").unwrap();
        let db = Store { conn };
        db.endpoint("183.56.143.147", Some(40), None).unwrap();
        db.endpoint("8.8.8.8", Some(80), None).unwrap();
        assert_eq!(db.best_ip().unwrap().unwrap().to_string(), "183.56.143.147");
        db.endpoint("183.56.143.147", None, Some("timeout"))
            .unwrap();
        assert_eq!(db.best_ip().unwrap().unwrap().to_string(), "8.8.8.8");
        db.conn
            .execute("UPDATE endpoints SET verified_at=0", [])
            .unwrap();
        assert!(db.best_ip().unwrap().is_none());
    }
    #[test]
    fn stale_endpoints_are_pruned() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE endpoints(ip TEXT PRIMARY KEY,latency_ms INTEGER,verified_at INTEGER NOT NULL,available INTEGER NOT NULL,error TEXT);").unwrap();
        let db = Store { conn };
        db.endpoint("1.0.0.1", Some(30), None).unwrap();
        db.endpoint("1.0.0.2", Some(10), None).unwrap();
        db.endpoint("1.0.0.3", Some(20), None).unwrap();
        db.endpoint("1.0.0.3", None, Some("timeout")).unwrap();
        db.endpoint("1.0.0.4", None, Some("never worked")).unwrap();
        db.conn
            .execute(
                "UPDATE endpoints SET verified_at=? WHERE ip='1.0.0.1'",
                [now() - 8 * 86400],
            )
            .unwrap();
        assert_eq!(db.rescan_candidates(256).unwrap(), ["1.0.0.2", "1.0.0.3"]);
        assert_eq!(db.rescan_candidates(1).unwrap(), ["1.0.0.2"]);
        assert_eq!(db.endpoints().unwrap().len(), 2);
    }
    #[test]
    fn logout_keeps_endpoints() {
        let path =
            std::env::temp_dir().join(format!("gdirect-logout-{}.sqlite3", std::process::id()));
        let db = Store::open(&path).unwrap();
        db.endpoint("8.8.8.8", Some(10), None).unwrap();
        db.save_auth("test@example.com", "secret").unwrap();
        db.set("oauth_credentials", "{}").unwrap();
        db.set("upload:x", "{}").unwrap();
        db.set("connections", "4").unwrap();
        assert_eq!(
            db.clear_auth().unwrap().as_deref(),
            Some("test@example.com")
        );
        assert!(db.account().unwrap().is_none());
        assert!(db.setting("oauth_credentials").unwrap().is_none());
        assert!(db.setting("upload:x").unwrap().is_none());
        assert_eq!(db.setting("connections").unwrap().as_deref(), Some("4"));
        assert!(db.best_ip().unwrap().is_some());
        assert!(db.clear_auth().unwrap().is_none());
        drop(db);
        let _ = std::fs::remove_file(path);
    }
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
