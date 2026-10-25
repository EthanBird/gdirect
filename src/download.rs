use crate::drive;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering},
        mpsc::Sender,
    },
    thread,
    time::{Duration, Instant},
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

#[derive(Serialize, Deserialize, Clone, Debug)]
struct SegmentState {
    start: u64,
    end: u64, // inclusive
    pos: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ControlFile {
    total: u64,
    segments: Vec<SegmentState>,
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

fn build_client(args: &Args, url: &Url) -> Result<reqwest::blocking::Client> {
    let mut builder = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(60));
    if let Some(ip) = args.ip {
        let addr = SocketAddr::new(ip, 443);
        for host in [
            "drive.google.com",
            "drive.usercontent.google.com",
            "www.googleapis.com",
            "content.googleapis.com",
            "drive.googleapis.com",
        ] {
            builder = builder.resolve(host, addr);
        }
        if let Some(h) = url.host_str() {
            builder = builder.resolve(h, addr);
        }
        if let Some(bind_ip) = crate::net::local_bind_ip()
            && bind_ip.is_ipv4() == ip.is_ipv4()
        {
            builder = builder.local_address(bind_ip);
        }
    } else if let Some(bind_ip) = crate::net::local_bind_ip() {
        builder = builder.local_address(bind_ip);
    }
    if let Some(ca_path) = args.ca_cert.clone().or_else(bundled_ca)
        && let Ok(bytes) = fs::read(&ca_path)
        && let Ok(certs) = reqwest::Certificate::from_pem_bundle(&bytes)
    {
        for cert in certs {
            builder = builder.add_root_certificate(cert);
        }
    }
    Ok(builder.build()?)
}

fn resolve_redirects(args: &Args, initial_url: &Url, token: Option<&str>) -> Result<Url> {
    let mut current = initial_url.clone();
    for _ in 0..8 {
        let client = build_client(args, &current)?;
        let mut req = client.get(current.clone()).header("Range", "bytes=0-0");
        if let Some(token) = token
            && current.host_str() == Some("www.googleapis.com")
        {
            req = req.bearer_auth(token);
        }
        let resp = req.send().map_err(|e| e.without_url())?;
        let status = resp.status();
        if status.is_redirection() {
            let loc = resp
                .headers()
                .get("location")
                .context("redirect missing location header")?
                .to_str()
                .context("invalid redirect location")?;
            let next = current.join(loc)?;
            ensure!(allowed_url(&next), "untrusted redirect destination");
            current = next;
            continue;
        }
        return Ok(current);
    }
    bail!("too many redirects while resolving download URL")
}

fn parse_total_size(resp: &reqwest::blocking::Response) -> Option<u64> {
    if let Some(cr) = resp.headers().get("content-range")
        && let Ok(s) = cr.to_str()
        && let Some((_, total_str)) = s.rsplit_once('/')
        && let Ok(total) = total_str.trim().parse::<u64>()
    {
        return Some(total);
    }
    resp.content_length()
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
    let control_path = PathBuf::from(format!("{}.aria2", output.display()));

    let _lock = SESSION
        .lock()
        .map_err(|_| anyhow::anyhow!("download session lock poisoned"))?;

    let final_url = resolve_redirects(args, url, token)?;
    let client = build_client(args, &final_url)?;

    let mut probe_req = client.get(final_url.clone()).header("Range", "bytes=0-0");
    if let Some(t) = token
        && final_url.host_str() == Some("www.googleapis.com")
    {
        probe_req = probe_req.bearer_auth(t);
    }
    let probe_resp = probe_req.send().map_err(|e| e.without_url())?;
    let status = probe_resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        bail!("download failed: Drive rejected the authorization; run gdirect login --reauth");
    }
    if status == reqwest::StatusCode::NOT_FOUND {
        bail!("download failed: file not found or not shared with this account");
    }
    if !status.is_success() {
        bail!("download failed: Drive returned HTTP {status}");
    }

    let supports_range = status == reqwest::StatusCode::PARTIAL_CONTENT;
    let total_size = parse_total_size(&probe_resp).unwrap_or(0);

