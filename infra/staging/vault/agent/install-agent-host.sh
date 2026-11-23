#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 || ! "$1" =~ ^(postgres|redis|nats)$ ]]; then
  printf 'Usage: %s <postgres|redis|nats>\n' "$0" >&2
  exit 2
fi
if (( EUID != 0 )); then
  printf '%s\n' "Run this installer as root." >&2
  exit 1
fi

service="$1"
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
stage_dir="$script_dir"
config_source="${stage_dir}/${service}.hcl"
role_id_source="${script_dir}/${service}-role-id"
wrapped_secret_id_source="${script_dir}/${service}-secret-id.wrap"
installed_role_id="/etc/payvia/staging/vault-agent/role_id"
installed_secret_id="/etc/payvia/staging/vault-agent/secret_id"

required_files=(
  "${stage_dir}/vault" \
  "${stage_dir}/payvia-install-pki-certificate.py" \
  "${stage_dir}/payvia-vault-agent.service" \
  "$config_source" \
  "$role_id_source" \
  "$wrapped_secret_id_source"
)
if [[ "$service" == "redis" ]]; then
  required_files+=("${stage_dir}/payvia-install-redis-tls-rotator.py")
fi

for required in "${required_files[@]}"; do
  if [[ ! -f "$required" || -L "$required" ]]; then
    printf 'Required staged file is missing or not regular: %s\n' "$required" >&2
    exit 1
  fi
done
if [[ "$(stat -c '%a' "$wrapped_secret_id_source")" != 600 ]]; then
  printf '%s\n' "Wrapped SecretID file must have mode 0600." >&2
  exit 1
fi
if [[ -e "$installed_role_id" || -e "$installed_secret_id" ]]; then
  printf '%s\n' "An AppRole credential is already installed; inspect it before continuing." >&2
  exit 1
fi

case "$service" in
  postgres)
    template_names=(postgres-server postgres-exporter)
    ;;
  redis)
    template_names=(redis-server redis-healthcheck redis-exporter redis-platform-runtime redis-tls-rotator)
    ;;
  nats)
    template_names=(nats-server)
    ;;
esac
for template_name in "${template_names[@]}"; do
  if [[ ! -f "${stage_dir}/templates/${template_name}.ctmpl" ]]; then
    printf 'Missing template: %s\n' "${template_name}.ctmpl" >&2
    exit 1
  fi
done

install -d -o root -g root -m 0700 /etc/payvia/staging/vault-agent
install -d -o root -g root -m 0700 \
  /etc/payvia/staging/vault-agent/templates \
  /etc/payvia/staging/vault-agent/rendered
install -o root -g root -m 0755 "${stage_dir}/vault" /usr/local/bin/vault
install -o root -g root -m 0755 \
  "${stage_dir}/payvia-install-pki-certificate.py" \
  /usr/local/libexec/payvia-install-pki-certificate.py
if [[ "$service" == "redis" ]]; then
  install -d -o root -g root -m 0755 \
    /etc/payvia/staging/redis/rotation-credentials
  install -o root -g root -m 0444 /dev/null \
    /etc/payvia/staging/redis/rotation-credentials/acl-activation-pending
  install -o root -g root -m 0755 \
    "${stage_dir}/payvia-install-redis-tls-rotator.py" \
    /usr/local/libexec/payvia-install-redis-tls-rotator.py
fi
install -o root -g root -m 0600 "$config_source" \
  /etc/payvia/staging/vault-agent/agent.hcl
install -o root -g root -m 0444 "$role_id_source" \
  /etc/payvia/staging/vault-agent/role_id
for template_name in "${template_names[@]}"; do
  install -o root -g root -m 0600 \
    "${stage_dir}/templates/${template_name}.ctmpl" \
    "/etc/payvia/staging/vault-agent/templates/${template_name}.ctmpl"
done

wrapped_temporary="/etc/payvia/staging/vault-agent/.secret-id.wrap"
secret_id_temporary="/etc/payvia/staging/vault-agent/.secret-id.temporary"
install -o root -g root -m 0600 "$wrapped_secret_id_source" "$wrapped_temporary"
cleanup() {
  rm -f -- "$wrapped_temporary" "$secret_id_temporary"
}
trap cleanup EXIT

wrapping_token="$(<"$wrapped_temporary")"
if [[ -z "$wrapping_token" || "$wrapping_token" == *$'\n'* ]]; then
  unset wrapping_token
  printf '%s\n' "Wrapped SecretID is empty or malformed." >&2
  exit 1
fi
if ! VAULT_ADDR=https://interserver-vault.tail4eafc5.ts.net:8190 \
  VAULT_TOKEN="$wrapping_token" \
  /usr/local/bin/vault unwrap -field=secret_id >"$secret_id_temporary"; then
  unset wrapping_token
  printf '%s\n' "Could not unwrap the AppRole SecretID; inspect Vault audit logs before retrying." >&2
  exit 1
fi
unset wrapping_token
if [[ ! -s "$secret_id_temporary" ]]; then
  printf '%s\n' "Vault returned an empty AppRole SecretID." >&2
  exit 1
fi
chmod 0400 "$secret_id_temporary"
mv -- "$secret_id_temporary" /etc/payvia/staging/vault-agent/secret_id
rm -f -- "$wrapped_temporary" "$wrapped_secret_id_source"
trap - EXIT

install -o root -g root -m 0644 \
  "${stage_dir}/payvia-vault-agent.service" \
  /etc/systemd/system/payvia-vault-agent.service
sed -i "s|/etc/payvia/staging/service|/etc/payvia/staging/${service}|g" \
  /etc/systemd/system/payvia-vault-agent.service

for template_name in "${template_names[@]}"; do
  if ! grep -Fq "templates/${template_name}.ctmpl" "$config_source"; then
    printf 'Template configuration does not reference %s.\n' "$template_name" >&2
    exit 1
  fi
done

/usr/local/bin/vault agent \
  -config=/etc/payvia/staging/vault-agent/agent.hcl \
  -exit-after-auth
systemctl daemon-reload
systemctl enable --now payvia-vault-agent.service
systemctl --no-pager --full status payvia-vault-agent.service
printf 'Vault Agent installed and enabled for %s; no data-service container was started or restarted.\n' "$service"
