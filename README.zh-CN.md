# gdirect

[English](README.md)

## 安装

**0.1.1 起已内置 Google 认证，不需要安装 gcloud CLI 或另外安装 Google SDK。** 首次使用仍需桌面 OAuth 客户端 JSON，安装后请按[独立配置指南](docs/configuration/README.zh-CN.md)授权。

| 用途 | 运行依赖 |
| --- | --- |
| 浏览、上传、下载 | gdirect、Google 账号；首次授权需要浏览器 |
| macOS 挂载 | rclone 1.75.2+；使用系统 NFS，无需 FUSE 驱动 |
| Linux 挂载 | rclone 1.75.2+、FUSE3（`fusermount3`） |
| Windows 挂载 | rclone 1.75.2+、WinFsp；前台运行 |

预编译包支持 macOS 15+（Apple Silicon / Intel）、Linux x86_64（glibc 2.35+）、Windows x86_64。Alpine/musl、旧版 glibc、Linux ARM64 暂无兼容预编译包。

Homebrew（macOS / Linux）：

```sh
brew tap mizorewww/gdirect https://github.com/mizorewww/gdirect && brew install mizorewww/gdirect/gdirect rclone
```

Linux 使用 Homebrew 时，挂载还需用发行版包管理器安装 FUSE3。升级命令：`brew update && brew upgrade gdirect`。

下面的一键安装命令包含挂载依赖；只要 CLI 时去掉 `--with-mount` / `-WithMount`。Linux 系统包通过 sudo 安装；需要时从 rclone 官方下载并校验安装包。

```sh
# macOS（无需预装 Homebrew）
curl -fsSL https://raw.githubusercontent.com/mizorewww/gdirect/main/scripts/install.sh | bash -s -- --with-mount

# Debian 12+ / Ubuntu 22.04+
sudo apt-get update && sudo apt-get install -y curl ca-certificates && curl -fsSL https://raw.githubusercontent.com/mizorewww/gdirect/main/scripts/install.sh | bash -s -- --with-mount

# Fedora / 使用 dnf 且 glibc >= 2.35 的发行版
sudo dnf install -y curl ca-certificates && curl -fsSL https://raw.githubusercontent.com/mizorewww/gdirect/main/scripts/install.sh | bash -s -- --with-mount

# Arch / Manjaro
sudo pacman -S --needed curl ca-certificates && curl -fsSL https://raw.githubusercontent.com/mizorewww/gdirect/main/scripts/install.sh | bash -s -- --with-mount

# openSUSE（glibc >= 2.35）
sudo zypper --non-interactive install curl ca-certificates && curl -fsSL https://raw.githubusercontent.com/mizorewww/gdirect/main/scripts/install.sh | bash -s -- --with-mount
```

Windows PowerShell（挂载依赖通过 [winget / 应用安装程序](https://learn.microsoft.com/windows/package-manager/winget/)安装）：

```powershell
& ([scriptblock]::Create((Invoke-RestMethod 'https://raw.githubusercontent.com/mizorewww/gdirect/main/scripts/install.ps1'))) -WithMount
```

脚本安装到 Unix 的 `~/.local/bin` 或 Windows 的 `%LOCALAPPDATA%\gdirect\bin`，并打印 PATH 设置方法。Windows 会更新用户 PATH，请重新打开终端；WinFsp 安装可能出现管理员授权窗口。

也可以下载 [Release 安装包](https://github.com/mizorewww/gdirect/releases/latest)，解压后将 `cacert.pem` 与可执行文件放在同一目录。源码编译需要 Rust 1.91+、C++ 编译器、make、curl、OpenSSL：

```sh
git clone https://github.com/mizorewww/gdirect.git
cd gdirect
# macOS：brew install openssl@3
# Debian / Ubuntu：sudo apt-get install build-essential pkg-config libssl-dev ca-certificates curl xz-utils
./scripts/build-libaria2.sh
cargo install --path . --locked
```

## 使用

**首次登录：** 先按[配置指南](docs/configuration/README.zh-CN.md)下载桌面 OAuth 客户端 JSON，然后执行：

```sh
gdirect login --client-secret ~/Downloads/client_secret.json
gdirect browse
```

首次使用自动扫描可用 Google IP 并存入本机 SQLite。`gdirect scan` 重新扫描，`gdirect scan --list` 查看结果。下载默认 8 个连接。

| 按键 | 文件浏览器操作 |
| --- | --- |
| `j/k`、`↑/↓` | 选择 |
| Enter、`l/→` | 进入文件夹 |
| `h/←`、Backspace | 返回上级 |
| `d` | 下载；Google 文档导出为 PDF |
| `+/-` | 调整下载连接数（1–16） |
| `r` | 刷新 |
| `c` | 取消并保留断点 |
| `q` | 退出 |

```sh
gdirect browse -o downloads -j 8
gdirect ls root --json
gdirect download FILE_ID -o downloads/file.zip
gdirect upload ./file.zip
gdirect upload ./file.zip --parent FOLDER_ID --name backup.zip
gdirect mount                    # ~/GoogleDrive；需要挂载依赖
gdirect mount ~/MyDrive --read-only
gdirect unmount                  # Windows 前台挂载按 Ctrl-C 停止
```

上传或下载中断后，重复同一命令续传。上传采用 8 MiB 分块并创建新文件，不覆盖云端同名文件。挂载写入通过本地缓存上传，缓存清理目标为 5 GiB（使用中或等待上传的文件可超出）；卸载前等待上传完成，日志路径由挂载命令打印。挂载走系统路由；CLI 直连传输需要系统 TUN 软件将选定 IP 路由为直连。

凭据、IP 和设置保存在本机 SQLite：macOS 为 `~/Library/Application Support/gdirect/state.sqlite3`，Linux 为 `~/.local/share/gdirect/state.sqlite3`，Windows 为 `%LOCALAPPDATA%\gdirect\state.sqlite3`。可用 `--db PATH` 改路径，也支持 `GDIRECT_ACCESS_TOKEN`。
