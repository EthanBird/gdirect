#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
root="$PWD"
version=1.37.0
sha=60a420ad7085eb616cb6e2bdf0a7206d68ff3d37fb5a956dc44242eb2f79b66b
prefix="$root/target/libaria2"
mkdir -p target/vendor
archive="$root/target/vendor/aria2-$version.tar.xz"
if [ ! -f "$archive" ]; then
  curl --fail --location --retry 3 "https://github.com/aria2/aria2/releases/download/release-$version/aria2-$version.tar.xz" -o "$archive"
fi
printf '%s  %s\n' "$sha" "$archive" | shasum -a 256 -c -
rm -rf "target/vendor/aria2-$version"
tar -xf "$archive" -C target/vendor
cd "target/vendor/aria2-$version"
patch -p1 < "$root/vendor/aria2-$version.patch"
cp "$root/vendor/gdirect_route.h" src/
if [ "$(uname -s)" = Darwin ]; then
  openssl_prefix="${OPENSSL_PREFIX:-$(brew --prefix openssl@3)}"
  export OPENSSL_CFLAGS="-I$openssl_prefix/include"
  export OPENSSL_LIBS="-L$openssl_prefix/lib -lssl -lcrypto"
fi
./configure --prefix="$prefix" --enable-libaria2 --enable-static --disable-shared \
  --with-openssl --without-appletls --without-gnutls --without-libnettle \
  --without-libgmp --without-libgcrypt --without-libcares --without-libssh2 \
  --without-sqlite3 --without-libxml2 --without-libexpat --without-libz \
  --disable-bittorrent --disable-metalink --disable-websocket --disable-nls \
  --disable-dependency-tracking
make -j "${JOBS:-4}" -C src libaria2.la
make -C src install-libLTLIBRARIES
mkdir -p "$prefix/include/aria2"
cp src/includes/aria2/aria2.h "$prefix/include/aria2/"
printf '\nBuilt patched libaria2 %s in %s\n' "$version" "$prefix"
