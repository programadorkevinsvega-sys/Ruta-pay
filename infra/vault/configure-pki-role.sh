#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "${script_dir}/vault-cli-common.sh"

if [[ $# -ne 6 ]]; then
  printf 'Usage: %s <staging|production> <role-slug> <server|client> <common-name> <dns-alt-names-csv-or-dash> <max-ttl-hours>\n' "$0" >&2
  exit 2
fi

environment="$1"
role_name="$2"
profile="$3"
common_name="$4"
dns_alt_names="$5"
max_ttl_hours="$6"

if [[ ! "$role_name" =~ ^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$ ]]; then
  printf '%s\n' "Role slug must be 1-63 lowercase letters, digits, or internal hyphens." >&2
  exit 2
fi
if [[ "$profile" != server && "$profile" != client ]]; then
  printf '%s\n' "Certificate profile must be server or client." >&2
  exit 2
fi
if [[ ! "$common_name" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,252}$ ]]; then
  printf '%s\n' "Common name contains unsupported characters or has an invalid length." >&2
  exit 2
fi
command -v python3 >/dev/null 2>&1 || {
  printf '%s\n' "Python 3 is required to validate certificate identities." >&2
  exit 127
}
if [[ ! "$max_ttl_hours" =~ ^[1-9][0-9]{0,3}$ ]] || (( max_ttl_hours > 2160 )); then
  printf '%s\n' "Maximum TTL must be an explicit number of hours from 1 to 2160." >&2
  exit 2
fi
if [[ "$profile" == server ]]; then
  if ! python3 - "$common_name" <<'PY'
import re
import sys

def valid_dns_name(name):
    labels = name.split(".")
    return len(name) <= 253 and all(
        re.fullmatch(r"[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?", label)
        for label in labels
    )

if not valid_dns_name(sys.argv[1]):
    raise SystemExit(1)
PY
  then
    printf '%s\n' "A server role requires an exact DNS common name." >&2
    exit 2
  fi
  if [[ "$dns_alt_names" != - ]]; then
    if ! python3 - "$common_name" "$dns_alt_names" <<'PY'
import re
import sys

common_name = sys.argv[1]
names = sys.argv[2].split(",")
if not names or any(not name for name in names) or len(names) != len(set(names)):
    raise SystemExit(1)
if common_name in names:
    raise SystemExit(1)
for name in names:
    labels = name.split(".")
    if len(name) > 253 or not all(
        re.fullmatch(r"[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?", label)
        for label in labels
    ):
        raise SystemExit(1)
PY
    then
      printf '%s\n' "DNS alt names must be unique, exact DNS names without wildcards, and must not repeat the common name." >&2
      exit 2
    fi
  fi
else
  if [[ "$dns_alt_names" != - ]]; then
    printf '%s\n' "Client roles use a common-name identity only; DNS alt names must be '-'." >&2
    exit 2
  fi
fi

vault_cli_prepare
vault_confirm_target "$environment"
vault_require_pki_mount
vault_require_signed_intermediate

existing_roles="$(vault list -format=json pki_int/roles)"
if jq -e --arg role "$role_name" '.data.keys | index($role) != null' <<<"$existing_roles" >/dev/null; then
  printf 'PKI role already exists: %s\n' "$role_name" >&2
  exit 1
fi

allowed_domains="$common_name"
if [[ "$dns_alt_names" != - ]]; then
  allowed_domains="${allowed_domains},${dns_alt_names}"
fi

role_arguments=(
  "allowed_domains=${allowed_domains}"
  "allow_bare_domains=true"
  "allow_subdomains=false"
  "allow_glob_domains=false"
  "allow_any_name=false"
  "allow_ip_sans=false"
  "allow_wildcard_certificates=false"
  "allow_localhost=false"
  "allowed_uri_sans="
  "allowed_other_sans="
  "enforce_hostnames=false"
  "key_type=ec"
  "key_bits=256"
  "key_usage=DigitalSignature"
  "ttl=${max_ttl_hours}h"
  "max_ttl=${max_ttl_hours}h"
  "issuer_ref=default"
)

if [[ "$profile" == server ]]; then
  role_arguments+=(
    "server_flag=true"
    "client_flag=false"
    "ext_key_usage=ServerAuth"
  )
else
  role_arguments+=(
    "server_flag=false"
    "client_flag=true"
    "ext_key_usage=ClientAuth"
  )
fi

vault write "pki_int/roles/${role_name}" "${role_arguments[@]}"
printf 'Created PKI role %s with exact CN %s, DNS SANs %s, %s profile, and maximum lifetime %sh.\n' \
  "$role_name" "$common_name" "$dns_alt_names" "$profile" "$max_ttl_hours"
