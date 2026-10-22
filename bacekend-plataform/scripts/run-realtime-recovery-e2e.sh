#!/usr/bin/env bash
set -euo pipefail

backend_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
workspace_root="$(cd -- "${backend_dir}/.." && pwd)"
set -a
source "${backend_dir}/.env"
set +a

python3 - "${PLATFORM_TEST_DATABASE_ADMIN_URL:?}" "${PLATFORM_REDIS_URL:?}" "${PLATFORM_NATS_URL:?}" <<'PY'
import sys
from urllib.parse import urlsplit

for value in sys.argv[1:]:
    endpoint = urlsplit(value)
    if endpoint.hostname not in {"localhost", "127.0.0.1", "::1"}:
        raise SystemExit("Recovery acceptance is restricted to loopback development services")
PY

for container in payvia-shared-infra-redis-1 payvia-shared-infra-nats-1; do
    docker inspect "${container}" >/dev/null
done

export PAYVIA_EVENT_RESTART_ACCEPTANCE=1
export PAYVIA_SECOND_REPLICA=1
export PAYVIA_E2E_BROWSER_SCRIPT=test:e2e:recovery:browser
exec bash "${backend_dir}/scripts/run-browser-e2e-backend.sh" frontend-plataform
