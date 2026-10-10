#!/usr/bin/env bash
set -euo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
# shellcheck source=/dev/null
source "$REPOSITORY_ROOT/deploy/common.sh"

TEST_ROOT="$(mktemp -d)"
SOURCE_FIXTURE="$TEST_ROOT/source"
INSTALL_ROOT="$TEST_ROOT/opt/isarmg/xcos"
CONFIG_ROOT="$TEST_ROOT/etc/isarmg"
STATE_ROOT="$TEST_ROOT/var/lib/isarmg/xcos"
RUNTIME_ROOT="$TEST_ROOT/run/isarmg/xcos"
BUILD_ROOT="$TEST_ROOT/build"
FAKE_BIN="$TEST_ROOT/test-bin"
OPERATION_LOCK_PID=""
ZOMBIE_PARENT_PID=""

cleanup() {
  if [[ -n "$OPERATION_LOCK_PID" ]]; then
    kill "$OPERATION_LOCK_PID" 2>/dev/null || true
    wait "$OPERATION_LOCK_PID" 2>/dev/null || true
  fi
  if [[ -n "$ZOMBIE_PARENT_PID" ]]; then
    kill "$ZOMBIE_PARENT_PID" 2>/dev/null || true
    wait "$ZOMBIE_PARENT_PID" 2>/dev/null || true
  fi
  for pid_file in "$RUNTIME_ROOT/app.pid" "$RUNTIME_ROOT/mediamtx.pid"; do
    if [[ -f "$pid_file" ]]; then
      pid="$(<"$pid_file")"
      if [[ "$pid" =~ ^[1-9][0-9]*$ ]]; then
        kill "$pid" 2>/dev/null || true
      fi
    fi
  done
  chmod -R u+w -- "$TEST_ROOT" 2>/dev/null || true
  rm -rf -- "$TEST_ROOT"
}
trap cleanup EXIT

fail() {
  echo "lifecycle test failed: $*" >&2
  exit 1
}

# A defunct process still answers kill -0, but cannot own a live service PID.
require_command python3
python3 - "$TEST_ROOT/zombie-child.pid" <<'PY' &
import os
import signal
import sys
import time

child = os.fork()
if child == 0:
    os._exit(0)
with open(sys.argv[1], 'w', encoding='ascii') as handle:
    handle.write(str(child))
def finish(_signum, _frame):
    os.waitpid(child, 0)
    sys.exit(0)
signal.signal(signal.SIGTERM, finish)
time.sleep(30)
PY
ZOMBIE_PARENT_PID="$!"
for _ in {1..50}; do
  if [[ -s "$TEST_ROOT/zombie-child.pid" ]]; then
    ZOMBIE_CHILD_PID="$(<"$TEST_ROOT/zombie-child.pid")"
    if [[ -r "/proc/$ZOMBIE_CHILD_PID/status" ]] &&
      grep -q '^State:[[:space:]]*Z' "/proc/$ZOMBIE_CHILD_PID/status"; then
      break
    fi
  fi
  sleep 0.02
done
if [[ -z "${ZOMBIE_CHILD_PID:-}" || ! -r "/proc/${ZOMBIE_CHILD_PID:-}/status" ]] ||
  ! grep -q '^State:[[:space:]]*Z' "/proc/${ZOMBIE_CHILD_PID:-}/status"; then
  fail "could not create the zombie PID fixture"
fi
printf '%s\n' "$ZOMBIE_CHILD_PID" >"$TEST_ROOT/zombie.pid"
chmod 0600 -- "$TEST_ROOT/zombie.pid"
if process_is_live "$ZOMBIE_CHILD_PID"; then
  fail "a zombie process passed the live-process check"
fi
if read_running_pid "$TEST_ROOT/zombie.pid"; then
  fail "a zombie process was treated as a running service"
