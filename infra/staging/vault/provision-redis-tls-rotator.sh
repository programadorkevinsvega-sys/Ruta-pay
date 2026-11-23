#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "${script_dir}/../../vault/vault-cli-common.sh"

if [[ $# -ne 1 || "$1" != "staging" ]]; then
  printf 'Usage: %s staging\n' "$0" >&2
  exit 2
fi

vault_cli_prepare
vault_confirm_target staging

mounts="$(vault secrets list -format=json)"
if ! jq -e '.["apps/platform/"].type == "kv" and .["apps/platform/"].options.version == "2"' \
  <<<"$mounts" >/dev/null; then
  printf '%s\n' "Vault KV v2 mount apps/platform/ is required." >&2
  exit 1
fi

role="$(vault read -format=json auth/approle/role/pki-agent-redis)"
if ! jq -e '(.data.token_policies // []) | index("pki-redis") != null' \
  <<<"$role" >/dev/null; then
  printf '%s\n' "The Redis Vault Agent AppRole must have the pki-redis policy." >&2
  exit 1
fi

policy_file=""
secret_file=""
payload_file=""
cleanup() {
  if [[ -n "$policy_file" ]]; then rm -f -- "$policy_file"; fi
  if [[ -n "$secret_file" ]]; then rm -f -- "$secret_file"; fi
  if [[ -n "$payload_file" ]]; then rm -f -- "$payload_file"; fi
}
trap cleanup EXIT
umask 077

current_policy="$(vault policy read pki-redis)"
policy_file="$(mktemp "${TMPDIR:-/tmp}/pki-redis-policy.XXXXXX")"
printf '%s\n' "$current_policy" >"$policy_file"
if ! grep -Fq 'path "apps/platform/data/redis-tls-rotator"' "$policy_file"; then
  cat >>"$policy_file" <<'EOF'

path "apps/platform/data/redis-tls-rotator" {
  capabilities = ["read"]
}
EOF
elif ! grep -A2 -F 'path "apps/platform/data/redis-tls-rotator"' \
  "$policy_file" | grep -Fq 'capabilities = ["read"]'; then
  printf '%s\n' "Existing Redis TLS-rotator policy stanza is not read-only." >&2
  exit 1
fi
vault policy write pki-redis "$policy_file"

if existing_secret="$(vault kv get -format=json -mount=apps/platform redis-tls-rotator)"; then
  if ! jq -e '.data.data.password | type == "string" and test("^[0-9a-f]{64}$")' \
    <<<"$existing_secret" >/dev/null; then
    printf '%s\n' "Existing Redis TLS-rotator secret has an unexpected format." >&2
    exit 1
  fi
  unset existing_secret
  printf '%s\n' "Redis TLS-rotator secret already exists with the expected format."
  exit 0
fi

secret_file="$(mktemp "${TMPDIR:-/tmp}/redis-tls-rotator-secret.XXXXXX")"
payload_file="$(mktemp "${TMPDIR:-/tmp}/redis-tls-rotator-payload.XXXXXX")"
openssl rand -hex 32 >"$secret_file"
python3 - "$secret_file" "$payload_file" <<'PY'
import json
import pathlib
import sys

password = pathlib.Path(sys.argv[1]).read_text().strip()
if len(password) != 64 or any(character not in "0123456789abcdef" for character in password):
    raise SystemExit("Generated Redis TLS-rotator secret has an unexpected format.")
pathlib.Path(sys.argv[2]).write_text(json.dumps({"password": password}))
PY
vault kv put -mount=apps/platform -cas=0 redis-tls-rotator "@${payload_file}"
printf '%s\n' "Created the Redis TLS-rotator credential in Vault KV; the secret was not displayed."
