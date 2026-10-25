# First-time Google Drive setup

[中文](README.zh-CN.md) · [Installation and usage](../../README.md)

**gdirect 0.1.1+ includes the official Google Rust authentication SDK. You do not need Google Cloud CLI or a separate SDK installation.** Installing the program does not authorize access to your Drive: complete the browser login below once.

## 1. Download your Desktop OAuth client JSON

1. Open [Google Cloud Console](https://console.cloud.google.com/) and select or create your own project.
2. Enable the [Google Drive API](https://console.cloud.google.com/apis/library/drive.googleapis.com) for that project.
3. Open **Google Auth Platform**. Configure the app name and contact email. For a personal Google account, choose an **External** audience; while the app is in **Testing**, add your own Google account as a test user.
4. Under **Data Access**, add `https://www.googleapis.com/auth/drive`. This is needed to browse existing files, upload, download, and use a writable mount.
5. Under **Clients**, create an OAuth client with application type **Desktop app**. Download its JSON file. A web client, API key, or service-account JSON is not a replacement.

Each user can use their own project/client. gdirect does not distribute the maintainer's personal Google credentials. For Google's description of this flow, see [OAuth for desktop apps](https://developers.google.com/identity/protocols/oauth2/native-app).

## 2. Sign in

macOS / Linux:

```sh
gdirect login --client-secret ~/Downloads/client_secret.json
gdirect ls root
gdirect browse
```

Windows PowerShell:

```powershell
gdirect login --client-secret "$HOME\Downloads\client_secret.json"
gdirect ls root
gdirect browse
```

Use your downloaded filename. The command opens a browser on the same computer; select the test-user account and grant Drive access. Return to the terminal and wait for `Logged in`. If the browser does not open, copy the printed URL into a browser on that computer. The local callback uses a temporary loopback port.

After login, gdirect saves refreshable credentials in its private local SQLite database and the embedded SDK refreshes access tokens. Routine use does not need the JSON file or gcloud. To sign in again with the saved client:

```sh
gdirect login --reauth
```

## 3. Mount, if needed

Install the optional mount dependencies listed in the [main README](../../README.md), or use the one-command installer with `--with-mount` / `-WithMount`. Then:

```sh
gdirect mount
gdirect unmount
```

The default mount directory is `~/GoogleDrive`; Windows uses foreground mode and stops with Ctrl-C. Mounts use rclone and system network routing. CLI uploads/downloads use scanned direct IPs. gdirect exports private credentials for rclone automatically; no second gcloud/ADC login is needed.

## Existing credentials and troubleshooting

- Existing `authorized_user` ADC can be imported with `gdirect login --credentials PATH`. The standard ADC location is also detected automatically. This reads a file; it does not launch gcloud.
- `access_denied` / a test-user error: add the selected account under the app's Audience settings and try again.
- `redirect_uri_mismatch`: use a **Desktop app** client, not a web client.
- Drive API disabled / insufficient permissions: enable Drive API in the same client project, then repeat browser login and grant the Drive scope.
- With an External app in Testing, Drive refresh credentials expire after **7 days**. Reauthorize, or follow Google’s production/verification requirements for longer-lived authorization. See [Google’s token expiration rules](https://developers.google.com/identity/protocols/oauth2#expiration).
- If a refresh credential expires or is revoked, run `gdirect login --reauth` (or supply the client JSON again). Unmount and remount an existing mount after reauthorizing, so its background process uses the new credentials.
- Headless servers can import an existing `authorized_user` credentials JSON with `--credentials PATH`. Browser login needs a browser on the same computer. Treat credential JSON and SQLite files like passwords; do not commit them or share them with others.
- Check `gdirect --version`: **0.1.0 still uses gcloud**. Upgrade before following this guide.
