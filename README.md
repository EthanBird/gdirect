# gdirect (Physical NIC Direct Bypass Edition)

[中文说明](README.zh-CN.md) · [Changelog](CHANGELOG.md)

`gdirect` is a terminal tool to browse, download, upload, and mount Google Drive by connecting straight to reachable Google IPs with multi-connection, resumable transfers.

> **Upstream Credit:** Based on [mizorewww/gdirect](https://github.com/mizorewww/gdirect). This standalone repository enhances the original project with **physical network interface binding to bypass global TUN/TAP proxies (such as Clash / Mihomo TUN)**, a **pure-Rust multi-connection resumable download engine** (removing the C++ `libaria2` / MinGW build dependency), and **direct Google IP routing for OAuth token exchange**.

---

## Key Enhancements over Upstream

1. **Physical NIC Binding — Hardware-Level Bypass of Global Proxies (Clash / Mihomo TUN & TAP)**
   - In the original `gdirect`, if a global TUN/TAP proxy (e.g., Clash TUN `198.18.0.0/15` or `cfw-tap` `10.0.0.1` with `0.0.0.0/1` and `128.0.0.0/1` hijack routes) is active, sockets bound to `0.0.0.0` are still captured by the virtual TUN adapter even when HTTP proxy environment variables are ignored.
   - This edition (`src/net.rs`) automatically detects when system routing is hijacked by a virtual TUN/TAP or virtual switch interface, probes the host's physical network interfaces (e.g., Ethernet `192.168.1.x`), verifies real external routing, and **binds all outbound TCP sockets directly to the physical NIC IP**.
   - As a result, `scan`, `ls`, `browse`, `download`, `upload`, and `login` traffic exits directly through your physical router gateway—**100% bypassing Clash/TUN without modifying your Clash rules or turning off TUN mode**.
   - You can also manually override the bound interface IP via `GDIRECT_BIND_IP=192.168.x.x` (or disable binding with `GDIRECT_BIND_IP=none`).

2. **Full-Chain Direct OAuth (`login` & Token Refresh)**
   - Upstream routes `oauth2.googleapis.com` through the system network/proxy. This edition binds `oauth2.googleapis.com` directly to the verified Google IP over the physical NIC as well, and automatically handles shared-client rate limits with backoff retries.

3. **Pure-Rust Multi-Connection Resumable Downloader**
   - Replaces the C++ `libaria2` FFI bridge with a pure-Rust multi-connection segmented HTTP Range downloader (`src/download.rs`) sharing the same `rustls` + physical NIC binding stack.
   - Compiles out of the box with standard `cargo build --release` on Windows MSVC, Linux, and macOS without requiring MSYS2, MinGW, C++ compilers, or external OpenSSL libraries.

---

## How It Works

| Traffic | Route |
| --- | --- |
| Drive API, downloads, uploads (`browse`, `ls`, `download`, `upload`) | Scanned Google IP + **Physical NIC Socket Binding** (bypasses Clash TUN/TAP & system proxy) |
| Sign-in and token refresh (`oauth2.googleapis.com`) | Scanned Google IP + **Physical NIC Socket Binding** |
| `mount` (rclone) | System network |

---

## Build from Source

Requires only a standard Rust toolchain (`cargo`):

```sh
git clone https://github.com/EthanBird/gdirect.git
cd gdirect
cargo build --release
```

---

## Daily Use

```sh
gdirect login                             # sign in via local Chrome browser
gdirect                                   # interactive TUI browser (same as gdirect browse)
gdirect ls root                           # list root folder
gdirect download FILE_ID_OR_URL           # multi-connection resumable download
gdirect upload ./video.mp4                # resumable chunked upload
gdirect scan                              # rescan reachable Google IPs via physical NIC
gdirect scan --list                       # view saved Google IPs and latencies
```

| Key in `browse` | Action |
| --- | --- |
| `j/k`, `↑/↓` | Select |
| Enter, `l/→` | Open folder |
| `h/←`, Backspace | Parent folder |
| `d` | Queue download (Google Docs/Sheets/Slides exported as PDF) |
| `+/-` | Connections for the next download (1–16), saved as default |
| `r` | Refresh |
| `c` | Cancel current download and keep partial data |
| `q` | Quit |

### Environment Variables

| Variable | Description |
| --- | --- |
| `GDIRECT_BIND_IP` | Explicitly specify the local physical NIC IPv4 address to bind (or `none` to use OS default routing) |
| `GDIRECT_DB` | Custom path for the `state.sqlite3` database file |
| `GDIRECT_ACCESS_TOKEN` | Supply an OAuth access token directly for scripting |

---

## License

GPL-2.0-or-later (see [LICENSE](LICENSE)). Original work by [mizorewww/gdirect](https://github.com/mizorewww/gdirect).
