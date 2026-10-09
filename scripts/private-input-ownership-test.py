#!/usr/bin/env python3
"""Exercise native environment input under root and a real service UID."""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


COMMON = Path(__file__).resolve().parents[1] / "deploy" / "common.sh"
SERVICE_UID = 65534


def capture(root):
    result = {}
    for path in [root, *sorted(root.rglob("*"))]:
        item = path.lstat()
        result[str(path.relative_to(root))] = (
            item.st_dev, item.st_ino, item.st_mode, item.st_uid, item.st_gid,
            item.st_nlink,
            hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else None,
        )
    return result


def run(script, *paths, uid=0):
    def credentials():
        os.setgroups([])
        os.setgid(uid)
        os.setuid(uid)
    return subprocess.run(
        ["/bin/bash", "-euc", 'source "$1"; shift; ' + script, "ownership-test", str(COMMON), *map(str, paths)],
        env={"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"},
        capture_output=True, text=True, timeout=10,
        preexec_fn=credentials if uid else None,
    )


def main():
    global COMMON
    if os.geteuid() != 0:
        raise SystemExit("this permission boundary test requires root")
    root = Path(tempfile.mkdtemp(prefix="xcos-native-owner-"))
    root.chmod(0o755)  # The real service process may traverse this root-owned fixture.
    checked_common = root / "common.sh"
    shutil.copyfile(COMMON, checked_common)
    checked_common.chmod(0o555)
    COMMON = checked_common
    try:
        config = root / "config"
        config.mkdir(mode=0o755)
        marker = root / "must-not-execute"
        environment = config / "xcos.env"
        environment.write_text(f'printf executed > "{marker}"\n')
        environment.chmod(0o600)
        os.chown(environment, SERVICE_UID, SERVICE_UID)
        command = 'XCOS_CONFIG_DIR="$1"; XCOS_ENV_FILE="$2"; load_deployment_env'
        before = capture(root)
        rejected = run(command, config, environment)
        assert rejected.returncode != 0 and not marker.exists()
        assert "owned by the executing user" in rejected.stderr
        assert capture(root) == before
        print("root refuses foreign-owned 0600 environment before execution or writes")

        os.chown(environment, 0, 0)
        os.chown(config, SERVICE_UID, SERVICE_UID)
        before = capture(root)
        rejected = run(command, config, environment)
        assert rejected.returncode != 0 and not marker.exists()
        assert "environment parent" in rejected.stderr
        assert capture(root) == before
        print("root refuses foreign-owned 0755 config parent with root-owned 0600 environment")
        os.chown(config, 0, 0)

        for name, owner, mode in (
            ("foreign-ancestor", SERVICE_UID, 0o755),
            ("writable-ancestor", 0, 0o777),
            ("unapproved-sticky-ancestor", 0, 0o1777),
        ):
            ancestor = root / name
            ancestor.mkdir(mode=mode)
            ancestor.chmod(mode)
            os.chown(ancestor, owner, owner)
            protected_config = ancestor / "config"
            protected_config.mkdir(mode=0o755)
            protected_environment = protected_config / "xcos.env"
            protected_environment.write_text(f'printf executed > "{marker}"\n')
            protected_environment.chmod(0o600)
            before = capture(root)
            rejected = run(command, protected_config, protected_environment)
            assert rejected.returncode != 0 and not marker.exists()
            assert "environment parent" in rejected.stderr
            assert capture(root) == before
        print("root refuses foreign, writable and nonstandard sticky ancestors before execution or writes")

        directory = root / "private-service-state"
        directory.mkdir(mode=0o700)
        os.chown(directory, SERVICE_UID, SERVICE_UID)
        before = capture(root)
        rejected = run('assert_private_directory "$1" private-state', directory)
        assert rejected.returncode != 0 and capture(root) == before
        rejected = run('ensure_directory "$1/db" 700 database', directory)
        assert rejected.returncode != 0 and not (directory / "db").exists()
        assert capture(root) == before
        print("root refuses foreign-owned 0700 state and creates no descendant")

        # A trusted parent still cannot be writable by another group or user.
        config.chmod(0o777)
        before = capture(root)
        rejected = run(command, config, environment)
        assert rejected.returncode != 0 and not marker.exists()
        assert capture(root) == before
        config.chmod(0o755)
        print("root refuses writable environment parents before execution or writes")

        # The ordinary owner can source its own input and create private state;
        # the immutable, root-owned common script remains readable/executable.
        environment.write_text("NATIVE_OWNED_INPUT=accepted\n")
        os.chown(environment, SERVICE_UID, SERVICE_UID)
        accepted = run(command + '; [[ "$NATIVE_OWNED_INPUT" == accepted ]]; '
                       'assert_private_directory "$3" private-state; '
                       'ensure_directory "$3/db" 700 database',
                       config, environment, directory, uid=SERVICE_UID)
        assert accepted.returncode == 0, accepted.stderr
        metadata = (directory / "db").stat()
        assert metadata.st_uid == SERVICE_UID and metadata.st_mode & 0o777 == 0o700
        print("actual UID65534 accepts its own private input and state under trusted root-owned parent")
    finally:
        shutil.rmtree(root)


if __name__ == "__main__":
    main()
