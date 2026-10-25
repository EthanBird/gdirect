# Google setup

[中文](README.zh-CN.md) · [Back to README](../../README.md)

gdirect signs in with an OAuth client that you create in your own Google Cloud project. Expect about 10 minutes, once. Google does not charge for this, and gdirect never ships or uses anyone else's client.

## 1. Create a Desktop OAuth client

1. Open the [Google Cloud console](https://console.cloud.google.com/) and select or create a project.
2. Enable the [Google Drive API](https://console.cloud.google.com/apis/library/drive.googleapis.com) for that project.
3. Open [Google Auth Platform](https://console.cloud.google.com/auth/overview) and click **Get started**. Under **Branding**, enter an app name and your email. For **Audience**, choose **External**.
4. Under **Data Access**, click **Add or remove scopes** and add `https://www.googleapis.com/auth/drive`. Browsing existing files, downloading, uploading and writable mounts all need it.
5. Under **Clients**, click **Create client**, choose application type **Desktop app**, and download the JSON. A web client, an API key or a service-account JSON will not work.
6. Under **Audience**, click **Publish app** to move it from **Testing** to **In production**. This is recommended for personal use. While the app is in Testing, Google expires your sign-in after **7 days** and you have to run `gdirect login --reauth` again. A published app does not need Google's verification for your own use. You will just see the warning screen described below.

   If you would rather stay in Testing, add your Google account under **Audience → Test users** instead.

## 2. Sign in

```sh
gdirect login --client-secret ~/Downloads/client_secret.json
```

On Windows PowerShell, use `"$HOME\Downloads\client_secret.json"`, and in both cases your actual file name.

1. A browser opens on this computer. If it does not, copy the printed URL into a browser **on the same computer**, because the sign-in returns to a temporary local port.
2. Choose your Google account.
3. Google shows **"Google hasn't verified this app"**. This is expected, because it is your own app. Click **Advanced**, then **Go to *your app name* (unsafe)**. In Testing mode, click **Continue** instead.
4. Allow access to Google Drive.
5. Return to the terminal and wait for `Logged in`.

You can then delete the downloaded JSON. Its client is saved in gdirect's private database for later `--reauth`.

```sh
gdirect ls root     # quick check
gdirect browse
```

## Troubleshooting

| Problem | Fix |
| --- | --- |
| `access_denied` or "app is being tested" | Publish the app (step 6), or add this account as a test user |
| `redirect_uri_mismatch` | The JSON is not a **Desktop app** client. Create one as in step 5 |
| `Drive API 403 … insufficient permission` | Enable the Drive API in the **same** project as the client and add the Drive scope (step 4). Then run `gdirect login --reauth` and allow Drive access |
| `refresh credential expired or revoked` | Run `gdirect login --reauth`. If this happens every 7 days, the app is still in Testing (step 6) |
| Sign-in times out or shows a network error | Your system network must reach `oauth2.googleapis.com` and `accounts.google.com`; see [How it works](../../README.md#how-it-works) |
| Wrong account, or switching accounts | `gdirect login --reauth`, then choose the other account |

- **Mounts and new credentials:** a running mount keeps its old credentials. After `--reauth`, unmount and mount again.
- **Headless servers:** browser sign-in needs a browser on the same machine. Sign in on a desktop, or import an existing `authorized_user` credentials JSON with `gdirect login --credentials PATH`. Google Cloud SDK application-default credentials usually lack the Drive scope.
- **Logging out:** `gdirect logout` deletes the stored credentials. You can also revoke access at [myaccount.google.com/permissions](https://myaccount.google.com/permissions).
- **Keep files private:** treat the client JSON, credentials JSON and the SQLite database like passwords. Do not commit or share them.

Google's reference: [OAuth for desktop apps](https://developers.google.com/identity/protocols/oauth2/native-app), [refresh token expiration](https://developers.google.com/identity/protocols/oauth2#expiration).
