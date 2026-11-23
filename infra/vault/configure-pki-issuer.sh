#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "${script_dir}/vault-cli-common.sh"

if [[ $# -ne 4 ]]; then
  printf 'Usage: %s <staging|production> <signed-intermediate-chain.pem> <offline-root-ca.pem> <issuer-base-https-url>\n' "$0" >&2
  exit 2
fi

environment="$1"
signed_chain="$2"
offline_root="$3"
issuer_base_url="$4"
if [[ ! -f "$signed_chain" || ! -r "$signed_chain" || ! -f "$offline_root" || ! -r "$offline_root" ]]; then
  printf '%s\n' "The signed intermediate chain and public offline root CA must be readable regular files." >&2
  exit 2
fi
if ! command -v python3 >/dev/null 2>&1; then
  printf '%s\n' "Python 3 is required to validate the issuer URL." >&2
  exit 127
fi
if ! python3 - "$issuer_base_url" <<'PY'
import sys
from urllib.parse import urlsplit

try:
    parsed = urlsplit(sys.argv[1])
    port = parsed.port
    valid = (
        parsed.scheme == "https"
        and bool(parsed.hostname)
        and parsed.username is None
        and parsed.password is None
        and parsed.path == ""
        and parsed.query == ""
        and parsed.fragment == ""
        and (port is None or 1 <= port <= 65535)
    )
except ValueError:
    valid = False

if not valid:
    raise SystemExit(1)
PY
then
  printf '%s\n' "Issuer base URL must be a valid HTTPS origin without credentials, path, query, or fragment." >&2
  exit 2
fi
if grep -Eq -- '-----BEGIN ([A-Z0-9 ]+ )?PRIVATE KEY-----' "$signed_chain" "$offline_root"; then
  printf '%s\n' "Private keys are forbidden in certificate input files." >&2
  exit 2
fi
command -v openssl >/dev/null 2>&1 || {
  printf '%s\n' "OpenSSL is required to verify the certificate chain." >&2
  exit 127
}

for certificate in "$offline_root" "$signed_chain"; do
  if ! openssl x509 -in "$certificate" -noout >/dev/null 2>&1; then
    printf 'Invalid X.509 certificate input: %s\n' "$certificate" >&2
    exit 2
  fi
  basic_constraints="$(openssl x509 -in "$certificate" -noout -ext basicConstraints 2>/dev/null)"
  key_usage="$(openssl x509 -in "$certificate" -noout -ext keyUsage 2>/dev/null)"
  if ! grep -q 'CA:TRUE' <<<"$basic_constraints"; then
    printf 'Certificate is not marked as a CA: %s\n' "$certificate" >&2
    exit 2
  fi
  if ! grep -q 'Certificate Sign' <<<"$key_usage"; then
    printf 'CA certificate lacks keyCertSign usage: %s\n' "$certificate" >&2
    exit 2
  fi
done
if ! openssl verify -CAfile "$offline_root" "$offline_root" >/dev/null; then
  printf '%s\n' "The supplied offline root CA is not a valid, self-signed trust anchor." >&2
  exit 1
fi
if ! openssl verify -CAfile "$offline_root" -untrusted "$signed_chain" "$signed_chain" >/dev/null; then
  printf '%s\n' "The intermediate chain does not verify against the supplied offline root CA." >&2
  exit 1
fi

vault_cli_prepare
vault_confirm_target "$environment"
if [[ "$issuer_base_url" != "${VAULT_ADDR%/}" ]]; then
  printf '%s\n' "Issuer base URL must match the confirmed VAULT_ADDR so consumers use the same verified API endpoint." >&2
  exit 2
fi
vault_require_pki_mount

if issuer="$(vault read -format=json pki_int/issuer/default/json 2>&1)"; then
  if jq -e '.data.certificate | type == "string" and length > 0' <<<"$issuer" >/dev/null; then
    printf '%s\n' "A signed intermediate is already installed. Refusing to replace issuer material." >&2
    exit 1
  fi
elif ! grep -Fq 'no default issuer currently configured' <<<"$issuer"; then
  printf '%s\n' "Could not inspect the current default issuer:" "$issuer" >&2
  exit 1
fi

vault write pki_int/intermediate/set-signed "certificate=@${signed_chain}"
vault write pki_int/config/urls \
  "issuing_certificates=${issuer_base_url}/v1/pki_int/ca" \
  "crl_distribution_points=${issuer_base_url}/v1/pki_int/crl"
vault_require_signed_intermediate

printf '%s\n' "Imported and verified the signed intermediate; configured issuer and CRL URLs."
printf '%s\n' "No leaf certificates, service files, Agents, or service processes were changed."
