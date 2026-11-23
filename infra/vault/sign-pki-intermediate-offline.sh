#!/usr/bin/env bash
set -euo pipefail
set +x

if [[ $# -ne 4 ]]; then
  printf 'Usage: %s <vault-intermediate.csr> <offline-root-ca.pem> <offline-root-private-key.pem> <validity-days>\n' "$0" >&2
  exit 2
fi

csr_file="$1"
root_cert="$2"
root_key="$3"
validity_days="$4"
output_dir="$(dirname -- "$csr_file")"

if [[ ! "$validity_days" =~ ^[1-9][0-9]{0,3}$ ]] || (( validity_days > 3650 )); then
  printf '%s\n' "Intermediate validity must be explicitly selected from 1 to 3650 days." >&2
  exit 2
fi
for input_file in "$csr_file" "$root_cert" "$root_key"; do
  if [[ ! -f "$input_file" || -L "$input_file" || ! -O "$input_file" || ! -r "$input_file" ]]; then
    printf 'Input must be a readable regular file owned by the current user, not a symlink: %s\n' "$input_file" >&2
    exit 2
  fi
done
if [[ ! -d "$output_dir" || ! -O "$output_dir" ]]; then
  printf '%s\n' "CSR directory must exist and be owned by the current user." >&2
  exit 2
fi
directory_mode="$(stat -c '%a' "$output_dir")"
if (( (8#$directory_mode & 077) != 0 )); then
  printf '%s\n' "CSR/output directory must not grant access to group or other users (use chmod 700)." >&2
  exit 2
fi
key_mode="$(stat -c '%a' "$root_key")"
if (( (8#$key_mode & 077) != 0 )); then
  printf '%s\n' "Offline root private key must not grant access to group or other users (use chmod 600)." >&2
  exit 2
fi
output="${output_dir}/intermediate-signed.pem"
if [[ -e "$output" || -L "$output" ]]; then
  printf 'Output already exists; refusing to overwrite: %s\n' "$output" >&2
  exit 1
fi
if grep -Eq -- '-----BEGIN ([A-Z0-9 ]+ )?PRIVATE KEY-----' "$csr_file" "$root_cert"; then
  printf '%s\n' "CSR and root certificate inputs must not contain private keys." >&2
  exit 2
fi

command -v openssl >/dev/null 2>&1 || {
  printf '%s\n' "OpenSSL is required." >&2
  exit 127
}
command -v mktemp >/dev/null 2>&1 || {
  printf '%s\n' "mktemp is required." >&2
  exit 127
}

if ! openssl req -in "$csr_file" -noout -verify >/dev/null 2>&1; then
  printf '%s\n' "Vault intermediate CSR is invalid or its signature does not verify." >&2
  exit 1
fi
csr_subject="$(openssl req -in "$csr_file" -noout -subject -nameopt RFC2253)"
if [[ "$csr_subject" != "subject=CN=Shared Services Issuing CA" ]]; then
  printf '%s\n' "CSR subject does not match the expected Shared Services Issuing CA identity." >&2
  exit 1
fi
if ! openssl x509 -in "$root_cert" -noout >/dev/null 2>&1; then
  printf '%s\n' "Offline root certificate is not a valid X.509 certificate." >&2
  exit 1
fi
root_constraints="$(openssl x509 -in "$root_cert" -noout -ext basicConstraints 2>/dev/null)"
root_key_usage="$(openssl x509 -in "$root_cert" -noout -ext keyUsage 2>/dev/null)"
if ! grep -q 'CA:TRUE' <<<"$root_constraints" ||
   ! grep -q 'Certificate Sign' <<<"$root_key_usage"; then
  printf '%s\n' "Offline root certificate must be a CA with keyCertSign usage." >&2
  exit 1
fi
if ! openssl verify -CAfile "$root_cert" "$root_cert" >/dev/null 2>&1; then
  printf '%s\n' "Offline root certificate is not a valid self-signed trust anchor." >&2
  exit 1
fi
if ! cmp -s \
  <(openssl x509 -in "$root_cert" -pubkey -noout) \
  <(openssl pkey -in "$root_key" -pubout); then
  printf '%s\n' "Offline root private key does not match the supplied root certificate." >&2
  exit 1
fi

temporary_dir="$(mktemp -d "${output_dir}/.payvia-intermediate.XXXXXX")"
temporary_cert="${temporary_dir}/intermediate.pem"
temporary_serial="${temporary_dir}/root.srl"
temporary_config="${temporary_dir}/intermediate-extensions.cnf"
cleanup() {
  rm -f -- "$temporary_cert" "$temporary_serial" "$temporary_config"
  rmdir -- "$temporary_dir"
}
trap cleanup EXIT

cat >"$temporary_config" <<'EOF'
[v3_intermediate_ca]
basicConstraints = critical,CA:true,pathlen:0
keyUsage = critical,keyCertSign,cRLSign
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid,issuer
EOF
chmod 0600 "$temporary_config"

printf '%s\n' "OpenSSL may prompt for the offline root key passphrase. Keep the root key disconnected after signing."
openssl x509 -req \
  -in "$csr_file" \
  -CA "$root_cert" \
  -CAkey "$root_key" \
  -CAserial "$temporary_serial" \
  -CAcreateserial \
  -days "$validity_days" \
  -sha256 \
  -extfile "$temporary_config" \
  -extensions v3_intermediate_ca \
  -out "$temporary_cert"

if ! openssl verify -CAfile "$root_cert" "$temporary_cert" >/dev/null; then
  printf '%s\n' "Signed intermediate did not validate against the offline root." >&2
  exit 1
fi
root_expiry="$(openssl x509 -in "$root_cert" -noout -enddate | cut -d= -f2-)"
intermediate_expiry="$(openssl x509 -in "$temporary_cert" -noout -enddate | cut -d= -f2-)"
if (( $(date -d "$intermediate_expiry" +%s) > $(date -d "$root_expiry" +%s) )); then
  printf '%s\n' "Signed intermediate would outlive the offline root certificate." >&2
  exit 1
fi
if ! openssl x509 -in "$temporary_cert" -noout -ext basicConstraints 2>/dev/null | grep -q 'CA:TRUE, pathlen:0'; then
  printf '%s\n' "Signed intermediate has unexpected basic constraints." >&2
  exit 1
fi
if ! openssl x509 -in "$temporary_cert" -noout -ext keyUsage 2>/dev/null | grep -q 'Certificate Sign'; then
  printf '%s\n' "Signed intermediate lacks keyCertSign usage." >&2
  exit 1
fi
chmod 0600 "$temporary_cert"
if ! ln -- "$temporary_cert" "$output"; then
  printf '%s\n' "Could not install the signed intermediate without overwriting an existing file." >&2
  exit 1
fi

printf 'Signed and verified intermediate written to %s (valid for %s days).\n' \
  "$output" "$validity_days"
printf '%s\n' "Transfer only this public certificate to the authorized Vault operator; never copy the root private key."
