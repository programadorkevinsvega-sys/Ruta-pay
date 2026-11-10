#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${VAULT_ADDR:-}" || "$VAULT_ADDR" != https://* ]]; then
  printf '%s\n' "Set VAULT_ADDR to the Vault HTTPS address." >&2
  exit 2
fi
if [[ -n "${VAULT_SKIP_VERIFY:-}" ]]; then
  printf '%s\n' "VAULT_SKIP_VERIFY is not allowed; validate the Vault TLS certificate." >&2
  exit 2
fi
if [[ ! -t 0 ]]; then
  printf '%s\n' "Run this command from an interactive terminal." >&2
  exit 2
fi
command -v vault >/dev/null 2>&1 || {
  printf '%s\n' "The Vault CLI is required." >&2
  exit 127
}

read -r -p "Userpass username: " username
if [[ ! "$username" =~ ^[a-z0-9][a-z0-9._-]{0,62}$ ]]; then
  printf '%s\n' "Username must be 1-63 lowercase letters, digits, dots, underscores, or hyphens." >&2
  exit 2
fi

read -r -p "Policy to assign (vault-operator or app-<slug>): " policy_name
if [[ ! "$policy_name" =~ ^[a-z0-9][a-z0-9._-]{0,62}$ ]]; then
  printf '%s\n' "Policy name contains unsupported characters." >&2
  exit 2
fi

read -r -s -p "Authorized administrator token: " VAULT_TOKEN
printf '\n'
if [[ -z "$VAULT_TOKEN" ]]; then
  printf '%s\n' "An authorized administrator token is required." >&2
  exit 2
fi
export VAULT_TOKEN
cleanup() {
  unset VAULT_TOKEN userpass_password
}
trap cleanup EXIT

vault status -format=json >/dev/null
auth_methods="$(vault auth list -format=json)"
if ! printf '%s' "$auth_methods" | grep -Eq '"userpass/"[[:space:]]*:'; then
  printf '%s\n' "Userpass is not enabled; run the Vault bootstrap first." >&2
  exit 1
fi
vault policy read "$policy_name" >/dev/null

read -r -s -p "New userpass password: " userpass_password
printf '\n'
if [[ -z "$userpass_password" ]]; then
  printf '%s\n' "Password must not be empty." >&2
  exit 2
fi

printf '%s' "$userpass_password" |
  vault write "auth/userpass/users/${username}" password=- policies="$policy_name"
unset userpass_password
printf 'Created or updated UI account %s with policy %s.\n' "$username" "$policy_name"
