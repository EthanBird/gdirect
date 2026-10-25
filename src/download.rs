use crate::drive;
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::CString,
    fs,
    io::Read,
    net::IpAddr,
    path::PathBuf,
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
        .map(|s| CString::new(if s == "off" { "" } else { s }))
        .transpose()?;
    let token = CString::new(token.unwrap_or_default())?;
    let ca = CString::new(
        args.ca_cert
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
            sni == "off"
                || (!sni.is_empty()
                    && sni.len() <= 253
                    && sni.split('.').all(|part| !part.is_empty()
                        && part.len() <= 63
                        && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))),
            "invalid SNI hostname"
        );
    }
    let token = args.token.clone();
    if let Some(token) = &token {
        ensure!(
            !token.is_empty() && token.bytes().all(|b| b.is_ascii_graphic()),
            "invalid access token"
        );
    }
    let mut url = file.url(token.is_some(), args.export.as_deref())?;
    // Never treat an unrelated existing file as a resumable download.
    let control = PathBuf::from(format!("{}.aria2", args.output.display()));
    ensure!(
        !args.output.exists() || control.exists(),
        "output already exists without a .aria2 control file; choose another -o path"
    );
    if progress.is_none() {
        eprintln!(
            "libaria2 1.37.0 · IP={} · SNI={} · certificate verification ON",
            args.ip
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "system DNS".into()),
            args.sni.as_deref().unwrap_or("original")
        );
    }
    let mut progress = progress;
    for attempt in 0..3 {
        download(&args, &url, token.as_deref(), progress.as_deref_mut())?;
        let mut prefix = Vec::new();
        fs::File::open(&args.output)?
            .take(1024 * 1024)
            .read_to_end(&mut prefix)?;
        let text = String::from_utf8_lossy(&prefix);
        // Public downloads can return the virus-scan confirmation form with HTTP 200.
        // Only follow an actual download form; legitimate HTML files stay untouched.
        if token.is_none() && text.contains("download-form") {
            let next = drive::confirmation(&text, &url)?;
            ensure!(attempt < 2, "Drive confirmation loop");
            fs::remove_file(&args.output)?;
            url = next;
            continue;
        }
        if token.is_none()
            && text.contains("<html")
            && (text.contains("accounts.google.com/ServiceLogin")
                || text.contains("Google Drive - Quota exceeded")
                || text.contains("Google Drive - Access denied"))
        {
            bail!(
                "Drive returned a login/quota error page; saved for inspection at {}",
                args.output.display()
            );
        }
        if progress.is_none() {
            println!("Saved {}", args.output.display());
        }
        return Ok(());
    }
    unreachable!()
}
