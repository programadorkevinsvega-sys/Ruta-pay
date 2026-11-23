#!/usr/bin/env python3

import os
import pathlib
import re
import subprocess
import sys
import tempfile


def fail(message):
    raise SystemExit(message)


def run(command, *, capture=False):
    return subprocess.run(
        command,
        check=True,
        stdout=subprocess.PIPE if capture else subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        text=capture,
    )


def atomic_install(path, content, mode, uid, gid):
    path = pathlib.Path(path)
    if not path.parent.is_dir() or path.is_symlink():
        fail(f"Refusing unsafe certificate destination: {path}")
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


def reload_service(mode, container_name):
    if mode == "redis" and pathlib.Path(
        "/etc/payvia/staging/redis/rotation-credentials/acl-activation-pending"
    ).exists():
        print(
            "Installed the validated Redis certificate; hot reload is waiting "
            "for the one-time ACL activation."
        )
        return

    inspect = subprocess.run(
        [
            "docker",
            "inspect",
            "--format",
            "{{.State.Running}}",
            container_name,
        ],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        timeout=20,
    )
    if inspect.returncode != 0:
        detail = inspect.stderr.strip()
        fail(
            f"Could not inspect the {mode} container before certificate reload"
            f"{': ' + detail if detail else '.'}"
        )
    if inspect.stdout.strip() != "true":
        print(f"Installed the validated certificate; {mode} is not running, reload skipped.")
        return

    if mode == "postgres":
        command = [
            "docker",
            "exec",
            "--user",
            "postgres",
            container_name,
            "sh",
            "-ec",
            'pg_ctl reload -D "$PGDATA"; pg_isready -q -U "$POSTGRES_USER" -d "$POSTGRES_DB"',
        ]
    elif mode == "redis":
        command = [
            "docker",
            "exec",
            container_name,
            "sh",
            "-ec",
            """
password_file=/run/payvia/staging/redis/rotation/tls-rotator-password
acl_marker=/run/payvia/staging/redis/rotation/acl-activation-pending
if [ -e "$acl_marker" ]; then
  printf 'REDIS_ACL_ACTIVATION_PENDING'
  exit 0
fi
test -s "$password_file"
password="$(cat "$password_file")"
printf '%s\\n' "$password" | grep -Eq '^[0-9a-f]{64}$'
result="$(REDISCLI_AUTH="$password" redis-cli --raw --tls --cacert /run/payvia/staging/redis/server-ca/ca.pem --cert /run/payvia/staging/redis/healthcheck/redis-healthcheck.crt --key /run/payvia/staging/redis/healthcheck/redis-healthcheck.key --user payvia_tls_rotator -h redis CONFIG SET tls-cert-file /run/payvia/staging/redis/tls/server.crt tls-key-file /run/payvia/staging/redis/tls/server.key)"
test "$result" = OK
unset password
healthcheck_password="$(cat /run/payvia/staging/redis/healthcheck-credentials/healthcheck-password)"
result="$(REDISCLI_AUTH="$healthcheck_password" redis-cli --raw --tls --cacert /run/payvia/staging/redis/server-ca/ca.pem --cert /run/payvia/staging/redis/healthcheck/redis-healthcheck.crt --key /run/payvia/staging/redis/healthcheck/redis-healthcheck.key --user payvia_healthcheck -h redis PING)"
test "$result" = PONG
unset healthcheck_password
""".strip(),
        ]
    elif mode == "nats":
        command = ["docker", "kill", "--signal=HUP", container_name]
    else:
        fail(f"Unsupported certificate reload mode: {mode}")

    result = subprocess.run(
        command,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        timeout=30,
    )
    if result.returncode != 0:
        detail = result.stderr.strip()
        fail(
            f"Could not hot-reload the {mode} certificate"
            f"{': ' + detail if detail else '.'}"
        )
    if mode == "nats":
        health = subprocess.run(
            [
                "docker",
                "exec",
                container_name,
                "sh",
                "-ec",
                """
attempt=0
while [ "$attempt" -lt 10 ]; do
  if wget -qO- http://127.0.0.1:8222/healthz | grep -q ok; then
    exit 0
  fi
  attempt=$((attempt + 1))
  sleep 1
done
exit 1
""".strip(),
            ],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=15,
        )
        if health.returncode != 0:
            fail("NATS did not return healthy after the certificate reload signal.")
    print(f"Installed and hot-reloaded the validated certificate for {mode}.")