fi
kill "$ZOMBIE_PARENT_PID"
wait "$ZOMBIE_PARENT_PID" 2>/dev/null || true
ZOMBIE_PARENT_PID=""

mkdir -p -- "$SOURCE_FIXTURE/deploy" "$SOURCE_FIXTURE/scripts" "$SOURCE_FIXTURE/config" "$FAKE_BIN"
install -m 0755 -- \
  "$REPOSITORY_ROOT/deploy/common.sh" \
  "$REPOSITORY_ROOT/deploy/xcosctl" \
  "$SOURCE_FIXTURE/deploy/"
install -m 0755 -- "$REPOSITORY_ROOT/scripts/build.sh" "$SOURCE_FIXTURE/scripts/build.sh"
install -m 0644 -- \
  "$REPOSITORY_ROOT/deploy/.bootstrap-action.sh" \
  "$REPOSITORY_ROOT/deploy/.start-action.sh" \
  "$REPOSITORY_ROOT/deploy/.status-action.sh" \
  "$REPOSITORY_ROOT/deploy/.stop-action.sh" \
  "$SOURCE_FIXTURE/deploy/"
install -m 0644 -- "$REPOSITORY_ROOT/config/mediamtx.yml" "$SOURCE_FIXTURE/config/mediamtx.yml"
printf '%s\n' \
  '[package]' \
  'name = "xcos"' \
  'version = "1.0.0"' \
  >"$SOURCE_FIXTURE/Cargo.toml"
FAKE_MEDIA="$TEST_ROOT/fake-mediamtx"
cat >"$FAKE_MEDIA" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "--version" ]]; then
  echo 'v1.20.0'
  exit 0
fi
if [[ -n "${FAKE_MEDIA_PID_AUDIT:-}" ]]; then
  printf '%s\n' "$$" >"$FAKE_MEDIA_PID_AUDIT"
fi
trap 'exit 0' TERM INT
while :; do sleep 1; done
EOF
chmod 0755 -- "$FAKE_MEDIA"
FAKE_MEDIA_SHA="$(sha256sum -- "$FAKE_MEDIA" | awk '{print $1}')"
FAKE_SOURCE_REVISION="0123456789abcdef0123456789abcdef01234567"
FAKE_CONFIG_SHA="$(sha256sum -- "$SOURCE_FIXTURE/config/mediamtx.yml" | awk '{print $1}')"
FAKE_RELEASE_CONTRACT="bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
printf '%s\n' \
  '# Xcos lifecycle fixture companion contract.' \
  'version=v1.20.0' \
  'platform=linux_amd64' \
  "sha256=$FAKE_MEDIA_SHA" \
  >"$SOURCE_FIXTURE/config/mediamtx.lock"

FAKE_APP="$TEST_ROOT/fake-xcos"
cat >"$FAKE_APP" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "${1:-missing-command}" in
  --version)
    echo "xcos 1.0.0 target=x86_64-unknown-linux-gnu source=${FAKE_SOURCE_REVISION:?}"
    ;;
  web-assets)
    printf '%s' "${FAKE_WEB_MANIFEST:?}"
    ;;
  static-contract)
    printf '%s\n' "${FAKE_STATIC_CONTRACT:?}"
    ;;
  release-manifest-header)
    cat <<HEADER
