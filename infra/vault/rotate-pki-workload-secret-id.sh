#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "${script_dir}/vault-cli-common.sh"

if [[ $# -ne 3 ]]; then
  printf 'Usage: %s <staging|production> <workload-slug> <secure-delivery-directory>\n' "$0" >&2
  exit 2
fi

environment="$1"
workload_slug="$2"
delivery_dir="$3"
if [[ ! "$workload_slug" =~ ^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$ ]]; then
  printf '%s\n' "Workload slug must be 1-63 lowercase letters, digits, or internal hyphens." >&2
  exit 2
fi
if [[ ! -d "$delivery_dir" || ! -O "$delivery_dir" ]]; then
  printf '%s\n' "Secure delivery directory must exist and be owned by the current user." >&2
  exit 2
fi
delivery_mode="$(stat -c '%a' "$delivery_dir")"
if (( (8#$delivery_mode & 077) != 0 )); then
  printf '%s\n' "Secure delivery directory must not grant access to group or other users (use chmod 700)." >&2
  exit 2
fi

vault_cli_prepare
vault_confirm_target "$environment"
vault_require_approle_mount

approle_name="pki-${workload_slug}"
policy_name="pki-${workload_slug}"
if ! approle_json="$(vault read -format=json "auth/approle/role/${approle_name}")"; then
  printf 'Workload AppRole does not exist or cannot be read: %s\n' "$approle_name" >&2
  exit 1
fi
if ! jq -e --arg policy "$policy_name" '
  (.data.bind_secret_id == true) and
  ((.data.secret_id_ttl | tostring) != "0") and
  (.data.secret_id_num_uses == 0) and
  (.data.token_policies | index($policy) != null)
' <<<"$approle_json" >/dev/null; then
  printf '%s\n' "AppRole does not match the reusable, time-limited SecretID policy; refusing to rotate it." >&2
  exit 1
fi

timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
wrapped_secret_id_output="${delivery_dir}/${workload_slug}-secret-id-${timestamp}.wrap"
if [[ -e "$wrapped_secret_id_output" || -L "$wrapped_secret_id_output" ]]; then
  printf '%s\n' "The timestamped SecretID delivery file already exists; refusing to overwrite it." >&2
  exit 1
fi

umask 077
wrapped_secret_id_temporary="$(mktemp "${delivery_dir}/.${workload_slug}-secret-id.XXXXXX")"
cleanup() {
  if [[ -n "${wrapped_secret_id_temporary:-}" ]]; then
    rm -f -- "$wrapped_secret_id_temporary"
  fi
}
trap cleanup EXIT

vault write -field=wrapping_token -wrap-ttl=15m -f \
  "auth/approle/role/${approle_name}/secret-id" >"$wrapped_secret_id_temporary"
if [[ ! -s "$wrapped_secret_id_temporary" ]]; then
  printf '%s\n' "Vault returned an empty wrapped SecretID; inspect Vault audit logs before retrying." >&2
  exit 1
fi
ln -- "$wrapped_secret_id_temporary" "$wrapped_secret_id_output"
rm -f -- "$wrapped_secret_id_temporary"
wrapped_secret_id_temporary=""

printf 'Created a new 30-day SecretID for AppRole %s.\n' "$approle_name"
printf 'Deliver %s within 15 minutes; retain the current VM SecretID until the new credential is installed and Agent authentication is verified.\n' \
  "$wrapped_secret_id_output"
