#!/usr/bin/env bash
set -euo pipefail

SOURCE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
# shellcheck source=/dev/null
source "$SOURCE_ROOT/deploy/common.sh"
OUTPUT="${1:?usage: package-release.sh ABSOLUTE_OUTPUT_DIRECTORY}"
validate_absolute_path "$OUTPUT" "output directory"
assert_directory "$OUTPUT" "output directory"
[[ "$OUTPUT" != "$SOURCE_ROOT" && "$OUTPUT" != "$SOURCE_ROOT/"* ]] || die "output must be outside the checkout"
[[ "$(stat -c '%u' -- "$OUTPUT")" == "$(id -u)" ]] || die "output must be owned by the packager"
[[ -z "$(find -P "$OUTPUT" -maxdepth 0 -perm /022 -print)" ]] || die "output must not be group/world writable"
ARCHIVE="xcos-$XCOS_VERSION-x86_64-unknown-linux-gnu.tar.gz"
[[ ! -e "$OUTPUT/$ARCHIVE" && ! -L "$OUTPUT/$ARCHIVE" && ! -e "$OUTPUT/SHA256SUMS" && ! -L "$OUTPUT/SHA256SUMS" ]] || die "refusing to overwrite release assets"
TEMPORARY="$(mktemp -d "$OUTPUT/.xcos-package.XXXXXXXX")"
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
XCOS_NATIVE_INSTALL_ROOT="$TEMPORARY/archive/opt/isarmg/xcos" \
  XCOS_BUILD_TARGET="${XCOS_BUILD_TARGET:-$TEMPORARY/build}" \
  XCOS_MEDIAMTX_SOURCE="$TEMPORARY/mediamtx" \
  bash "$SOURCE_ROOT/scripts/build.sh"
release_root="$TEMPORARY/archive/opt/isarmg/xcos/releases/$XCOS_VERSION"
verify_release "$release_root"
install -m 0444 "$TEMPORARY/LICENSE" "$TEMPORARY/archive/licenses/MediaMTX-LICENSE"
for name in OFL.txt CJK-LICENSE.txt NORMAL-LICENSE.txt; do
  install -m 0444 "$SOURCE_ROOT/web/node_modules/@xcss/web-fonts/dist/$name" "$TEMPORARY/archive/licenses/$name"
done
install -m 0444 "$SOURCE_ROOT/docs/releases/$XCOS_VERSION.md" "$TEMPORARY/archive/README.md"
epoch="$(git -C "$SOURCE_ROOT" show -s --format=%ct HEAD)"
tar --sort=name --mtime="@$epoch" --owner=0 --group=0 --numeric-owner \
  -C "$TEMPORARY/archive" -czf "$TEMPORARY/$ARCHIVE" README.md licenses opt
mkdir "$TEMPORARY/extracted"
tar -xzf "$TEMPORARY/$ARCHIVE" -C "$TEMPORARY/extracted"
verify_release "$TEMPORARY/extracted/opt/isarmg/xcos/releases/$XCOS_VERSION"
python3 - "$TEMPORARY" "$OUTPUT" "$ARCHIVE" <<'PUBLISH'
import hashlib, os, stat, sys

def physical_directory(path):
    if not os.path.isabs(path) or os.path.normpath(path) != path:
        raise ValueError('directory must be an absolute normalized path')
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
    descriptor = os.open('/', flags)
    try:
        for name in path.split('/')[1:]:
            if not name: continue
            next_descriptor = os.open(name, flags, dir_fd=descriptor)
            os.close(descriptor)
            descriptor = next_descriptor
        metadata = os.fstat(descriptor)
        if metadata.st_uid != os.geteuid() or metadata.st_mode & 0o022:
            raise ValueError('publication directory must be owned and private from other writers')
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise

staging = physical_directory(sys.argv[1])
output = physical_directory(sys.argv[2])
archive = sys.argv[3]
created = []
try:
    descriptor = os.open(archive, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=staging)
    with os.fdopen(descriptor, 'rb') as stream:
        metadata = os.fstat(stream.fileno())
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1 or metadata.st_uid != os.geteuid():
            raise ValueError('staged archive must be one owned regular file')
        digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        os.fsync(stream.fileno())
    descriptor = os.open('SHA256SUMS', os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=staging)
    with os.fdopen(descriptor, 'w', encoding='ascii') as stream:
        stream.write(digest + '  ' + archive + '\n')
        stream.flush()
        os.fsync(stream.fileno())
    os.fsync(staging)
    # The complete archive is published last. link is atomic no-clobber even
    # if another invocation races these already-verified destination names.
    for name in ['SHA256SUMS', archive]:
        identity = os.stat(name, dir_fd=staging, follow_symlinks=False)
        os.link(name, name, src_dir_fd=staging, dst_dir_fd=output, follow_symlinks=False)
        created.append((name, identity.st_dev, identity.st_ino))
    os.fsync(output)
except BaseException:
    # Roll back only links published by this invocation, never a pre-existing
    # archive/checksum or a replacement created by another process.
    for name, device, inode in reversed(created):
        try:
            current = os.stat(name, dir_fd=output, follow_symlinks=False)
            if (current.st_dev, current.st_ino) == (device, inode):
                os.unlink(name, dir_fd=output)
        except FileNotFoundError:
            pass
    os.fsync(output)
    raise
finally:
    os.close(staging)
    os.close(output)
PUBLISH
printf 'Verified immutable release archive: %s\n' "$OUTPUT/$ARCHIVE"