format=xcos-release-v1
application=xcos
application_version=1.0.0
source_revision=${FAKE_SOURCE_REVISION:?}
target=x86_64-unknown-linux-gnu
wire_protocol=xcos-wire-v2
api_prefix=/api/v1
schema_revision=2
schema_sha256=4d20083821ff39d78792d0795b26206e851c2e6d0523109ee49cfc06666a1d4a
credential_envelope_revision=1
credential_contract_sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
static_contract_sha256=${FAKE_STATIC_CONTRACT:?}
mediamtx_version=v1.20.0
mediamtx_platform=linux_amd64
mediamtx_sha256=${FAKE_MEDIA_SHA:?}
mediamtx_config_sha256=${FAKE_CONFIG_SHA:?}
release_contract_sha256=${FAKE_RELEASE_CONTRACT:?}
HEADER
    ;;
  verify-release)
    root="${2:?}"
    [[ "$(readlink -f -- "$0")" == "$(readlink -f -- "$root/bin/xcos")" ]]
    ;;
  init)
    [[ "${2:-}" == "--username" && "${3:-}" == "admin" ]]
    IFS= read -r password
    [[ "$password" == "operator-reviewed-password-1.0.0" ]]
    [[ ! -e "${DATABASE_URL#sqlite://}" ]]
    printf '%s' current-initialized-state >"${DATABASE_URL#sqlite://}"
    mkdir -m 0700 -- "$(dirname "${DATABASE_URL#sqlite://}")/logs"
    ;;
  config)
    [[ "${2:-}" == "validate" && -f "${DATABASE_URL#sqlite://}" ]]
    ;;
  status)
    [[ -s "${XCOS_RUNTIME_DIR:?}/app.pid" ]] || exit 1
    printf '%s\n' '{"ready":true,"state":"ready"}'
    ;;
  run)
    [[ "${2:-}" == "--release-root" ]]
    root="${3:?}"
    [[ "$(readlink -f -- "$0")" == "$(readlink -f -- "$root/bin/xcos")" ]]
    [[ -f "${DATABASE_URL#sqlite://}" && -z "${BOOTSTRAP_ADMIN_PASSWORD:-}" ]]
    umask 077
    printf '%s\n' "$$" >"${XCOS_RUNTIME_DIR:?}/app.pid"
    cleanup() { rm -f -- "$XCOS_RUNTIME_DIR/app.pid"; }
    trap cleanup EXIT
    trap 'exit 0' TERM INT
    while :; do sleep 1; done
    ;;
  *)
    exit 2
    ;;
esac
EOF
chmod 0755 -- "$FAKE_APP"

cat >"$FAKE_BIN/curl" <<'EOF'
#!/usr/bin/env bash
if [[ "${XCOS_TEST_CURL_FAILURE:-}" == "1" ]]; then
  exit 22
fi
if [[ "${*: -1}" == */readyz ]]; then
  printf '%s' '{"ready":true}:200'
fi
exit 0
EOF
chmod 0755 -- "$FAKE_BIN/curl"

refresh_static_contract() {
  # Serialized fixture bytes are passed as data, never executed.
  # shellcheck disable=SC2089
  FAKE_WEB_MANIFEST='{"format":"lifecycle-fixture"}'
  FAKE_STATIC_CONTRACT="$(printf '%s' "$FAKE_WEB_MANIFEST" | sha256sum | awk '{print $1}')"
  # shellcheck disable=SC2090
  export FAKE_WEB_MANIFEST FAKE_STATIC_CONTRACT
}

run_build() {
  env \
    PATH="$FAKE_BIN:$PATH" \
    XCOS_NATIVE_INSTALL_ROOT="$INSTALL_ROOT" \
    XCOS_NATIVE_TEST_ROOT="$TEST_ROOT" \
    XCOS_BUILD_TARGET="$BUILD_ROOT" \
    XCOS_MEDIAMTX_SOURCE="$FAKE_MEDIA" \
    XCOS_APP_BINARY_SOURCE="$FAKE_APP" \
    XCOS_SOURCE_REVISION="$FAKE_SOURCE_REVISION" \
    FAKE_WEB_MANIFEST="$FAKE_WEB_MANIFEST" \
    FAKE_STATIC_CONTRACT="$FAKE_STATIC_CONTRACT" \
    FAKE_SOURCE_REVISION="$FAKE_SOURCE_REVISION" \
    FAKE_MEDIA_SHA="$FAKE_MEDIA_SHA" \
    FAKE_CONFIG_SHA="$FAKE_CONFIG_SHA" \
    FAKE_RELEASE_CONTRACT="$FAKE_RELEASE_CONTRACT" \
    "$SOURCE_FIXTURE/scripts/build.sh"
}

