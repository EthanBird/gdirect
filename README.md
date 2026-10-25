# gdirect

[中文](README.zh-CN.md)

## Installation

**Version 0.1.1+ includes Google authentication. No gcloud CLI or separate Google SDK installation is required.** First-time users still need a Desktop OAuth client JSON: follow the [Google configuration guide](docs/configuration/README.md) after installation.

| Use | Runtime dependencies |
| --- | --- |
| Browse, upload, download | gdirect and a Google account; a browser for initial authorization |
| Mount on macOS | rclone 1.75.2+; built-in NFS, no FUSE driver |
| Mount on Linux | rclone 1.75.2+, FUSE3 (`fusermount3`) |
| Mount on Windows | rclone 1.75.2+, WinFsp; foreground mode |

Prebuilt binaries: macOS 15+ (Apple Silicon / Intel), Linux x86_64 with glibc 2.35+, Windows x86_64. Alpine/musl, older glibc and Linux ARM64 do not have a compatible prebuilt package.

Homebrew (macOS / Linux):

```sh
brew tap mizorewww/gdirect https://github.com/mizorewww/gdirect && brew install mizorewww/gdirect/gdirect rclone
```

Linux Homebrew users also need their distro's FUSE3 package for mounting. To upgrade: `brew update && brew upgrade gdirect`.

One-command installers below include optional mount dependencies. Omit `--with-mount` / `-WithMount` for only the CLI. Linux system packages use sudo; rclone is installed from its verified upstream archive when needed.

```sh
# macOS (does not require Homebrew)
curl -fsSL https://raw.githubusercontent.com/mizorewww/gdirect/main/scripts/install.sh | bash -s -- --with-mount

# Debian 12+ / Ubuntu 22.04+
sudo apt-get update && sudo apt-get install -y curl ca-certificates && curl -fsSL https://raw.githubusercontent.com/mizorewww/gdirect/main/scripts/install.sh | bash -s -- --with-mount

# Fedora / dnf distros with glibc >= 2.35
sudo dnf install -y curl ca-certificates && curl -fsSL https://raw.githubusercontent.com/mizorewww/gdirect/main/scripts/install.sh | bash -s -- --with-mount

# Arch / Manjaro
sudo pacman -S --needed curl ca-certificates && curl -fsSL https://raw.githubusercontent.com/mizorewww/gdirect/main/scripts/install.sh | bash -s -- --with-mount

# openSUSE with glibc >= 2.35
sudo zypper --non-interactive install curl ca-certificates && curl -fsSL https://raw.githubusercontent.com/mizorewww/gdirect/main/scripts/install.sh | bash -s -- --with-mount
```

Windows PowerShell (mount dependency installation uses [winget / App Installer](https://learn.microsoft.com/windows/package-manager/winget/)):

```powershell
& ([scriptblock]::Create((Invoke-RestMethod 'https://raw.githubusercontent.com/mizorewww/gdirect/main/scripts/install.ps1'))) -WithMount
```

The script installs to `~/.local/bin` on Unix or `%LOCALAPPDATA%\gdirect\bin` on Windows and prints PATH instructions. Windows updates the user PATH; open a new terminal. WinFsp installation may request administrator approval.

Alternatively, download a [release](https://github.com/mizorewww/gdirect/releases/latest), extract it, and keep `cacert.pem` beside the executable. Source builds require Rust 1.91+, a C++ compiler, make, curl and OpenSSL:

```sh
git clone https://github.com/mizorewww/gdirect.git
cd gdirect
# macOS: brew install openssl@3
# Debian / Ubuntu: sudo apt-get install build-essential pkg-config libssl-dev ca-certificates curl xz-utils
./scripts/build-libaria2.sh
cargo install --path . --locked
```

## Usage

**First login:** download a Desktop OAuth client JSON using the [configuration guide](docs/configuration/README.md), then:

```sh
gdirect login --client-secret ~/Downloads/client_secret.json
gdirect browse
```

Working Google IPs are discovered automatically and stored in local SQLite. `gdirect scan` rescans; `gdirect scan --list` lists results. Downloads default to 8 connections.

| Key | Browser action |
| --- | --- |
| `j/k`, `↑/↓` | Select |
| Enter, `l/→` | Open folder |
| `h/←`, Backspace | Parent folder |
| `d` | Download; export Google documents as PDF |
| `+/-` | Download connections (1–16) |
| `r` | Refresh |
| `c` | Cancel, keeping partial data |
| `q` | Quit |

```sh
gdirect browse -o downloads -j 8
gdirect ls root --json
gdirect download FILE_ID -o downloads/file.zip
gdirect upload ./file.zip
gdirect upload ./file.zip --parent FOLDER_ID --name backup.zip
gdirect mount                    # ~/GoogleDrive; requires mount dependencies
gdirect mount ~/MyDrive --read-only
gdirect unmount                  # Windows foreground mount: Ctrl-C
```

Repeat a download/upload command after an interruption to resume. Uploads use 8 MiB chunks and create new files; they do not overwrite existing names. Mount writes upload through a local cache with a 5 GiB cleanup target (open/pending files may exceed it). Wait for uploads before unmounting; the mount command prints the log path. Mount traffic follows system routing; direct CLI transfers require system TUN software to route selected IPs directly.

Credentials, scanned IPs and settings use local SQLite: macOS `~/Library/Application Support/gdirect/state.sqlite3`, Linux `~/.local/share/gdirect/state.sqlite3`, Windows `%LOCALAPPDATA%\gdirect\state.sqlite3`. Override with `--db PATH`. `GDIRECT_ACCESS_TOKEN` is also supported for CLI transfers; mounting needs the refreshable login above.
