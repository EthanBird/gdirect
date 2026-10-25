# 第一次配置 Google Drive

[English](README.md) · [安装与使用](../../README.zh-CN.md)

**gdirect 0.1.1 起已内置 Google 官方 Rust 认证 SDK，不需要安装 gcloud CLI，也不需要单独安装 SDK。** 安装程序并不等于已经授权访问云盘；首次使用请完成下面的浏览器登录。

## 1. 下载自己的桌面 OAuth 客户端 JSON

1. 打开 [Google Cloud 控制台](https://console.cloud.google.com/)，选择或创建自己的项目。
2. 在这个项目中启用 [Google Drive API](https://console.cloud.google.com/apis/library/drive.googleapis.com)。
3. 打开 **Google Auth Platform**，填写应用名称和联系邮箱。个人 Google 账号选择 **External（外部）**；应用处于 **Testing（测试）** 时，把自己的 Google 账号加入测试用户。
4. 在 **Data Access（数据访问）** 中添加 `https://www.googleapis.com/auth/drive`。浏览已有文件、上传下载和可写挂载都需要相应的 Drive 权限。
5. 在 **Clients（客户端）** 中创建 **Desktop app（桌面应用）** 类型的 OAuth 客户端，下载 JSON 文件。网页客户端、API Key、服务账号 JSON 都不能代替它。

每位用户可以使用自己的项目和客户端；软件不会分发维护者的个人 Google 凭据。详见 [Google 桌面应用 OAuth 文档](https://developers.google.com/identity/protocols/oauth2/native-app)。

## 2. 登录

macOS / Linux：

```sh
gdirect login --client-secret ~/Downloads/client_secret.json
gdirect ls root
gdirect browse
```

Windows PowerShell：

```powershell
gdirect login --client-secret "$HOME\Downloads\client_secret.json"
gdirect ls root
gdirect browse
```

把文件名替换成实际下载的 JSON。程序会在当前电脑打开浏览器；选择已加入测试用户的账号并授予 Drive 权限，回到终端等到显示 `Logged in`。如果浏览器没有自动打开，把终端打印的链接复制到这台电脑的浏览器中；登录回调使用临时的本机回环端口。

登录后，可刷新的凭据保存在本机私有 SQLite 中，由内置 SDK 自动刷新访问令牌。日常使用不再需要 JSON 文件或 gcloud。需要重新授权时可使用已保存的客户端：

```sh
gdirect login --reauth
```

## 3. 需要时再挂载

先按[主 README](../../README.zh-CN.md)安装可选挂载依赖，或给一键安装器加上 `--with-mount` / `-WithMount`，然后执行：

```sh
gdirect mount
gdirect unmount
```

默认目录是 `~/GoogleDrive`；Windows 在前台运行，按 Ctrl-C 停止。挂载由 rclone 实现，走系统网络路由；CLI 上传、下载使用扫描到的直连 IP。程序会自动为 rclone 导出私有凭据，不需要再做一次 gcloud/ADC 登录。

## 已有凭据与常见问题

- 已有 `authorized_user` 类型 ADC：执行 `gdirect login --credentials 路径` 导入，也会自动识别标准 ADC 路径。这只是读取凭据文件，不会启动 gcloud。
- `access_denied` / 测试用户错误：在应用的 Audience 设置里加入当前登录账号，再重试。
- `redirect_uri_mismatch`：请下载**桌面应用**客户端 JSON，不要使用网页客户端。
- Drive API 未启用 / 权限不足：确认客户端所属项目已启用 Drive API，再重新登录并授予 Drive 权限。
- 外部应用处于 Testing 时，Drive 刷新凭据会在 **7 天后过期**。届时重新授权，或按 Google 要求调整发布状态并完成所需验证。详见 [Google 的令牌到期规则](https://developers.google.com/identity/protocols/oauth2#expiration)。
- 刷新凭据过期或被撤销：执行 `gdirect login --reauth`，或重新指定客户端 JSON。
- 无浏览器的服务器：先在有浏览器的电脑上授权，再安全传送 gdirect 的 SQLite 数据库，或导入 `authorized_user` 凭据 JSON。这些文件应像密码一样保管，不要提交到 Git 或分享给别人。
- 先确认 `gdirect --version`：**0.1.0 仍依赖 gcloud**，请升级后再按本文配置。
