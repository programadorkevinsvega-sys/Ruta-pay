#!/usr/bin/env bash
set -euo pipefail

backend_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
workspace_root="$(cd -- "${backend_dir}/.." && pwd)"

cd "${workspace_root}"
exec cargo build --locked --release --manifest-path "${workspace_root}/Cargo.toml" --package bacekend-plataform --bin bacekend-plataform --bin migrate-platform
