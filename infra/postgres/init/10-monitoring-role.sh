#!/usr/bin/env bash
set -euo pipefail

psql --username "${POSTGRES_USER}" --dbname "${POSTGRES_DB}" \
  --set=exporter_password="${POSTGRES_EXPORTER_PASSWORD}" <<'SQL'
CREATE ROLE postgres_exporter LOGIN PASSWORD :'exporter_password';
GRANT pg_monitor TO postgres_exporter;
SQL
