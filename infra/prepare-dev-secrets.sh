#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
secret_dir="${root_dir}/infra/.local/secrets"
env_file="${root_dir}/bacekend-plataform/.env"
htpasswd_image="httpd:2.4.69-alpine3.24"

umask 077
mkdir -p "${secret_dir}"
chmod 700 "${root_dir}/infra/.local" "${secret_dir}"
if ! command -v docker >/dev/null 2>&1 || ! docker info >/dev/null 2>&1; then
  printf '%s\n' 'Docker debe estar instalado y activo para preparar los hashes de autenticación local.' >&2
  exit 1
fi
generate_secret() {
  openssl rand -hex 32
}

read_secret() {
  local key="$1"
  sed -n "s/^${key}=//p" "${env_file}" | head -n 1
}

ensure_value() {
  local key="$1"
  local default_value="$2"
  local configured_value
  configured_value="$(read_secret "${key}")"
  if [[ -z "${configured_value}" ]]; then
    configured_value="${default_value}"
    printf '%s=%s\n' "${key}" "${configured_value}" >> "${env_file}"
  fi
  printf -v "${key}" '%s' "${configured_value}"
}

ensure_secret() {
  local key="$1"
  local configured_value
  configured_value="$(read_secret "${key}")"
  if [[ -z "${configured_value}" ]]; then
    configured_value="$(generate_secret)"
    printf '%s=%s\n' "${key}" "${configured_value}" >> "${env_file}"
  elif [[ ! "${configured_value}" =~ ^[a-f0-9]{64}$ ]]; then
    printf 'El valor local %s no es un secreto generado válido; no se modificó.\n' "${key}" >&2
    exit 1
  fi
  printf -v "${key}" '%s' "${configured_value}"
}

write_htpasswd() {
  local username="$1"
  local password="$2"
  local output_path="$3"
  printf '%s\n' "${password}" | docker run --rm -i --entrypoint htpasswd "${htpasswd_image}" -B -i -n "${username}" > "${output_path}"
  chmod 600 "${output_path}"
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
      printf '%s\n' 'bacekend-plataform/.env existe, pero falta un secreto generado válido; no se modificó.' >&2
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

ensure_value PGADMIN_DEFAULT_EMAIL admin@payvia.dev
ensure_secret PGADMIN_DEFAULT_PASSWORD
ensure_value PROMETHEUS_ADMIN_USER payvia_metrics_admin
ensure_secret PROMETHEUS_ADMIN_PASSWORD
ensure_value INFRA_UI_USER payvia_ops
ensure_secret INFRA_UI_PASSWORD
ensure_secret REDISINSIGHT_ENCRYPTION_KEY
ensure_value NATS_SURVEYOR_USER payvia_surveyor
ensure_secret NATS_SURVEYOR_PASSWORD
ensure_value NATS_SURVEYOR_HTTP_USER payvia_scraper
ensure_secret NATS_SURVEYOR_HTTP_PASSWORD
ensure_value NATS_USER payvia_dev

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
    { user: "${NATS_USER}", password: "${nats_password}" }
  ]
}
accounts {
  SYS {
    users = [
      {
        user: "${NATS_SURVEYOR_USER}"
        password: "${NATS_SURVEYOR_PASSWORD}"
      }
    ]
  }
}
system_account: SYS
jetstream {
  store_dir: "/data"
}
EOF
chmod 600 "${env_file}"
printf '%s' "${platform_metrics_token}" > "${secret_dir}/platform_metrics_token"
printf '%s' "${pasarela_metrics_token}" > "${secret_dir}/pasarela_metrics_token"
printf '%s' "${PROMETHEUS_ADMIN_PASSWORD}" > "${secret_dir}/prometheus_admin_password"
printf '%s' "${NATS_SURVEYOR_HTTP_PASSWORD}" > "${secret_dir}/nats_surveyor_http_password"
printf 'basic_auth_users:\n  "%s": "%s"\n' "${PROMETHEUS_ADMIN_USER}" "$(printf '%s\n' "${PROMETHEUS_ADMIN_PASSWORD}" | docker run --rm -i --entrypoint htpasswd "${htpasswd_image}" -B -i -n "${PROMETHEUS_ADMIN_USER}" | cut -d: -f2-)" > "${secret_dir}/prometheus-web.yml"
write_htpasswd "${INFRA_UI_USER}" "${INFRA_UI_PASSWORD}" "${secret_dir}/infra-ui.htpasswd"
chmod 644 "${secret_dir}"/*
printf '%s\n' 'Secretos locales preparados en bacekend-plataform/.env e infra/.local/secrets/. No se imprimieron en pantalla.'
