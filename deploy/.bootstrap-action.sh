#!/usr/bin/env bash
set -euo pipefail

[[ "${XCOS_LIFECYCLE_DISPATCH:-}" == "bootstrap" ]] || {
  echo "Use deploy/xcosctl bootstrap" >&2
  exit 2
}

SCRIPT_PATH="${BASH_SOURCE[0]}"
# shellcheck source=/dev/null
source "$(cd "$(dirname "$SCRIPT_PATH")" && pwd -P)/common.sh"
resolve_release_context "$SCRIPT_PATH"
deployment_paths
verify_release "$XCOS_RELEASE_ROOT"

if (( $# == 0 )); then
  MODE="create"
elif (( $# == 1 )) && [[ "$1" == "--confirm-config" ]]; then
  MODE="confirm"
else
  die "Usage: xcosctl bootstrap [--confirm-config]"
fi

ensure_directory "$(dirname "$XCOS_CONFIG_DIR")" 755 "configuration parent"
ensure_directory "$XCOS_CONFIG_DIR" 755 "Xcss configuration directory"
ensure_directory "$(dirname "$XCOS_STATE_DIR")" 755 "state parent"
ensure_directory "$XCOS_STATE_DIR" 700 "state directory"
ensure_directory "$XCOS_STATE_DIR/db" 700 "database directory"
ensure_directory "$XCOS_STATE_DIR/recordings" 700 "recordings directory"
ensure_directory "$XCOS_STATE_DIR/logs" 700 "log directory"
ensure_directory "$(dirname "$XCOS_RUNTIME_PATH")" 755 "runtime parent"
ensure_directory "$XCOS_RUNTIME_PATH" 700 "runtime directory"

if [[ "$MODE" == "confirm" ]]; then
  load_deployment_env
  require_runtime_contract
  require_command flock
  umask 077
  acquire_native_operation_lock
  APP_BIN="$XCOS_RELEASE_ROOT/bin/xcos"
  if [[ -e "$XCOS_STATE_DIR/db/app.db" || -L "$XCOS_STATE_DIR/db/app.db" ]]; then
    "$APP_BIN" config validate
  else
    [[ -n "${BOOTSTRAP_ADMIN_PASSWORD:-}" ]] ||
      die "Explicit init requires the reviewed administrator password"
    printf '%s\n' "$BOOTSTRAP_ADMIN_PASSWORD" |
      "$APP_BIN" init --username "${BOOTSTRAP_ADMIN_USERNAME:-admin}"
    "$APP_BIN" config validate
  fi
  # Clear the transient input only after explicit init and current-data checks
  # have succeeded. Normal startup never creates an administrator or database.
  remove_bootstrap_password
  if [[ -e "$XCOS_REVIEW_MARKER" || -L "$XCOS_REVIEW_MARKER" ]]; then
    assert_private_file "$XCOS_REVIEW_MARKER" "configuration review marker"
    rm -- "$XCOS_REVIEW_MARKER"
  fi
  echo "Xcos $XCOS_VERSION current state explicitly initialized and configuration accepted. Start it with: $XCOS_RELEASE_ROOT/deploy/xcosctl start"
  exit 0
fi

if [[ -e "$XCOS_ENV_FILE" || -L "$XCOS_ENV_FILE" ]]; then
  assert_private_file "$XCOS_ENV_FILE" "Xcos environment file"
  echo "Configuration already exists and was not changed: $XCOS_ENV_FILE"
  if [[ -e "$XCOS_REVIEW_MARKER" || -L "$XCOS_REVIEW_MARKER" ]]; then
    assert_private_file "$XCOS_REVIEW_MARKER" "configuration review marker"
    echo "Review it, then run: $XCOS_RELEASE_ROOT/deploy/xcosctl bootstrap --confirm-config"
  fi
  exit 0
fi

require_command openssl
JWT_SECRET="$(openssl rand -hex 48)"
CREDENTIAL_KEY="$(openssl rand -base64 32 | tr -d '\n')"
ADMIN_PASSWORD="$(openssl rand -hex 24)"

# Publish the review gate before the configuration. A crash can therefore
# leave the deployment unconfigured or unconfirmed, but never startable with
# an administrator password the operator has not reviewed.
if [[ -e "$XCOS_REVIEW_MARKER" || -L "$XCOS_REVIEW_MARKER" ]]; then
  assert_private_file "$XCOS_REVIEW_MARKER" "configuration review marker"
else
  (umask 077; set -o noclobber; : >"$XCOS_REVIEW_MARKER") ||
    die "Failed to create the configuration review marker"
  chmod 600 -- "$XCOS_REVIEW_MARKER"
  assert_private_file "$XCOS_REVIEW_MARKER" "configuration review marker"
fi

CONFIG_TEMP="$(mktemp "$XCOS_CONFIG_DIR/.xcos.env.XXXXXX")"
cleanup_config_temp() {
  if [[ -n "${CONFIG_TEMP:-}" && ( -e "$CONFIG_TEMP" || -L "$CONFIG_TEMP" ) ]]; then
    rm -- "$CONFIG_TEMP"
  fi
}
trap cleanup_config_temp EXIT
(
  umask 077
  {
    echo "BIND_ADDR=127.0.0.1:8080"
    echo "DATABASE_URL=sqlite://$XCOS_STATE_DIR/db/app.db"
    echo "APP_JWT_SECRET=$JWT_SECRET"
    echo "CREDENTIALS_KEY=$CREDENTIAL_KEY"
    echo "BOOTSTRAP_ADMIN_USERNAME=admin"
    echo "BOOTSTRAP_ADMIN_PASSWORD=$ADMIN_PASSWORD"
    echo "APP_ENV=production"
    echo "MEDIA_TOKEN_TTL_SECS=120"
    echo "LOGIN_BODY_LIMIT_BYTES=16384"
    echo "LOGIN_RATE_CAPACITY=4096"
    echo "LOGIN_SOURCE_ATTEMPTS=30"
    echo "LOGIN_SOURCE_WINDOW_SECS=60"
    echo "LOGIN_ACCOUNT_ATTEMPTS=10"
    echo "LOGIN_ACCOUNT_WINDOW_SECS=300"
    echo "LOGIN_ARGON2_PARALLELISM=2"
    echo "LOGIN_ARGON2_TIMEOUT_MS=5000"
    echo "MEDIAMTX_API_URL=http://127.0.0.1:9997"
    echo "MEDIAMTX_PLAYBACK_URL=http://127.0.0.1:9996"
    echo "MEDIAMTX_CONFIG=$XCOS_RELEASE_ROOT/config/mediamtx.yml"
    echo "MEDIAMTX_CONTRACT=$XCOS_RELEASE_ROOT/config/mediamtx.lock"
    echo "MEDIAMTX_BINARY=$XCOS_RELEASE_ROOT/bin/mediamtx"
    echo "RECORDINGS_DIR=$XCOS_STATE_DIR/recordings"
    echo "XCOS_RUNTIME_DIR=$XCOS_RUNTIME_PATH"
    echo "PUBLIC_WEBRTC_BASE_URL=/media-webrtc"
    echo "PUBLIC_HLS_BASE_URL=/media-hls"
    echo "PUBLIC_RTSP_PUBLISH_BASE_URL=REPLACE_WITH_PUBLIC_RTSPS_ORIGIN"
    echo "MEDIAMTX_RTSP_CERT=$XCOS_CONFIG_DIR/xcos-rtsp.crt"
    echo "MEDIAMTX_RTSP_KEY=$XCOS_CONFIG_DIR/xcos-rtsp.key"
    echo "MEDIA_PUBLIC_HOSTS=127.0.0.1"
    echo "STATUS_INTERVAL_SECS=10"
    echo "RECONCILE_INTERVAL_SECS=60"
    echo "REQUEST_TIMEOUT_SECS=20"
    echo "RUST_LOG=info,tower_http=info"
  } >"$CONFIG_TEMP"
)
chmod 600 -- "$CONFIG_TEMP"
assert_private_file "$CONFIG_TEMP" "temporary Xcos environment file"

# link(2) supplies no-overwrite publication in the same private directory.
# Until the temporary name is removed the nlink check keeps consumers closed.
ln -- "$CONFIG_TEMP" "$XCOS_ENV_FILE" ||
  die "Xcos environment file appeared during bootstrap; it was not overwritten"
rm -- "$CONFIG_TEMP"
CONFIG_TEMP=""
trap - EXIT
assert_private_file "$XCOS_ENV_FILE" "Xcos environment file"
unset JWT_SECRET CREDENTIAL_KEY ADMIN_PASSWORD

echo "Created private configuration without printing its secrets: $XCOS_ENV_FILE"
echo "Replace the generated administrator password, set PUBLIC_RTSP_PUBLISH_BASE_URL to the"
echo "client-reachable rtsps:// origin, provision its trusted certificate/key, and review every setting with a protected editor."
echo "Then run: $XCOS_RELEASE_ROOT/deploy/xcosctl bootstrap --confirm-config"
