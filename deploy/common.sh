#!/usr/bin/env bash

# Shared with scripts/build.sh.
# shellcheck disable=SC2034
readonly XCOS_PRODUCT="xcos"
readonly XCOS_VERSION="1.0.0"

die() {
  echo "$*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "Missing required command: $1"
}

validate_absolute_path() {
  local path="$1"
  local label="$2"
  [[ "$path" == /* ]] || die "$label must be an absolute path"
  [[ "$path" != *$'\n'* && "$path" != *$'\r'* ]] || die "$label contains a newline"
  [[ "$path" =~ ^/[A-Za-z0-9._/-]+$ ]] || die "$label contains non-portable characters"
  [[ "/$path/" != *'/../'* && "/$path/" != *'/./'* && "$path" != *'//'* ]] ||
    die "$label must be lexically normalized"
}

assert_no_symlink_components() {
  local path="$1"
  local label="$2"
  validate_absolute_path "$path" "$label"
  local current=""
  local component
  local -a components
  IFS='/' read -r -a components <<<"${path#/}"
  for component in "${components[@]}"; do
    [[ -n "$component" ]] || continue
    current="${current}/${component}"
    [[ ! -L "$current" ]] || die "$label must not traverse a symbolic link: $current"
    if [[ -e "$current" ]]; then
      [[ -d "$current" ]] || die "$label has a non-directory component: $current"
    fi
  done
}

ensure_directory() {
  local path="$1"
  local mode="$2"
  local label="$3"
  validate_absolute_path "$path" "$label"
  local current=""
  local component
  local -a components
  IFS='/' read -r -a components <<<"${path#/}"
  for component in "${components[@]}"; do
    [[ -n "$component" ]] || continue
    current="${current}/${component}"
    [[ ! -L "$current" ]] || die "$label must not traverse a symbolic link: $current"
    if [[ -e "$current" ]]; then
      [[ -d "$current" ]] || die "$label has a non-directory component: $current"
      if [[ "$mode" == 700 && "$(stat -c '%a' -- "$current")" == 700 ]]; then
        [[ "$(stat -c '%u' -- "$current")" == "$(id -u)" ]] ||
          die "$label must not traverse another owner's private directory"
      fi
    else
      mkdir -- "$current"
      chmod "$mode" -- "$current"
    fi
  done
  [[ "$(stat -c '%a' -- "$path")" == "$mode" ]] ||
    die "$label must have mode 0$mode: $path"
}

assert_directory() {
  local path="$1"
  local label="$2"
  assert_no_symlink_components "$path" "$label"
  [[ -d "$path" ]] || die "$label is missing: $path"
}

assert_private_directory() {
  local path="$1"
  local label="$2"
  assert_directory "$path" "$label"
  [[ "$(stat -c '%a' -- "$path")" == "700" ]] ||
    die "$label must have mode 0700: $path"
  [[ "$(stat -c '%u' -- "$path")" == "$(id -u)" ]] ||
    die "$label must be owned by the executing user"
}

assert_regular_file() {
  local path="$1"
  local label="$2"
  if [[ "$path" == /* ]]; then
    assert_no_symlink_components "$(dirname "$path")" "$label parent"
  fi
  [[ ! -L "$path" && -f "$path" ]] || die "$label must be a regular non-symlink file: $path"
  [[ "$(stat -c '%h' -- "$path")" == "1" ]] || die "$label must not be hard linked: $path"
}

assert_private_file() {
  local path="$1"
  local label="$2"
  assert_regular_file "$path" "$label"
  [[ "$(stat -c '%a' -- "$path")" == "600" ]] || die "$label must have mode 0600: $path"
  [[ "$(stat -c '%u' -- "$path")" == "$(id -u)" ]] ||
    die "$label must be owned by the executing user"
}

# Only executable environment input needs this owner/write trust boundary.
# Published read-only release resources retain their existing public guards.
assert_trusted_environment_parent() {
  local path="$1" executing_uid current="" component owner mode
  local -a components
  executing_uid="$(id -u)"
  assert_no_symlink_components "$path" "environment parent"
  IFS='/' read -r -a components <<<"${path#/}"
  for component in "${components[@]}"; do
    [[ -n "$component" ]] || continue
    current="${current}/${component}"
    [[ -d "$current" && ! -L "$current" ]] || die "environment parent must be physical"
    owner="$(stat -c '%u' -- "$current")"
    mode="$(stat -c '%a' -- "$current")"
    [[ "$owner" == 0 || "$owner" == "$executing_uid" ]] ||
      die "environment parent must be owned by root or the executing user"
    if (( (8#$mode & 0022) != 0 )); then
      if [[ "$owner" != 0 || ( "$current" != /tmp && "$current" != /var/tmp ) ]] ||
        (( (8#$mode & 01000) == 0 )); then
        die "environment parent must exclude other writers"
      fi
    fi
  done
}

portable_relative_path() {
  local path="$1"
  [[ -n "$path" && "$path" != /* && "$path" != *'//'* ]] || return 1
  [[ "/$path/" != *'/../'* && "/$path/" != *'/./'* ]] || return 1
  [[ "$path" =~ ^[A-Za-z0-9._/-]+$ ]]
}

write_release_manifest() {
  local root="$1"
  local output="$2"
  assert_directory "$root" "release staging directory"
  assert_regular_file "$root/bin/xcos" "release Xcos binary"
  local temporary="${output}.tmp.$$"
  (
    umask 077
    {
      "$root/bin/xcos" release-manifest-header
      while IFS= read -r -d '' relative; do
        [[ "$relative" != "RELEASE-MANIFEST" ]] || continue
        portable_relative_path "$relative" || die "Release contains a non-portable path"
        local path="$root/$relative"
        [[ ! -L "$path" ]] || die "Release contains a symbolic link: $relative"
        local mode
        mode="$(stat -c '%a' -- "$path")"
        if [[ -d "$path" ]]; then
          echo "directory $mode $relative"
        elif [[ -f "$path" ]]; then
          [[ "$(stat -c '%h' -- "$path")" == "1" ]] ||
            die "Release contains a hard-linked file: $relative"
          local size
          local digest
          size="$(stat -c '%s' -- "$path")"
          digest="$(sha256sum -- "$path" | awk '{print $1}')"
          echo "file $mode $size $digest $relative"
        else
          die "Release contains a special file: $relative"
        fi
      done < <(find -P "$root" -mindepth 1 -printf '%P\0' | LC_ALL=C sort -z)
    } >"$temporary"
  )
  mv -T -- "$temporary" "$output"
}

verify_release() {
  local root="$1"
  assert_directory "$root" "release directory"
  [[ "$(stat -c '%a' -- "$root")" == "555" ]] || die "Release root must have mode 0555: $root"
  [[ -z "$(find -P "$root" -mindepth 1 -perm /222 -print -quit)" ]] ||
    die "Release content must not have writable permission bits: $root"

  local directory
  for directory in bin config deploy share; do
    assert_directory "$root/$directory" "release $directory directory"
    [[ "$(stat -c '%a' -- "$root/$directory")" == "555" ]] ||
      die "Release $directory directory must have mode 0555"
  done
  local executable
  for executable in \
    bin/xcos \
    bin/mediamtx \
    deploy/common.sh \
    deploy/xcosctl; do
    assert_regular_file "$root/$executable" "release executable $executable"
    [[ "$(stat -c '%a' -- "$root/$executable")" == "555" ]] ||
      die "Release executable must have mode 0555: $executable"
  done
  local action
  for action in \
    deploy/.bootstrap-action.sh \
    deploy/.start-action.sh \
    deploy/.status-action.sh \
    deploy/.stop-action.sh; do
    assert_regular_file "$root/$action" "release lifecycle module $action"
    [[ "$(stat -c '%a' -- "$root/$action")" == "444" ]] ||
      die "Release lifecycle module must have mode 0444: $action"
  done
  local configuration
  for configuration in config/mediamtx.yml config/mediamtx.lock; do
    assert_regular_file "$root/$configuration" "release configuration $configuration"
    [[ "$(stat -c '%a' -- "$root/$configuration")" == "444" ]] ||
      die "Release configuration must have mode 0444: $configuration"
  done

  local manifest="$root/RELEASE-MANIFEST"
  assert_regular_file "$manifest" "release manifest"
  [[ "$(stat -c '%a' -- "$manifest")" == "444" ]] || die "Release manifest must have mode 0444"
  local generated
  generated="$(mktemp)"
  if ! write_release_manifest "$root" "$generated"; then
    rm -f -- "$generated"
    return 1
  fi
  if ! cmp -s -- "$manifest" "$generated"; then
    rm -f -- "$generated"
    die "Release content does not match RELEASE-MANIFEST: $root"
  fi
  rm -f -- "$generated"
  "$root/bin/xcos" verify-release "$root" >/dev/null ||
    die "Release binary rejected its physical tree: $root"
}

resolve_release_context() {
  local script_path="$1"
  [[ "$script_path" == /* ]] || die "Operational scripts require their absolute physical path"
  validate_absolute_path "$script_path" "operational script path"
  local script_directory
  script_directory="$(cd "$(dirname "$script_path")" && pwd -P)"
  local physical_script
  physical_script="$script_directory/$(basename "$script_path")"
  [[ "$script_path" == "$physical_script" && ! -L "$script_path" ]] ||
    die "Operational scripts must be invoked through their physical release path"
  XCOS_RELEASE_ROOT="$(cd "$script_directory/.." && pwd -P)"
  [[ "$(basename "$XCOS_RELEASE_ROOT")" == "$XCOS_VERSION" ]] ||
    die "Operational scripts must run from releases/$XCOS_VERSION"
  local releases_root
  releases_root="$(dirname "$XCOS_RELEASE_ROOT")"
  [[ "$(basename "$releases_root")" == "releases" ]] ||
    die "Operational scripts must run from an immutable releases directory"
  XCOS_INSTALL_ROOT="$(dirname "$releases_root")"
  [[ "$XCOS_RELEASE_ROOT" == */opt/isarmg/xcos/releases/1.0.0 ]] ||
    die "Operational scripts must use the fixed Xcos 1.0.0 physical release suffix"
  validate_absolute_path "$XCOS_INSTALL_ROOT" "install root"
  if [[ -n "${XCOS_NATIVE_INSTALL_ROOT:-}" ]]; then
    validate_absolute_path "$XCOS_NATIVE_INSTALL_ROOT" "XCOS_NATIVE_INSTALL_ROOT"
    [[ "$XCOS_NATIVE_INSTALL_ROOT" == "$XCOS_INSTALL_ROOT" ]] ||
      die "XCOS_NATIVE_INSTALL_ROOT does not match the executing release"
  fi
  readonly XCOS_RELEASE_ROOT XCOS_INSTALL_ROOT
}

