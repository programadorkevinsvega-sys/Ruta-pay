#!/usr/bin/env bash
set -euo pipefail

target_dir="${1:-/etc/payvia/staging/postgres}"
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

if (( EUID != 0 )); then
  printf 'Run this script as root to install the PostgreSQL secret with container ownership.\n' >&2
  exit 1
fi

install -d -m 0755 \
  "$target_dir" \
  "$target_dir/tls/payvia/staging" \
  "$target_dir/client-ca" \
  "$target_dir/server-ca" \
  "$target_dir/exporter-tls"

if [[ -e "$target_dir/pg_hba.conf" ]]; then
  if ! cmp -s "$script_dir/pg_hba.conf" "$target_dir/pg_hba.conf"; then
    printf 'Refusing to replace different file: %s/pg_hba.conf\n' "$target_dir" >&2
    exit 1
  fi
else
  install -m 0444 "$script_dir/pg_hba.conf" "$target_dir/pg_hba.conf"
fi

if [[ ! -e "$target_dir/bootstrap-password" ]]; then
  temporary_password="$(mktemp "$target_dir/.bootstrap-password.XXXXXX")"
  trap 'rm -f -- "$temporary_password"' EXIT
  openssl rand -hex 32 | tr -d '\n' > "$temporary_password"
  chown 70:70 "$temporary_password"
  chmod 0400 "$temporary_password"
  mv -- "$temporary_password" "$target_dir/bootstrap-password"
elif [[ "$(stat -c '%u:%g:%a' "$target_dir/bootstrap-password")" != '70:70:400' ]]; then
  printf 'Existing bootstrap password has unexpected ownership or mode: %s/bootstrap-password\n' "$target_dir" >&2
  exit 1
fi

printf 'Prepared PostgreSQL host files under %s; no service was started or restarted.\n' "$target_dir"
