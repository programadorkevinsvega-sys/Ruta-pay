#!/usr/bin/env bash
set -euo pipefail

target_dir="${1:-/etc/payvia/staging/redis}"

if (( EUID != 0 )); then
  printf 'Run this script as root to protect credentials and assign container ownership.\n' >&2
  exit 1
fi

install -d -m 0755 \
  "$target_dir" \
  "$target_dir/tls" \
  "$target_dir/client-ca" \
  "$target_dir/server-ca" \
  "$target_dir/acl" \
  "$target_dir/rotation-credentials" \
  "$target_dir/healthcheck-client" \
  "$target_dir/exporter-client"
install -d -m 0700 "$target_dir/application-credentials"
install -d -m 0700 "$target_dir/acceptance-credentials"
install -d -m 0711 "$target_dir/healthcheck-credentials" "$target_dir/exporter-credentials"

platform_password="$target_dir/application-credentials/platform-runtime-password"
if [[ ! -f "$platform_password" || -L "$platform_password" ]] ||
  [[ "$(stat -c '%u:%g:%a' "$platform_password")" != '0:0:400' ]]; then
  printf 'Vault Agent must render the protected platform-runtime password before Redis preparation.\n' >&2
  exit 1
fi
rotation_password="$target_dir/rotation-credentials/tls-rotator-password"
if [[ ! -f "$rotation_password" || -L "$rotation_password" ]] ||
  [[ "$(stat -c '%u:%g:%a' "$rotation_password")" != '999:1000:400' ]]; then
  printf 'Vault Agent must render the protected Redis TLS-rotator password before Redis preparation.\n' >&2
  exit 1
fi
if ! grep -Eq '^[0-9a-f]{64}$' "$platform_password"; then
  printf 'Vault-rendered platform-runtime password has an unexpected format.\n' >&2
  exit 1
fi
if ! grep -Eq '^[0-9a-f]{64}$' "$rotation_password"; then
  printf 'Vault-rendered Redis TLS-rotator password has an unexpected format.\n' >&2
  exit 1
fi
acceptance_password="$target_dir/acceptance-credentials/phase4-acceptance-password"
if [[ -e "$acceptance_password" ]]; then
  if [[ ! -f "$acceptance_password" || -L "$acceptance_password" ]] ||
    [[ "$(stat -c '%u:%g:%a' "$acceptance_password")" != '0:0:400' ]] ||
    ! grep -Eq '^[0-9a-f]{64}$' "$acceptance_password"; then
    printf 'Vault-rendered phase-four acceptance password has an unexpected format or permissions.\n' >&2
    exit 1
  fi
fi

acceptance_acl_line=""
if [[ -f "$acceptance_password" ]]; then
  acceptance_hash="$(tr -d '\n' <"$acceptance_password" | openssl dgst -sha256 | awk '{print $NF}')"
  acceptance_acl_line="user phase4_acceptance on #${acceptance_hash} resetkeys resetchannels &payvia:platform:realtime:v1 +ping +hello +client|setinfo +publish +subscribe"
fi

outputs=(
  "$target_dir/acl/users.acl"
  "$target_dir/healthcheck-credentials/healthcheck-password"
  "$target_dir/exporter-credentials/exporter-passwords.json"
)

existing_outputs=0
for output in "${outputs[@]}"; do
  if [[ -e "$output" ]]; then
    existing_outputs=$((existing_outputs + 1))
  fi
done

