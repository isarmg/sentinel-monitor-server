#!/usr/bin/env bash
set -euo pipefail

SOURCE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
# shellcheck source=/dev/null
source "$SOURCE_ROOT/deploy/common.sh"

INSTALL_ROOT="${XCOS_NATIVE_INSTALL_ROOT:-/opt/isarmg/xcos}"
BUILD_TARGET="${XCOS_BUILD_TARGET:-/var/tmp/xcos-build}"
MEDIA_SOURCE="${XCOS_MEDIAMTX_SOURCE:-}"
APP_SOURCE="${XCOS_APP_BINARY_SOURCE:-}"
SOURCE_REVISION="${XCOS_SOURCE_REVISION:-}"

validate_absolute_path "$INSTALL_ROOT" "XCOS_NATIVE_INSTALL_ROOT"
validate_absolute_path "$BUILD_TARGET" "XCOS_BUILD_TARGET"
[[ -n "$MEDIA_SOURCE" ]] || die "XCOS_MEDIAMTX_SOURCE is required"
validate_absolute_path "$MEDIA_SOURCE" "XCOS_MEDIAMTX_SOURCE"
assert_regular_file "$MEDIA_SOURCE" "MediaMTX source binary"

# Prebuilt inputs exist only to keep the temporary-root lifecycle test fast.
# They can never publish under a production path.
if [[ -n "$APP_SOURCE" ]]; then
  FIXTURE_ROOT="${XCOS_NATIVE_TEST_ROOT:-}"
  [[ -n "$FIXTURE_ROOT" ]] ||
    die "prebuilt application inputs are restricted to the native lifecycle test"
  validate_absolute_path "$FIXTURE_ROOT" "XCOS_NATIVE_TEST_ROOT"
  assert_private_directory "$FIXTURE_ROOT" "native lifecycle test root"
  for fixture_path in "$SOURCE_ROOT" "$INSTALL_ROOT" "$BUILD_TARGET" "$MEDIA_SOURCE" "$APP_SOURCE"; do
    [[ -z "$fixture_path" || "$fixture_path" == "$FIXTURE_ROOT/"* ]] ||
      die "native lifecycle fixture paths must remain below XCOS_NATIVE_TEST_ROOT"
  done
fi
require_command awk
require_command cmp
require_command find
require_command flock
require_command sha256sum
require_command sort
[[ "$(uname -m)" == "x86_64" ]] ||
  die "Xcos formal release builds require an x86_64 Linux builder"
[[ "$(uname -s)" == "Linux" ]] ||
  die "Xcos formal release builds require Linux"
RUST_TARGET="x86_64-unknown-linux-gnu"

is_source_revision() {
  [[ "$1" =~ ^[0-9a-f]{40}$ ]]
}

require_empty_release_destination() {
  local entry
  local -a entries
  assert_no_symlink_components "$INSTALL_ROOT" "install root"
  if [[ ! -e "$INSTALL_ROOT" && ! -L "$INSTALL_ROOT" ]]; then
    return
  fi
  [[ -d "$INSTALL_ROOT" && ! -L "$INSTALL_ROOT" ]] ||
    die "Install root must be a real directory"
  entries=("$INSTALL_ROOT"/*)
  for entry in "${entries[@]}"; do
    [[ "$entry" == "$INSTALL_ROOT/releases" ]] ||
      die "Install root contains an unexpected pre-existing entry"
  done
  if [[ -e "$INSTALL_ROOT/releases" || -L "$INSTALL_ROOT/releases" ]]; then
    [[ -d "$INSTALL_ROOT/releases" && ! -L "$INSTALL_ROOT/releases" ]] ||
      die "Releases destination must be a real directory"
    entries=("$INSTALL_ROOT/releases"/*)
    (( ${#entries[@]} == 0 )) ||
      die "Releases destination is not empty; Xcos 1.0.0 publication is one-shot"
  fi
}

shopt -s nullglob dotglob
require_empty_release_destination

if [[ -n "$APP_SOURCE" ]]; then
  is_source_revision "$SOURCE_REVISION" ||
    die "XCOS_SOURCE_REVISION must be a full lowercase commit in lifecycle tests"
else
  require_command git
  [[ -d "$SOURCE_ROOT/.git" || -f "$SOURCE_ROOT/.git" ]] ||
    die "Official publication requires a Git checkout"
  [[ -z "$(git -C "$SOURCE_ROOT" status --porcelain=v1 --untracked-files=all)" ]] ||
    die "Official publication requires a completely clean source tree"
  SOURCE_REVISION="$(git -C "$SOURCE_ROOT" rev-parse --verify HEAD)"
  is_source_revision "$SOURCE_REVISION" || die "Git HEAD is not a full lowercase commit"
  [[ "$(git -C "$SOURCE_ROOT" cat-file -t "refs/tags/v$XCOS_VERSION" 2>/dev/null || true)" == "tag" ]] ||
    die "Official publication requires annotated tag v$XCOS_VERSION"
  [[ "$(git -C "$SOURCE_ROOT" rev-parse "refs/tags/v$XCOS_VERSION^{commit}")" == "$SOURCE_REVISION" ]] ||
    die "Annotated tag v$XCOS_VERSION must identify HEAD"
fi

SOURCE_VERSION="$(awk '
  /^\[package\]$/ { package = 1; next }
  /^\[/ { package = 0 }
  package && /^version[[:space:]]*=/ {
    value = $0
    sub(/^[^=]*=[[:space:]]*"/, "", value)
    sub(/".*/, "", value)
    print value
    exit
  }