run_operation() {
  env \
    PATH="$FAKE_BIN:$PATH" \
    XCOS_NATIVE_INSTALL_ROOT="$INSTALL_ROOT" \
    XCOS_NATIVE_CONFIG_DIR="$CONFIG_ROOT" \
    XCOS_NATIVE_STATE_DIR="$STATE_ROOT" \
    XCOS_NATIVE_RUNTIME_DIR="$RUNTIME_ROOT" \
    FAKE_WEB_MANIFEST="$FAKE_WEB_MANIFEST" \
    FAKE_STATIC_CONTRACT="$FAKE_STATIC_CONTRACT" \
    FAKE_SOURCE_REVISION="$FAKE_SOURCE_REVISION" \
    FAKE_MEDIA_SHA="$FAKE_MEDIA_SHA" \
    FAKE_CONFIG_SHA="$FAKE_CONFIG_SHA" \
    FAKE_RELEASE_CONTRACT="$FAKE_RELEASE_CONTRACT" \
    "$@"
}

refresh_static_contract
run_build >/dev/null
[[ -d "$INSTALL_ROOT/releases/1.0.0" && ! -L "$INSTALL_ROOT/releases/1.0.0" ]] ||
  fail "physical release directory is missing"
[[ ! -e "$INSTALL_ROOT/current" && ! -L "$INSTALL_ROOT/current" ]] ||
  fail "publisher created a mutable current alias"
[[ -x "$INSTALL_ROOT/releases/1.0.0/deploy/xcosctl" ]] || fail "release lifecycle command is missing"
[[ ! -x "$INSTALL_ROOT/releases/1.0.0/deploy/.start-action.sh" ]] ||
  fail "internal lifecycle modules must not be executable"
[[ "$(find "$INSTALL_ROOT/releases" -maxdepth 1 -name '.0.5.1.stage.*' -print -quit)" == "" ]] ||
  fail "physical release publication left a staging directory"
[[ -z "$(find -P "$INSTALL_ROOT/releases/1.0.0" -perm /222 -print -quit)" ]] ||
  fail "published release contains writable entries"

# Neither a mutable alias nor an ordinary source-bound run command is a
# valid way to enter the current product.
ln -s -- releases/1.0.0 "$INSTALL_ROOT/current"
if run_operation "$INSTALL_ROOT/current/deploy/xcosctl" status >"$TEST_ROOT/alias-status.out" 2>&1; then
  fail "an operational script accepted a mutable release alias"
fi
rm -- "$INSTALL_ROOT/current"
if run_operation "$INSTALL_ROOT/releases/1.0.0/bin/xcos" run \
  >"$TEST_ROOT/ordinary-run.out" 2>&1; then
  fail "a source-bound Xcos binary accepted run without its physical release root"
fi

# A physical version is a one-shot destination even when all supplied bytes
# are identical. Rejection must not change the installed tree.
FIRST_MANIFEST="$(sha256sum "$INSTALL_ROOT/releases/1.0.0/RELEASE-MANIFEST" | awk '{print $1}')"
FIRST_LAYOUT="$(find -P "$INSTALL_ROOT" -printf '%P %y %m %s\n' | LC_ALL=C sort | sha256sum | awk '{print $1}')"
if run_build >"$TEST_ROOT/second-build.out" 2>&1; then
  fail "publisher accepted a second Xcos 1.0.0 publication"
fi
grep -q 'one-shot' "$TEST_ROOT/second-build.out" ||
  fail "second publication did not identify the one-shot boundary"
[[ "$(sha256sum "$INSTALL_ROOT/releases/1.0.0/RELEASE-MANIFEST" | awk '{print $1}')" == "$FIRST_MANIFEST" ]] ||
  fail "rejected second publication changed the release manifest"
