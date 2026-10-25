# gdirect

[English](README.md)

## 安装

```sh
brew tap mizorewww/gdirect https://github.com/mizorewww/gdirect
brew install mizorewww/gdirect/gdirect
```

从 [Releases](https://github.com/mizorewww/gdirect/releases/latest) 下载对应平台的压缩包，完整解压后把目录加入 `PATH`。请保留 `cacert.pem` 与 `gdirect` / `gdirect.exe` 在同一目录。

支持 macOS 15+（Apple Silicon / Intel）、Linux x86_64（glibc 2.35+）和 Windows x86_64。云盘登录还需安装 [Google Cloud CLI](https://cloud.google.com/sdk/docs/install)。

如需从源码构建，先安装 Rust、C++ 编译器、make、curl 和 OpenSSL，然后执行：

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

## 使用

```sh
gdirect login --reauth       # 通过 gcloud 授权 Google Drive
gdirect browse              # 浏览文件夹、下载文件
```

已有 Drive 权限的 gcloud 登录可直接运行 `gdirect login`，无需 `--reauth`。

首次使用会自动扫描可用 Google IP 并保存到本机 SQLite，无需手动填写 IP。手动重新扫描或查看结果：

```sh
gdirect scan
gdirect scan --list
```

界面上方浏览目录、右侧预览，下方显示下载进度；默认使用 8 个下载连接。

| 按键 | 操作 |
| --- | --- |
| `j/k`、`↑/↓` | 选择 |
| Enter、`l/→` | 打开文件夹 |
| `h/←`、Backspace | 返回上级 |
| `d` | 下载文件；Google 文档导出 PDF |
| `+/-` | 调整后续任务连接数（1–16） |
| `r` | 刷新 |
| `c` | 取消当前下载，保留断点 |
| `q` | 退出 |

```sh
gdirect browse -o downloads -j 8
gdirect browse FOLDER_ID
gdirect ls root --json
gdirect download FILE_ID -o downloads/file.zip
gdirect download 'https://drive.google.com/file/d/FILE_ID/view' -o downloads/file.zip
gdirect download DOCUMENT_ID --export application/pdf -o document.pdf
```

中断后执行相同下载命令即可续传，已有完整文件不会被覆盖。网络发生变化时运行 `gdirect scan` 重新检测。系统若启用了 TUN，需要将这些连接配置为直连。

登录状态、可用 IP、目录缓存、下载记录和设置保存在本机 SQLite：macOS 为 `~/Library/Application Support/gdirect/state.sqlite3`，Linux 为 `~/.local/share/gdirect/state.sqlite3`。Windows 为 `%LOCALAPPDATA%\gdirect\state.sqlite3`。可用 `--db PATH` 修改位置。刷新凭据由 gcloud 管理，也支持 `GDIRECT_ACCESS_TOKEN` 环境变量。


```sh
gdirect upload ./file.zip                        # 上传到云盘根目录
gdirect upload ./file.zip --parent FOLDER_ID --name backup.zip
brew install rclone                             # macOS
gcloud auth login --enable-gdrive-access --update-adc
gdirect mount                                   # ~/GoogleDrive
gdirect mount ~/MyDrive --read-only
gdirect unmount
```

上传采用 8 MiB 分块，显示字节进度；中断后重复同一命令续传。上传会创建新文件，不覆盖云端同名文件。挂载后可直接在 Finder 中读写文件，写入经本地缓存上传，默认缓存清理目标为 5 GiB（正在使用或等待上传的文件可超出）。卸载前等待上传完成，可查看命令输出的日志路径。挂载使用 rclone 和系统网络路由，不使用扫描出的直连 IP。macOS 使用内置 NFS，无需 FUSE 驱动；Linux 需要 FUSE3，Windows 需要 WinFsp 并以前台运行。`--foreground` 可在前台挂载，Ctrl-C 停止。
