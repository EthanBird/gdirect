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
}
#[derive(Clone)]
pub struct Api {
    pub(crate) client: Client,
    pub db: PathBuf,
}
impl Api {
    pub fn new(db: PathBuf, ip: Option<IpAddr>) -> Result<Self> {
        let mut builder = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(30));
        if let Some(ip) = ip {
            builder = builder.resolve("www.googleapis.com", SocketAddr::new(ip, 443));
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
        let token = validate_token(crate::auth::token(&credentials)?)?;
        let email = self.account_for_token(&token)?;
        if let Some(account) = db.account()? {
            ensure!(
                email == account,
                "credentials belong to a different account; run gdirect login explicitly to switch accounts"
            );
        }
        db.set("oauth_credentials", &serde_json::to_string(&credentials)?)?;
        db.save_auth(&email, &token)?;
        Ok(token)
    }
    fn account_for_token(&self, token: &str) -> Result<String> {
        let response = self
            .client
            .get("https://www.googleapis.com/drive/v3/about")
            .query(&[("fields", "user(emailAddress)")])
            .bearer_auth(token)
            .send()
            .map_err(|e| e.without_url())?;
        let data = checked(response)?;
        Ok(data["user"]["emailAddress"]
            .as_str()
            .context("Drive API did not return an account")?
            .into())
    }
    pub fn login(
        &self,
        reauth: bool,
        client: Option<&Path>,
        import: Option<&Path>,
    ) -> Result<String> {
        let db = Store::open(&self.db)?;
        let credentials = crate::auth::prepare_login(&db, client, import, reauth)?;
        let token = validate_token(crate::auth::token(&credentials)?)?;
        let email = self.account_for_token(&token)?;
        db.set("oauth_credentials", &serde_json::to_string(&credentials)?)?;
        if client.is_some() {
            db.set("oauth_client", &serde_json::to_string(&serde_json::json!({
                "client_id": credentials["client_id"], "client_secret": credentials["client_secret"]
            }))?)?;
        }
        db.save_auth(&email, &token)?;
        Ok(email)
    }
    fn get(&self, path: &str, query: &[(String, String)]) -> Result<serde_json::Value> {
        let token = self.token()?;
        let url = format!("https://www.googleapis.com/drive/v3/{path}");
        let mut response = self
            .client
            .get(&url)
            .query(query)
            .bearer_auth(token)
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
        checked(response)
    }
    pub fn list(&self, folder: &str) -> Result<Vec<File>> {
        ensure!(
            folder
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-'),
            "invalid folder ID"
        );
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
        files.sort_by(|a, b| {
            b.folder()
                .cmp(&a.folder())
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
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
fn validate_token(token: String) -> Result<String> {
    ensure!(
        !token.is_empty() && token.bytes().all(|b| b.is_ascii_graphic()),
        "invalid access token"
    );
    Ok(token)
}
fn checked(response: reqwest::blocking::Response) -> Result<serde_json::Value> {
    let status = response.status();
    let data: serde_json::Value = response.json()?;
    ensure!(
        status.is_success(),
        "Drive API {status}: {}. For insufficient scopes, run gdirect login --client-secret PATH",
        data["error"]["message"]
            .as_str()
            .unwrap_or("request failed")
    );
    Ok(data)
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
    fn names_cannot_escape() {
        assert_eq!(safe_name("../a\n"), ".._a_");
        assert_eq!(safe_name(".."), "download");
    }
}