if (( existing_outputs == ${#outputs[@]} )); then
  if [[ "$(stat -c '%u:%g:%a' "${outputs[0]}")" != '0:0:444' ||
    "$(stat -c '%u:%g:%a' "${outputs[1]}")" != '999:1000:400' ||
    "$(stat -c '%u:%g:%a' "${outputs[2]}")" != '59000:59000:400' ]]; then
    printf 'Existing Redis files have unexpected ownership or mode; inspect them before continuing.\n' >&2
    exit 1
  fi
  python3 - "${outputs[2]}" <<'PY'
import json
import pathlib
import sys

passwords = json.loads(pathlib.Path(sys.argv[1]).read_text())
if set(passwords) != {"rediss://payvia_exporter@redis:6379"} or not all(
    isinstance(password, str) and password for password in passwords.values()
):
    raise SystemExit("Existing exporter password file has an unexpected format.")
PY
  rotation_hash="$(tr -d '\n' <"$rotation_password" | openssl dgst -sha256 | awk '{print $NF}')"
  expected_rotator="user payvia_tls_rotator on #${rotation_hash} resetkeys resetchannels -@all +config|set +ping"
  if ! grep -Fxq "$expected_rotator" "${outputs[0]}"; then
    printf '%s\n' "Redis TLS-rotator ACL is missing or differs; inspect it before continuing."
    exit 1
  fi
  if [[ -n "$acceptance_acl_line" ]] && ! grep -Fxq "$acceptance_acl_line" "${outputs[0]}"; then
    if grep -Eq '^user phase4_acceptance ' "${outputs[0]}"; then
      printf '%s\n' "A different phase4_acceptance ACL already exists; inspect it before continuing." >&2
      exit 1
    fi
    temporary_acl="$(mktemp "${target_dir}/acl/.users.XXXXXX")"
    trap 'rm -f -- "$temporary_acl"' EXIT
    cat "${outputs[0]}" >"$temporary_acl"
    printf '%s\n' "$acceptance_acl_line" >>"$temporary_acl"
    chown 0:0 "$temporary_acl"
    chmod 0444 "$temporary_acl"
    mv -- "$temporary_acl" "${outputs[0]}"
    trap - EXIT
    printf '%s\n' "Added the isolated phase-four Redis ACL; restart Redis once to load it."
    exit 0
  fi
  printf 'Redis healthcheck/exporter credentials and ACL already exist; no changes made.\n'
  exit 0
fi
if (( existing_outputs > 0 )); then
  printf 'Partial Redis credential/ACL set exists; inspect it before continuing.\n' >&2
  exit 1
fi

temporary_dir="$(mktemp -d "$target_dir/.prepare.XXXXXX")"
trap 'rm -rf -- "$temporary_dir"' EXIT
umask 077

openssl rand -hex 32 | tr -d '\n' > "$temporary_dir/healthcheck-password"
openssl rand -hex 32 | tr -d '\n' > "$temporary_dir/exporter-password"

platform_hash="$(openssl dgst -sha256 "$platform_password" | awk '{print $NF}')"
healthcheck_hash="$(openssl dgst -sha256 "$temporary_dir/healthcheck-password" | awk '{print $NF}')"
exporter_hash="$(openssl dgst -sha256 "$temporary_dir/exporter-password" | awk '{print $NF}')"
rotation_hash="$(tr -d '\n' <"$rotation_password" | openssl dgst -sha256 | awk '{print $NF}')"

python3 - "$temporary_dir/exporter-password" "$temporary_dir/exporter-passwords.json" <<'PY'
import json
import pathlib
import sys

password = pathlib.Path(sys.argv[1]).read_text()
with pathlib.Path(sys.argv[2]).open("x") as password_file:
    json.dump(
        {"rediss://payvia_exporter@redis:6379": password},
        password_file,
        separators=(",", ":"),
    )
PY

cat > "$temporary_dir/users.acl" <<EOF
user default off resetkeys resetchannels nocommands
user platform-runtime on #${platform_hash} resetkeys ~platform:identity:* resetchannels &payvia:platform:realtime:v1 +ping +hello +client|setinfo +eval +incr +expire +publish +subscribe
user payvia_healthcheck on #${healthcheck_hash} resetkeys resetchannels +ping
user payvia_exporter on #${exporter_hash} resetkeys resetchannels +ping +info +@read +config|get +client|list +slowlog|get +command|info +memory|stats +latency|latest
user payvia_tls_rotator on #${rotation_hash} resetkeys resetchannels -@all +config|set +ping
EOF
if [[ -n "$acceptance_acl_line" ]]; then
  printf '%s\n' "$acceptance_acl_line" >>"$temporary_dir/users.acl"
fi

install -o 999 -g 1000 -m 0400 "$temporary_dir/healthcheck-password" \
  "$target_dir/healthcheck-credentials/healthcheck-password"
install -o 59000 -g 59000 -m 0400 "$temporary_dir/exporter-passwords.json" \
  "$target_dir/exporter-credentials/exporter-passwords.json"
install -m 0444 "$temporary_dir/users.acl" "$target_dir/acl/users.acl"

printf 'Prepared Redis ACL and credential files under %s; passwords were not displayed.\n' "$target_dir"
