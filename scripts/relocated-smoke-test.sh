#!/usr/bin/env bash
set -euo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
# shellcheck source=/dev/null
source "$REPOSITORY_ROOT/deploy/common.sh"

TEST_ROOT="$(mktemp -d)"
APP_PID=""
cleanup() {
  if [[ -n "$APP_PID" ]]; then
    kill "$APP_PID" 2>/dev/null || true
    wait "$APP_PID" 2>/dev/null || true
  fi
  chmod -R u+w -- "$TEST_ROOT" 2>/dev/null || true
  rm -rf -- "$TEST_ROOT"
}
trap cleanup EXIT

WEB_ROOT="$TEST_ROOT/release/web"
APP_ROOT="$TEST_ROOT/release/bin"
STATE_ROOT="$TEST_ROOT/state"
RUNTIME_ROOT="$TEST_ROOT/runtime"
mkdir -p -- "$WEB_ROOT" "$APP_ROOT" "$STATE_ROOT/db" "$RUNTIME_ROOT"
chmod 0700 -- "$STATE_ROOT" "$STATE_ROOT/db" "$RUNTIME_ROOT"

SMOKE_SCOPE="${XCOS_SMOKE_SCOPE:-all}"
[[ "$SMOKE_SCOPE" == all || "$SMOKE_SCOPE" == unbound ]] || die "Unknown smoke scope"
if [[ -n "${XCOS_SMOKE_UNBOUND_BINARY:-}" ]]; then
  BUILT_APP="$XCOS_SMOKE_UNBOUND_BINARY"
  [[ "$BUILT_APP" == /* && -f "$BUILT_APP" && -x "$BUILT_APP" && ! -L "$BUILT_APP" ]] ||
    die "Prebuilt unbound binary must be an absolute regular executable"
  [[ "$("$BUILT_APP" --version)" == *' source=unbound' ]] ||
    die "Development relocation requires an unbound binary"
  cp -a -- "$REPOSITORY_ROOT/web/dist/." "$WEB_ROOT/"
else
  npm ci --prefix "$REPOSITORY_ROOT/web" >/dev/null
  CARGO_PROFILE_DEV_DEBUG=0 "$REPOSITORY_ROOT/web/node_modules/.bin/xcss-build-server" \
    --config "$REPOSITORY_ROOT/xcss-web-build.json" --mode development \
    --no-install --dist "$WEB_ROOT" >"$TEST_ROOT/unified-build.log"
  # The common entry reports Cargo's actual executable path as its final output.
  BUILT_APP="$(tail -n 1 "$TEST_ROOT/unified-build.log")"
fi
[[ "$BUILT_APP" == /* && -f "$BUILT_APP" && -x "$BUILT_APP" && ! -L "$BUILT_APP" ]] || {
  [[ ! -f "$TEST_ROOT/unified-build.log" ]] || cat "$TEST_ROOT/unified-build.log" >&2
  echo "Common builder did not report a real executable" >&2
  exit 1
}
install -m 0555 -- "$BUILT_APP" "$APP_ROOT/xcos"
STATIC_MANIFEST="$TEST_ROOT/web-assets.json"
"$APP_ROOT/xcos" web-assets >"$STATIC_MANIFEST"
EXPECTED_CONTRACT="$(sha256sum -- "$STATIC_MANIFEST" | awk '{print $1}')"
[[ "$("$APP_ROOT/xcos" static-contract | tr -d '\r\n')" == "$EXPECTED_CONTRACT" ]] || {
  echo "Relocated binary has the wrong embedded Web inventory" >&2
  exit 1
}
# The executable must not need the built Web directory at runtime.
mv -- "$WEB_ROOT" "$TEST_ROOT/build-input-hidden"

PORT="$((24000 + (BASHPID % 10000)))"
APP_LOG="$TEST_ROOT/app.log"
(
  cd /
  export BIND_ADDR="127.0.0.1:$PORT"
  export DATABASE_URL="sqlite://$STATE_ROOT/db/app.db"
  export APP_JWT_SECRET="relocated-smoke-jwt-secret-at-least-32-bytes"
  export CREDENTIALS_KEY="AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
  export APP_ENV=production XCOS_RUNTIME_DIR="$RUNTIME_ROOT"
  export MEDIAMTX_API_URL="http://127.0.0.1:9" MEDIAMTX_PLAYBACK_URL="http://127.0.0.1:9"
  export PUBLIC_RTSP_PUBLISH_BASE_URL="rtsps://xcos.example:8322"
  export STATUS_INTERVAL_SECS=60 RECONCILE_INTERVAL_SECS=60 REQUEST_TIMEOUT_SECS=1 RUST_LOG=warn
  # Missing data must fail before explicit init, without creating database or
  # administrator, database or lingering PID. Common lease files may remain.
  # This is a real binary, not the lifecycle fixture.
  if "$APP_ROOT/xcos" run --json >"$TEST_ROOT/before-init.json" 2>"$TEST_ROOT/before-init.log"; then
    echo "Run initialized missing data without explicit init" >&2
    exit 1
  fi
  [[ ! -e "$STATE_ROOT/db/app.db" && -z "$(find "$RUNTIME_ROOT" -mindepth 1 -maxdepth 1 ! -name .xcss-instance.lock ! -name .xcss-maintenance.lock -print -quit)" ]] || {
    cat "$TEST_ROOT/before-init.json" "$TEST_ROOT/before-init.log" >&2
    echo "Missing-data rejection left a database or unexpected runtime state" >&2
    exit 1
  }
  if ! printf '%s\n' 'relocated-smoke-bootstrap-password' |
    "$APP_ROOT/xcos" init --username smoke-admin >"$TEST_ROOT/init.log" 2>&1; then
    cat "$TEST_ROOT/init.log" >&2
    echo "Explicit init failed after missing-data run rejection" >&2
    exit 1
  fi
  "$APP_ROOT/xcos" run >"$APP_LOG" 2>&1
) &
APP_PID="$!"

READY=false
for _ in {1..120}; do
  if curl --fail --silent "http://127.0.0.1:$PORT/healthz" >/dev/null; then
    READY=true
    break
  fi
  if ! kill -0 "$APP_PID" 2>/dev/null; then
    break
  fi
  sleep 0.1
done
if [[ "$READY" != true ]]; then
  [[ ! -f "$APP_LOG" ]] || sed -n '1,160p' "$APP_LOG" >&2
  echo "Relocated Xcos binary did not start" >&2
  exit 1
fi

python3 - "$STATIC_MANIFEST" "http://127.0.0.1:$PORT" <<'HTTP'
import hashlib,json,sys,urllib.error,urllib.request
manifest=json.load(open(sys.argv[1]))
assert manifest['format']=='xcss-web-assets-v1'
for asset in manifest['files']:
    url=sys.argv[2]+'/'+asset['path']
    with urllib.request.urlopen(url) as response:
        body=response.read()
        assert len(body)==asset['size'] and hashlib.sha256(body).hexdigest()==asset['sha256'],asset['path']
        assert response.headers['content-type']==asset['content_type'],asset['path']
        assert response.headers['x-content-type-options']=='nosniff'
        etag=response.headers['etag']
    with urllib.request.urlopen(urllib.request.Request(url,method='HEAD')) as response:
        assert not response.read() and int(response.headers['content-length'])==asset['size']
    if not asset['path'].endswith('.html'):
        try: urllib.request.urlopen(urllib.request.Request(url,headers={'If-None-Match':etag}))
        except urllib.error.HTTPError as error: assert error.code==304 and not error.read()
        else: raise AssertionError('asset did not revalidate')
try: urllib.request.urlopen(sys.argv[2]+'/assets/missing-resource.js')
except urllib.error.HTTPError as error: assert error.code==404
else: raise AssertionError('missing asset did not return 404')
HTTP

kill "$APP_PID"
wait "$APP_PID" 2>/dev/null || true
APP_PID=""

# Production rejects an external-directory override before touching new state.
REJECTED_STATE="$TEST_ROOT/rejected-override"
mkdir -p -- "$REJECTED_STATE/db" "$REJECTED_STATE/runtime"
chmod 0700 -- "$REJECTED_STATE" "$REJECTED_STATE/db" "$REJECTED_STATE/runtime"
if env BIND_ADDR="127.0.0.1:$PORT" DATABASE_URL="sqlite://$REJECTED_STATE/db/app.db" \
  APP_JWT_SECRET="relocated-smoke-jwt-secret-at-least-32-bytes" \
  CREDENTIALS_KEY="AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=" \
  APP_ENV=production XCOS_RUNTIME_DIR="$REJECTED_STATE/runtime" \
  XCSS_DEV_WEB_DIR="$TEST_ROOT/build-input-hidden" \
  PUBLIC_RTSP_PUBLISH_BASE_URL="rtsps://xcos.example:8322" \
  "$APP_ROOT/xcos" run --json >"$TEST_ROOT/override.json" 2>"$TEST_ROOT/override.log"; then
  echo "Production accepted a development Web override" >&2
  exit 1
fi
python3 - "$TEST_ROOT/override.json" <<'ERROR'
import json,sys
error=json.load(open(sys.argv[1]))
assert error['code'] == 'invalid_development_override'
assert error['retryable'] is False
assert 'rejected-override' not in json.dumps(error)
ERROR
[[ ! -e "$REJECTED_STATE/db/app.db" ]]
[[ -z "$(find "$REJECTED_STATE/runtime" -mindepth 1 -print -quit)" ]]
mv -- "$TEST_ROOT/build-input-hidden" "$WEB_ROOT"

if [[ "$SMOKE_SCOPE" == unbound ]]; then
  echo "Xcos real unbound relocation/assets/init/override smoke passed; source-bound release checks not executed"
  exit 0
fi

# A source-bound binary must reject run without its physical release root and
# a missing command; neither case initializes state or starts a listener.
# before it creates a database or runtime lock. Full physical release startup
# is exercised by the native publication test with the pinned companion.
BOUND_APP="$TEST_ROOT/source-bound-xcos"
if [[ -n "${XCOS_SMOKE_BOUND_BINARY:-}" ]]; then
  [[ "$XCOS_SMOKE_BOUND_BINARY" == /* && -f "$XCOS_SMOKE_BOUND_BINARY" && -x "$XCOS_SMOKE_BOUND_BINARY" && ! -L "$XCOS_SMOKE_BOUND_BINARY" ]] ||
    die "Prebuilt bound binary must be an absolute regular executable"
  [[ "$("$XCOS_SMOKE_BOUND_BINARY" --version)" =~ source=[0-9a-f]{40}$ ]] ||
    die "Release smoke requires a full source-bound identity"
  install -m 0555 -- "$XCOS_SMOKE_BOUND_BINARY" "$BOUND_APP"
else
  BOUND_REVISION="$(git -C "$REPOSITORY_ROOT" rev-parse HEAD)"
  "$REPOSITORY_ROOT/web/node_modules/.bin/xcss-build-server" \
    --config "$REPOSITORY_ROOT/xcss-web-build.json" --mode release \
    --no-install --rust-only --source-revision "$BOUND_REVISION" >"$TEST_ROOT/bound-build.log"
  BUILT_BOUND_APP="$(tail -n 1 "$TEST_ROOT/bound-build.log")"
  [[ "$BUILT_BOUND_APP" == /* && -f "$BUILT_BOUND_APP" && -x "$BUILT_BOUND_APP" && ! -L "$BUILT_BOUND_APP" ]] ||
    die "Common builder did not report the newly built bound executable"
  install -m 0555 -- "$BUILT_BOUND_APP" "$BOUND_APP"
fi
[[ "$("$BOUND_APP" --version)" =~ source=[0-9a-f]{40}$ ]] ||
  die "Release smoke requires the actual source-bound binary"
for command in run implicit; do
  BOUND_STATE="$TEST_ROOT/source-bound-$command"
  mkdir -p -- "$BOUND_STATE/db" "$BOUND_STATE/runtime"
  chmod 0700 -- "$BOUND_STATE" "$BOUND_STATE/db" "$BOUND_STATE/runtime"
  arguments=(run --json)
  if [[ "$command" == "implicit" ]]; then
    arguments=(--json)
  fi
  if env \
    BIND_ADDR="127.0.0.1:$PORT" \
    DATABASE_URL="sqlite://$BOUND_STATE/db/app.db" \
    APP_JWT_SECRET="relocated-smoke-jwt-secret-at-least-32-bytes" \
    CREDENTIALS_KEY="AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=" \
    APP_ENV=production \
    XCOS_RUNTIME_DIR="$BOUND_STATE/runtime" \
    "$BOUND_APP" "${arguments[@]}" >"$BOUND_STATE/rejection.json" 2>"$BOUND_STATE/rejection.log"; then
    echo "Source-bound binary accepted $command without its physical release root" >&2
    exit 1
  fi
  python3 - "$BOUND_STATE/rejection.json" "$command" <<'ERROR'
import json,sys
error=json.load(open(sys.argv[1]))
expected = 'release_root_required' if sys.argv[2] == 'run' else 'bad_request'
assert error['code'] == expected, error
assert error['retryable'] is False
assert 'source-bound-' not in json.dumps(error)
ERROR
  [[ ! -e "$BOUND_STATE/db/app.db" && -z "$(find "$BOUND_STATE/runtime" -mindepth 1 -print -quit)" ]] || {
    echo "Source-bound $command rejection wrote runtime state" >&2
    exit 1
  }
done

echo "Xcos relocated binary and exact static asset smoke tests passed"
