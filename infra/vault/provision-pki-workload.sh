#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "${script_dir}/vault-cli-common.sh"

if [[ $# -ne 4 ]]; then
  printf 'Usage: %s <staging|production> <workload-slug> <pki-role-slug> <secure-delivery-directory>\n' "$0" >&2
  exit 2
fi

environment="$1"
workload_slug="$2"
pki_role="$3"
delivery_dir="$4"
for slug in "$workload_slug" "$pki_role"; do
  if [[ ! "$slug" =~ ^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$ ]]; then
    printf '%s\n' "Workload and PKI role slugs must be 1-63 lowercase letters, digits, or internal hyphens." >&2
    exit 2
  fi
done
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
vault_require_pki_mount
vault_require_signed_intermediate
vault_require_approle_mount

if ! pki_role_json="$(vault read -format=json "pki_int/roles/${pki_role}")"; then
  printf 'PKI role does not exist or cannot be read: %s\n' "$pki_role" >&2
  exit 1
fi
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
' <<<"$pki_role_json" >/dev/null; then
  printf '%s\n' "PKI role is not a single-identity, non-wildcard EC role; refusing to create a workload issuer identity." >&2
  exit 1
fi

policy_name="pki-${workload_slug}"
approle_name="pki-${workload_slug}"
policy_file=""
role_id_temporary=""
wrapped_secret_id_temporary=""
cleanup() {
  if [[ -n "$policy_file" ]]; then
    rm -f -- "$policy_file"
  fi
  if [[ -n "$role_id_temporary" ]]; then
    rm -f -- "$role_id_temporary"
  fi
  if [[ -n "$wrapped_secret_id_temporary" ]]; then
    rm -f -- "$wrapped_secret_id_temporary"
  fi
}
trap cleanup EXIT

existing_policies="$(vault policy list -format=json)"
existing_roles="$(vault list -format=json auth/approle/role)"
if jq -e --arg policy "$policy_name" '. | index($policy) != null' <<<"$existing_policies" >/dev/null; then
  printf 'Policy already exists: %s\n' "$policy_name" >&2
  exit 1
fi
if jq -e --arg role "$approle_name" '.data.keys | index($role) != null' <<<"$existing_roles" >/dev/null; then
  printf 'AppRole already exists: %s\n' "$approle_name" >&2
  exit 1
fi

role_id_output="${delivery_dir}/${workload_slug}-role-id"
wrapped_secret_id_output="${delivery_dir}/${workload_slug}-secret-id.wrap"
if [[ -e "$role_id_output" || -e "$wrapped_secret_id_output" ]]; then
  printf '%s\n' "An AppRole delivery file already exists; inspect it before provisioning." >&2
  exit 1
fi

umask 077
policy_file="$(mktemp "${TMPDIR:-/tmp}/vault-pki-policy.XXXXXX")"
cat >"$policy_file" <<EOF
path "pki_int/issue/${pki_role}" {
  capabilities = ["update"]
}

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

vault policy write "$policy_name" "$policy_file"
vault write "auth/approle/role/${approle_name}" \
  token_policies="$policy_name" \
  token_no_default_policy=true \
  token_period=1h \
  secret_id_ttl=720h \
  secret_id_num_uses=0 \
  bind_secret_id=true

role_id_temporary="$(mktemp "${delivery_dir}/.${workload_slug}-role-id.XXXXXX")"
wrapped_secret_id_temporary="$(mktemp "${delivery_dir}/.${workload_slug}-secret-id.XXXXXX")"
vault read -field=role_id "auth/approle/role/${approle_name}/role-id" >"$role_id_temporary"
vault write -field=wrapping_token -wrap-ttl=15m -f \
  "auth/approle/role/${approle_name}/secret-id" >"$wrapped_secret_id_temporary"
if [[ ! -s "$role_id_temporary" || ! -s "$wrapped_secret_id_temporary" ]]; then
  printf '%s\n' "Vault returned an empty AppRole credential; inspect the partial provisioning before retrying." >&2
  exit 1
fi

ln -- "$role_id_temporary" "$role_id_output"
ln -- "$wrapped_secret_id_temporary" "$wrapped_secret_id_output"
rm -f -- "$role_id_temporary" "$wrapped_secret_id_temporary"
role_id_temporary=""
wrapped_secret_id_temporary=""
printf 'Created workload AppRole %s scoped to PKI role %s.\n' "$approle_name" "$pki_role"
printf '%s\n' "The response-wrapped delivery token is single-use and expires in 15 minutes; the underlying SecretID can be reused for 30 days for Agent recovery."
printf '%s\n' "Deliver the files using an approved channel; never commit either credential file."