def main():
    if len(sys.argv) != 12:
        fail(
            "Usage: payvia-install-pki-certificate.py <bundle> <certificate> "
            "<private-key> <root-ca> <common-name> <dns-sans-or-dash> "
            "<server|client> <uid> <gid> <none|postgres|redis|nats> <container-name>"
        )

    (
        bundle_name,
        certificate_name,
        private_key_name,
        root_ca_name,
        common_name,
        dns_sans,
        profile,
        uid_text,
        gid_text,
        reload_mode,
        container_name,
    ) = sys.argv[1:]
    if profile not in ("server", "client") or reload_mode not in (
        "none",
        "postgres",
        "redis",
        "nats",
    ):
        fail("Invalid certificate profile or reload mode.")
    if reload_mode != "none" and profile != "server":
        fail("Only server certificates may request a service reload.")
    uid, gid = int(uid_text), int(gid_text)
    if reload_mode != "none" and not re.fullmatch(
        r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}", container_name
    ):
        fail("Invalid service container name.")
    bundle_path = pathlib.Path(bundle_name)
    root_ca = pathlib.Path(root_ca_name)
    if bundle_path.is_symlink() or not bundle_path.is_file():
        fail("Rendered certificate bundle is missing or unsafe.")
    if root_ca.is_symlink() or not root_ca.is_file():
        fail("Trusted root certificate is missing or unsafe.")

    bundle = bundle_path.read_bytes()
    certificate_blocks = re.findall(
        rb"-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----",
        bundle,
        re.DOTALL,
    )
    private_key_blocks = re.findall(
        rb"-----BEGIN (?:EC |RSA |ENCRYPTED )?PRIVATE KEY-----.*?"
        rb"-----END (?:EC |RSA |ENCRYPTED )?PRIVATE KEY-----",
        bundle,
        re.DOTALL,
    )
    if len(certificate_blocks) < 2 or len(private_key_blocks) != 1:
        fail("Rendered bundle must contain one leaf key and its issuing chain.")

    certificate = b"\n".join(certificate_blocks) + b"\n"
    private_key = private_key_blocks[0] + b"\n"
    chain = b"\n".join(certificate_blocks[1:]) + b"\n"
    with tempfile.TemporaryDirectory(prefix="payvia-pki-install-") as directory:
        temporary = pathlib.Path(directory)
        leaf_path = temporary / "leaf.pem"
        chain_path = temporary / "chain.pem"
        key_path = temporary / "key.pem"
        leaf_path.write_bytes(certificate_blocks[0] + b"\n")
        chain_path.write_bytes(chain)
        key_path.write_bytes(private_key)
        subject = run(
            [
                "openssl",
                "x509",
                "-in",
                str(leaf_path),
                "-noout",
                "-subject",
                "-nameopt",
                "RFC2253",
            ],
            capture=True,
        ).stdout.strip()
        if subject != f"subject=CN={common_name}":
            fail("Issued certificate common name does not match the expected identity.")

        extension_usage = run(
            ["openssl", "x509", "-in", str(leaf_path), "-noout", "-ext", "extendedKeyUsage"],
            capture=True,
        ).stdout
        expected_usage = (
            "TLS Web Server Authentication"
            if profile == "server"
            else "TLS Web Client Authentication"
        )
        if expected_usage not in extension_usage:
            fail("Issued certificate EKU does not match the expected profile.")

        subject_alt_names = run(
            ["openssl", "x509", "-in", str(leaf_path), "-noout", "-ext", "subjectAltName"],
            capture=True,
        ).stdout
        expected_sans = [] if dns_sans == "-" else dns_sans.split(",")
        for expected_san in expected_sans:
            if f"DNS:{expected_san}" not in subject_alt_names:
                fail("Issued certificate is missing a required DNS SAN.")
        if "IP Address:" in subject_alt_names:
            fail("Issued certificate contains a prohibited IP SAN.")

        run(
            [
                "openssl",
                "verify",
                "-CAfile",
                str(root_ca),
                "-untrusted",
                str(chain_path),
                str(leaf_path),
            ]
        )
        certificate_public_key = run(
            ["openssl", "x509", "-in", str(leaf_path), "-pubkey", "-noout"],
            capture=True,
        ).stdout
        private_public_key = run(
            ["openssl", "pkey", "-in", str(key_path), "-pubout"],
            capture=True,
        ).stdout
        if certificate_public_key != private_public_key:
            fail("Issued certificate and private key do not match.")

    atomic_install(certificate_name, certificate, 0o444, 0, 0)
    atomic_install(private_key_name, private_key, 0o400, uid, gid)
    if reload_mode != "none":
        reload_service(reload_mode, container_name)
    else:
        print(f"Installed validated certificate for {common_name}.")


if __name__ == "__main__":
    main()
