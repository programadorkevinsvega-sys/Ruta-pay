#!/usr/bin/env bash
set -euo pipefail

backend_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
workspace_root="$(cd -- "${backend_dir}/.." && pwd)"
frontend_name="${1:?usage: run-browser-e2e-backend.sh frontend-pasarela|frontend-plataform}"
case "${frontend_name}" in
    frontend-pasarela|frontend-plataform) frontend_dir="${workspace_root}/frontends/${frontend_name}" ;;
    *) printf 'Unsupported frontend: %s\n' "${frontend_name}" >&2; exit 2 ;;
esac
browser_script="${PAYVIA_E2E_BROWSER_SCRIPT:-test:e2e:browser}"
case "${browser_script}" in
    test:e2e:browser|test:e2e:recovery:browser) ;;
    *) printf 'Unsupported browser test script: %s\n' "${browser_script}" >&2; exit 2 ;;
esac
set -a
source "${backend_dir}/.env"
set +a

: "${PLATFORM_TEST_DATABASE_ADMIN_URL:?PLATFORM_TEST_DATABASE_ADMIN_URL must point to local development PostgreSQL}"
: "${PLATFORM_REDIS_URL:?PLATFORM_REDIS_URL must point to local development Redis}"
: "${PLATFORM_NATS_URL:?PLATFORM_NATS_URL must point to local development JetStream}"
: "${NATS_USER:?NATS_USER is required}"
: "${NATS_PASSWORD:?NATS_PASSWORD is required}"

suffix="$(python3 -c 'import secrets; print(secrets.token_hex(8))')"
database_name="payvia_browser_e2e_${suffix}"
runtime_role="payvia_e2e_${suffix}"
runtime_password="$(python3 -c 'import secrets; print(secrets.token_urlsafe(36))')"
migration_role="payvia_migrator_e2e_${suffix}"
migration_password="$(python3 -c 'import secrets; print(secrets.token_urlsafe(36))')"
operator_email="e2e_${suffix}@example.test"
operator_password="$(python3 -c 'import secrets; print("Aa1!" + secrets.token_urlsafe(30))')"
credentials_file="/tmp/payvia-platform-e2e-operator.json"
backend_log="$(mktemp)"
frontend_log="$(mktemp)"
second_backend_log="$(mktemp)"
second_frontend_log="$(mktemp)"
backend_pid=""
frontend_pid=""
second_backend_pid=""
second_frontend_pid=""
admin_url="${PLATFORM_TEST_DATABASE_ADMIN_URL}"
mapfile -t e2e_ports < <(python3 - <<'PYE2EPORTS'
import socket

reservations = []
for _ in range(8):
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.bind(("127.0.0.1", 0))
    reservations.append(listener)
for listener in reservations:
    print(listener.getsockname()[1])
for listener in reservations:
    listener.close()
PYE2EPORTS
)
if [[ "${#e2e_ports[@]}" -ne 8 ]]; then
    printf 'Could not reserve loopback ports for browser acceptance\n' >&2
    exit 1
fi
app_http_port="${e2e_ports[0]}"
app_grpc_port="${e2e_ports[1]}"
app_metrics_port="${e2e_ports[2]}"
frontend_port="${e2e_ports[3]}"
second_app_http_port="${e2e_ports[4]}"
second_app_grpc_port="${e2e_ports[5]}"
second_app_metrics_port="${e2e_ports[6]}"
second_frontend_port="${e2e_ports[7]}"
export PAYVIA_E2E_FRONTEND_ORIGIN="http://localhost:${frontend_port}"
export PAYVIA_E2E_SECOND_FRONTEND_ORIGIN="http://localhost:${second_frontend_port}"
urls="$(python3 - "${admin_url}" "${database_name}" "${migration_role}" "${migration_password}" "${runtime_role}" "${runtime_password}" <<'PYURL'
import sys
from urllib.parse import urlsplit, urlunsplit, quote
source, database, migration_role, migration_password, runtime_role, runtime_password = sys.argv[1:]
url = urlsplit(source)
def build(database_name, username, secret):
    netloc = f"{quote(username, safe='')}:{quote(secret, safe='')}@{url.hostname}"
    if url.port is not None:
        netloc += f":{url.port}"
    return urlunsplit((url.scheme, netloc, f"/{database_name}", url.query, url.fragment))
