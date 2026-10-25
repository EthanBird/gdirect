mod api;
mod auth;
mod download;
mod drive;
mod mount;
mod scan;
mod store;
mod tui;
mod upload;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use download::allowed_url;
use std::{net::IpAddr, path::PathBuf};

#[derive(Parser)]
#[command(version, about = "Browse, download and upload Google Drive files")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
    /// Legacy download: file ID or sharing URL
    file: Option<String>,
    /// Output file; in browse mode, download directory
    #[arg(short, long, global = true)]
    output: Option<PathBuf>,
    #[arg(long, global = true)]
    ip: Option<IpAddr>,
    #[arg(long, global = true, requires = "ip")]
    sni: Option<String>,
    /// Connections per download (default: saved setting or 8)
    #[arg(short='j',long,global=true,value_parser=clap::value_parser!(u8).range(1..=16))]
    connections: Option<u8>,
    /// Local SQLite state file
    #[arg(long, global = true)]
    db: Option<PathBuf>,
    #[arg(long, global = true)]
    ca_cert: Option<PathBuf>,
    #[arg(long, global = true)]
    export: Option<String>,
    /// Use authenticated Drive API (also automatic after login)
    #[arg(long = "auth", alias = "gcloud", global = true)]
    authenticated: bool,
}
#[derive(Subcommand, Clone)]
enum Commands {
    /// Probe Google endpoints and save working IPs in SQLite
    Scan {
        #[arg(long)]
        ips: Option<PathBuf>,
        #[arg(long)]
        list: bool,
    },
    /// Sign in with a Desktop OAuth client JSON; no gcloud installation needed
    Login {
        #[arg(long)]
        reauth: bool,
        /// Google Desktop OAuth client JSON (opens browser)
        #[arg(long, conflicts_with = "credentials")]
        client_secret: Option<PathBuf>,
        /// Import existing authorized_user credentials
        #[arg(long, conflicts_with = "reauth")]
        credentials: Option<PathBuf>,
    },
    /// Browse folders; j/k navigate, Enter open, d download, +/- connections
    Browse {
        #[arg(default_value = "root")]
        folder: String,
    },
    /// List a Drive folder
    Ls {
        #[arg(default_value = "root")]
        folder: String,
        #[arg(long)]
        json: bool,
    },
    /// Mount Drive locally using rclone (macOS NFS; Linux/Windows FUSE)
    Mount {
        path: Option<PathBuf>,
        #[arg(long)]
        read_only: bool,
        #[arg(long)]
        foreground: bool,
    },
    /// Unmount this database's background mount
    Unmount,
    /// Upload a local file (repeat the command to resume)
    Upload {
        file: PathBuf,
        #[arg(long, default_value = "root")]
        parent: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// Download a file ID or sharing URL
    Download { file: String },
}
fn run(mut cli: Cli) -> Result<()> {
    let db_path = cli.db.clone().map(Ok).unwrap_or_else(store::default_path)?;
    let db = store::Store::open(&db_path)?;
    let connections = cli
        .connections
        .or_else(|| {
            db.setting("connections")
                .ok()
                .flatten()
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(8)
        .clamp(1, 16);
    if cli.connections.is_some() {
        db.set("connections", &connections.to_string())?;
    }
    if let Some(Commands::Scan { ips, list }) = &cli.command {
        if *list {
            for (ip, latency, available) in db.endpoints()? {
                println!(
                    "{}\t{}\t{}",
                    ip,
                    if available { "OK" } else { "unavailable" },
                    latency.map(|v| format!("{v} ms")).unwrap_or_default()
                );
            }
        } else {
            scan::run(&db, ips.as_deref())?;
        }
        return Ok(());
    }
    if matches!(cli.command, Some(Commands::Unmount)) {
        return mount::unmount(&db);
    }
    if cli.ip.is_none() {
        cli.ip = Some(match db.best_ip()? {
            Some(ip) => ip,
            None => scan::run(&db, None)?,
        });
    }
    let api = api::Api::new(db_path, cli.ip)?;
    match cli.command.clone() {
        Some(Commands::Scan { .. } | Commands::Unmount) => unreachable!(),
        Some(Commands::Mount {
            path,
            read_only,
            foreground,
        }) => {
            mount::run(&api, path.as_deref(), read_only, foreground, connections)?;
        }
        Some(Commands::Login {
            reauth,
            client_secret,
            credentials,
        }) => {
            let email = api.login(reauth, client_secret.as_deref(), credentials.as_deref())?;
            println!("Logged in: {email}\nSQLite: {}", api.db.display());
        }
        Some(Commands::Ls { folder, json }) => {
            let files = api.list(&folder)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&files)?);
            } else {
                for f in files {
                    println!(
                        "{}\t{}\t{}\t{}",
                        if f.folder() { "DIR" } else { "FILE" },
                        f.id,
                        f.size.as_deref().unwrap_or("-"),
                        f.name
                    );
                }
            }
        }
        Some(Commands::Upload { file, parent, name }) => {
            let uploaded = api.upload(&file, &parent, name.as_deref())?;
            println!("{}\t{}", uploaded.id, uploaded.name);
        }
        Some(Commands::Download { file }) => start_download(&cli, &api, &db, file, connections)?,
        Some(Commands::Browse { folder }) => tui::run(
            api,
            tui::Options {
                folder,
                output: cli.output.unwrap_or_else(|| "downloads".into()),
                ip: cli.ip,
                sni: cli.sni,
                connections,
                ca_cert: cli.ca_cert,
            },
        )?,
        None => {
            if let Some(file) = cli.file.clone() {
                start_download(&cli, &api, &db, file, connections)?;
            } else {
                tui::run(
                    api,
                    tui::Options {
                        folder: "root".into(),
                        output: cli.output.unwrap_or_else(|| "downloads".into()),
                        ip: cli.ip,
                        sni: cli.sni,
                        connections,
                        ca_cert: cli.ca_cert,
                    },
                )?;
            }
        }
    }
    Ok(())
}
fn start_download(
    cli: &Cli,
    api: &api::Api,
    db: &store::Store,
    file: String,
    connections: u8,
) -> Result<()> {
    let token = if cli.authenticated
        || db.account()?.is_some()
        || std::env::var_os("GDIRECT_ACCESS_TOKEN").is_some()
    {
        Some(api.download_token()?)
    } else {
        None
    };
    let id = drive::FileRef::parse(&file)?.id;
    let metadata = if token.is_some() {
        Some(api.metadata(&id)?)
    } else {
        None
    };
    if let Some(meta) = &metadata {
        anyhow::ensure!(
            !meta.folder(),
            "select a file inside the folder with gdirect browse"
        );
    }
    let output = cli
        .output
        .clone()
        .or_else(|| {
            metadata
                .as_ref()
                .map(|meta| api::destination(std::path::Path::new("downloads"), meta))
        })
        .context("public download requires -o FILE")?;
    let name = metadata
        .as_ref()
        .map(|meta| meta.name.as_str())
        .unwrap_or(&id);
    let job = db.start_download(&id, name, &output)?;
    let result = download::run(
        download::Args {
            file,
            output,
            ip: cli.ip,
            sni: cli.sni.clone(),
            token,
            export: cli.export.clone(),
            connections,
            ca_cert: cli.ca_cert.clone(),
        },
        None,
    );
    db.finish_download(job, if result.is_ok() { "complete" } else { "failed" })?;
    result
}
fn main() {
    if let Err(e) = run(Cli::parse()) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commands_and_global_options() {
        let cli = Cli::try_parse_from(["gdirect", "browse", "-j", "8"]).unwrap();
        assert_eq!(cli.connections, Some(8));
        assert!(matches!(cli.command, Some(Commands::Browse { .. })));
        assert!(Cli::try_parse_from(["gdirect", "browse", "-j", "0"]).is_err());
        let cli = Cli::try_parse_from(["gdirect", "file_id", "-o", "out.bin"]).unwrap();
        assert_eq!(cli.file.as_deref(), Some("file_id"));
        assert!(Cli::try_parse_from(["gdirect", "scan", "--list"]).is_ok());
    }
}
