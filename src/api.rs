use crate::store::Store;
use anyhow::{Context, Result, ensure};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct File {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size: Option<String>,
    pub modified_time: Option<String>,
    pub md5_checksum: Option<String>,
    pub resource_key: Option<String>,
}
impl File {
    pub fn folder(&self) -> bool {
        self.mime_type == "application/vnd.google-apps.folder"
    }
    pub fn workspace(&self) -> bool {
        self.mime_type.starts_with("application/vnd.google-apps.")
    }
    /// Binary files, or Google documents that Drive can export (as PDF in browse).
    pub fn downloadable(&self) -> bool {
        !self.workspace()
            || matches!(
                self.mime_type
                    .trim_start_matches("application/vnd.google-apps."),
                "document" | "spreadsheet" | "presentation" | "drawing"
            )
    }
}
#[derive(Clone)]
pub struct Api {
    pub(crate) client: Client,
    pub db: PathBuf,
}
impl Api {
    pub fn new(db: PathBuf, ip: Option<IpAddr>) -> Result<Self> {
        let endpoint_ip = ip.or_else(|| {
            Store::open(&db)
                .ok()
                .and_then(|s| s.best_ip().ok().flatten())
        });
        let mut builder = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(30));
        if let Some(ip) = endpoint_ip {
            builder = builder.resolve("www.googleapis.com", SocketAddr::new(ip, 443));
            if let Some(bind_ip) = crate::net::local_bind_ip()
                && bind_ip.is_ipv4() == ip.is_ipv4()
            {
                builder = builder.local_address(bind_ip);
            }
        } else if let Some(bind_ip) = crate::net::local_bind_ip() {
            builder = builder.local_address(bind_ip);
        }
        Ok(Self {
            client: builder.build()?,
            db,
        })
    }
    pub fn token(&self) -> Result<String> {
        if let Ok(token) = std::env::var("GDIRECT_ACCESS_TOKEN") {
            return validate_token(token);
        }
        let db = Store::open(&self.db)?;
        if let Some(token) = db.token()? {
            return Ok(token);
        }
        self.refresh()
    }
    pub fn download_token(&self) -> Result<String> {
        if let Ok(token) = std::env::var("GDIRECT_ACCESS_TOKEN") {
            return validate_token(token);
        }
        self.refresh()
    }
    fn refresh(&self) -> Result<String> {
        let db = Store::open(&self.db)?;
        let credentials = crate::auth::credentials(&db)?;
        let token = crate::auth::refresh(&credentials)?;
        // The account is verified at login; refreshing the same credentials cannot change it.
        let email = match db.account()? {
            Some(account) => account,
            None => self.account_for_token(&token.access_token)?,
        };
        db.save_auth(&email, &token.access_token, token.expires_in)?;
        Ok(token.access_token)
    }
    fn account_for_token(&self, token: &str) -> Result<String> {
        let mut last_err = None;
        for attempt in 0..8 {
            let response = self
                .client
                .get("https://www.googleapis.com/drive/v3/about")
                .query(&[("fields", "user(emailAddress)")])
                .bearer_auth(token)
                .send()
                .map_err(|e| e.without_url())?;
            match checked(response) {
                Ok(data) => {
                    return Ok(data["user"]["emailAddress"]
                        .as_str()
                        .context("Drive API did not return an account")?
                        .into());
                }
                Err(e) if e.to_string().contains("Quota exceeded") && attempt < 7 => {
                    last_err = Some(e);
                    std::thread::sleep(Duration::from_millis(800 * (attempt + 1)));
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err.unwrap())
    }
    pub fn login(
        &self,
        reauth: bool,
        client: Option<&Path>,
        import: Option<&Path>,
    ) -> Result<String> {
        let db = Store::open(&self.db)?;
        let credentials = crate::auth::prepare_login(&db, client, import, reauth)?;
        let token = crate::auth::refresh(&credentials)?;
        let email = self.account_for_token(&token.access_token)?;
        db.set("oauth_credentials", &serde_json::to_string(&credentials)?)?;
        if client.is_some() {
            db.set(
                "oauth_client",
                &serde_json::to_string(&credentials.client())?,
            )?;
        }
        db.save_auth(&email, &token.access_token, token.expires_in)?;
        Ok(email)
    }
    fn get(&self, path: &str, query: &[(String, String)]) -> Result<serde_json::Value> {
        let token = self.token()?;
        let url = format!("https://www.googleapis.com/drive/v3/{path}");
        let mut last_err = None;
        for attempt in 0..8 {
            let mut response = self
                .client
                .get(&url)
                .query(query)
                .bearer_auth(&token)
                .send()
                .map_err(|e| e.without_url())?;
            if response.status() == reqwest::StatusCode::UNAUTHORIZED
                && std::env::var_os("GDIRECT_ACCESS_TOKEN").is_none()
            {
                response = self
                    .client
                    .get(&url)
                    .query(query)
                    .bearer_auth(self.refresh()?)
                    .send()
                    .map_err(|e| e.without_url())?;
            }
            match checked(response) {
                Ok(val) => return Ok(val),
                Err(e) if e.to_string().contains("Quota exceeded") && attempt < 7 => {
                    last_err = Some(e);
                    std::thread::sleep(Duration::from_millis(800 * (attempt + 1)));
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err.unwrap())
    }
    pub fn list(&self, folder: &str) -> Result<Vec<File>> {
        ensure!(crate::drive::valid_id(folder), "invalid folder ID");
        let mut files = Vec::new();
        let mut page = String::new();
        loop {
            let mut query=vec![("q".into(),format!("'{folder}' in parents and trashed = false")),
                ("fields".into(),"nextPageToken,files(id,name,mimeType,size,modifiedTime,md5Checksum,resourceKey)".into()),
                ("pageSize".into(),"1000".into()),("orderBy".into(),"folder,name_natural".into()),
                ("supportsAllDrives".into(),"true".into()),("includeItemsFromAllDrives".into(),"true".into())];
            if !page.is_empty() {
                query.push(("pageToken".into(), page));
            }
            let data = self.get("files", &query)?;
            files.extend(serde_json::from_value::<Vec<File>>(data["files"].clone())?);
            page = data["nextPageToken"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            if page.is_empty() {
                break;
            }
        }
        Store::open(&self.db)?.cache(folder, &serde_json::to_string(&files)?)?;
        Ok(files)
    }
    pub fn metadata(&self, id: &str) -> Result<File> {
        let id = crate::drive::FileRef::parse(id)?.id;
        let data = self.get(
            &format!("files/{id}"),
            &[
                (
                    "fields".into(),
                    "id,name,mimeType,size,modifiedTime,md5Checksum,resourceKey".into(),
                ),
                ("supportsAllDrives".into(), "true".into()),
            ],
        )?;
        Ok(serde_json::from_value(data)?)
    }
    pub fn cached(&self, folder: &str) -> Result<Vec<File>> {
        Store::open(&self.db)?
            .cached(folder)?
            .map(|json| serde_json::from_str(&json).map_err(Into::into))
            .unwrap_or_else(|| Ok(Vec::new()))
    }
}
pub fn validate_token(token: String) -> Result<String> {
    ensure!(
        !token.is_empty() && token.bytes().all(|b| b.is_ascii_graphic()),
        "invalid access token"
    );
    Ok(token)
}
fn checked(response: reqwest::blocking::Response) -> Result<serde_json::Value> {
    let status = response.status();
    let body = response.text().map_err(|e| e.without_url())?;
    if status.is_success() {
        return serde_json::from_str(&body).context("Drive API returned invalid JSON");
    }
    anyhow::bail!("{}", drive_error(status, &body))
}
fn drive_error(status: reqwest::StatusCode, body: &str) -> String {
    let data: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let message = data["error"]["message"]
        .as_str()
        .unwrap_or("request failed");
    // Missing scope or a disabled Drive API; per-file permission errors use other reasons.
    let text = data.to_string();
    if status == reqwest::StatusCode::FORBIDDEN
        && [
            "insufficientPermissions",
            "ACCESS_TOKEN_SCOPE_INSUFFICIENT",
            "accessNotConfigured",
            "SERVICE_DISABLED",
        ]
        .iter()
        .any(|reason| text.contains(reason))
    {
        return format!(
            "Drive API {status}: insufficient permission ({message}). Enable the Drive API in the OAuth client's project, then run gdirect login --reauth and allow Drive access"
        );
    }
    format!("Drive API {status}: {message}")
}
pub fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "download".into()
    } else {
        cleaned
    }
}
pub fn destination(dir: &Path, file: &File) -> PathBuf {
    dir.join(safe_name(&file.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drive_errors_keep_status_and_hint_only_for_scope() {
        use reqwest::StatusCode;
        assert_eq!(
            drive_error(StatusCode::BAD_GATEWAY, "<html>bad gateway</html>"),
            "Drive API 502 Bad Gateway: request failed"
        );
        assert_eq!(
            drive_error(
                StatusCode::NOT_FOUND,
                r#"{"error":{"message":"File not found: x."}}"#
            ),
            "Drive API 404 Not Found: File not found: x."
        );
        let scope = drive_error(
            StatusCode::FORBIDDEN,
            r#"{"error":{"message":"Request had insufficient authentication scopes.","errors":[{"reason":"insufficientPermissions"}]}}"#,
        );
        assert!(scope.starts_with("Drive API 403 Forbidden: insufficient permission"));
        assert!(scope.contains("gdirect login --reauth"));
        let file = drive_error(
            StatusCode::FORBIDDEN,
            r#"{"error":{"message":"No access.","errors":[{"reason":"insufficientFilePermissions"}]}}"#,
        );
        assert_eq!(file, "Drive API 403 Forbidden: No access.");
    }
    #[test]
    fn only_binary_and_exportable_items_download() {
        let file = |mime: &str| File {
            id: "x".into(),
            name: "x".into(),
            mime_type: mime.into(),
            size: None,
            modified_time: None,
            md5_checksum: None,
            resource_key: None,
        };
        assert!(file("application/zip").downloadable());
        assert!(file("application/vnd.google-apps.document").downloadable());
        assert!(!file("application/vnd.google-apps.shortcut").downloadable());
        assert!(!file("application/vnd.google-apps.form").downloadable());
    }
    #[test]
    fn names_cannot_escape() {
        assert_eq!(safe_name("../a\n"), ".._a_");
        assert_eq!(safe_name(".."), "download");
    }
}