print(build(database, migration_role, migration_password))
print(build(database, runtime_role, runtime_password))
PYURL
)"
migration_url="$(printf '%s\n' "${urls}" | sed -n '1p')"
database_url="$(printf '%s\n' "${urls}" | sed -n '2p')"
stop_tree() {
    local parent="$1"
    local child
    while read -r child; do
        [[ -n "${child}" ]] && stop_tree "${child}"
    done < <(pgrep -P "${parent}" || true)
    kill -TERM "${parent}" 2>/dev/null || true
}
cleanup() {
    if [[ -n "${frontend_pid}" ]]; then stop_tree "${frontend_pid}"; wait "${frontend_pid}" 2>/dev/null || true; fi
    if [[ -n "${backend_pid}" ]]; then stop_tree "${backend_pid}"; wait "${backend_pid}" 2>/dev/null || true; fi
    if [[ -n "${second_frontend_pid}" ]]; then stop_tree "${second_frontend_pid}"; wait "${second_frontend_pid}" 2>/dev/null || true; fi
    if [[ -n "${second_backend_pid}" ]]; then stop_tree "${second_backend_pid}"; wait "${second_backend_pid}" 2>/dev/null || true; fi
    rm -f "${credentials_file}" "${backend_log}" "${frontend_log}" "${second_backend_log}" "${second_frontend_log}"
    psql "${admin_url}" -qAtc "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '${database_name}'" >/dev/null || true
    dropdb --if-exists --maintenance-db="${admin_url}" "${database_name}" || true
    psql "${admin_url}" -qAtc "DROP ROLE IF EXISTS \"${runtime_role}\"" >/dev/null || true
    psql "${admin_url}" -qAtc "DROP ROLE IF EXISTS \"${migration_role}\"" >/dev/null || true
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

createdb --maintenance-db="${admin_url}" "${database_name}"
psql "${admin_url}" -v ON_ERROR_STOP=1 -c "CREATE ROLE \"${migration_role}\" LOGIN PASSWORD '${migration_password}' NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS" >/dev/null
psql "${admin_url}" -v ON_ERROR_STOP=1 -c "GRANT CONNECT, CREATE ON DATABASE \"${database_name}\" TO \"${migration_role}\"" >/dev/null
psql "${admin_url}" -v ON_ERROR_STOP=1 -c "CREATE ROLE \"${runtime_role}\" LOGIN PASSWORD '${runtime_password}' NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS" >/dev/null
psql "${admin_url}" -v ON_ERROR_STOP=1 -c "GRANT CONNECT ON DATABASE \"${database_name}\" TO \"${runtime_role}\"" >/dev/null
PLATFORM_MIGRATION_DATABASE_URL="${migration_url}" PLATFORM_DATABASE_URL="${database_url}" PLATFORM_RUNTIME_DB_ROLE="${runtime_role}" cargo run --locked --manifest-path "${workspace_root}/Cargo.toml" -p bacekend-plataform --bin migrate-platform
psql "${migration_url}" -v ON_ERROR_STOP=1 -c "GRANT USAGE ON SCHEMA identity_access, tenant_registry, realtime TO \"${runtime_role}\"; GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA identity_access, tenant_registry, realtime TO \"${runtime_role}\"; GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA identity_access, tenant_registry, realtime TO \"${runtime_role}\"; ALTER DEFAULT PRIVILEGES IN SCHEMA identity_access, tenant_registry, realtime GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO \"${runtime_role}\"; ALTER DEFAULT PRIVILEGES IN SCHEMA identity_access, tenant_registry, realtime GRANT USAGE, SELECT ON SEQUENCES TO \"${runtime_role}\"" >/dev/null
export PLATFORM_DATABASE_URL="${database_url}"
export PLATFORM_EVENT_CONSUMER_NAMESPACE="E2E_${suffix}"
export E2E_OPERATOR_EMAIL="${operator_email}"
export E2E_OPERATOR_PASSWORD="${operator_password}"
E2E_OPERATOR_CREDENTIALS="${credentials_file}" python3 -c 'import json, os; path=os.environ["E2E_OPERATOR_CREDENTIALS"]; os.umask(0o077); open(path,"w").write(json.dumps({"email":os.environ["E2E_OPERATOR_EMAIL"],"password":os.environ["E2E_OPERATOR_PASSWORD"]}))'
printf '%s\n%s\n' "${operator_password}" "${operator_password}" | cargo run --locked --manifest-path "${workspace_root}/Cargo.toml" -p bacekend-plataform --bin bootstrap-platform-operator -- "${operator_email}" --password-stdin
export APP_ENV=test
export APP_HTTP_ADDR="127.0.0.1:${app_http_port}"
export APP_MODULE_GRPC_ADDR="127.0.0.1:${app_grpc_port}"
export APP_METRICS_ADDR="127.0.0.1:${app_metrics_port}"
cargo run --locked --manifest-path "${workspace_root}/Cargo.toml" -p bacekend-plataform --bin bacekend-plataform >"${backend_log}" 2>&1 &
backend_pid=$!
(cd "${frontend_dir}" && VITE_PLATFORM_BFF_URL="http://127.0.0.1:${app_http_port}" exec npm run dev -- --host localhost --port "${frontend_port}") >"${frontend_log}" 2>&1 &
frontend_pid=$!
if [[ "${PAYVIA_SECOND_REPLICA:-0}" == "1" ]]; then
    APP_MODULE_GRPC_ADDR="127.0.0.1:${second_app_grpc_port}" APP_HTTP_ADDR="127.0.0.1:${second_app_http_port}" APP_METRICS_ADDR="127.0.0.1:${second_app_metrics_port}" \
        cargo run --locked --manifest-path "${workspace_root}/Cargo.toml" -p bacekend-plataform --bin bacekend-plataform >"${second_backend_log}" 2>&1 &
    second_backend_pid=$!
    (cd "${frontend_dir}" && VITE_PLATFORM_BFF_URL="http://127.0.0.1:${second_app_http_port}" exec npm run dev -- --host localhost --port "${second_frontend_port}") >"${second_frontend_log}" 2>&1 &
    second_frontend_pid=$!
fi
for attempt in $(seq 1 120); do
    if curl --fail --silent "http://127.0.0.1:${app_http_port}/health/live" >/dev/null && curl --fail --silent "http://localhost:${frontend_port}/" >/dev/null; then break; fi
    if ! kill -0 "${backend_pid}" 2>/dev/null; then cat "${backend_log}"; exit 1; fi
    if [[ "${attempt}" == 120 ]]; then cat "${backend_log}" "${frontend_log}"; exit 1; fi
    sleep 1
done
if [[ -n "${second_backend_pid}" ]]; then
    for attempt in $(seq 1 120); do
        if curl --fail --silent "http://127.0.0.1:${second_app_http_port}/health/live" >/dev/null && curl --fail --silent "http://localhost:${second_frontend_port}/" >/dev/null; then break; fi
        if ! kill -0 "${second_backend_pid}" 2>/dev/null; then cat "${second_backend_log}"; exit 1; fi
        if [[ "${attempt}" == 120 ]]; then cat "${second_backend_log}" "${second_frontend_log}"; exit 1; fi
        sleep 1
    done
fi
(cd "${frontend_dir}" && npm run "${browser_script}")
