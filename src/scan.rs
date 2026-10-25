use crate::store::Store;
use anyhow::{Context, Result, ensure};
use reqwest::blocking::Client;
use std::{
    collections::BTreeSet,
    io::Read,
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    path::Path,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

// A bootstrap candidate from Cealing-Host; always probed, never assumed reachable.
const SEED: &str = "183.56.143.147";
const LIMIT: usize = 256;
fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && !(ip.octets()[0] == 198 && (ip.octets()[1] == 18 || ip.octets()[1] == 19))
        }
        IpAddr::V6(ip) => {
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_unique_local()
                && !ip.is_unicast_link_local()
        }
    }
}
fn candidates(db: &Store, path: Option<&Path>) -> Result<Vec<IpAddr>> {
    let mut ips = BTreeSet::new();
    if let Some(path) = path {
        for line in std::fs::read_to_string(path)?.lines() {
            let value = line.split('#').next().unwrap_or("").trim();
            if !value.is_empty() {
                let ip = value
                    .parse::<IpAddr>()
                    .context("candidate file must contain one IP per line")?;
                ensure!(public(ip), "candidate must be a public IP");
                ips.insert(ip);
            }
        }
    } else {
        ips.insert(SEED.parse()?);
        // DNS supplies candidates only. TLS and both Drive endpoints still have to pass.
        for host in [
            "www.googleapis.com",
            "drive.usercontent.google.com",
            "drive.google.com",
        ] {
            if let Ok(addresses) = (host, 443).to_socket_addrs() {
                ips.extend(addresses.map(|a| a.ip()).filter(|ip| public(*ip)));
            }
        }
        for ip in db.rescan_candidates(LIMIT)? {
            if ips.len() >= LIMIT {
                break;
            }
            if let Ok(ip) = ip.parse()
                && public(ip)
            {
                ips.insert(ip);
            }
        }
    }
    ensure!(!ips.is_empty(), "no candidate IPs");
    Ok(ips.into_iter().collect())
}
// A missing or changed public test file is reported as a warning, not as a dead IP.
fn probe(ip: IpAddr) -> Result<(i64, Option<String>)> {
    let mut builder = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(5))
        .resolve("www.googleapis.com", SocketAddr::new(ip, 443))
        .resolve("drive.usercontent.google.com", SocketAddr::new(ip, 443));
    if let Some(bind_ip) = crate::net::local_bind_ip()
        && bind_ip.is_ipv4() == ip.is_ipv4()
    {
        builder = builder.local_address(bind_ip);
    }
    let client = builder.build()?;
    let start = Instant::now();
    let response = client
        .get("https://www.googleapis.com/drive/v3/about?fields=user")
        .send()
        .map_err(|e| e.without_url())?;
    ensure!(
        response.status() == reqwest::StatusCode::UNAUTHORIZED,
        "unexpected Drive API status {}",
        response.status()
    );
    let data: serde_json::Value = response.json()?;
    ensure!(data["error"]["code"] == 401, "invalid Drive API response");
    let response=client.get("https://drive.usercontent.google.com/download?id=0B9P1L--7Wd2vU3VUVlFnbTgtS2c&export=download&confirm=t").send().map_err(|e|e.without_url())?;
    let status = response.status();
    let warning = if matches!(
        status,
        reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::NOT_FOUND
    ) {
        Some(format!("HTTP {status}"))
    } else {
        ensure!(status.is_success(), "Drive download probe HTTP {status}");
        let mut bytes = Vec::new();
        response.take(32).read_to_end(&mut bytes)?;
        (bytes != b"spam\n").then(|| "unexpected content".into())
    };
    Ok((start.elapsed().as_millis() as i64, warning))
}
pub fn run(db: &Store, path: Option<&Path>) -> Result<IpAddr> {
    let ips = candidates(db, path)?;
    eprintln!(
        "Scanning {} candidates (TLS + Drive API + test download)…",
        ips.len()
    );
    let (tx, rx) = mpsc::channel();
    let candidates = Arc::new(Mutex::new(ips.into_iter()));
    let mut warned = false;
    thread::scope(|scope| {
        for _ in 0..8 {
            let tx = tx.clone();
            let candidates = candidates.clone();
            scope.spawn(move || {
                loop {
                    let next = candidates.lock().unwrap().next();
                    let Some(ip) = next else {
                        break;
                    };
                    let result = probe(ip).map_err(|e| format!("{e:#}"));
                    let _ = tx.send((ip, result));
                }
            });
        }
        drop(tx);
        let mut results = Vec::new();
        for (ip, result) in rx {
            results.push((ip, result));
        }
        results
    })
    .into_iter()
    .try_for_each(|(ip, result)| -> Result<()> {
        match result {
            Ok((ms, problem)) => {
                if let Some(problem) = problem
                    && !warned
                {
                    eprintln!(
                        "warning: Drive test file probe failed ({problem}); IPs were verified with the Drive API probe only"
                    );
                    warned = true;
                }
                db.endpoint(&ip.to_string(), Some(ms), None)?;
                eprintln!("  OK  {ip}  {ms} ms");
            }
            Err(error) => {
                db.endpoint(&ip.to_string(), None, Some(&error))?;
                eprintln!("  --  {ip}  unavailable");
            }
        }
        Ok(())
    })?;
    let ip=db.best_ip()?.context("no usable IP found; verify system direct routing or provide candidates with gdirect scan --ips FILE")?;
    eprintln!("Selected a verified endpoint; saved in SQLite.");
    Ok(ip)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excludes_fake_and_local_addresses() {
        for s in [
            "127.0.0.1",
            "10.0.0.1",
            "198.18.0.1",
            "198.19.0.2",
            "::1",
            "fc00::1",
        ] {
            assert!(!public(s.parse().unwrap()));
        }
        assert!(public(SEED.parse().unwrap()));
    }
}
