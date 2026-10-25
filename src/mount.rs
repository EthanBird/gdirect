use crate::{api::Api, store::Store};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

#[derive(Serialize, Deserialize)]
struct Mounted {
    path: PathBuf,
    pid: u32,
    config: PathBuf,
}
fn default_mount() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("cannot locate home directory")?
        .join("GoogleDrive"))
}
fn gcloud() -> Command {
    Command::new(if cfg!(windows) {
        "gcloud.cmd"
    } else {
        "gcloud"
    })
}
fn output(args: &[&str]) -> Result<String> {
    let result = gcloud()
        .args(args)
        .output()
        .context("Google Cloud CLI is required")?;
    ensure!(
        result.status.success(),
        "gcloud ADC unavailable; run gcloud auth login --enable-gdrive-access --update-adc"
    );
    Ok(String::from_utf8(result.stdout)?.trim().into())
}
pub fn run(
    api: &Api,
    path: Option<&Path>,
    read_only: bool,
    foreground: bool,
    connections: u8,
) -> Result<()> {
    let version = Command::new("rclone")
        .arg("version")
        .output()
        .context("install rclone first (macOS: brew install rclone)")?;
    ensure!(version.status.success(), "rclone is unavailable");
    let db = Store::open(&api.db)?;
    ensure!(
        db.setting("mount")?.is_none(),
        "a mount is already recorded; run gdirect unmount first"
    );
    let config_dir = output(&["info", "--format=value(config.paths.global_config_dir)"])?;
    let credentials = Path::new(&config_dir).join("application_default_credentials.json");
    ensure!(
        credentials.is_file(),
        "mount needs refreshable ADC; run gcloud auth login --enable-gdrive-access --update-adc"
    );
    // Explicitly select the same ADC file for validation and for the daemon.
    let token = gcloud()
        .args(["auth", "application-default", "print-access-token"])
        .env("GOOGLE_APPLICATION_CREDENTIALS", &credentials)
        .output()?;
    ensure!(
        token.status.success(),
        "ADC refresh failed; run gcloud auth login --enable-gdrive-access --update-adc"
    );
    let token = String::from_utf8(token.stdout)?;
    let response = api
        .client
        .get("https://www.googleapis.com/drive/v3/about")
        .query(&[("fields", "user(emailAddress)")])
        .bearer_auth(token.trim())
        .send()
        .map_err(|e| e.without_url())?;
    ensure!(
        response.status().is_success(),
        "ADC lacks Drive access; run gcloud auth login --enable-gdrive-access --update-adc"
    );
    let user: serde_json::Value = response.json()?;
    let email = user["user"]["emailAddress"]
        .as_str()
        .context("missing ADC account")?;
    let account = db.account()?.context("run gdirect login first")?;
    ensure!(
        email == account,
        "ADC belongs to a different account; select the same gcloud account before mounting"
    );
    let path = path
        .map(Path::to_path_buf)
        .map(Ok)
        .unwrap_or_else(default_mount)?;
    #[cfg(windows)]
    let path = {
        ensure!(
            !path.exists(),
            "Windows mount directory must not already exist"
        );
        let absolute = std::path::absolute(&path)?;
        let parent = absolute
            .parent()
            .context("mount requires a directory path")?;
        fs::create_dir_all(parent)?;
        absolute
    };
    #[cfg(not(windows))]
    let path = {
        fs::create_dir_all(&path)?;
        let path = path.canonicalize()?;
        ensure!(
            fs::read_dir(&path)?.next().is_none(),
            "mount directory must be empty"
        );
        path
    };
    let state = api
        .db
        .canonicalize()?
        .parent()
        .context("missing state directory")?
        .join("mount")
        .join(crate::api::safe_name(email));
    fs::create_dir_all(&state)?;
    let config = state.join("rclone.conf");
    fs::write(&config, "[gdirect]\ntype = drive\nenv_auth = true\n")?;
    let log = state.join("rclone.log");
    #[cfg(unix)]
    let mut command = {
        if foreground {
            Command::new("rclone")
        } else {
            let mut cmd = Command::new("nohup");
            cmd.arg("rclone");
            cmd
        }
    };
    #[cfg(not(unix))]
    let mut command = Command::new("rclone");
    command
        .arg(if cfg!(target_os = "macos") {
            "nfsmount"
        } else {
            "mount"
        })
        .arg("gdirect:")
        .arg(&path)
        .arg("--config")
        .arg(&config)
        .env("GOOGLE_APPLICATION_CREDENTIALS", &credentials)
        .args([
            "--vfs-cache-mode",
            "full",
            "--vfs-cache-max-size",
            "5G",
            "--dir-cache-time",
            "1m",
            "--poll-interval",
            "30s",
            "--file-perms",
            "0600",
            "--dir-perms",
            "0700",
        ])
        .arg("--transfers")
        .arg(connections.to_string())
        .arg("--vfs-read-chunk-streams")
        .arg(connections.to_string())
        .args(["--vfs-read-chunk-size", "4M"])
        .arg("--cache-dir")
        .arg(state.join("cache"))
        .arg("--log-file")
        .arg(&log)
        .args(["--log-level", "INFO"]);
    if read_only {
        command.arg("--read-only");
    }
    println!(
        "Mount: {}\nAccount: {email}\nLog: {}\nMount traffic follows system routing.",
        path.display(),
        log.display()
    );
    if foreground || cfg!(windows) {
        let status = command.status()?;
        ensure!(
            status.success(),
            "rclone mount failed; see {}",
            log.display()
        );
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let log_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(log_file.try_clone()?)
        .stderr(log_file)
        .spawn()?;
    let mounted = Mounted {
        path,
        pid: child.id(),
        config,
    };
    db.set("mount", &serde_json::to_string(&mounted)?)?;
    for _ in 0..60 {
        if let Some(status) = child.try_wait()? {
            db.conn
                .execute("DELETE FROM settings WHERE key='mount'", [])?;
            anyhow::bail!("rclone mount exited ({status}); see {}", log.display());
        }
        if is_mounted(&mounted.path)? {
            println!("Mounted. Use gdirect unmount to detach.");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    child.kill()?;
    child.wait()?;
    // Keep the record if the OS mounted at the timeout boundary.
    if !is_mounted(&mounted.path)? {
        db.conn
            .execute("DELETE FROM settings WHERE key='mount'", [])?;
    }
    anyhow::bail!(
        "mount did not become ready within 30s; see {}",
        log.display()
    )
}
fn is_mounted(path: &Path) -> Result<bool> {
    let result = Command::new("mount").output()?;
    ensure!(
        result.status.success(),
        "cannot inspect mounted filesystems"
    );
    let marker = format!(" on {} ", path.display());
    Ok(String::from_utf8_lossy(&result.stdout)
        .lines()
        .any(|line| line.contains(&marker)))
}
pub fn unmount(db: &Store) -> Result<()> {
    let raw = db
        .setting("mount")?
        .context("no background mount recorded; foreground mounts stop with Ctrl-C")?;
    let mounted: Mounted = serde_json::from_str(&raw)?;
    if is_mounted(&mounted.path)? {
        let status = if cfg!(target_os = "macos") {
            Command::new("diskutil")
                .arg("unmount")
                .arg(&mounted.path)
                .status()?
        } else {
            Command::new("fusermount3")
                .arg("-u")
                .arg(&mounted.path)
                .status()?
        };
        ensure!(
            status.success(),
            "unmount failed; close files using the mounted directory and retry"
        );
    }
    #[cfg(unix)]
    {
        // Avoid signalling a reused PID after a crash or reboot.
        let process = Command::new("ps")
            .args(["-p", &mounted.pid.to_string(), "-o", "command="])
            .output()?;
        let command = String::from_utf8_lossy(&process.stdout);
        if command.contains("rclone") && command.contains(mounted.config.to_string_lossy().as_ref())
        {
            let status = Command::new("kill")
                .args(["-TERM", &mounted.pid.to_string()])
                .status()?;
            ensure!(status.success(), "could not stop mount process");
        }
    }
    db.conn
        .execute("DELETE FROM settings WHERE key='mount'", [])?;
    println!(
        "Unmounted {}. Cached writes are retained for the next mount.",
        mounted.path.display()
    );
    Ok(())
}
