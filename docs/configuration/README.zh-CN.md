# 配置 Google

[English](README.md) · [返回 README](../../README.zh-CN.md)

gdirect 使用你在自己的 Google Cloud 项目里创建的 OAuth 客户端登录。只需配置一次，大约 10 分钟，Google 不收费。gdirect 不会分发或使用任何他人的客户端。

## 1. 创建桌面 OAuth 客户端

1. 打开 [Google Cloud 控制台](https://console.cloud.google.com/)，选择或新建一个项目。
2. 为该项目启用 [Google Drive API](https://console.cloud.google.com/apis/library/drive.googleapis.com)。
3. 打开 [Google Auth Platform](https://console.cloud.google.com/auth/overview)，点击 **Get started（开始）**。在 **Branding（品牌塑造）** 中填写应用名称和邮箱，**Audience（目标对象）** 选择 **External（外部）**。
4. 在 **Data Access（数据访问）** 中点击 **Add or remove scopes（添加或移除范围）**，添加 `https://www.googleapis.com/auth/drive`。浏览已有文件、下载、上传和可写挂载都需要这项权限。
5. 在 **Clients（客户端）** 中点击 **Create client（创建客户端）**，应用类型选择 **Desktop app（桌面应用）**，然后下载 JSON。网页客户端、API Key 和服务账号 JSON 都不能用。
6. 推荐个人使用时在 **Audience（目标对象）** 中点击 **Publish app（发布应用）**，把状态从 **Testing（测试）** 改为 **In production（正式版）**。如果应用一直处于测试状态，登录 **7 天** 后就会过期，需要重新执行 `gdirect login --reauth`。自己使用的已发布应用不需要经过 Google 审核，只是登录时会看到下文提到的警告页。

   如果想保持测试状态，请改为在 **Audience → Test users（测试用户）** 中添加自己的 Google 账号。

## 2. 登录

```sh
gdirect login --client-secret ~/Downloads/client_secret.json
```

在 Windows PowerShell 中，路径写成 `"$HOME\Downloads\client_secret.json"`。无论哪个平台，都要换成你实际下载的文件名。

1. 程序会在本机打开浏览器。如果没有自动打开，请把终端里打印的链接复制到**同一台电脑**的浏览器中，因为授权完成后会回调到本机的临时端口。
2. 选择你的 Google 账号。
3. 页面会显示 **"Google hasn't verified this app"（Google 尚未验证此应用）**。这是正常的，因为这是你自己创建的应用。点击 **Advanced（高级）**，再点击 **Go to 你的应用名 (unsafe)（转至……（不安全））**。如果应用处于测试状态，直接点击 **Continue（继续）**。
4. 允许访问 Google 云端硬盘。
5. 回到终端，等待显示 `Logged in`。

登录完成后可以删除这个 JSON 文件。客户端信息已保存在 gdirect 的私有数据库中，以后执行 `--reauth` 时会用到。

```sh
gdirect ls root     # 快速检查
gdirect browse
```

## 常见问题

| 现象 | 处理方法 |
| --- | --- |
| `access_denied` 或提示应用正在测试 | 按第 6 步发布应用，或把当前账号加为测试用户 |
| `redirect_uri_mismatch` | 这个 JSON 不是 **Desktop app** 客户端，按第 5 步重新创建 |
| `Drive API 403 … insufficient permission` | 在客户端所在的**同一个项目**中启用 Drive API，并按第 4 步添加 Drive 权限范围；然后执行 `gdirect login --reauth` 并允许访问云端硬盘 |
| `refresh credential expired or revoked` | 执行 `gdirect login --reauth`。如果每 7 天出现一次，说明应用仍处于测试状态（见第 6 步） |
| 登录超时或报网络错误 | 系统网络需要能访问 `oauth2.googleapis.com` 和 `accounts.google.com`，见[工作原理](../../README.zh-CN.md#工作原理) |
| 登错账号或需要切换账号 | 执行 `gdirect login --reauth`，选择另一个账号 |

- **挂载与新凭据：** 正在运行的挂载会继续使用旧凭据。执行 `--reauth` 后，请先卸载再重新挂载。
- **无界面服务器：** 浏览器登录要求在同一台机器上打开浏览器。可以先在桌面电脑上登录，或者用 `gdirect login --credentials 路径` 导入已有的 `authorized_user` 凭据 JSON。Google Cloud SDK 生成的应用默认凭据（ADC）通常不包含 Drive 权限。
- **退出登录：** `gdirect logout` 会删除已保存的凭据。也可以在 [myaccount.google.com/permissions](https://myaccount.google.com/permissions) 撤销授权。
- **妥善保管：** 客户端 JSON、凭据 JSON 和 SQLite 数据库都要像密码一样对待，不要提交到 Git，也不要分享给别人。

Google 官方文档：[桌面应用 OAuth](https://developers.google.com/identity/protocols/oauth2/native-app)、[刷新令牌的过期规则](https://developers.google.com/identity/protocols/oauth2#expiration)。
