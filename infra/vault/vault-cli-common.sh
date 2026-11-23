#!/usr/bin/env bash
set -euo pipefail
set +x

vault_cli_prepare() {
  if [[ -z "${VAULT_ADDR:-}" || "$VAULT_ADDR" != https://* ]]; then
    printf '%s\n' "Set VAULT_ADDR to the verified Vault HTTPS address." >&2
    return 2
  fi
  if [[ -n "${VAULT_SKIP_VERIFY:-}" ]]; then
    printf '%s\n' "VAULT_SKIP_VERIFY is not allowed." >&2
    return 2
  fi
  command -v vault >/dev/null 2>&1 || {
    printf '%s\n' "The Vault CLI is required." >&2
    return 127
  }
  command -v jq >/dev/null 2>&1 || {
    printf '%s\n' "jq is required." >&2
    return 127
  }

  if [[ -z "${VAULT_TOKEN:-}" ]]; then
    if [[ ! -t 0 ]]; then
      printf '%s\n' "Set VAULT_TOKEN securely or run from an interactive terminal." >&2
      return 2
    fi
    read -r -s -p "Authorized Vault provisioning token: " VAULT_TOKEN
    printf '\n'
    if [[ -z "$VAULT_TOKEN" ]]; then
      printf '%s\n' "An authorized Vault token is required." >&2
      return 2
    fi
    export VAULT_TOKEN
  fi

  local status
  status="$(vault status -format=json)"
  if ! jq -e '.initialized == true and .sealed == false' <<<"$status" >/dev/null; then
    printf '%s\n' "Vault must be initialized and unsealed." >&2
    return 1
  fi
}

vault_confirm_target() {
  local environment="$1"
  local confirmation

  if [[ "$environment" != staging && "$environment" != production ]]; then
    printf '%s\n' "Environment must be staging or production." >&2
    return 2
  fi
  if [[ ! -t 0 ]]; then
    printf '%s\n' "Vault configuration changes require an interactive confirmation." >&2
    return 2
  fi
  printf 'Target Vault (%s): %s\nType "%s %s" to continue: ' \
    "$environment" "$VAULT_ADDR" "$environment" "$VAULT_ADDR"
  read -r confirmation
  if [[ "$confirmation" != "${environment} ${VAULT_ADDR}" ]]; then
    printf '%s\n' "Target confirmation did not match; no Vault changes were made." >&2
    return 1
  fi
}

vault_require_pki_mount() {
  local mounts
  mounts="$(vault secrets list -format=json)"
  if ! jq -e '."pki_int/".type == "pki"' <<<"$mounts" >/dev/null; then
    printf '%s\n' "The pki_int/ PKI secrets engine is not enabled." >&2
    return 1
  fi
}

vault_require_approle_mount() {
  local auth_methods
  auth_methods="$(vault auth list -format=json)"
  if ! jq -e '."approle/".type == "approle"' <<<"$auth_methods" >/dev/null; then
    printf '%s\n' "The AppRole auth method is not enabled." >&2
    return 1
  fi
}

vault_require_signed_intermediate() {
  local issuer
  if ! issuer="$(vault read -format=json pki_int/issuer/default/json)"; then
    printf '%s\n' "Cannot read the default pki_int/ issuer; verify operator permissions and PKI state." >&2
    return 1
  fi
  if ! jq -e '.data.certificate | type == "string" and length > 0' <<<"$issuer" >/dev/null; then
    printf '%s\n' "The pki_int/ intermediate has not been signed and imported." >&2
    return 1
  fi
}
