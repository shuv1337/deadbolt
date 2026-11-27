#!/bin/sh
# Packs one platform's CLI release tarball, shuvbolt-<version>-<platform>.tar.gz,
# from a Cargo release directory: the headless `shuvbolt` at the root; on
# macOS the Touch ID helper `deadbolt-enclave` beside it (the daemon looks for
# it next to its own executable; docs/touch-id.md), the app shipping only as
# Shuvbolt.app; on glibc Linux the desktop app, `shuvbolt-ui`, beside it (which
# `shuvbolt app` finds there). Install all files into one directory.
#
#   scripts/package-cli.sh <version> <platform> <cargo release dir> <out>
#
# <platform> is linux-x64, linux-arm64 (glibc, with the app),
# linux-x64-musl, linux-arm64-musl (static, no app), darwin-x64 or darwin-arm64.
set -eu

[ $# -eq 4 ] || { echo "Usage: $0 <version> <platform> <cargo release dir> <out>" >&2; exit 1; }
version=$1 platform=$2 bin=$3 out=$4
root=$(cd "$(dirname "$0")/.." && pwd)

case "$platform" in
  linux-x64 | linux-arm64) files="shuvbolt shuvbolt-ui" ;;
  linux-x64-musl | linux-arm64-musl) files=shuvbolt ;;
  darwin-x64 | darwin-arm64) files="shuvbolt deadbolt-enclave" ;;
  *) echo "Unknown platform $platform." >&2; exit 1 ;;
esac

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
for file in $files; do
  [ -f "$bin/$file" ] || { echo "$bin/$file is missing." >&2; exit 1; }
  cp "$bin/$file" "$stage/$file"
  chmod 755 "$stage/$file"
done
"$root/scripts/check-ca-roots.sh" "$stage/shuvbolt"

mkdir -p "$out"
tarball="$out/shuvbolt-$version-$platform.tar.gz"
# shellcheck disable=SC2086 # $files is a list of plain names.
COPYFILE_DISABLE=1 tar -czf "$tarball" -C "$stage" $files
echo "$tarball"
