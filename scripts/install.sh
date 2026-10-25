#!/usr/bin/env bash
# Install a verified release; --with-mount also installs rclone and Linux FUSE3.
set -euo pipefail
version=""
prefix="${GDIRECT_PREFIX:-$HOME/.local}"
from_dir=""
with_mount=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --version) version="$2"; shift 2 ;;
    --prefix) prefix="$2"; shift 2 ;;
    --from-dir) from_dir="$2"; shift 2 ;;
    --with-mount) with_mount=1; shift ;;
    --help) echo 'install.sh [--version X.Y.Z] [--prefix DIR] [--with-mount] [--from-dir ARTIFACTS]'; exit 0 ;;
    *) echo "Unknown option: $1" >&2; exit 1 ;;
  esac
done
os=$(uname -s)
arch=$(uname -m)
case "$os/$arch" in
  Darwin/arm64) target=aarch64-apple-darwin; rclone_platform=osx-arm64 ;;
  Darwin/x86_64) target=x86_64-apple-darwin; rclone_platform=osx-amd64 ;;
  Linux/x86_64) target=x86_64-unknown-linux-gnu; rclone_platform=linux-amd64 ;;
  *) echo "No prebuilt release for $os/$arch. See the source-build instructions." >&2; exit 1 ;;
esac
if [ "$os" = Darwin ] && [ "$(sw_vers -productVersion | cut -d. -f1)" -lt 15 ]; then
  echo 'macOS 15 or newer is required.' >&2; exit 1
fi
if [ "$os" = Linux ]; then
  libc=$(getconf GNU_LIBC_VERSION 2>/dev/null || true)
  if ! awk -v v="${libc#glibc }" 'BEGIN {split(v,a,".");exit !(a[1]>2 || (a[1]==2 && a[2]>=35))}'; then
    echo 'This binary needs glibc >= 2.35 (e.g. Ubuntu 22.04+, Debian 12+). Alpine/musl and older glibc need a source build.' >&2; exit 1
  fi
fi
fetch() { curl --fail --location --retry 3 --connect-timeout 20 --silent --show-error "$1" -o "$2"; }
run_root() { if [ "$(id -u)" -eq 0 ]; then "$@"; else sudo "$@"; fi; }
if [ "$with_mount" -eq 1 ] && [ "$os" = Linux ]; then
  packages=()
  command -v fusermount3 >/dev/null || packages+=(fuse3)
  command -v unzip >/dev/null || packages+=(unzip)
  if [ "${#packages[@]}" -gt 0 ]; then
    if command -v apt-get >/dev/null; then run_root apt-get update; run_root apt-get install -y "${packages[@]}" ca-certificates
    elif command -v dnf >/dev/null; then run_root dnf install -y "${packages[@]}" ca-certificates
    elif command -v pacman >/dev/null; then run_root pacman -S --needed --noconfirm "${packages[@]}" ca-certificates
    elif command -v zypper >/dev/null; then run_root zypper --non-interactive install "${packages[@]}" ca-certificates
    else echo 'Install FUSE3 (fusermount3) and unzip using your package manager, then rerun.' >&2; exit 1
    fi
  fi
fi
if [ -z "$version" ]; then
  version=$(curl -fsSL --retry 3 https://api.github.com/repos/mizorewww/gdirect/releases/latest |
    sed -nE 's/^[[:space:]]*"tag_name":[[:space:]]*"v([0-9]+\.[0-9]+\.[0-9]+)".*/\1/p')
fi
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo 'Cannot determine release version.' >&2; exit 1; }
archive="gdirect-$version-$target.tar.gz"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
if [ -n "$from_dir" ]; then
  cp "$from_dir/$archive" "$from_dir/SHA256SUMS" "$tmp/"
else
  base="https://github.com/mizorewww/gdirect/releases/download/v$version"
  fetch "$base/$archive" "$tmp/$archive"
  fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS"
fi
verify() {
  local expected actual
  expected=$(tr -d '\r' < "$2" | awk -v name="$3" '$2==name {print $1}')
  [[ "$expected" =~ ^[0-9a-fA-F]{64}$ ]] || { echo "Missing checksum for $3" >&2; exit 1; }
  if command -v sha256sum >/dev/null; then actual=$(sha256sum "$1" | awk '{print $1}')
  else actual=$(shasum -a 256 "$1" | awk '{print $1}'); fi
  [ "$actual" = "$expected" ] || { echo "Checksum mismatch: $3" >&2; exit 1; }
}
verify "$tmp/$archive" "$tmp/SHA256SUMS" "$archive"
tar -xzf "$tmp/$archive" -C "$tmp"
mkdir -p "$prefix/bin" "$prefix/libexec/gdirect"
prefix=$(cd "$prefix" && pwd)
libexec="$prefix/libexec/gdirect/$version"
mkdir -p "$libexec"
cp -R "$tmp/gdirect-$version-$target/." "$libexec/"
chmod 755 "$libexec/gdirect"
# Keep the executable and its CA bundle adjacent, including when prefix contains spaces.
printf '#!/usr/bin/env bash\nexec %q "$@"\n' "$libexec/gdirect" > "$prefix/bin/gdirect"
chmod 755 "$prefix/bin/gdirect"
if [ "$with_mount" -eq 1 ]; then
  rclone_version=1.75.2
  current=$(rclone version 2>/dev/null | sed -n '1s/^rclone v//p' || true)
  if ! awk -v v="$current" 'BEGIN {split(v,a,".");exit !(a[1]>1 || (a[1]==1 && (a[2]>75 || (a[2]==75 && a[3]>=2))))}'; then
    rclone_archive="rclone-v$rclone_version-$rclone_platform.zip"
    fetch "https://downloads.rclone.org/v$rclone_version/$rclone_archive" "$tmp/$rclone_archive"
    fetch "https://downloads.rclone.org/v$rclone_version/SHA256SUMS" "$tmp/rclone-SHA256SUMS"
    verify "$tmp/$rclone_archive" "$tmp/rclone-SHA256SUMS" "$rclone_archive"
    unzip -q "$tmp/$rclone_archive" -d "$tmp/rclone"
    install -m 755 "$tmp/rclone/rclone-v$rclone_version-$rclone_platform/rclone" "$prefix/bin/rclone"
    mkdir -p "$prefix/share/licenses/rclone"
    cp "$tmp/rclone/rclone-v$rclone_version-$rclone_platform/README.txt" "$prefix/share/licenses/rclone/" 2>/dev/null || true
  fi
fi
"$prefix/bin/gdirect" --version
printf '\nInstalled in %s/bin\n' "$prefix"
# The printed command intentionally keeps $PATH literal.
# shellcheck disable=SC2016
case ":$PATH:" in *":$prefix/bin:"*) ;; *) printf 'Add this to your shell configuration: export PATH=%q:"$PATH"\n' "$prefix/bin" ;; esac
printf '\nFirst login (no gcloud required):\n  gdirect login --client-secret /path/to/client_secret.json\nSetup: https://github.com/mizorewww/gdirect/blob/main/docs/configuration/README.md\n'
