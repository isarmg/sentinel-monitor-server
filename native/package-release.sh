#!/usr/bin/env bash
set -euo pipefail

SOURCE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
# shellcheck source=/dev/null
source "$SOURCE_ROOT/native/common.sh"
OUTPUT="${1:?usage: package-release.sh ABSOLUTE_OUTPUT_DIRECTORY}"
validate_absolute_path "$OUTPUT" "output directory"
assert_directory "$OUTPUT" "output directory"
[[ "$OUTPUT" != "$SOURCE_ROOT" && "$OUTPUT" != "$SOURCE_ROOT/"* ]] || die "output must be outside the checkout"
ARCHIVE="sentinel-monitor-$SENTINEL_VERSION-x86_64-unknown-linux-gnu.tar.gz"
[[ ! -e "$OUTPUT/$ARCHIVE" && ! -e "$OUTPUT/SHA256SUMS" ]] || die "refusing to overwrite release assets"
TEMPORARY="$(mktemp -d)"
cleanup() {
  chmod -R u+w -- "$TEMPORARY"
  rm -rf -- "$TEMPORARY"
}
trap cleanup EXIT
media_version="$(awk -F= '$1 == "version" { print $2 }' "$SOURCE_ROOT/config/mediamtx.lock")"
[[ "$media_version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "invalid pinned MediaMTX version"
curl --fail --location --retry 3 --output "$TEMPORARY/mediamtx.tar.gz" \
  "https://github.com/bluenviron/mediamtx/releases/download/$media_version/mediamtx_${media_version}_linux_amd64.tar.gz"
tar -xzf "$TEMPORARY/mediamtx.tar.gz" -C "$TEMPORARY" mediamtx LICENSE
mkdir -p "$TEMPORARY/archive/licenses"
SENTINEL_NATIVE_INSTALL_ROOT="$TEMPORARY/archive/opt/isarmg/sentinel-monitor" \
  SENTINEL_BUILD_TARGET="$TEMPORARY/build" \
  SENTINEL_MEDIAMTX_SOURCE="$TEMPORARY/mediamtx" \
  bash "$SOURCE_ROOT/native/build.sh"
release_root="$TEMPORARY/archive/opt/isarmg/sentinel-monitor/releases/$SENTINEL_VERSION"
verify_release "$release_root"
install -m 0444 "$TEMPORARY/LICENSE" "$TEMPORARY/archive/licenses/MediaMTX-LICENSE"
for name in OFL.txt CJK-LICENSE.txt NORMAL-LICENSE.txt; do
  install -m 0444 "$SOURCE_ROOT/clients/web/fonts/$name" "$TEMPORARY/archive/licenses/$name"
done
install -m 0444 "$SOURCE_ROOT/docs/releases/$SENTINEL_VERSION.md" "$TEMPORARY/archive/README.md"
epoch="$(git -C "$SOURCE_ROOT" show -s --format=%ct HEAD)"
tar --sort=name --mtime="@$epoch" --owner=0 --group=0 --numeric-owner \
  -C "$TEMPORARY/archive" -czf "$OUTPUT/$ARCHIVE" README.md licenses opt
mkdir "$TEMPORARY/extracted"
tar -xzf "$OUTPUT/$ARCHIVE" -C "$TEMPORARY/extracted"
verify_release "$TEMPORARY/extracted/opt/isarmg/sentinel-monitor/releases/$SENTINEL_VERSION"
(cd "$OUTPUT" && sha256sum "$ARCHIVE" > SHA256SUMS)
printf 'Verified immutable release archive: %s\n' "$OUTPUT/$ARCHIVE"