[[ "$(find -P "$INSTALL_ROOT" -printf '%P %y %m %s\n' | LC_ALL=C sort | sha256sum | awk '{print $1}')" == "$FIRST_LAYOUT" ]] ||
  fail "rejected second publication changed the installed layout"

# Any pre-existing physical version is refused before invoking a build or
# changing its bytes. The publisher only creates an empty first destination.
OTHER_INSTALL="$TEST_ROOT/existing/opt/isarmg/xcos"
mkdir -p -- "$OTHER_INSTALL/releases/0.1.0"
printf '%s' retained-physical-release >"$OTHER_INSTALL/releases/0.1.0/xcos"
OTHER_LAYOUT="$(find -P "$OTHER_INSTALL" -printf '%P %y %m %s\n' | LC_ALL=C sort | sha256sum | awk '{print $1}')"
if env \
  PATH="$FAKE_BIN:$PATH" \
  XCOS_NATIVE_INSTALL_ROOT="$OTHER_INSTALL" \
  XCOS_NATIVE_TEST_ROOT="$TEST_ROOT" \
  XCOS_BUILD_TARGET="$TEST_ROOT/other-build" \
  XCOS_MEDIAMTX_SOURCE="$FAKE_MEDIA" \
  XCOS_APP_BINARY_SOURCE="$FAKE_APP" \
  XCOS_SOURCE_REVISION="$FAKE_SOURCE_REVISION" \
  "$SOURCE_FIXTURE/scripts/build.sh" >"$TEST_ROOT/other-version-build.out" 2>&1; then
  fail "publisher accepted a destination containing a different version"
fi
[[ "$(cat "$OTHER_INSTALL/releases/0.1.0/xcos")" == retained-physical-release ]] ||
  fail "rejected publication changed existing physical bytes"
[[ "$(find -P "$OTHER_INSTALL" -printf '%P %y %m %s\n' | LC_ALL=C sort | sha256sum | awk '{print $1}')" == "$OTHER_LAYOUT" ]] ||
  fail "rejected publication changed the existing destination layout"
[[ ! -e "$TEST_ROOT/other-build" && ! -e "$OTHER_INSTALL/current" && ! -L "$OTHER_INSTALL/current" ]] ||
  fail "rejected publication created build output or a mutable alias"

# A symlinked deployment parent is rejected before publication.
EVIL_INSTALL="$TEST_ROOT/evil/opt/isarmg/xcos"
mkdir -p -- "$EVIL_INSTALL" "$TEST_ROOT/evil-target"
ln -s -- "$TEST_ROOT/evil-target" "$EVIL_INSTALL/releases"
if env \
  PATH="$FAKE_BIN:$PATH" \
  XCOS_NATIVE_INSTALL_ROOT="$EVIL_INSTALL" \
  XCOS_NATIVE_TEST_ROOT="$TEST_ROOT" \
  XCOS_BUILD_TARGET="$TEST_ROOT/evil-build" \
  XCOS_MEDIAMTX_SOURCE="$FAKE_MEDIA" \
  XCOS_APP_BINARY_SOURCE="$FAKE_APP" \
  XCOS_WEB_SOURCE="$SOURCE_FIXTURE/web-dist" \
  FAKE_STATIC_CONTRACT="$FAKE_STATIC_CONTRACT" \
  "$SOURCE_FIXTURE/scripts/build.sh" >"$TEST_ROOT/symlink-build.out" 2>&1; then
  fail "build accepted a symlinked releases directory"
fi

