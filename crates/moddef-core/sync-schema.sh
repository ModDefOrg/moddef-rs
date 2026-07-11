#!/usr/bin/env sh
# Sync the vendored proto schema from the moddef repo (CI diffs for drift).
set -eu
src="${1:-../../../moddef/proto/moddef/v1}"
dst="$(dirname "$0")/proto/moddef/v1"
cp "$src"/*.proto "$dst"/
echo "synced $(ls "$dst" | wc -l) proto files from $src"
