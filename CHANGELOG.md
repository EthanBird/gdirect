# Changelog

## 0.2.0

### Breaking changes

- The top-level `gdirect FILE` download form is removed. Use `gdirect download FILE`. A mistyped subcommand now reports an unknown subcommand instead of starting a download.
- `--auth` / `--gcloud` are removed. Authentication is automatic after `gdirect login` or when `GDIRECT_ACCESS_TOKEN` is set.
- Transfer options are now accepted only by the commands that use them:
  - `download`: `--ip`, `--sni`, `--no-sni`, `--ca-cert`, `-j`, `-o FILE`, `--export MIME`
  - `browse`: `--ip`, `--sni`, `--no-sni`, `--ca-cert`, `-j`, `-o DIR`
  - `ls`, `upload`: `--ip`
  - `mount`: `-j`

  `--db` is still global.
- `--sni off` is replaced by `--no-sni`.
- `-j` on the command line applies to that command only and is no longer saved. The saved default changes only with `+/-` in `browse`.
- Google application-default credentials are no longer picked up automatically. Import them explicitly with `gdirect login --credentials PATH`.

### Added

- `gdirect logout` removes stored credentials and keeps scanned IPs.
- Downloads are checked against Drive's MD5 checksum. A file that is already complete is reported as already downloaded instead of failing.
- `gdirect scan --list` shows the last error for unavailable IPs.
- Help text for every option.

### Fixed

- `login`, `logout`, `mount` and `unmount` no longer run an IP scan first, so sign-in works even when no direct IP is reachable.
- Stored IPs that have not worked for 7 days are pruned. Previously the list grew until every scan failed with "at most 256 candidates".
- An IP that passes the Drive API check is no longer rejected just because the public test file is unavailable.
- Token refresh errors keep Google's reason, for example an expired or revoked refresh credential versus a network error.
- Drive API errors keep their HTTP status, and the re-login hint appears only for permission errors.
- Folder listings keep Drive's natural name order (`file2` before `file10`).
- Public downloads are only checked for Drive's HTML confirmation page when the saved file is actually HTML.
- `unmount`, and a failed mount, delete the credentials file exported for rclone. A foreground mount stopped with Ctrl-C on macOS/Linux still leaves it until the next mount.
- Shortcuts and Google Forms/Sites/Maps are no longer offered for PDF export in `browse`.
- Download failures show one short message for the actual error.
- Uploads use 32 MiB chunks, which is faster on high-latency links.

## 0.1.2

- HTTP downloads no longer depend on OpenSSL's legacy provider module.

## 0.1.1

- Built-in Google sign-in (`gdirect login --client-secret`). The Google Cloud CLI (gcloud) is no longer required.

## 0.1.0

- First release. Authentication relied on the Google Cloud CLI (gcloud).