# A configuration path with an intermediate symlink is rejected.
mkdir -p -- "$TEST_ROOT/bad-config-real"
ln -s -- "$TEST_ROOT/bad-config-real" "$TEST_ROOT/bad-config-link"
if env \
  PATH="$FAKE_BIN:$PATH" \
  XCOS_NATIVE_INSTALL_ROOT="$INSTALL_ROOT" \
  XCOS_NATIVE_CONFIG_DIR="$TEST_ROOT/bad-config-link" \
  XCOS_NATIVE_STATE_DIR="$STATE_ROOT" \
  XCOS_NATIVE_RUNTIME_DIR="$RUNTIME_ROOT" \
  "$INSTALL_ROOT/releases/1.0.0/deploy/xcosctl" bootstrap >"$TEST_ROOT/symlink-config.out" 2>&1; then
  fail "bootstrap accepted a symlinked configuration path"
fi

# A final configuration file symlink is never treated as an existing config.
BAD_FINAL_CONFIG="$TEST_ROOT/bad-final-config"
mkdir -p -- "$BAD_FINAL_CONFIG"
chmod 0755 -- "$BAD_FINAL_CONFIG"
printf '%s\n' 'must-not-be-read' >"$TEST_ROOT/symlink-env-target"
ln -s -- "$TEST_ROOT/symlink-env-target" "$BAD_FINAL_CONFIG/xcos.env"
if env \
  PATH="$FAKE_BIN:$PATH" \
  XCOS_NATIVE_INSTALL_ROOT="$INSTALL_ROOT" \
  XCOS_NATIVE_CONFIG_DIR="$BAD_FINAL_CONFIG" \
  XCOS_NATIVE_STATE_DIR="$STATE_ROOT" \
  XCOS_NATIVE_RUNTIME_DIR="$RUNTIME_ROOT" \
  "$INSTALL_ROOT/releases/1.0.0/deploy/xcosctl" bootstrap >"$TEST_ROOT/symlink-env.out" 2>&1; then
  fail "bootstrap accepted a symbolic-link environment file"
fi

BOOTSTRAP_OUTPUT="$TEST_ROOT/bootstrap.out"
run_operation "$INSTALL_ROOT/releases/1.0.0/deploy/xcosctl" bootstrap >"$BOOTSTRAP_OUTPUT"
ENV_FILE="$CONFIG_ROOT/xcos.env"
[[ "$(stat -c '%a' "$ENV_FILE")" == "600" ]] || fail "environment file is not mode 0600"
! grep -q '^XCSS_DEV_WEB_DIR=' "$ENV_FILE" || fail "production sets a development Web override"
grep -q '^PUBLIC_RTSP_PUBLISH_BASE_URL=REPLACE_WITH_PUBLIC_RTSPS_ORIGIN$' "$ENV_FILE" ||
  fail "bootstrap did not require an explicit public RTSPS publish origin"
JWT_VALUE="$(sed -n 's/^APP_JWT_SECRET=//p' "$ENV_FILE")"
KEY_VALUE="$(sed -n 's/^CREDENTIALS_KEY=//p' "$ENV_FILE")"
PASSWORD_VALUE="$(sed -n 's/^BOOTSTRAP_ADMIN_PASSWORD=//p' "$ENV_FILE")"
for secret in "$JWT_VALUE" "$KEY_VALUE" "$PASSWORD_VALUE"; do
  if grep -Fq -- "$secret" "$BOOTSTRAP_OUTPUT"; then
    fail "bootstrap printed a generated secret"
  fi
done

ENV_DIGEST="$(sha256sum "$ENV_FILE" | awk '{print $1}')"
run_operation "$INSTALL_ROOT/releases/1.0.0/deploy/xcosctl" bootstrap >/dev/null
[[ "$(sha256sum "$ENV_FILE" | awk '{print $1}')" == "$ENV_DIGEST" ]] ||
  fail "bootstrap overwrote an existing environment file"
if run_operation "$INSTALL_ROOT/releases/1.0.0/deploy/xcosctl" start >"$TEST_ROOT/unconfirmed.out" 2>&1; then
  fail "start accepted an unconfirmed generated administrator password"
fi

