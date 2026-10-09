#!/usr/bin/env bash
set -euo pipefail

[[ "${XCOS_LIFECYCLE_DISPATCH:-}" == "stop" ]] || {
  echo "Use deploy/xcosctl stop" >&2
  exit 2
}

SCRIPT_PATH="${BASH_SOURCE[0]}"
# shellcheck source=/dev/null
source "$(cd "$(dirname "$SCRIPT_PATH")" && pwd -P)/common.sh"
resolve_release_context "$SCRIPT_PATH"
deployment_paths
verify_release "$XCOS_RELEASE_ROOT"
require_command flock
acquire_native_operation_lock

stop_pid() {
  local name="$1"
  local pid_file="$2"
  local binary="$3"
  local pid
  pid="$(read_optional_running_pid "$pid_file" "$binary")"
  if [[ -z "$pid" ]]; then
    if [[ -e "$pid_file" || -L "$pid_file" ]]; then
      assert_private_file "$pid_file" "$name PID file"
      rm -- "$pid_file"
    fi
    echo "$name: already stopped"
    return
  fi
  kill "$pid"
  for _ in {1..40}; do
    process_is_live "$pid" || break
    sleep 0.25
  done
  process_is_live "$pid" && die "$name did not stop within 10 seconds"
  if [[ -e "$pid_file" || -L "$pid_file" ]]; then
    assert_private_file "$pid_file" "$name PID file"
    rm -- "$pid_file"
  fi
  echo "$name stopped"
}

# Keep the established shutdown order: application/database/runtime locks first,
# then the MediaMTX companion lock.
stop_pid "Rust application" "$XCOS_RUNTIME_PATH/app.pid" \
  "$XCOS_RELEASE_ROOT/bin/xcos"
stop_pid "MediaMTX" "$XCOS_RUNTIME_PATH/mediamtx.pid" \
  "$XCOS_RELEASE_ROOT/bin/mediamtx"
