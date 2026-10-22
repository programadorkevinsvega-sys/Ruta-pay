#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
sentry_version="26.10.0"
sentry_dir="${root_dir}/infra/.local/sentry-self-hosted-${sentry_version}"

if ! command -v git >/dev/null || ! command -v docker >/dev/null; then
  printf '%s\n' 'Se requieren git y Docker con Docker Compose plugin para instalar Sentry.' >&2
  exit 1
fi

mkdir -p "${root_dir}/infra/.local"
if [[ -e "${sentry_dir}" ]]; then
  printf 'Ya existe %s; revísalo antes de volver a ejecutar el instalador.\n' "${sentry_dir}" >&2
  exit 1
fi

git clone --depth 1 --branch "${sentry_version}" https://github.com/getsentry/self-hosted.git "${sentry_dir}"
git -C "${sentry_dir}" describe --tags --exact-match | grep -Fx "${sentry_version}" >/dev/null
cd "${sentry_dir}"
./install.sh
docker compose up -d