' "$SOURCE_ROOT/Cargo.toml")"
[[ "$SOURCE_VERSION" == "$XCOS_VERSION" ]] ||
  die "Cargo package version must be exactly $XCOS_VERSION"

manifest_value() {
  local key="$1"
  awk -F= -v key="$key" '$1 == key { print $2 }' "$SOURCE_ROOT/config/mediamtx.lock"
}

EXPECTED_MEDIA_VERSION="$(manifest_value version)"
EXPECTED_MEDIA_PLATFORM="$(manifest_value platform)"
EXPECTED_MEDIA_SHA256="$(manifest_value sha256)"
[[ "$EXPECTED_MEDIA_PLATFORM" == "linux_amd64" ]] ||
  die "Unsupported MediaMTX companion platform: $EXPECTED_MEDIA_PLATFORM"
[[ "$($MEDIA_SOURCE --version | tr -d '\r\n')" == "$EXPECTED_MEDIA_VERSION" ]] ||
  die "MediaMTX source version does not match config/mediamtx.lock"
[[ "$(sha256sum -- "$MEDIA_SOURCE" | awk '{print $1}')" == "$EXPECTED_MEDIA_SHA256" ]] ||
  die "MediaMTX source SHA-256 does not match config/mediamtx.lock"

ensure_directory "$BUILD_TARGET" 755 "build target"
TEMPORARY="$(mktemp -d "$BUILD_TARGET/release-build.XXXXXX")"
STAGE=""
STAGE_CONTAINER=""
cleanup() {
  if [[ -n "$STAGE" && -d "$STAGE" ]]; then
    chmod -R u+w -- "$STAGE" 2>/dev/null || true
    rm -rf -- "$STAGE"
  fi
  if [[ -n "$STAGE_CONTAINER" && -d "$STAGE_CONTAINER" && ! -L "$STAGE_CONTAINER" ]]; then
    chmod -R u+w -- "$STAGE_CONTAINER" 2>/dev/null || true
    rm -rf -- "$STAGE_CONTAINER"
  fi
  chmod -R u+w -- "$TEMPORARY" 2>/dev/null || true
  rm -rf -- "$TEMPORARY"
}
trap cleanup EXIT

APP_STAGE="$TEMPORARY/xcos"
if [[ -n "$APP_SOURCE" ]]; then
  validate_absolute_path "$APP_SOURCE" "XCOS_APP_BINARY_SOURCE"
  assert_regular_file "$APP_SOURCE" "prebuilt Xcos binary"
  install -m 0755 -- "$APP_SOURCE" "$APP_STAGE"
else
  require_command npm
  npm ci --prefix "$SOURCE_ROOT/web"
  CARGO_TARGET_DIR="$BUILD_TARGET/cargo" \
    "$SOURCE_ROOT/web/node_modules/.bin/xcss-build-server" \
    --config "$SOURCE_ROOT/foundation-web-build.json" --mode release \
    --no-install --source-revision "$SOURCE_REVISION" >"$TEMPORARY/common-build.log"
  BUILT_APP="$(tail -n 1 "$TEMPORARY/common-build.log")"
  validate_absolute_path "$BUILT_APP" "common builder executable"
  # Cargo may hard-link its named output to the content-addressed deps binary.
  # Treat that output only as a build input; publish an independent private copy.
  assert_no_symlink_components "$(dirname "$BUILT_APP")" "common builder executable parent"
  [[ -f "$BUILT_APP" && ! -L "$BUILT_APP" ]] ||
    die "Common builder output must be a regular non-symlink file"
  [[ -x "$BUILT_APP" ]] || die "Common builder did not report an executable"
  install -m 0755 -- "$BUILT_APP" "$APP_STAGE"
fi
assert_regular_file "$APP_STAGE" "staged Xcos binary"
STATIC_MANIFEST="$TEMPORARY/web-assets.json"
"$APP_STAGE" web-assets >"$STATIC_MANIFEST"
STATIC_CONTRACT="$(sha256sum -- "$STATIC_MANIFEST" | awk '{print $1}')"
[[ "$($APP_STAGE --version | tr -d '\r\n')" == "$XCOS_PRODUCT $XCOS_VERSION target=$RUST_TARGET source=$SOURCE_REVISION" ]] ||
  die "Xcos application binary version is not $XCOS_VERSION"
[[ "$($APP_STAGE static-contract | tr -d '\r\n')" == "$STATIC_CONTRACT" ]] ||
  die "Xcos binary is not bound to the staged Web asset contract"
[[ "$($APP_STAGE release-manifest-header | awk -F= '$1 == "source_revision" { print $2 }')" == "$SOURCE_REVISION" ]] ||
  die "Xcos binary is not bound to the exact source revision"

