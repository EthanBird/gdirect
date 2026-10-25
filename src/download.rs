use crate::drive;
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::CString,
    fs,
    io::Read,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
};
use url::Url;

static SESSION: Mutex<()> = Mutex::new(());
#[derive(Clone)]
pub struct Args {
    pub file: String,
    pub output: PathBuf,
    pub ip: Option<IpAddr>,
    pub sni: Option<String>,
    pub token: Option<String>,
    pub export: Option<String>,
    /// Drive's md5Checksum, when known, to verify the saved file.
    pub md5: Option<String>,
    pub connections: u8,
    pub ca_cert: Option<PathBuf>,
}
#[derive(Clone, Copy, Default, Debug)]
pub struct Stats {
    pub done: u64,
    pub total: u64,
    pub speed: u64,
    pub connections: i32,
}
pub struct Progress {
    pub tx: Sender<Stats>,
    pub cancel: Arc<AtomicBool>,
}
extern "C" fn on_progress(
    done: i64,
    total: i64,
    speed: i32,
    connections: i32,
    data: *mut std::ffi::c_void,
) -> i32 {
    // Only invoked synchronously by libaria2 while Progress lives on this worker's stack.
    let p = unsafe { &*(data as *const Progress) };
    let _ = p.tx.send(Stats {
        done: done.max(0) as u64,
        total: total.max(0) as u64,
        speed: speed.max(0) as u64,
        connections,
    });
    i32::from(p.cancel.load(Ordering::Relaxed))
}
unsafe extern "C" {
    fn gdirect_download_ex(
        url: *const std::ffi::c_char,
        dir: *const std::ffi::c_char,
        name: *const std::ffi::c_char,
        ip: *const std::ffi::c_char,
        sni: *const std::ffi::c_char,
        token: *const std::ffi::c_char,
        ca: *const std::ffi::c_char,
        connections: i32,
        callback: Option<extern "C" fn(i64, i64, i32, i32, *mut std::ffi::c_void) -> i32>,
        data: *mut std::ffi::c_void,
        signals: bool,
    ) -> i32;
}

pub fn allowed_url(url: &Url) -> bool {
    let h = url.host_str().unwrap_or_default();
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && (matches!(
            h,
            "drive.google.com"
                | "drive.usercontent.google.com"
                | "www.googleapis.com"
                | "content.googleapis.com"
                | "drive.googleapis.com"
        ) || h.ends_with(".googleusercontent.com"))
}

fn download(
    args: &Args,
    url: &Url,
    token: Option<&str>,
    progress: Option<&mut Progress>,
) -> Result<()> {
    let output = std::path::absolute(&args.output)?;
    let parent = output.parent().context("output needs a parent directory")?;
    fs::create_dir_all(parent)?;
    let name = output
        .file_name()
        .context("output needs a filename")?
        .to_str()
        .context("output must be UTF-8")?;
    let url = CString::new(url.as_str())?;
    let dir = CString::new(parent.to_str().context("output must be UTF-8")?)?;
    let name = CString::new(name)?;
    let ip = CString::new(args.ip.map(|x| x.to_string()).unwrap_or_default())?;
    let sni = args
        .sni
        .as_ref()
        .map(|s| CString::new(s.as_str()))
        .transpose()?;
    let token = CString::new(token.unwrap_or_default())?;
    let ca_path = args.ca_cert.clone().or_else(bundled_ca);
    let ca = CString::new(
        ca_path
            .as_ref()
            .map(|p| p.to_str().context("CA path must be UTF-8"))
            .transpose()?
            .unwrap_or_default(),
    )?;
    // A single worker owns the native session; strings live until its synchronous call returns.
    let signals = progress.is_none();
    let data = progress.map_or(std::ptr::null_mut(), |p| {
        p as *mut Progress as *mut std::ffi::c_void
    });
    let _lock = SESSION
        .lock()
        .map_err(|_| anyhow::anyhow!("download session lock poisoned"))?;
    let code = unsafe {
        gdirect_download_ex(
            url.as_ptr(),
            dir.as_ptr(),
            name.as_ptr(),
            ip.as_ptr(),
            sni.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            token.as_ptr(),
            ca.as_ptr(),
            args.connections.into(),
            if data.is_null() {
                None
            } else {
                Some(on_progress)
            },
            data,
            signals,
        )
    };
    ensure!(
        code == 0,
        "libaria2 failed (code {code}); 1=general/TLS failure, 2=timeout, 3=not found, 6=network, 22=HTTP error, 24=authorization. Check IP/SNI, file permissions, Drive quota and OAuth scope; partial data is retained."
    );
    Ok(())
}

