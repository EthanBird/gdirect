mod drive;
use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use std::{ffi::CString, fs, io::Read, net::IpAddr, path::PathBuf, process::Command};
use url::Url;

/// Download a Google Drive file directly with libaria2 1.37.0.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// File ID or Google Drive sharing URL
    file: String,
    /// Destination filename (partial downloads use a .aria2 sidecar)
    #[arg(short, long)]
    output: PathBuf,
    /// Connect to this IP instead of system DNS, including media redirects
    #[arg(long)]
    ip: Option<IpAddr>,
    /// TLS SNI hostname; use 'off' to omit SNI. Certificate checks use the real host.
    #[arg(long, requires = "ip")]
    sni: Option<String>,
    /// Read a Drive-scoped access token from gcloud (otherwise GDIRECT_ACCESS_TOKEN)
    #[arg(long)]
    gcloud: bool,
    /// Export a Workspace document, e.g. application/pdf (requires authentication)
    #[arg(long)]
    export: Option<String>,
    /// Parallel connections
    #[arg(short = 'j', long, default_value_t = 4, value_parser = clap::value_parser!(u8).range(1..=16))]
    connections: u8,
    /// Custom PEM CA bundle (certificate verification remains enabled)
    #[arg(long)]
    ca_cert: Option<PathBuf>,
}

unsafe extern "C" {
    fn gdirect_download(
        url: *const std::ffi::c_char,
        dir: *const std::ffi::c_char,
        name: *const std::ffi::c_char,
        ip: *const std::ffi::c_char,
        sni: *const std::ffi::c_char,
        token: *const std::ffi::c_char,
        ca: *const std::ffi::c_char,
        connections: i32,
    ) -> i32;
}

fn allowed_url(url: &Url) -> bool {
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

fn download(args: &Args, url: &Url, token: Option<&str>) -> Result<()> {
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
    // The CLI is single-threaded; all strings stay alive throughout the synchronous session.
    let code = unsafe {
        gdirect_download(
            url.as_ptr(),
            dir.as_ptr(),
            name.as_ptr(),
            ip.as_ptr(),
            sni.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            token.as_ptr(),
            ca.as_ptr(),
            args.connections.into(),
        )
    };
    ensure!(
        code == 0,
        "libaria2 failed (code {code}); 1=general/TLS failure, 2=timeout, 3=not found, 6=network, 22=HTTP error, 24=authorization. Check IP/SNI, file permissions, Drive quota and OAuth scope; partial data is retained."
    );
    Ok(())
}

fn run(args: Args) -> Result<()> {
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
    let token = if args.gcloud {
        let out = Command::new("gcloud")
            .args(["auth", "print-access-token"])
            .output()
            .context("run gcloud auth login with Drive scope first")?;
        ensure!(
            out.status.success(),
            "gcloud could not obtain an access token; check gcloud authentication"
        );
        Some(String::from_utf8(out.stdout)?.trim().to_owned())
    } else {
        std::env::var("GDIRECT_ACCESS_TOKEN").ok()
    };
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
    eprintln!(
        "libaria2 1.37.0 · IP={} · SNI={} · certificate verification ON",
        args.ip
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "system DNS".into()),
        args.sni.as_deref().unwrap_or("original")
    );
    for attempt in 0..3 {
        download(&args, &url, token.as_deref())?;
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
        println!("Saved {}", args.output.display());
        return Ok(());
    }
    unreachable!()
}
fn main() {
    if let Err(error) = run(Args::parse()) {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_untrusted_urls() {
        for url in [
            "http://drive.google.com/x",
            "https://drive.google.com.evil.test/x",
            "https://evilgoogleusercontent.com/x",
            "https://user@drive.google.com/x",
            "https://drive.google.com:444/x",
        ] {
            assert!(!allowed_url(&Url::parse(url).unwrap()));
        }
    }
}
