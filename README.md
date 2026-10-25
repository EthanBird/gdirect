# gdirect

Rust Google Drive 直连下载 CLI，直接链接 **libaria2 1.37.0**。

Rust 负责参数、Drive 分享链接和确认页面；一个小型 C++ C ABI 接口调用 libaria2，完成多连接下载、进度和断点续传。无 aria2c 子进程、RPC 或本地代理服务。

## 构建

需要 Rust stable、C++11 编译器、make、curl、Python 3（测试）、OpenSSL 开发库。

macOS：

```sh
brew install openssl@3
./scripts/build-libaria2.sh
cargo build --release --locked
```

Debian / Ubuntu：

```sh
sudo apt-get install build-essential pkg-config libssl-dev ca-certificates curl xz-utils
./scripts/build-libaria2.sh
cargo build --release --locked
```

构建脚本下载固定的 aria2 1.37.0 发布包，校验 SHA-256，然后应用仓库内补丁，编译精简的静态 libaria2。OpenSSL 动态链接。依赖源码、库和构建结果都位于被 Git 忽略的 `target/`。若修改补丁或路由头文件，重新运行构建脚本。

可选安装：`cargo install --path . --locked`。

## 下载

```sh
# 本机实际验证成功的公开小文件；原始 SNI，严格证书校验
./target/release/gdirect \
  'https://drive.google.com/file/d/0B9P1L--7Wd2vU3VUVlFnbTgtS2c/view' \
  --ip 183.56.143.147 -o downloads/spam.txt

# 自己的文件：支持文件 ID、/file/d/ID/view、/open?id=ID、/uc?id=ID
./target/release/gdirect FILE_ID --ip YOUR_REACHABLE_GOOGLE_IP -o downloads/file.zip -j 8
```

`--ip` 对所有允许的 Drive 下载域名和媒体重定向生效。未提供时使用系统 DNS。IP 可达性取决于所在地、运营商和时间；示例 IP 不代表长期可用，也不作为内置默认值。

默认保留原始 SNI。只有已经确认目标服务器兼容时，才显式调整：

```sh
./target/release/gdirect FILE_ID --ip YOUR_IP --sni YOUR_SNI -o downloads/file.zip
./target/release/gdirect FILE_ID --ip YOUR_IP --sni off -o downloads/file.zip
```

**无论 SNI 如何设置，始终校验原始下载域名的证书及信任链。** Cealing-Host 的 `g.cn` 规则在本机返回 CN 域名证书，无法覆盖 `drive.usercontent.google.com`，因此 `--sni g.cn` 会被拒绝。不能把 Google 首页连通等同于 Drive 下载可用。

必要时通过 `--ca-cert /path/to/ca.pem` 指定可信 CA 包。没有跳过证书校验选项。

## 断点续传

下载中按 Ctrl-C，libaria2 保存 `.aria2` 控制文件。以相同文件、输出路径和网络参数重新运行，即可继续下载。成功后 libaria2 删除控制文件。

已有输出文件但没有对应 `.aria2` 时，CLI 会拒绝覆盖。需要重新下载时请选新路径。不要把其他文件的残留数据或控制文件用于当前任务。

公开文件的病毒扫描确认页会自动解析 `download-form` 并重新请求；登录、配额或不支持的页面不会自动绕过。若重启后确认页已改变并导致续传失败，需要重新解析下载或换新输出路径。

## 私有文件与文档导出

使用已有 gcloud 登录（需 Drive 权限）：

```sh
gcloud auth login --enable-gdrive-access
./target/release/gdirect FILE_ID --gcloud --ip YOUR_IP -o downloads/private.zip

# Workspace 文档必须导出；示例导出 PDF
./target/release/gdirect DOCUMENT_ID --gcloud --export application/pdf \
  --ip YOUR_IP -o downloads/document.pdf
```

也可通过 `GDIRECT_ACCESS_TOKEN` 环境变量提供已经具有 Drive 读取权限的 OAuth access token。令牌不放进命令行参数或文件。`--gcloud` 优先于环境变量；令牌只发往 `www.googleapis.com`，不会随媒体重定向发送。一次任务读取一次令牌，过期后重新执行命令。

认证走 `files.get?alt=media`，导出走 `files.export`。文件权限、Drive API 配额、导出格式/大小限制仍然适用。Google Cloud 的一般登录权限不等于 Drive 权限。公开文件下载无需 gcloud。

当前不支持目录递归、浏览器 Cookie 导入和 Google Vids 长任务下载。私有文件及实际 Workspace 导出尚未做线上账号验证。

## 本机 sing-box / Clash

CLI 禁用 libaria2 的 HTTP/HTTPS 代理，但无法自行绕过系统 TUN。网络环境配置应在 sing-box 中完成。

本次检查的 `../subconvert/config/out/sing.generated.json` 包含 TUN，Clash API 位于 `127.0.0.1:9090`。通过 API 将 `🇨🇳 Google-CN` 选择器设为 `direct` 后，连接记录为：

```text
chains: [direct, 🇨🇳 Google-CN]
rule: rule_set=google-cn => route(🇨🇳 Google-CN)
```

该选择器已保留为 `direct`，原选择为 `🧭 Manual`。未改写生成配置文件。运行时选择是否跨重启保留，取决于 sing-box 自身设置。全局 mode PATCH 在本机没有生效，不能依赖其 HTTP 成功状态来判断路由已改变。

## 测试

```sh
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
python3 tests/native_integration.py
```

原生集成测试使用本地临时 HTTPS 服务、自建测试 CA 和实际 libaria2；覆盖 IP/SNI、原始域名证书校验、分段字节一致性、拒绝外站重定向、跨域令牌移除和中断续传。无需外网或管理员端口。

线上验证：公开测试文件下载结果为 `spam\n`，5 字节，SHA-256：

```text
284e3029cce3ae5ee0b05866100e300046359f53ae4c77fe6b34c05aa7a72cee
```

## 实现与参考

- `src/`：Rust CLI、Drive URL 和确认表单解析。
- `native/bridge.cc`：同步 libaria2 生命周期、下载和进度。
- `vendor/aria2-1.37.0.patch`：IP/SNI 分离、限制 HTTPS Google Drive 目标、OAuth 头跨域过滤。原始 Host 与证书校验主机名不变。
- `scripts/build-libaria2.sh`：版本、源码哈希、编译参数固定。

参考 [GotoX](https://github.com/SeaHOH/GotoX) 的连接 IP / SNI / 原始域名分离思路，以及 [Cealing-Host](https://github.com/SpaceTimee/Cealing-Host) 的候选规则；没有复制它们的代理架构或内置规则表。

接口依据：[libaria2 文档](https://aria2.github.io/manual/en/html/libaria2.html)、[aria2 1.37.0 源码](https://github.com/aria2/aria2/tree/release-1.37.0)、[Drive 下载与导出文档](https://developers.google.com/workspace/drive/api/guides/manage-downloads)。公开测试文件来自 [gdown README](https://github.com/wkentaro/gdown#files)。

GPL-2.0-or-later，见 `LICENSE`。aria2 保留其原始版权及 OpenSSL 链接例外；构建时使用的完整上游源码可由固定 URL 和哈希取得。
