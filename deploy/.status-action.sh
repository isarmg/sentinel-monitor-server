#!/usr/bin/env bash
set -euo pipefail

[[ "${XCOS_LIFECYCLE_DISPATCH:-}" == "status" ]] || {
  echo "Use deploy/xcosctl status" >&2
  exit 2
}

SCRIPT_PATH="${BASH_SOURCE[0]}"
# shellcheck source=/dev/null
source "$(cd "$(dirname "$SCRIPT_PATH")" && pwd -P)/common.sh"
resolve_release_context "$SCRIPT_PATH"
deployment_paths
verify_release "$XCOS_RELEASE_ROOT"

show_status() {
  local name="$1"
  local pid_file="$2"
  local binary="$3"
  local pid
  pid="$(read_optional_running_pid "$pid_file" "$binary")"
  if [[ -n "$pid" ]]; then
    echo "$name: running (PID $pid)"
  else
    echo "$name: stopped"
  fi
}

show_status "Rust application" "$XCOS_RUNTIME_PATH/app.pid" \
  "$XCOS_RELEASE_ROOT/bin/xcos"
show_status "MediaMTX" "$XCOS_RUNTIME_PATH/mediamtx.pid" \
  "$XCOS_RELEASE_ROOT/bin/mediamtx"
load_deployment_env
# The binary owns bounded readiness, service identity and the error envelope.
# A stale/foreign HTTP service or stopped application must remain a failure.
"$XCOS_RELEASE_ROOT/bin/xcos" status
