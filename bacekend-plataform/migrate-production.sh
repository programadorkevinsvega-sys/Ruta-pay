#!/usr/bin/env bash
set -euo pipefail

backend_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
workspace_root="$(cd -- "${backend_dir}/.." && pwd)"
environment_file="${backend_dir}/.env"

if [[ ! -r "${environment_file}" ]]; then
    printf '%s\n' 'No se puede leer bacekend-plataform/.env.' >&2
    exit 1
fi

set -a
source "${environment_file}"
set +a

if [[ -z "${PLATFORM_MIGRATION_DATABASE_URL:-}" ]]; then
    printf '%s\n' 'Falta PLATFORM_MIGRATION_DATABASE_URL en bacekend-plataform/.env.' >&2
    exit 1
fi

target_directory="${CARGO_TARGET_DIR:-${workspace_root}/target}"
binary_path="${PLATFORM_MIGRATOR_BINARY_PATH:-${target_directory}/release/migrate-platform}"

if [[ ! -x "${binary_path}" ]]; then
    printf '%s\n' 'No existe el migrador release de plataforma; ejecuta bacekend-plataform/build-production.sh.' >&2
    exit 1
fi

cd "${workspace_root}"
exec "${binary_path}"
