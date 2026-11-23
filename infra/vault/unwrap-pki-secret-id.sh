#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "${script_dir}/vault-cli-common.sh"

if [[ $# -ne 3 ]]; then
  printf 'Usage: %s <staging|production> <wrapped-secret-id-file> <destination-secret-id-file>\n' "$0" >&2
  exit 2
fi

environment="$1"
wrapped_file="$2"
destination="$3"
destination_dir="$(dirname -- "$destination")"

if [[ ! -f "$wrapped_file" || -L "$wrapped_file" || ! -O "$wrapped_file" || ! -r "$wrapped_file" ]]; then
  printf '%s\n' "Wrapped SecretID must be a readable regular file owned by the current user, not a symlink." >&2
  exit 2
fi
wrapped_mode="$(stat -c '%a' "$wrapped_file")"
if (( (8#$wrapped_mode & 077) != 0 )); then
  printf '%s\n' "Wrapped SecretID file must not grant access to group or other users (use chmod 600)." >&2
  exit 2
fi
if [[ ! -d "$destination_dir" || ! -O "$destination_dir" ]]; then
  printf '%s\n' "Destination directory must exist and be owned by the current user." >&2
  exit 2
fi
destination_mode="$(stat -c '%a' "$destination_dir")"
if (( (8#$destination_mode & 077) != 0 )); then
  printf '%s\n' "Destination directory must not grant access to group or other users (use chmod 700)." >&2
  exit 2
fi
if [[ -e "$destination" || -L "$destination" ]]; then
  printf '%s\n' "Destination already exists; refusing to overwrite a SecretID." >&2
  exit 1
fi
if [[ -z "${VAULT_ADDR:-}" || "$VAULT_ADDR" != https://* || -n "${VAULT_SKIP_VERIFY:-}" ]]; then
  printf '%s\n' "Set VAULT_ADDR to the verified Vault HTTPS address; VAULT_SKIP_VERIFY is not allowed." >&2
  exit 2
fi
command -v vault >/dev/null 2>&1 || {
  printf '%s\n' "The Vault CLI is required." >&2
  exit 127
}
command -v jq >/dev/null 2>&1 || {
  printf '%s\n' "jq is required." >&2
  exit 127
}

vault_confirm_target "$environment"

secret_id_temporary=""
cleanup() {
  unset wrapped_token unwrap_response secret_id
  if [[ -n "${secret_id_temporary:-}" ]]; then
    rm -f -- "$secret_id_temporary"
  fi
}
trap cleanup EXIT

wrapped_token="$(<"$wrapped_file")"
if [[ -z "$wrapped_token" ]]; then
  printf '%s\n' "Wrapped SecretID file is empty." >&2
  exit 1
fi
if ! unwrap_response="$(VAULT_TOKEN="$wrapped_token" vault unwrap -format=json)"; then
  unset wrapped_token
  printf '%s\n' "Vault could not unwrap the SecretID; verify the target address and that the 15-minute wrapping token is unused." >&2
  exit 1
fi
unset wrapped_token

if ! secret_id="$(jq -er '.data.secret_id | strings | select(length > 0)' <<<"$unwrap_response")"; then
  unset unwrap_response
  printf '%s\n' "Vault unwrap response did not contain a SecretID." >&2
  exit 1
fi
unset unwrap_response
if [[ "$secret_id" == *$'\n'* ]]; then
  unset secret_id
  printf '%s\n' "Vault returned an invalid multiline SecretID." >&2
  exit 1
fi

umask 077
secret_id_temporary="$(mktemp "${destination_dir}/.secret-id.XXXXXX")"
if ! printf '%s' "$secret_id" >"$secret_id_temporary"; then
  unset secret_id
  rm -f -- "$secret_id_temporary"
  secret_id_temporary=""
  printf '%s\n' "Could not write the unwrapped SecretID; the wrapping token has already been consumed." >&2
  exit 1
fi
unset secret_id
chmod 0400 "$secret_id_temporary"
if ! ln -- "$secret_id_temporary" "$destination"; then
  rm -f -- "$secret_id_temporary"
  secret_id_temporary=""
  printf '%s\n' "Could not install the SecretID file; the wrapping token has already been consumed." >&2
  exit 1
fi
rm -f -- "$secret_id_temporary"
secret_id_temporary=""
if ! rm -- "$wrapped_file"; then
  printf '%s\n' "SecretID was saved, but the consumed wrapping-token file could not be removed." >&2
  exit 1
fi

printf 'Stored the reusable 30-day SecretID at %s with mode 0400 and removed the consumed wrapping token.\n' \
  "$destination"
