#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
secret_dir="${root_dir}/infra/.local/secrets"
env_file="${root_dir}/.env"

umask 077
mkdir -p "${secret_dir}"
chmod 700 "${root_dir}/infra/.local" "${secret_dir}"
generate_secret() {
  openssl rand -hex 32
}

read_secret() {
  local key="$1"
  sed -n "s/^${key}=//p" "${env_file}" | head -n 1
}

if [[ -e "${env_file}" ]]; then
  pg_password="$(read_secret POSTGRES_PASSWORD)"
  pg_exporter_password="$(read_secret POSTGRES_EXPORTER_PASSWORD)"
  redis_password="$(read_secret REDIS_PASSWORD)"
  nats_password="$(read_secret NATS_PASSWORD)"
  grafana_password="$(read_secret GRAFANA_ADMIN_PASSWORD)"
  platform_metrics_token="$(read_secret PLATFORM_METRICS_TOKEN)"
  pasarela_metrics_token="$(read_secret PASARELA_METRICS_TOKEN)"
  for secret_value in "${pg_password}" "${pg_exporter_password}" "${redis_password}" "${nats_password}" "${grafana_password}" "${platform_metrics_token}" "${pasarela_metrics_token}"; do
    if [[ ! "${secret_value}" =~ ^[a-f0-9]{64}$ ]]; then
      printf '%s\n' '.env existe, pero falta un secreto generado válido; no se modificó.' >&2
      exit 1
    fi
  done
else
  pg_password="$(generate_secret)"
  pg_exporter_password="$(generate_secret)"
  redis_password="$(generate_secret)"
  nats_password="$(generate_secret)"
  grafana_password="$(generate_secret)"
  platform_metrics_token="$(generate_secret)"
  pasarela_metrics_token="$(generate_secret)"
  cat > "${env_file}" <<EOF
POSTGRES_DB=payvia_dev
POSTGRES_USER=payvia_dev
POSTGRES_PASSWORD=${pg_password}
POSTGRES_EXPORTER_PASSWORD=${pg_exporter_password}
REDIS_PASSWORD=${redis_password}
NATS_USER=payvia_dev
NATS_PASSWORD=${nats_password}
GRAFANA_ADMIN_USER=payvia_admin
GRAFANA_ADMIN_PASSWORD=${grafana_password}
PLATFORM_METRICS_TOKEN=${platform_metrics_token}
PASARELA_METRICS_TOKEN=${pasarela_metrics_token}
EOF
fi

printf '%s' "${platform_metrics_token}" > "${secret_dir}/platform_metrics_token"
printf '%s' "${pasarela_metrics_token}" > "${secret_dir}/pasarela_metrics_token"
cat > "${secret_dir}/redis.conf" <<EOF
appendonly yes
requirepass ${redis_password}
EOF
cat > "${secret_dir}/nats-server.conf" <<EOF
port: 4222
http_port: 8222
authorization {
  users = [
    { user: "payvia_dev", password: "${nats_password}" }
  ]
}
jetstream {
  store_dir: "/data"
}
EOF
chmod 600 "${env_file}"
chmod 644 "${secret_dir}"/*
printf '%s\n' 'Secretos locales generados en .env e infra/.local/secrets/. No se imprimieron en pantalla.'