deployment_paths() {
  XCOS_CONFIG_DIR="${XCOS_NATIVE_CONFIG_DIR:-/etc/isarmg}"
  XCOS_STATE_DIR="${XCOS_NATIVE_STATE_DIR:-/var/lib/isarmg/xcos}"
  XCOS_RUNTIME_PATH="${XCOS_NATIVE_RUNTIME_DIR:-/run/isarmg/xcos}"
  validate_absolute_path "$XCOS_CONFIG_DIR" "XCOS_NATIVE_CONFIG_DIR"
  validate_absolute_path "$XCOS_STATE_DIR" "XCOS_NATIVE_STATE_DIR"
  validate_absolute_path "$XCOS_RUNTIME_PATH" "XCOS_NATIVE_RUNTIME_DIR"
  XCOS_ENV_FILE="$XCOS_CONFIG_DIR/xcos.env"
  # Consumed by the operational scripts that source this file.
  # shellcheck disable=SC2034
  XCOS_REVIEW_MARKER="$XCOS_CONFIG_DIR/xcos.REVIEW-SECRETS-BEFORE-START"
  readonly XCOS_CONFIG_DIR XCOS_STATE_DIR XCOS_RUNTIME_PATH
  # shellcheck disable=SC2034
  readonly XCOS_ENV_FILE XCOS_REVIEW_MARKER
}

