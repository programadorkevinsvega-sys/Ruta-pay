#!/usr/bin/env bash
set -euo pipefail

domain="vault.example.invalid"
certificate_dir="/opt/vault/tls"
temporary_dir="$(mktemp -d "${certificate_dir}/.renew.XXXXXX")"

cleanup() {
  rm -rf -- "$temporary_dir"
}
trap cleanup EXIT

tailscale cert \
  --min-validity=720h \
  --cert-file "${temporary_dir}/tailscale.crt" \
  --key-file "${temporary_dir}/tailscale.key" \
  "$domain"

openssl x509 -in "${temporary_dir}/tailscale.crt" -noout -checkhost "$domain"
openssl x509 -in "${temporary_dir}/tailscale.crt" -noout -checkend 1209600
openssl pkey -in "${temporary_dir}/tailscale.key" -noout
certificate_key_hash="$(
  openssl x509 -in "${temporary_dir}/tailscale.crt" -pubkey -noout |
    openssl pkey -pubin -outform DER |
    sha256sum |
    cut -d ' ' -f 1
)"
private_key_hash="$(
  openssl pkey -in "${temporary_dir}/tailscale.key" -pubout -outform DER |
    sha256sum |
    cut -d ' ' -f 1
)"
if [[ "$certificate_key_hash" != "$private_key_hash" ]]; then
  printf '%s\n' "Tailscale TLS certificate and private key do not match." >&2
  exit 1
fi

install -o 100 -g 1000 -m 0644 "${temporary_dir}/tailscale.crt" "${certificate_dir}/tailscale.crt.new"
install -o 100 -g 1000 -m 0600 "${temporary_dir}/tailscale.key" "${certificate_dir}/tailscale.key.new"
mv -f "${certificate_dir}/tailscale.crt.new" "${certificate_dir}/tailscale.crt"
mv -f "${certificate_dir}/tailscale.key.new" "${certificate_dir}/tailscale.key"

container_id="$(docker ps -q --filter 'name=^/shared-vault$')"
if [[ -n "$container_id" ]]; then
  docker kill --signal=HUP "$container_id"
else
  printf '%s\n' "Vault is not running; the certificate will be used on its next start."
fi
