#!/bin/sh
set -eu

if [ ! -t 0 ]; then
  printf '%s\n' "Run this bootstrap from an interactive terminal after initializing and unsealing Vault." >&2
  exit 1
fi

printf '%s' "Initial Vault root token: "
stty -echo
if ! IFS= read -r VAULT_TOKEN; then
  stty echo
  printf '\n' >&2
  exit 1
fi
stty echo
printf '\n'
export VAULT_TOKEN

vault status -format=json >/dev/null

audit_devices="$(vault audit list -format=json)"
if ! printf '%s' "$audit_devices" | grep -Eq '"file/"[[:space:]]*:'; then
  vault audit enable file file_path=/opt/vault/audit/audit.log
fi

secrets_engines="$(vault secrets list -format=json)"
if printf '%s' "$secrets_engines" | grep -Eq '"pki_int/"[[:space:]]*:'; then
  printf '%s\n' "pki_int/ already exists; inspect its type and configuration before continuing." >&2
  exit 1
fi
vault secrets enable -path=pki_int pki
vault secrets tune -max-lease-ttl=43800h pki_int

auth_methods="$(vault auth list -format=json)"
if ! printf '%s' "$auth_methods" | grep -Eq '"approle/"[[:space:]]*:'; then
  vault auth enable approle
fi
if ! printf '%s' "$auth_methods" | grep -Eq '"userpass/"[[:space:]]*:'; then
  vault auth enable userpass
fi

vault policy write vault-operator /vault/config/policies/vault-operator.hcl
vault policy write vault-snapshot /vault/config/policies/vault-snapshot.hcl
vault write auth/approle/role/vault-snapshot \
  token_policies=vault-snapshot \
  token_no_default_policy=true \
  token_period=1h \
  secret_id_ttl=0 \
  secret_id_num_uses=0

role_id_file="/opt/vault/snapshot-agent/role_id"
secret_id_file="/opt/vault/snapshot-agent/secret_id"
if [ ! -e "$role_id_file" ] && [ ! -e "$secret_id_file" ]; then
  role_id_temporary="$(mktemp /opt/vault/snapshot-agent/role-id.XXXXXX)"
  secret_id_temporary="$(mktemp /opt/vault/snapshot-agent/secret-id.XXXXXX)"
  trap 'rm -f "$role_id_temporary" "$secret_id_temporary"' EXIT HUP INT TERM
  vault read -field=role_id auth/approle/role/vault-snapshot/role-id >"$role_id_temporary"
  vault write -field=secret_id -f auth/approle/role/vault-snapshot/secret-id >"$secret_id_temporary"
  if [ ! -s "$role_id_temporary" ] || [ ! -s "$secret_id_temporary" ]; then
    printf '%s\n' "Vault returned an empty snapshot AppRole credential." >&2
    exit 1
  fi
  chmod 0400 "$role_id_temporary" "$secret_id_temporary"
  mv "$role_id_temporary" "$role_id_file"
  mv "$secret_id_temporary" "$secret_id_file"
  trap - EXIT HUP INT TERM
elif [ ! -s "$role_id_file" ] || [ ! -s "$secret_id_file" ]; then
  printf '%s\n' "Snapshot AppRole credentials are incomplete; refusing to overwrite them." >&2
  exit 1
fi

if [ ! -s /opt/vault/pki/intermediate.csr ]; then
  csr_temporary="$(mktemp /opt/vault/pki/intermediate.csr.XXXXXX)"
  trap 'rm -f "$csr_temporary"' EXIT HUP INT TERM
  vault write -field=csr pki_int/intermediate/generate/internal \
      common_name="Shared Services Issuing CA" \
    ttl=43800h >"$csr_temporary"
  if [ ! -s "$csr_temporary" ]; then
    printf '%s\n' "Vault returned an empty PKI intermediate CSR." >&2
    exit 1
  fi
  chmod 0640 "$csr_temporary"
  mv "$csr_temporary" /opt/vault/pki/intermediate.csr
  trap - EXIT HUP INT TERM
fi

unset VAULT_TOKEN
printf '%s\n' "Userpass, AppRole, local file audit, shared PKI intermediate CSR, operator policy, and restricted snapshot AppRole are configured."
printf '%s\n' "No shared KV mount was created; provision each application with provision-app.sh."
printf '%s\n' "Create named human userpass accounts before revoking the bootstrap root token."
