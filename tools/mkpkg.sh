#!/bin/sh
# usage: mkpkg.sh <version> <rvp-pack> <screen-record ARM binary> <out.rvp>
set -eu
[ $# -eq 4 ] || { echo "usage: $0 <version> <rvp-pack> <binary> <out.rvp>" >&2; exit 2; }
ver=$1
case $ver in ''|*[!0-9A-Za-z.+~-]*) echo "bad version: $ver" >&2; exit 2;; esac
abs() { case $1 in /*) echo "$1";; *) echo "$PWD/$1";; esac; }
pack=$(abs "$2"); bin=$(abs "$3"); out=$(abs "$4")
cd "$(dirname "$0")/.."
[ -f "$bin" ] || { echo "missing binary: $bin" >&2; exit 1; }
rm -rf build-pkg
cp -r pkg build-pkg
mkdir -p build-pkg/payload/bin
cp "$bin" build-pkg/payload/bin/screen-record
sed "s/\"version\": \"0.0.0\"/\"version\": \"$ver\"/" pkg/manifest.json > build-pkg/manifest.json
grep -qF "\"version\": \"$ver\"" build-pkg/manifest.json || { echo "version rewrite failed" >&2; exit 1; }
"$pack" build-pkg "$out"
