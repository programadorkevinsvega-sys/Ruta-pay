#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  printf 'Usage: %s <app-slug> <secure-delivery-directory>\n' "$0" >&2
  exit 2
fi

app_slug="$1"
delivery_dir="$2"
if [[ ! "$app_slug" =~ ^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$ ]]; then
  printf '%s\n' "App slug must be 1-63 lowercase letters, digits, or internal hyphens." >&2
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
if [[ ! -t 0 ]]; then
  printf '%s\n' "Run this command from an interactive terminal." >&2
  exit 2
fi

if [[ -z "${VAULT_ADDR:-}" || "$VAULT_ADDR" != https://* ]]; then
  printf '%s\n' "Set VAULT_ADDR to the Vault HTTPS address." >&2
  exit 2
fi
if [[ -n "${VAULT_SKIP_VERIFY:-}" ]]; then
  printf '%s\n' "VAULT_SKIP_VERIFY is not allowed; validate the Vault TLS certificate." >&2
  exit 2
fi
command -v vault >/dev/null 2>&1 || {
  printf '%s\n' "The Vault CLI is required." >&2
  exit 127
}
if [[ -z "${VAULT_TOKEN:-}" ]]; then
  read -r -s -p "Authorized Vault provisioning token: " VAULT_TOKEN
  printf '\n'
  if [[ -z "$VAULT_TOKEN" ]]; then
    printf '%s\n' "An authorized administrator token is required." >&2
    exit 2
  fi
  export VAULT_TOKEN
fi
cleanup() {
  unset VAULT_TOKEN
  if [[ -n "${policy_file:-}" ]]; then
    rm -f -- "$policy_file"
  fi
  if [[ -n "${role_id_temporary:-}" ]]; then
    rm -f -- "$role_id_temporary"
  fi
  if [[ -n "${wrapped_secret_id_temporary:-}" ]]; then
    rm -f -- "$wrapped_secret_id_temporary"
  fi
}
trap cleanup EXIT

vault status -format=json >/dev/null
auth_methods="$(vault auth list -format=json)"
if ! printf '%s' "$auth_methods" | grep -Eq '"approle/"[[:space:]]*:'; then
  printf '%s\n' "AppRole is not enabled; run the Vault bootstrap first." >&2
  exit 1
fi

mount_path="apps/${app_slug}"
policy_name="app-${app_slug}"
role_name="app-${app_slug}"
existing_mounts="$(vault secrets list -format=json)"
existing_policies="$(vault policy list -format=json)"
existing_roles="$(vault list -format=json auth/approle/role)"

if printf '%s' "$existing_mounts" | grep -Fq "\"${mount_path}/\""; then
  printf 'Secrets mount already exists: %s\n' "$mount_path" >&2
  exit 1
fi
if printf '%s' "$existing_policies" | grep -Fq "\"${policy_name}\""; then
  printf 'Policy already exists: %s\n' "$policy_name" >&2
  exit 1
fi
if printf '%s' "$existing_roles" | grep -Fq "\"${role_name}\""; then
  printf 'AppRole already exists: %s\n' "$role_name" >&2
  exit 1
fi
role_id_output="${delivery_dir}/${app_slug}-role-id"
wrapped_secret_id_output="${delivery_dir}/${app_slug}-secret-id.wrap"
if [[ -e "$role_id_output" || -e "$wrapped_secret_id_output" ]]; then
  printf '%s\n' "An AppRole delivery file already exists; inspect it before provisioning." >&2
  exit 1
fi

umask 077
policy_file="$(mktemp "${TMPDIR:-/tmp}/vault-app-policy.XXXXXX")"
cat >"$policy_file" <<EOF
path "${mount_path}/data/*" {
  capabilities = ["create", "read", "update", "delete"]
}

path "${mount_path}/metadata" {
  capabilities = ["list"]
}

path "${mount_path}/metadata/*" {
  capabilities = ["list", "read"]
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

vault secrets enable -path="$mount_path" -version=2 kv
vault policy write "$policy_name" "$policy_file"
vault write "auth/approle/role/${role_name}" \
  token_policies="$policy_name" \
  token_no_default_policy=true \
  token_period=1h \
  secret_id_ttl=24h \
  secret_id_num_uses=1 \
  bind_secret_id=true

role_id_temporary="$(mktemp "${delivery_dir}/.${app_slug}-role-id.XXXXXX")"
wrapped_secret_id_temporary="$(mktemp "${delivery_dir}/.${app_slug}-secret-id.XXXXXX")"
vault read -field=role_id "auth/approle/role/${role_name}/role-id" >"$role_id_temporary"
vault write -field=wrapping_token -wrap-ttl=15m -f \
  "auth/approle/role/${role_name}/secret-id" >"$wrapped_secret_id_temporary"
if [[ ! -s "$role_id_temporary" || ! -s "$wrapped_secret_id_temporary" ]]; then
  printf '%s\n' "Vault returned an empty AppRole credential; inspect the partial provisioning before retrying." >&2
  exit 1
fi

ln -- "$role_id_temporary" "$role_id_output"
ln -- "$wrapped_secret_id_temporary" "$wrapped_secret_id_output"
rm -f -- "$role_id_temporary" "$wrapped_secret_id_temporary"
role_id_temporary=""
wrapped_secret_id_temporary=""
printf 'Created KV v2 mount %s, scoped policy %s, and AppRole %s.\n' \
  "$mount_path" "$policy_name" "$role_name"
printf 'Credential files are stored in the specified secure delivery directory; deliver them only through an approved channel.\n'
