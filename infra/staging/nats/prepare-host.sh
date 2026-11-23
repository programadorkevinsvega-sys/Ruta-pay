#!/usr/bin/env bash
set -euo pipefail

target_dir="${1:-/etc/payvia/staging/nats}"
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

if [[ "$target_dir" == /etc/payvia/staging/nats ]] && (( EUID != 0 )); then
  printf 'Run this script as root to prepare files under /etc/payvia/staging/nats.\n' >&2
  exit 1
fi

install -d -m 0755 \
  "$target_dir" \
  "$target_dir/tls/payvia/staging" \
  "$target_dir/auth"

if [[ -e "$target_dir/auth/authorization.conf" ]]; then
  if ! cmp -s "$script_dir/auth/authorization.conf" "$target_dir/auth/authorization.conf"; then
    printf 'Refusing to replace different file: %s/auth/authorization.conf\n' "$target_dir" >&2
    exit 1
  fi
else
  install -m 0444 "$script_dir/auth/authorization.conf" "$target_dir/auth/authorization.conf"
fi

printf 'Prepared NATS authorization files under %s; no service was started or restarted.\n' "$target_dir"