require_empty_release_destination
ensure_directory "$INSTALL_ROOT" 755 "install root"
RELEASES_ROOT="$INSTALL_ROOT/releases"
ensure_directory "$RELEASES_ROOT" 755 "releases directory"
STAGE_CONTAINER="$(mktemp -d "$RELEASES_ROOT/.${XCOS_VERSION}.stage.XXXXXX")"
STAGE="$STAGE_CONTAINER/opt/isarmg/xcos/releases/$XCOS_VERSION"
mkdir -p -- "$STAGE/bin" "$STAGE/config" "$STAGE/deploy" "$STAGE/share"
install -m 0555 -- "$APP_STAGE" "$STAGE/bin/xcos"
install -m 0555 -- "$MEDIA_SOURCE" "$STAGE/bin/mediamtx"
install -m 0444 -- "$SOURCE_ROOT/config/mediamtx.yml" "$STAGE/config/mediamtx.yml"
install -m 0444 -- "$SOURCE_ROOT/config/mediamtx.lock" "$STAGE/config/mediamtx.lock"
for script in common.sh xcosctl; do
  install -m 0555 -- "$SOURCE_ROOT/deploy/$script" "$STAGE/deploy/$script"
done
for action in .bootstrap-action.sh .start-action.sh .status-action.sh .stop-action.sh; do
  install -m 0444 -- "$SOURCE_ROOT/deploy/$action" "$STAGE/deploy/$action"
done
install -m 0444 -- "$STATIC_MANIFEST" "$STAGE/share/web-assets.json"
find -P "$STAGE" -mindepth 1 -type d -exec chmod 0555 -- {} +

# Revalidate the bytes that will actually be published, closing mutation
# windows between source validation and installation into the release stage.
[[ "$("$STAGE/bin/mediamtx" --version | tr -d '\r\n')" == "$EXPECTED_MEDIA_VERSION" ]] ||
  die "Staged MediaMTX version does not match the pinned contract"
[[ "$(sha256sum -- "$STAGE/bin/mediamtx" | awk '{print $1}')" == "$EXPECTED_MEDIA_SHA256" ]] ||
  die "Staged MediaMTX SHA-256 does not match the pinned contract"
[[ "$(awk -F= '$1 == "version" { print $2 }' "$STAGE/config/mediamtx.lock")" == "$EXPECTED_MEDIA_VERSION" ]] ||
  die "Staged MediaMTX lock changed during publication"
[[ "$(awk -F= '$1 == "platform" { print $2 }' "$STAGE/config/mediamtx.lock")" == "$EXPECTED_MEDIA_PLATFORM" ]] ||
  die "Staged MediaMTX platform lock changed during publication"
[[ "$(awk -F= '$1 == "sha256" { print $2 }' "$STAGE/config/mediamtx.lock")" == "$EXPECTED_MEDIA_SHA256" ]] ||
  die "Staged MediaMTX digest lock changed during publication"
[[ "$(sha256sum -- "$STAGE/share/web-assets.json" | awk '{print $1}')" == "$STATIC_CONTRACT" ]] ||
  die "Staged Web inventory changed during publication"
[[ "$("$STAGE/bin/xcos" static-contract | tr -d '\r\n')" == "$STATIC_CONTRACT" ]] ||
  die "Staged Xcos application is not bound to the staged Web inventory"

RELEASE_MANIFEST="$TEMPORARY/RELEASE-MANIFEST"
write_release_manifest "$STAGE" "$RELEASE_MANIFEST"
install -m 0444 -- "$RELEASE_MANIFEST" "$STAGE/RELEASE-MANIFEST"
chmod 0555 -- "$STAGE"
verify_release "$STAGE"

RELEASE_ROOT="$RELEASES_ROOT/$XCOS_VERSION"
[[ ! -e "$RELEASE_ROOT" && ! -L "$RELEASE_ROOT" ]] ||
  die "Release $XCOS_VERSION destination appeared during publication"
# Open only the staging root mode for the rename boundary; every payload entry
# remains read-only, and no runtime alias or service points at the destination.
chmod 0755 -- "$STAGE"
mv -T -n -- "$STAGE" "$RELEASE_ROOT"
[[ ! -e "$STAGE" && ! -L "$STAGE" ]] ||
  die "Release $XCOS_VERSION destination appeared concurrently"
STAGE=""
chmod 0555 -- "$RELEASE_ROOT"
chmod -R u+w -- "$STAGE_CONTAINER" 2>/dev/null || true
rm -rf -- "$STAGE_CONTAINER"
STAGE_CONTAINER=""
verify_release "$RELEASE_ROOT"

if [[ -z "${XCOS_NATIVE_TEST_ROOT:-}" ]]; then
  [[ -z "$(git -C "$SOURCE_ROOT" status --porcelain=v1 --untracked-files=all)" ]] ||
    die "Official publication changed the source tree"
fi

echo "Published immutable Xcos release: $RELEASE_ROOT"
echo "Next: $RELEASE_ROOT/deploy/xcosctl bootstrap"
