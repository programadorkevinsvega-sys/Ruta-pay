#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "${script_dir}/../../../vault/vault-cli-common.sh"

if [[ $# -ne 3 ]]; then
  printf 'Usage: %s <staging> <workload-slug> <comma-separated-pki-roles>\n' "$0" >&2
  exit 2
fi

environment="$1"
workload_slug="$2"
role_list="$3"
if [[ ! "$workload_slug" =~ ^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$ ]]; then
  printf '%s\n' "Workload slug must be 1-63 lowercase letters, digits, or internal hyphens." >&2
  exit 2
fi
if [[ ! "$role_list" =~ ^[a-z0-9][a-z0-9,-]*$ ]]; then
  printf '%s\n' "PKI role list must contain comma-separated lowercase role slugs." >&2
  exit 2
fi

IFS=',' read -r -a pki_roles <<<"$role_list"
if (( ${#pki_roles[@]} == 0 )); then
  printf '%s\n' "At least one PKI role is required." >&2
  exit 2
fi
declare -A seen_roles=()
for role in "${pki_roles[@]}"; do
  if [[ ! "$role" =~ ^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$ ]] ||
    [[ -n "${seen_roles[$role]:-}" ]]; then
    printf 'Invalid or duplicate PKI role: %s\n' "$role" >&2
    exit 2
  fi
  seen_roles["$role"]=1
done

vault_cli_prepare
vault_confirm_target "$environment"
vault_require_pki_mount
vault_require_signed_intermediate
vault_require_approle_mount

for role in "${pki_roles[@]}"; do
  role_json="$(vault read -format=json "pki_int/roles/${role}")"
  if ! jq -e '
    (.data.allowed_domains | length >= 1 and length <= 2) and
    (.data.allow_any_name == false) and
    (.data.allow_ip_sans == false) and
    (.data.allow_subdomains == false) and
    (.data.allow_glob_domains == false) and
    (.data.allow_wildcard_certificates == false) and
    (.data.allowed_uri_sans | length == 0) and
    (.data.allowed_other_sans | length == 0) and
    (.data.allow_bare_domains == true) and
    (.data.key_type == "ec") and
    (.data.key_bits == 256) and
    ((.data.server_flag == true) != (.data.client_flag == true))
  ' <<<"$role_json" >/dev/null; then
    printf 'PKI role is not a constrained single-profile EC role: %s\n' "$role" >&2
    exit 1
  fi
done

policy_name="pki-${workload_slug}"
approle_name="pki-agent-${workload_slug}"
existing_policies="$(vault policy list -format=json)"
existing_roles="$(vault list -format=json auth/approle/role)"
if jq -e --arg policy "$policy_name" '(.data.keys // .) | index($policy) != null' <<<"$existing_policies" >/dev/null; then
  printf 'Policy already exists: %s; inspect it before continuing.\n' "$policy_name" >&2
  exit 1
fi
if jq -e --arg role "$approle_name" '(.data.keys // .) | index($role) != null' <<<"$existing_roles" >/dev/null; then
  printf 'AppRole already exists: %s; inspect it before continuing.\n' "$approle_name" >&2
  exit 1
fi

delivery_dir="${HOME}/.payvia-vault-delivery"
if [[ ! -d "$delivery_dir" ]]; then
  install -d -m 0700 "$delivery_dir"
fi
if [[ "$(stat -c '%u:%a' "$delivery_dir")" != "$(id -u):700" ]]; then
  printf 'Delivery directory must be owned by the current user and mode 0700: %s\n' "$delivery_dir" >&2
  exit 1
fi
role_id_output="${delivery_dir}/${workload_slug}-role-id"
wrapped_output="${delivery_dir}/${workload_slug}-secret-id.wrap"
if [[ -e "$role_id_output" || -e "$wrapped_output" ]]; then
  printf 'Credential delivery files already exist for %s; inspect them before retrying.\n' "$workload_slug" >&2
  exit 1
fi

policy_file=""
role_id_temporary=""
wrapped_temporary=""
cleanup() {
  if [[ -n "$policy_file" ]]; then rm -f -- "$policy_file"; fi
  if [[ -n "$role_id_temporary" ]]; then rm -f -- "$role_id_temporary"; fi
  if [[ -n "$wrapped_temporary" ]]; then rm -f -- "$wrapped_temporary"; fi
}
trap cleanup EXIT

umask 077
policy_file="$(mktemp "${TMPDIR:-/tmp}/vault-agent-policy.XXXXXX")"
{
  for role in "${pki_roles[@]}"; do
    printf 'path "pki_int/issue/%s" {\n  capabilities = ["update"]\n}\n\n' "$role"
  done
  if [[ "$workload_slug" == "redis" ]]; then
    cat <<'EOF'
path "apps/platform/data/redis" {
  capabilities = ["read"]
}

path "apps/platform/data/redis-tls-rotator" {
  capabilities = ["read"]
}

EOF
  fi
  cat <<'EOF'
path "pki_int/cert/ca_chain" {
  capabilities = ["read"]
}

path "auth/token/lookup-self" {
  capabilities = ["read"]
}

path "auth/token/renew-self" {
  capabilities = ["update"]
}

path "auth/token/revoke-self" {
  capabilities = ["update"]
}
EOF
} >"$policy_file"

vault policy write "$policy_name" "$policy_file"
vault write "auth/approle/role/${approle_name}" \
  token_policies="$policy_name" \
  token_no_default_policy=true \
  token_period=1h \
  secret_id_ttl=720h \
  secret_id_num_uses=0 \
  bind_secret_id=true

role_id_temporary="$(mktemp "${delivery_dir}/.${workload_slug}-role-id.XXXXXX")"
wrapped_temporary="$(mktemp "${delivery_dir}/.${workload_slug}-secret-id.XXXXXX")"
vault read -field=role_id "auth/approle/role/${approle_name}/role-id" >"$role_id_temporary"
vault write -field=wrapping_token -wrap-ttl=15m -f \
  "auth/approle/role/${approle_name}/secret-id" >"$wrapped_temporary"
if [[ ! -s "$role_id_temporary" || ! -s "$wrapped_temporary" ]]; then
  printf '%s\n' "Vault returned an empty AppRole credential; inspect Vault audit logs." >&2
  exit 1
fi

ln -- "$role_id_temporary" "$role_id_output"
ln -- "$wrapped_temporary" "$wrapped_output"
rm -f -- "$role_id_temporary" "$wrapped_temporary"
role_id_temporary=""
wrapped_temporary=""
printf 'Created host-scoped AppRole %s for PKI roles: %s\n' "$approle_name" "$role_list"
printf 'Wrapped credential files are under %s; transfer them only to the owning VM and unwrap there.\n' "$delivery_dir"