load_deployment_env() {
  assert_directory "$XCOS_CONFIG_DIR" "Xcss configuration directory"
  [[ "$(stat -c '%a' -- "$XCOS_CONFIG_DIR")" == "755" ]] ||
    die "Xcss configuration directory must have mode 0755: $XCOS_CONFIG_DIR"
  assert_trusted_environment_parent "$XCOS_CONFIG_DIR"
  assert_private_file "$XCOS_ENV_FILE" "Xcos environment file"
  set -a
  # shellcheck disable=SC1090
  source "$XCOS_ENV_FILE"
  set +a
}

acquire_native_operation_lock() {
  assert_private_directory "$XCOS_RUNTIME_PATH" "Xcos runtime directory"
  local path="$XCOS_RUNTIME_PATH/operations.lock"
  ensure_lock_file "$path" "native operation lock"
  # File descriptor 9 is reserved by native operational scripts. Long-lived
  # child processes must close it explicitly before exec.
  exec 9<>"$path"
  flock --exclusive --nonblock 9 ||
    die "another Xcos start or stop operation is active"
}

require_runtime_contract() {
  [[ "${DATABASE_URL:-}" == "sqlite://$XCOS_STATE_DIR/db/app.db" ]] ||
    die "DATABASE_URL must target the selected state directory"
  [[ -z "${XCSS_DEV_WEB_DIR:-}" ]] || die "formal releases cannot override embedded Web assets"
  [[ "${MEDIAMTX_CONFIG:-}" == "$XCOS_RELEASE_ROOT/config/mediamtx.yml" ]] ||
    die "MEDIAMTX_CONFIG must target the executing immutable release"
  [[ "${MEDIAMTX_CONTRACT:-}" == "$XCOS_RELEASE_ROOT/config/mediamtx.lock" ]] ||
    die "MEDIAMTX_CONTRACT must target the executing immutable release"
  [[ "${MEDIAMTX_BINARY:-}" == "$XCOS_RELEASE_ROOT/bin/mediamtx" ]] ||
    die "MEDIAMTX_BINARY must target the executing immutable release"
  [[ "${RECORDINGS_DIR:-}" == "$XCOS_STATE_DIR/recordings" ]] ||
    die "RECORDINGS_DIR must target the selected state directory"
  [[ "${XCOS_RUNTIME_DIR:-}" == "$XCOS_RUNTIME_PATH" ]] ||
    die "XCOS_RUNTIME_DIR must target the configured runtime directory"
  [[ -n "${APP_JWT_SECRET:-}" && ${#APP_JWT_SECRET} -ge 32 ]] ||
    die "APP_JWT_SECRET must contain at least 32 characters"
  [[ -n "${CREDENTIALS_KEY:-}" ]] || die "CREDENTIALS_KEY is required"
  [[ "${PUBLIC_RTSP_PUBLISH_BASE_URL:-}" =~ ^rtsps://[^/[:space:]]+/?$ ]] ||
    die "PUBLIC_RTSP_PUBLISH_BASE_URL must be an rtsps origin reachable by paired clients"
  [[ "$PUBLIC_RTSP_PUBLISH_BASE_URL" != *'@'* ]] ||
    die "PUBLIC_RTSP_PUBLISH_BASE_URL must not contain credentials"
  local rtsp_authority="${PUBLIC_RTSP_PUBLISH_BASE_URL#*://}"
  rtsp_authority="${rtsp_authority%/}"
  case "$rtsp_authority" in
    127.*|0.0.0.0|0.0.0.0:*|localhost|localhost:*|\[::1\]|\[::1\]:*|\[::\]|\[::\]:*)
      die "PUBLIC_RTSP_PUBLISH_BASE_URL must not use a loopback host in production"
      ;;
  esac
  validate_absolute_path "${MEDIAMTX_RTSP_CERT:-}" "MEDIAMTX_RTSP_CERT"
  validate_absolute_path "${MEDIAMTX_RTSP_KEY:-}" "MEDIAMTX_RTSP_KEY"
  assert_regular_file "$MEDIAMTX_RTSP_CERT" "MediaMTX RTSPS certificate"
  assert_private_file "$MEDIAMTX_RTSP_KEY" "MediaMTX RTSPS private key"
}

remove_bootstrap_password() {
  assert_private_file "$XCOS_ENV_FILE" "Xcos environment file"
  grep -q '^BOOTSTRAP_ADMIN_PASSWORD=' "$XCOS_ENV_FILE" || return 0
  local temporary
  temporary="$(mktemp "$XCOS_CONFIG_DIR/.xcos.env.XXXXXX")"
  (umask 077; awk '!/^BOOTSTRAP_ADMIN_PASSWORD=/' "$XCOS_ENV_FILE" >"$temporary")
  chmod 600 -- "$temporary"
  assert_private_file "$temporary" "temporary Xcos environment file"
  mv -T -- "$temporary" "$XCOS_ENV_FILE"
  unset BOOTSTRAP_ADMIN_PASSWORD
}

ensure_lock_file() {
  local path="$1"
  local label="$2"
  if [[ -e "$path" || -L "$path" ]]; then
    assert_private_file "$path" "$label"
    return
  fi
  (umask 077; set -o noclobber; : >"$path") || die "Failed to create $label"
  chmod 600 -- "$path"
  assert_private_file "$path" "$label"
}

process_is_live() {
  local pid="$1"
  kill -0 "$pid" 2>/dev/null || return 1
  if [[ -r "/proc/$pid/status" ]]; then
    local process_state
    process_state="$(awk '$1 == "State:" { print $2; exit }' "/proc/$pid/status" 2>/dev/null)" || return 1
    [[ -n "$process_state" && "$process_state" != "Z" && "$process_state" != "X" ]] || return 1
  fi
  return 0
}

read_running_pid() {
  local path="$1"
  local expected_binary="${2:-}"
  if [[ ! -e "$path" && ! -L "$path" ]]; then
    return 1
  fi
  (assert_private_file "$path" "PID file") || return 2
  local pid
  pid="$(<"$path")"
  if [[ ! "$pid" =~ ^[1-9][0-9]*$ ]]; then
    echo "PID file is invalid: $path" >&2
    return 2
  fi
  process_is_live "$pid" || return 1
  if [[ -n "$expected_binary" && -e "/proc/$pid/exe" ]]; then
    local actual
    actual="$(readlink -f -- "/proc/$pid/exe")" || return 2
    if [[ "$actual" != "$expected_binary" ]]; then
      local matches_script=false
      local argument
      while IFS= read -r -d '' argument; do
        if [[ "$argument" == "$expected_binary" ]]; then
          matches_script=true
        fi
      done <"/proc/$pid/cmdline"
      if [[ "$matches_script" != true ]]; then
        echo "PID $pid does not belong to the expected release binary" >&2
        return 2
      fi
    fi
  fi
  echo "$pid"
}

read_optional_running_pid() {
  local pid status
  if pid="$(read_running_pid "$@")"; then
    printf '%s\n' "$pid"
  else
    status=$?
    [[ "$status" == 1 ]] || die "Could not validate the service PID file"
  fi
}

write_pid_file() {
  local path="$1"
  local pid="$2"
  if [[ -e "$path" || -L "$path" ]]; then
    assert_private_file "$path" "PID file"
  fi
  local temporary
  temporary="$(mktemp "${path}.tmp.XXXXXX")"
  (umask 077; printf '%s\n' "$pid" >"$temporary")
  chmod 600 -- "$temporary"
  assert_private_file "$temporary" "temporary PID file"
  mv -T -- "$temporary" "$path"
}