    // If range requests are not supported or file is small (< 1 MiB) or HTML confirmation page, stream in one connection
    if !supports_range || total_size <= 1024 * 1024 || args.connections <= 1 {
        drop(probe_resp);
        let mut req = client.get(final_url.clone());
        if let Some(t) = token
            && final_url.host_str() == Some("www.googleapis.com")
        {
            req = req.bearer_auth(t);
        }
        let mut resp = req.send().map_err(|e| e.without_url())?;
        ensure!(
            resp.status().is_success(),
            "download failed: Drive returned HTTP {}",
            resp.status()
        );
        let total = resp.content_length().unwrap_or(total_size);
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&output)?;
        let mut done = 0u64;
        let mut buf = vec![0u8; 64 * 1024];
        let start_inst = Instant::now();
        let mut last_report = Instant::now();
        loop {
            if let Some(p) = progress.as_deref()
                && p.cancel.load(Ordering::Relaxed)
            {
                bail!("download cancelled");
            }
            let n = resp.read(&mut buf)?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            done += n as u64;
            if last_report.elapsed() >= Duration::from_millis(100) {
                let elapsed = start_inst.elapsed().as_secs_f64().max(0.001);
                let speed = (done as f64 / elapsed) as u64;
                if let Some(p) = progress.as_deref() {
                    let _ = p.tx.send(Stats {
                        done,
                        total,
                        speed,
                        connections: 1,
                    });
                } else {
                    eprint!("\rDownloading: {} KiB/s", speed / 1024);
                }
                last_report = Instant::now();
            }
        }
        file.sync_all()?;
        if progress.is_none() {
            eprintln!();
        }
        if control_path.exists() {
            let _ = fs::remove_file(&control_path);
        }
        return Ok(());
    }

    drop(probe_resp);

    // Multi-connection ranged download with resume support via .aria2 JSON state file
    let mut state = if output.exists()
        && control_path.exists()
        && let Ok(raw) = fs::read_to_string(&control_path)
        && let Ok(saved) = serde_json::from_str::<ControlFile>(&raw)
        && saved.total == total_size
    {
        saved
    } else {
        let f = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&output)?;
        f.set_len(total_size)?;
        let num_conns = (args.connections.max(1) as u64).min(total_size.div_ceil(1024 * 1024));
        let chunk = total_size / num_conns;
        let mut segments = Vec::with_capacity(num_conns as usize);
        for i in 0..num_conns {
            let start = i * chunk;
            let end = if i + 1 == num_conns {
                total_size - 1
            } else {
                (i + 1) * chunk - 1
            };
            segments.push(SegmentState {
                start,
                end,
                pos: start,
            });
        }
        let s = ControlFile {
            total: total_size,
            segments,
        };
        fs::write(&control_path, serde_json::to_string(&s)?)?;
        s
    };

    let initial_done: u64 = state
        .segments
        .iter()
        .map(|s| s.pos.saturating_sub(s.start))
        .sum();
    let total_done = Arc::new(AtomicU64::new(initial_done));
    let active_conns = Arc::new(AtomicI32::new(0));
    let cancel_flag = progress
        .as_deref()
        .map(|p| p.cancel.clone())
        .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
    let state_arc = Arc::new(Mutex::new(state.clone()));
    let error_slot: Arc<Mutex<Option<anyhow::Error>>> = Arc::new(Mutex::new(None));

    thread::scope(|scope| {
        for idx in 0..state.segments.len() {
            let client = client.clone();
            let final_url = final_url.clone();
            let output = output.clone();
            let token = token.map(str::to_owned);
            let total_done = total_done.clone();
            let active_conns = active_conns.clone();
            let cancel_flag = cancel_flag.clone();
            let state_arc = state_arc.clone();
            let error_slot = error_slot.clone();

            scope.spawn(move || {
                let (mut pos, end) = {
                    let guard = state_arc.lock().unwrap();
                    let seg = &guard.segments[idx];
                    (seg.pos, seg.end)
                };
                if pos > end {
                    return;
                }
                active_conns.fetch_add(1, Ordering::Relaxed);
                let res = (|| -> Result<()> {
                    let mut file = fs::OpenOptions::new().write(true).open(&output)?;
                    file.seek(SeekFrom::Start(pos))?;
                    for attempt in 0..3 {
                        if cancel_flag.load(Ordering::Relaxed) {
                            return Ok(());
                        }
                        let mut req = client
                            .get(final_url.clone())
                            .header("Range", format!("bytes={pos}-{end}"));
                        if let Some(t) = &token
                            && final_url.host_str() == Some("www.googleapis.com")
                        {
                            req = req.bearer_auth(t);
                        }
                        let mut resp = match req.send() {
                            Ok(r) if r.status() == reqwest::StatusCode::PARTIAL_CONTENT => r,
                            Ok(r) => {
                                if attempt == 2 {
                                    bail!("segment HTTP {}", r.status());
                                }
                                thread::sleep(Duration::from_secs(1));
                                continue;
                            }
                            Err(e) => {
                                if attempt == 2 {
                                    return Err(e.without_url().into());
                                }
                                thread::sleep(Duration::from_secs(1));
                                continue;
                            }
                        };
                        let mut buf = vec![0u8; 64 * 1024];
                        loop {
                            if cancel_flag.load(Ordering::Relaxed) {
                                return Ok(());
                            }
                            let max_read = ((end - pos + 1) as usize).min(buf.len());
                            if max_read == 0 {
                                break;
                            }
                            match resp.read(&mut buf[..max_read]) {
                                Ok(0) => break,
                                Ok(n) => {
                                    file.write_all(&buf[..n])?;
                                    pos += n as u64;
                                    total_done.fetch_add(n as u64, Ordering::Relaxed);
                                    state_arc.lock().unwrap().segments[idx].pos = pos;
                                }
                                Err(e) => {
                                    if attempt == 2 {
                                        return Err(e.into());
                                    }
                                    break;
                                }
                            }
                        }
                        if pos > end {
                            file.flush()?;
                            return Ok(());
                        }
                    }
                    bail!("segment incomplete after retries")
                })();
                active_conns.fetch_sub(1, Ordering::Relaxed);
                if let Err(e) = res {
                    cancel_flag.store(true, Ordering::Relaxed);
                    let mut slot = error_slot.lock().unwrap();
                    if slot.is_none() {
                        *slot = Some(e);
                    }
                }
            });
        }

        let start_inst = Instant::now();
        let mut last_save = Instant::now();
        while active_conns.load(Ordering::Relaxed) > 0 {
            thread::sleep(Duration::from_millis(100));
            let done = total_done.load(Ordering::Relaxed);
            let newly_done = done.saturating_sub(initial_done);
            let elapsed = start_inst.elapsed().as_secs_f64().max(0.001);
            let speed = (newly_done as f64 / elapsed) as u64;
            let conns = active_conns.load(Ordering::Relaxed);
            if let Some(p) = progress.as_deref() {
                let _ = p.tx.send(Stats {
                    done,
                    total: total_size,
                    speed,
                    connections: conns,
                });
            } else {
                eprint!("\rDownloading: {} KiB/s", speed / 1024);
            }
            if last_save.elapsed() >= Duration::from_secs(2) {
                let snap = state_arc.lock().unwrap().clone();
                let _ = fs::write(
                    &control_path,
                    serde_json::to_string(&snap).unwrap_or_default(),
                );
                last_save = Instant::now();
            }
        }
    });

    if progress.is_none() {
        eprintln!();
    }

    state = state_arc.lock().unwrap().clone();
    let _ = fs::write(&control_path, serde_json::to_string(&state)?);

    if let Some(err) = error_slot.lock().unwrap().take() {
        bail!("download failed: {err}; partial data is kept, rerun to resume");
    }
    if cancel_flag.load(Ordering::Relaxed) {
        bail!("download cancelled; partial data is kept, rerun to resume");
    }

    let all_done = state.segments.iter().all(|s| s.pos > s.end);
    ensure!(
        all_done,
        "download incomplete; partial data is kept, rerun to resume"
    );
    let _ = fs::remove_file(&control_path);
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
            "gdirect-rust · IP={} · BIND={} · SNI={} · certificate verification ON",
            args.ip
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "system DNS".into()),
            crate::net::local_bind_ip()
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "default".into()),
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
        if let Ok(db_path) = crate::store::default_path()
            && let Ok(db) = crate::store::Store::open(&db_path)
        {
            let size = fs::metadata(&args.output).ok().map(|m| m.len());
            let _ = db.record_activity(
                "DOWNLOAD",
                "COMPLETED",
                &args.output.display().to_string(),
                Some(&file.id),
                size,
                None,
            );
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
