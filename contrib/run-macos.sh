#!/bin/sh
set -eu

binary=$1
shift
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

/usr/bin/codesign \
    --sign - \
    --entitlements "$script_dir/macos-hypervisor.entitlements" \
    --force \
    "$binary" >/dev/null

exec "$binary" "$@"