pub fn run(args: Args, progress: Option<&mut Progress>) -> Result<()> {
    let file = drive::FileRef::parse(&args.file)?;
    if let Some(sni) = &args.sni {
        ensure!(
            sni.is_empty()
                || (sni.len() <= 253
                    && sni.split('.').all(|part| !part.is_empty()
                        && part.len() <= 63
                        && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))),
            "invalid SNI hostname"
        );
    }
    let token = args
        .token
        .clone()
        .map(crate::api::validate_token)
        .transpose()?;
    let mut url = file.url(token.is_some(), args.export.as_deref())?;
    // Never treat an unrelated existing file as a resumable download.
    let control = PathBuf::from(format!("{}.aria2", args.output.display()));
    if args.output.exists() && !control.exists() {
        ensure!(
            matches_md5(&args.output, args.md5.as_deref())?,
            "output already exists; choose another -o path"
        );
        if progress.is_none() {
            println!("Already downloaded {}", args.output.display());
        }
        return Ok(());
    }
    if progress.is_none() {
        eprintln!(
            "libaria2 1.37.0 · IP={} · SNI={} · certificate verification ON",
            args.ip
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "system DNS".into()),
            match args.sni.as_deref() {
                None => "original",
                Some("") => "none",
                Some(sni) => sni,
            }
        );
    }
    let mut progress = progress;
    for attempt in 0..3 {
        download(&args, &url, token.as_deref(), progress.as_deref_mut())?;
        let mut prefix = Vec::new();
        fs::File::open(&args.output)?
            .take(1024 * 1024)
            .read_to_end(&mut prefix)?;
        // Public downloads can return the virus-scan confirmation form with HTTP 200.
        // Only Drive's own HTML pages are inspected; other files stay untouched.
        if token.is_none() && html_page(&prefix) {
            let text = String::from_utf8_lossy(&prefix);
            if text.contains("download-form") {
                let next = drive::confirmation(&text, &url)?;
                ensure!(attempt < 2, "Drive confirmation loop");
                fs::remove_file(&args.output)?;
                url = next;
                continue;
            }
            if text.contains("accounts.google.com/ServiceLogin")
                || text.contains("Google Drive - Quota exceeded")
                || text.contains("Google Drive - Access denied")
            {
                bail!(
                    "Drive returned a login/quota error page; saved for inspection at {}",
                    args.output.display()
                );
            }
        }
        ensure!(
            args.md5.is_none() || matches_md5(&args.output, args.md5.as_deref())?,
            "MD5 does not match Drive's checksum; the file is kept at {} for inspection",
            args.output.display()
        );
        if progress.is_none() {
            println!("Saved {}", args.output.display());
        }
        return Ok(());
    }
    unreachable!()
}

fn html_page(prefix: &[u8]) -> bool {
    let start = prefix.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(prefix);
    let start = &start[start
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(start.len())..];
    let head = String::from_utf8_lossy(&start[..start.len().min(16)]).to_ascii_lowercase();
    head.starts_with("<!doctype html") || head.starts_with("<html")
}
fn file_md5(path: &Path) -> Result<String> {
    use md5::{Digest, Md5};
    let mut file = fs::File::open(path)?;
    let mut hasher = Md5::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn matches_md5(path: &Path, expected: Option<&str>) -> Result<bool> {
    let Some(expected) = expected else {
        return Ok(false);
    };
    Ok(file_md5(path)?.eq_ignore_ascii_case(expected))
}

fn bundled_ca() -> Option<PathBuf> {
    let path = std::env::current_exe().ok()?.parent()?.join("cacert.pem");
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_html_pages_are_sniffed() {
        assert!(html_page(
            b"\n  <!DOCTYPE html><html><form id=\"download-form\">"
        ));
        assert!(html_page(b"\xEF\xBB\xBF<HTML lang=en>"));
        assert!(!html_page(b"PK\x03\x04 download-form"));
        assert!(!html_page(b"notes about <html> and download-form"));
        assert!(!html_page(b""));
    }
    #[test]
    fn md5_matches_drive_checksums() {
        let path = std::env::temp_dir().join(format!("gdirect-md5-{}", std::process::id()));
        fs::write(&path, b"abc").unwrap();
        assert_eq!(file_md5(&path).unwrap(), "900150983cd24fb0d6963f7d28e17f72");
        assert!(matches_md5(&path, Some("900150983CD24FB0D6963F7D28E17F72")).unwrap());
        assert!(!matches_md5(&path, Some("d41d8cd98f00b204e9800998ecf8427e")).unwrap());
        assert!(!matches_md5(&path, None).unwrap());
        fs::remove_file(path).unwrap();
    }
}
