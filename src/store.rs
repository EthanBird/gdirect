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
    if let Ok(p) = std::env::var("GDIRECT_DB")
        && !p.trim().is_empty()
    {
        return Ok(PathBuf::from(p));
    }
    let e_drive = Path::new("E:\\");
    if e_drive.exists() {
        return Ok(e_drive.join("gdirect\\state.sqlite3"));
    }
    Ok(dirs::data_local_dir()
        .context("cannot locate local data directory")?
        .join("gdirect/state.sqlite3"))
}
pub struct Endpoint {
    pub ip: String,
    pub latency_ms: Option<i64>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ActivityRecord {
    pub id: i64,
    pub created_at: i64,
    pub action: String,
    pub status: String,
    pub name: String,
    pub target_id: Option<String>,
    pub size_bytes: Option<i64>,
    pub detail: Option<String>,
}
pub struct Store {
    conn: Connection,
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
            CREATE TABLE IF NOT EXISTS activity_log(id INTEGER PRIMARY KEY AUTOINCREMENT,created_at INTEGER NOT NULL,action TEXT NOT NULL,status TEXT NOT NULL,name TEXT NOT NULL,target_id TEXT,size_bytes INTEGER,detail TEXT);
            DROP TABLE IF EXISTS downloads;")?;
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
    pub fn endpoints(&self) -> Result<Vec<Endpoint>> {
        let mut query = self.conn.prepare(
            "SELECT ip,latency_ms,error FROM endpoints ORDER BY available DESC,latency_ms",
        )?;
        Ok(query
            .query_map([], |r| {
                Ok(Endpoint {
                    ip: r.get(0)?,
                    latency_ms: r.get(1)?,
                    error: r.get(2)?,
                })
            })?
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
    pub fn unset(&self, key: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM settings WHERE key=?", [key])?;
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
    pub fn save_auth(&self, account: &str, token: &str, expires_in: i64) -> Result<()> {
        // Refresh five minutes before Google's stated expiry.
        self.conn.execute(
            "INSERT OR REPLACE INTO auth VALUES(1,?,?,?)",
            params![account, token, now() + (expires_in - 300).max(60)],
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
    pub fn record_activity(
        &self,
        action: &str,
        status: &str,
        name: &str,
        target_id: Option<&str>,
        size_bytes: Option<u64>,
        detail: Option<&str>,
    ) -> Result<()> {
        let ts = now();
        let size_i64 = size_bytes.map(|v| v as i64);
        let _ = self.conn.execute(
            "INSERT INTO activity_log(created_at,action,status,name,target_id,size_bytes,detail) VALUES(?,?,?,?,?,?,?)",
            params![ts, action, status, name, target_id, size_i64, detail],
        );
        let log_path = Path::new("E:\\gdirect\\activity.log");
        if let Some(p) = log_path.parent()
            && p.exists()
            && let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_path)
        {
            use std::io::Write;
            let _ = writeln!(
                f,
                "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                ts,
                action,
                status,
                size_bytes
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "-".into()),
                target_id.unwrap_or("-"),
                name,
                detail.unwrap_or("")
            );
        }
        Ok(())
    }
    pub fn activities(&self, limit: usize) -> Result<Vec<ActivityRecord>> {
        let mut query = self.conn.prepare(
            "SELECT id,created_at,action,status,name,target_id,size_bytes,detail FROM activity_log ORDER BY id DESC LIMIT ?",
        )?;
        Ok(query
            .query_map([limit as i64], |r| {
                Ok(ActivityRecord {
                    id: r.get(0)?,
                    created_at: r.get(1)?,
                    action: r.get(2)?,
                    status: r.get(3)?,
                    name: r.get(4)?,
                    target_id: r.get(5)?,
                    size_bytes: r.get(6)?,
                    detail: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn active_uploads(&self) -> Result<Vec<(String, String)>> {
        let mut query = self
            .conn
            .prepare("SELECT key,value FROM settings WHERE key LIKE 'upload:%'")?;
        Ok(query
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn active_downloads(&self) -> Result<Vec<(String, String)>> {
        let mut query = self
            .conn
            .prepare("SELECT key,value FROM settings WHERE key LIKE 'download:%'")?;
        Ok(query
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
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
        db.save_auth("test@example.com", "secret", 3600).unwrap();
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
        db.save_auth("test@example.com", "secret", 3600).unwrap();
        assert_eq!(db.token().unwrap().as_deref(), Some("secret"));
        db.conn.execute("UPDATE auth SET expires_at=0", []).unwrap();
        assert!(db.token().unwrap().is_none());
        db.set("connections", "8").unwrap();
        assert_eq!(db.setting("connections").unwrap().as_deref(), Some("8"));
    }
}
