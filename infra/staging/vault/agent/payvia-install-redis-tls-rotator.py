#!/usr/bin/env python3

import hashlib
import os
import pathlib
import re
import sys
import tempfile


ROTATOR_USER = "payvia_tls_rotator"
ROTATOR_ACL = (
    "user payvia_tls_rotator on #{password_hash} resetkeys resetchannels "
    "-@all +config|set +ping\n"
)


def fail(message):
    raise SystemExit(message)


def atomic_install(path, content, mode, uid, gid):
    path = pathlib.Path(path)
    if not path.parent.is_dir() or path.is_symlink():
        fail(f"Refusing unsafe Redis credential destination: {path}")
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", dir=path.parent
    )
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(content)
            output.flush()
            os.fsync(output.fileno())
            os.fchown(output.fileno(), uid, gid)
            os.fchmod(output.fileno(), mode)
        os.replace(temporary_name, path)
        directory_descriptor = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory_descriptor)
        finally:
            os.close(directory_descriptor)
    except Exception:
        try:
            os.unlink(temporary_name)
        except FileNotFoundError:
            pass
        raise


def main():
    if len(sys.argv) != 4:
        fail(
            "Usage: payvia-install-redis-tls-rotator.py "
            "<rendered-secret> <credential-file> <acl-file>"
        )

    rendered_secret, credential_file, acl_file = map(pathlib.Path, sys.argv[1:])
    if rendered_secret.is_symlink() or not rendered_secret.is_file():
        fail("Vault-rendered Redis TLS-rotator secret is missing or unsafe.")

    password = rendered_secret.read_bytes().strip()
    if not re.fullmatch(rb"[0-9a-f]{64}", password):
        fail("Vault-rendered Redis TLS-rotator secret has an unexpected format.")

    password_hash = hashlib.sha256(password).hexdigest()
    acl_entry = ROTATOR_ACL.format(password_hash=password_hash).encode()
    credential_file.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
    if credential_file.parent.is_symlink():
        fail("Refusing unsafe Redis credential directory.")
    os.chown(credential_file.parent, 0, 0)
    os.chmod(credential_file.parent, 0o755)
    atomic_install(credential_file, password + b"\n", 0o400, 999, 1000)
    if acl_file.is_symlink():
        fail("Redis ACL file is unsafe.")
    if not acl_file.exists():
        print(
            "Installed Redis TLS-rotator credential; Redis ACL is not prepared yet. "
            "Run the Redis host preparation before starting the service."
        )
        return
    if not acl_file.is_file():
        fail("Redis ACL path is not a regular file.")

    existing_acl = acl_file.read_bytes()
    matching_entries = [
        line
        for line in existing_acl.splitlines()
        if line.startswith(b"user " + ROTATOR_USER.encode() + b" ")
    ]
    if matching_entries and matching_entries != [acl_entry.rstrip(b"\n")]:
        fail(
            "Redis ACL already contains a different TLS-rotator identity; "
            "inspect it before changing credentials."
        )
    if matching_entries:
        print("Redis TLS-rotator credential and ACL are already current.")
        return

    acl_mode = acl_file.stat().st_mode & 0o777
    acl_uid = acl_file.stat().st_uid
    acl_gid = acl_file.stat().st_gid
    separator = b"" if not existing_acl or existing_acl.endswith(b"\n") else b"\n"
    atomic_install(
        acl_file,
        existing_acl + separator + acl_entry,
        acl_mode,
        acl_uid,
        acl_gid,
    )
    marker = credential_file.parent / "acl-activation-pending"
    atomic_install(
        marker,
        b"Redis TLS-rotator ACL activation is pending.\n",
        0o444,
        0,
        0,
    )
    print(
        "Installed Redis TLS-rotator credential and staged its ACL entry; "
        "the Redis ACL must be activated once before certificate hot reload."
    )


if __name__ == "__main__":
    main()
