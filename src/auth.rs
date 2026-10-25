use crate::store::Store;
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

pub const CONFIG_GUIDE: &str =
    "https://github.com/mizorewww/gdirect/blob/main/docs/configuration/README.md";
const TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/drive";

fn required<'a>(data: &'a Value, field: &str) -> Result<&'a str> {
    data[field]
        .as_str()
        .filter(|s| !s.is_empty())
        .with_context(|| format!("credentials are missing {field}"))
}
pub fn normalize(data: Value) -> Result<Value> {
    ensure!(
        data["type"] == "authorized_user",
        "expected authorized_user credentials, not a service account"
    );
    // Never accept an imported token_uri or arbitrary executable credential source.
    Ok(
        json!({"type": "authorized_user", "client_id": required(&data,"client_id")?,
        "client_secret": data["client_secret"].as_str().unwrap_or(""),
        "refresh_token": required(&data,"refresh_token")?}),
    )
}
fn adc_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("GOOGLE_APPLICATION_CREDENTIALS") {
        return Some(path.into());
    }
    if let Some(path) = std::env::var_os("CLOUDSDK_CONFIG") {
        return Some(PathBuf::from(path).join("application_default_credentials.json"));
    }
    let base = if cfg!(windows) {
        dirs::config_dir()?.join("gcloud")
    } else {
        dirs::home_dir()?.join(".config/gcloud")
    };
    Some(base.join("application_default_credentials.json"))
}
pub fn credentials(db: &Store) -> Result<Value> {
    if let Some(raw) = db.setting("oauth_credentials")? {
        return normalize(serde_json::from_str(&raw)?);
    }
    if let Some(path) = adc_path().filter(|p| p.is_file()) {
        return normalize(serde_json::from_slice(&fs::read(path)?)?);
    }
    bail!(
        "first login requires a Desktop OAuth client JSON: gdirect login --client-secret PATH\nSetup: {CONFIG_GUIDE}"
    )
}
pub fn token(data: &Value) -> Result<String> {
    sdk_token(normalize(data.clone())?, TOKEN_URI)
}
fn sdk_token(data: Value, endpoint: &str) -> Result<String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        // The SDK starts its credential cache task while building credentials.
        let credentials = google_cloud_auth::credentials::user_account::Builder::new(data)
            .with_token_uri(endpoint).build_access_token_credentials()
            .map_err(|_| anyhow::anyhow!("invalid Google user credentials"))?;
        let token = tokio::time::timeout(Duration::from_secs(45), credentials.access_token()).await
            .context("Google token refresh timed out; check access to oauth2.googleapis.com")?
            .map_err(|_| anyhow::anyhow!("Google credential refresh failed; sign in again with gdirect login --client-secret PATH"))?;
        Ok(token.token)
    })
}
pub fn prepare_login(
    db: &Store,
    client: Option<&Path>,
    import: Option<&Path>,
    reauth: bool,
) -> Result<Value> {
    if let Some(path) = import {
        return normalize(serde_json::from_slice(&fs::read(path)?)?);
    }
    if let Some(path) = client {
        let data: Value =
            serde_json::from_slice(&fs::read(path).context("cannot read OAuth client JSON")?)?;
        ensure!(
            data["installed"].is_object(),
            "download a Desktop app OAuth client JSON (web clients are not supported)"
        );
        return browser_login(&data["installed"]);
    }
    if reauth {
        let client = match db.setting("oauth_client")? {
            Some(raw) => serde_json::from_str(&raw)?,
            None => credentials(db)?,
        };
        return browser_login(&client);
    }
    credentials(db)
}
fn random_string() -> Result<String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes)
        .map_err(|_| anyhow::anyhow!("system random generator unavailable"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
fn callback(target: &str, state: &str) -> Result<Option<String>> {
    ensure!(target.starts_with("/callback?"), "unexpected callback path");
    let url = url::Url::parse(&format!("http://127.0.0.1{target}"))?;
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    ensure!(
        query.get("state").map(String::as_str) == Some(state),
        "OAuth state mismatch"
    );
    if query.contains_key("error") {
        return Ok(None);
    }
    Ok(Some(
        query
            .get("code")
            .filter(|s| !s.is_empty())
            .context("missing authorization code")?
            .clone(),
    ))
}
fn browser_login(client: &Value) -> Result<Value> {
    let client_id = required(client, "client_id")?;
    ensure!(
        client_id.ends_with(".apps.googleusercontent.com"),
        "invalid Google OAuth client ID"
    );
    let secret = client["client_secret"].as_str().unwrap_or("");
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    let redirect = format!(
        "http://127.0.0.1:{}/callback",
        listener.local_addr()?.port()
    );
    let state = random_string()?;
    let verifier = random_string()?;
    let mut url = url::Url::parse("https://accounts.google.com/o/oauth2/v2/auth")?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", client_id),
        ("redirect_uri", redirect.as_str()),
        ("response_type", "code"),
        ("scope", SCOPE),
        ("access_type", "offline"),
        ("prompt", "consent select_account"),
        ("state", state.as_str()),
        ("code_challenge", challenge(&verifier).as_str()),
        ("code_challenge_method", "S256"),
    ]);
    eprintln!("Open this URL in a browser on this computer:\n{url}");
    let mut browser = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(windows) {
        let mut c = Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        Command::new("xdg-open")
    };
    let _ = browser
        .arg(url.as_str())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let deadline = Instant::now() + Duration::from_secs(300);
    let code = loop {
        ensure!(
            Instant::now() < deadline,
            "browser login timed out; run the login command again"
        );
        let (mut stream, _) = match listener.accept() {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        let mut first = String::new();
        if BufReader::new(stream.try_clone()?.take(8192))
            .read_line(&mut first)
            .is_err()
        {
            continue;
        }
        let parts: Vec<_> = first.split_whitespace().collect();
        let result = if parts.first() == Some(&"GET") && parts.len() == 3 {
            callback(parts[1], &state)
        } else {
            Err(anyhow::anyhow!("invalid callback"))
        };
        let (status, body) = if result.is_ok() {
            (
                "200 OK",
                "Authorization received. Return to the terminal to finish login.",
            )
        } else {
            (
                "400 Bad Request",
                "Invalid authorization callback. Return to the login page.",
            )
        };
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        if let Ok(code) = result {
            break code.context("Google authorization was declined")?;
        }
    };
    let http = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let response = http
        .post(TOKEN_URI)
        .form(&[
            ("client_id", client_id),
            ("client_secret", secret),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .map_err(|e| e.without_url())?;
    ensure!(
        response.status().is_success(),
        "Google rejected the authorization code ({}); verify the Desktop OAuth client configuration",
        response.status()
    );
    let token: Value = response.json().map_err(|e| e.without_url())?;
    Ok(
        json!({"type":"authorized_user","client_id":client_id,"client_secret":secret,
        "refresh_token":required(&token,"refresh_token")?}),
    )
}
pub fn export_credentials(data: &Value, path: &Path) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(serde_json::to_string(&normalize(data.clone())?)?.as_bytes())?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sdk_refresh_runs_inside_tokio_and_can_repeat() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let request: Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(request["refresh_token"], "fixture-refresh");
                assert_eq!(request["grant_type"], "refresh_token");
                let body =
                    r#"{"access_token":"fixture-access","expires_in":3600,"token_type":"Bearer"}"#;
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        for _ in 0..2 {
            assert_eq!(sdk_token(json!({"type":"authorized_user","client_id":"fixture","client_secret":"fixture","refresh_token":"fixture-refresh"}),&endpoint).unwrap(),"fixture-access");
        }
        server.join().unwrap();
    }
    #[test]
    fn pkce_rfc7636_vector() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
    #[test]
    fn callback_rejects_wrong_state_and_missing_code() {
        assert!(callback("/callback?state=other&code=secret", "expected").is_err());
        assert!(callback("/callback?state=expected", "expected").is_err());
        assert_eq!(
            callback("/callback?state=expected&code=ok", "expected").unwrap(),
            Some("ok".into())
        );
        assert!(
            callback("/callback?state=expected&error=access_denied", "expected")
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn imported_credentials_cannot_redirect_tokens() {
        let value=normalize(json!({"type":"authorized_user","client_id":"test","client_secret":"test","refresh_token":"test","token_uri":"https://evil.test"})).unwrap();
        assert!(value.get("token_uri").is_none());
    }
}
