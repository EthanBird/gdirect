# gdirect（物理网卡直连 · 绕过全局代理增强版）

[English](README.md) · [更新日志](CHANGELOG.md)

`gdirect` 是一个在终端里浏览、多线程下载、分块上传和挂载 Google 云盘（Google Drive）的工具。它会自动扫描当前网络可直连的 Google IP，让云盘流量直接连过去，支持多连接并发下载与断点续传。

> **致谢与声明：** 本项目基于原项目 [mizorewww/gdirect](https://github.com/mizorewww/gdirect) 改造并独立维护。核心改动为新增了**物理网卡强绑定机制以在硬件网卡层面彻底绕过全局 TUN/TAP 代理（如 Clash / Mihomo TUN 模式）**、将下载引擎重写为**纯 Rust 多连接断点续传引擎**（无需编译 `libaria2` C++ 依赖），以及实现了 **OAuth 登录与刷新令牌全链路直连**。

---

## 相比原版的核心改进

1. **物理网卡强绑定 —— 彻底绕过 Clash / Mihomo 全局代理与 TUN 虚拟网卡（核心特性）**
   - **原版痛点**：当本机开启了 Clash / Mihomo 的 **TUN 模式**（如 `198.18.0.1/16`）或 **TAP 网卡**（如 `cfw-tap 10.0.0.1`）时，系统路由表会被注入 `0.0.0.0/1` 和 `128.0.0.0/1` 的高优先级全局劫持路由。原版仅忽略了 HTTP 代理环境变量（`.no_proxy()`），但底层 Socket 依然绑定在 `0.0.0.0`，发出的流量仍会被 Clash TUN 虚拟网卡强制接管，导致扫描为空或消耗代理流量。
   - **本版方案（`src/net.rs`）**：程序启动时自动检测默认路由是否落入虚拟网卡（`198.18.0.0/15`、`10.0.0.1`、WSL/VMware 虚拟交换机等），自动枚举并探测本机真正具备外网出口能力的**物理网卡局域网 IP**（如以太网 `192.168.1.x`），并将所有出站 TCP Socket **强制绑定（Bind）到该物理网卡 IP**。
   - **效果**：无需关闭 Clash TUN、无需修改任何 Clash 分流或 fake-ip 规则，`gdirect` 的所有扫描、浏览、上传、下载流量均直接从物理网卡经由局域网路由器网关出站，**100% 物理绕过全局代理，跑满宽带直连带宽且零代理流量消耗**。
   - 支持通过环境变量 `GDIRECT_BIND_IP=192.168.x.x` 手动指定绑定的物理网卡 IP（或设置 `GDIRECT_BIND_IP=none` 关闭网卡绑定）。

2. **登录与刷新令牌（`oauth2.googleapis.com`）全链路直连 + 自动退避重试**
   - 原版在 `login` 和刷新 Token 时依赖系统网络/代理访问 `oauth2.googleapis.com`；本版将其同样接入 **Google 直连 IP + 物理网卡强绑定**，并内置了桌面客户端支持、优先唤起本地 Chrome 浏览器，同时针对 API 瞬时配额限制增加了自动退避重试机制。

3. **纯 Rust 多连接分段断点续传下载器（移除 `libaria2` C++ 编译依赖）**
   - 原版依赖 C++ `libaria2` 静态库（Windows 下需配置 MSYS2 / MinGW / OpenSSL 编译，且内部 Socket 不受 Rust 侧网卡绑定控制）。
   - 本版（`src/download.rs`）采用纯 Rust 实现了基于 HTTP `Range` 的多线程并发分块下载器，完整保留 `.aria2` 状态文件断点续传、MD5 校验、大文件病毒扫描确认页解析以及 TUI 实时速度/进度条展示，且所有连接均严格绑定物理网卡。

---

## 工作原理

| 流量类型 | 走向 |
| --- | --- |
| Drive API、下载、上传（`browse`、`ls`、`download`、`upload`） | **扫描到的 Google IP + 物理网卡 Socket 强绑定**（彻底绕过 Clash TUN/TAP 与系统代理） |
| 登录与刷新令牌（`oauth2.googleapis.com`） | **扫描到的 Google IP + 物理网卡 Socket 强绑定** |
| `mount`（rclone） | 系统网络 |

---

## 从源码编译

仅需标准 Rust 工具链（无需 C++ 编译器、无需 MSYS2/MinGW、无需外部 OpenSSL）：

```powershell
git clone https://github.com/EthanBird/gdirect.git
cd gdirect
cargo build --release
```

---

## 日常使用

```powershell
gdirect login                             # 唤起本地 Chrome 浏览器完成 Google 账号授权
gdirect                                   # 打开终端全屏交互界面（等同于 gdirect browse）
gdirect ls root                           # 列出云端硬盘根目录文件
gdirect download 文件ID或分享链接          # 多线程断点续传下载文件
gdirect upload .\video.mp4                # 32MiB 分块断点续传上传大文件
gdirect scan                              # 通过物理网卡重新扫描可用的 Google IP
gdirect scan --list                       # 查看已保存的 Google IP 及延迟
```

| `browse` 交互界面按键 | 操作 |
| --- | --- |
| `j/k`、`↑/↓` | 上下选择文件 |
| Enter、`l/→` | 进入文件夹 |
| `h/←`、Backspace | 返回上级目录 |
| `d` | 加入下载队列（Google 文档/表格/幻灯片自动导出为 PDF） |
| `+/-` | 调整下载并发连接数（1–16），并保存为默认值 |
| `r` | 刷新当前目录 |
| `c` | 取消当前下载并保留已下载分块 |
| `q` | 退出并保存未完成的下载进度 |

### 环境变量配置

| 变量名 | 说明 |
| --- | --- |
| `GDIRECT_BIND_IP` | 手动指定要绑定的本机物理网卡 IPv4 地址（设为 `none` 则使用系统默认路由） |
| `GDIRECT_DB` | 自定义 `state.sqlite3` 状态数据库保存路径（若存在 `E:\` 盘，默认优先使用 `E:\gdirect\state.sqlite3`） |
| `GDIRECT_ACCESS_TOKEN` | 直接提供 OAuth 访问令牌（适合脚本调用） |

---

## 许可证

遵循 GPL-2.0-or-later 许可证（详见 [LICENSE](LICENSE)）。原项目作者及仓库：[mizorewww/gdirect](https://github.com/mizorewww/gdirect)。