sed -i 's/^BOOTSTRAP_ADMIN_PASSWORD=.*/BOOTSTRAP_ADMIN_PASSWORD=operator-reviewed-password-1.0.0/' "$ENV_FILE"
sed -i 's|^PUBLIC_RTSP_PUBLISH_BASE_URL=.*|PUBLIC_RTSP_PUBLISH_BASE_URL=rtsps://xcos.example:8322|' "$ENV_FILE"
printf '%s\n' 'test certificate' >"$CONFIG_ROOT/xcos-rtsp.crt"
printf '%s\n' 'test private key' >"$CONFIG_ROOT/xcos-rtsp.key"
chmod 0644 -- "$CONFIG_ROOT/xcos-rtsp.crt"
chmod 0600 -- "$CONFIG_ROOT/xcos-rtsp.key"
chmod 0600 -- "$ENV_FILE"
run_operation "$INSTALL_ROOT/releases/1.0.0/deploy/xcosctl" bootstrap --confirm-config >/dev/null
[[ ! -e "$CONFIG_ROOT/xcos.REVIEW-SECRETS-BEFORE-START" ]] || fail "review marker was not cleared"
[[ -f "$STATE_ROOT/db/app.db" && -d "$STATE_ROOT/db/logs" ]] || fail "confirmation did not explicitly init current state"
if grep -q '^BOOTSTRAP_ADMIN_PASSWORD=' "$ENV_FILE"; then
  fail "confirmation retained the transient administrator password after successful init"
fi
CURRENT_STATE_SHA="$(sha256sum "$STATE_ROOT/db/app.db" | awk '{print $1}')"
run_operation "$INSTALL_ROOT/releases/1.0.0/deploy/xcosctl" bootstrap --confirm-config >/dev/null
[[ "$(sha256sum "$STATE_ROOT/db/app.db" | awk '{print $1}')" == "$CURRENT_STATE_SHA" ]] || fail "confirmation reinitialized existing state"

# Failure after spawning the companion rolls back only this invocation and
# leaves no stale PID file or surviving process.
FAILED_MEDIA_PID_AUDIT="$TEST_ROOT/failed-media.pid"
if run_operation env \
  XCOS_TEST_CURL_FAILURE=1 \
  FAKE_MEDIA_PID_AUDIT="$FAILED_MEDIA_PID_AUDIT" \
  "$INSTALL_ROOT/releases/1.0.0/deploy/xcosctl" start >"$TEST_ROOT/readiness-failure.out" 2>&1; then
  fail "start succeeded while its readiness probe failed"
fi
[[ -s "$FAILED_MEDIA_PID_AUDIT" ]] || fail "failed start never launched the companion fixture"
FAILED_MEDIA_PID="$(<"$FAILED_MEDIA_PID_AUDIT")"
if kill -0 "$FAILED_MEDIA_PID" 2>/dev/null; then
  fail "failed start left its companion process running"
fi
[[ ! -e "$RUNTIME_ROOT/mediamtx.pid" ]] || fail "failed start left a companion PID file"
[[ ! -e "$RUNTIME_ROOT/app.pid" ]] || fail "failed start left an application PID file"

# start and stop share a short-lived operation lock, so concurrent launchers
# cannot race while publishing PID files.
(
  exec 8<>"$RUNTIME_ROOT/operations.lock"
  flock --exclusive 8
  # The child closes FD 8; only this subshell owns the test lock.
  sleep 30 8>&-
) &
OPERATION_LOCK_PID="$!"
OPERATION_LOCK_HELD=false
for _ in {1..20}; do
  if ! flock --exclusive --nonblock "$RUNTIME_ROOT/operations.lock" -c true; then
    OPERATION_LOCK_HELD=true
    break
  fi
  sleep 0.01
done
[[ "$OPERATION_LOCK_HELD" == true ]] || fail "operation-lock fixture did not acquire its lock"
START_IGNORED_OPERATION_LOCK=false
if run_operation "$INSTALL_ROOT/releases/1.0.0/deploy/xcosctl" start >"$TEST_ROOT/operation-lock.out" 2>&1; then
  START_IGNORED_OPERATION_LOCK=true
