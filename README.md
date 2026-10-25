# gdirect

[中文](README.zh-CN.md)

## Installation

Requires Rust, a C++ compiler, make, curl, OpenSSL and [Google Cloud CLI](https://cloud.google.com/sdk/docs/install).

```sh
git clone git@github.com:mizorewww/gdirect.git
cd gdirect

# macOS
brew install openssl@3
# Debian / Ubuntu
# sudo apt-get install build-essential pkg-config libssl-dev ca-certificates curl xz-utils

./scripts/build-libaria2.sh
cargo install --path . --locked
```

## Usage

```sh
gdirect login --reauth       # Authorize Google Drive through gcloud
gdirect browse              # Browse folders and download files
```

If gcloud already has Drive access, use `gdirect login` without `--reauth`.

Working Google IPs are discovered automatically on first use and stored in local SQLite. To rescan or inspect saved results:

```sh
gdirect scan
gdirect scan --list
```

The browser shows folders at the top, a preview on the right, and download progress below. Downloads use 8 connections by default.

| Key | Action |
| --- | --- |
| `j/k`, `↑/↓` | Select |
| Enter, `l/→` | Open folder |
| `h/←`, Backspace | Parent folder |
| `d` | Download; export Google documents as PDF |
| `+/-` | Connections for subsequent downloads (1–16) |
| `r` | Refresh |
| `c` | Cancel current download and retain partial data |
| `q` | Quit |

```sh
gdirect browse -o downloads -j 8
gdirect browse FOLDER_ID
gdirect ls root --json
gdirect download FILE_ID -o downloads/file.zip
gdirect download 'https://drive.google.com/file/d/FILE_ID/view' -o downloads/file.zip
gdirect download DOCUMENT_ID --export application/pdf -o document.pdf
```

Repeat a download command to resume it. Completed files are not overwritten. If connectivity changes, run `gdirect scan` again. System TUN software must route these connections directly.

Login state, verified IPs, cache, history and settings use `~/Library/Application Support/gdirect/state.sqlite3` on macOS or `~/.local/share/gdirect/state.sqlite3` on Linux. Override with `--db PATH`. gcloud manages credential refresh; `GDIRECT_ACCESS_TOKEN` is also supported.