fi
kill "$OPERATION_LOCK_PID" 2>/dev/null || true
wait "$OPERATION_LOCK_PID" 2>/dev/null || true
OPERATION_LOCK_PID=""
[[ "$START_IGNORED_OPERATION_LOCK" != true ]] || fail "start ignored another active native operation"

# Runtime entries and artifacts must remain complete after the source fixture disappears.
chmod -R u+w -- "$SOURCE_FIXTURE"
rm -rf -- "$SOURCE_FIXTURE"
run_operation "$INSTALL_ROOT/releases/1.0.0/deploy/xcosctl" start >/dev/null
for _ in {1..40}; do
  [[ -s "$RUNTIME_ROOT/app.pid" && -s "$RUNTIME_ROOT/mediamtx.pid" ]] && break
  sleep 0.05
done
[[ -s "$RUNTIME_ROOT/app.pid" && -s "$RUNTIME_ROOT/mediamtx.pid" ]] ||
  fail "release processes did not publish their PID files"
STATUS_OUTPUT="$(run_operation "$INSTALL_ROOT/releases/1.0.0/deploy/xcosctl" status)"
[[ "$STATUS_OUTPUT" == *'Rust application: running'* ]] || fail "status missed the application"
[[ "$STATUS_OUTPUT" == *'MediaMTX: running'* ]] || fail "status missed MediaMTX"
ORIGINAL_APP_PID="$(<"$RUNTIME_ROOT/app.pid")"
printf '%s\n' invalid >"$RUNTIME_ROOT/app.pid"
if run_operation "$INSTALL_ROOT/releases/1.0.0/deploy/xcosctl" status >"$TEST_ROOT/invalid-pid.out" 2>&1; then
  fail "status treated a malformed PID file as a stopped service"
fi
printf '%s\n' "$$" >"$RUNTIME_ROOT/app.pid"
if run_operation "$INSTALL_ROOT/releases/1.0.0/deploy/xcosctl" status >"$TEST_ROOT/foreign-pid.out" 2>&1; then
  fail "status accepted a PID owned by a different process"
fi
printf '%s\n' "$ORIGINAL_APP_PID" >"$RUNTIME_ROOT/app.pid"
run_operation "$INSTALL_ROOT/releases/1.0.0/deploy/xcosctl" stop >/dev/null
if STATUS_OUTPUT="$(run_operation "$INSTALL_ROOT/releases/1.0.0/deploy/xcosctl" status)"; then
  fail "status reported success for a stopped application"
fi
[[ "$STATUS_OUTPUT" == *'Rust application: stopped'* ]] || fail "application did not stop"
[[ "$STATUS_OUTPUT" == *'MediaMTX: stopped'* ]] || fail "MediaMTX did not stop"

# Existing release files with hard-link aliases fail closed.
RELEASE_ROOT="$INSTALL_ROOT/releases/1.0.0"
chmod 0755 -- "$RELEASE_ROOT" "$RELEASE_ROOT/share"
ln -- "$RELEASE_ROOT/share/web-assets.json" "$TEST_ROOT/release-hardlink-alias"
chmod 0555 -- "$RELEASE_ROOT/share" "$RELEASE_ROOT"
if run_operation "$INSTALL_ROOT/releases/1.0.0/deploy/xcosctl" status >"$TEST_ROOT/hardlink.out" 2>&1; then
  fail "release verification accepted a hard-linked asset"
fi

grep -q '^sha256=25947caac403f37ec881c9be213af2cad67e344a6c7098905b0d31c17f40e336$' \
  "$REPOSITORY_ROOT/config/mediamtx.lock" || fail "the reviewed production MediaMTX digest changed"

echo "Xcos native temporary-root lifecycle tests passed"
