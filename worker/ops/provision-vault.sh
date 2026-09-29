#!/usr/bin/env bash
set -euo pipefail
ROOT="${CPR_TWOFA_ROOT:-/opt/cpr-twofa}"
VAULT_DIR="$ROOT/data/credentials"
KEY_DIR="$ROOT/secrets"
KEY_FILE="$ROOT/secrets/twofa-key"
IMAGE="${WORKER_IMAGE:?WORKER_IMAGE must identify the immutable worker image}"
WORKER_UID=$(docker run --rm --entrypoint id "$IMAGE" -u)
WORKER_GID=$(docker run --rm --entrypoint id "$IMAGE" -g)
[[ "$WORKER_UID" =~ ^[0-9]+$ && "$WORKER_GID" =~ ^[0-9]+$ ]]
[[ ! -L "$VAULT_DIR" && ! -L "$KEY_FILE" ]]
install -d -m 0700 -o "$WORKER_UID" -g "$WORKER_GID" "$VAULT_DIR"
install -d -m 0700 "$KEY_DIR"
if [[ ! -e "$KEY_FILE" ]]; then
  if [[ -n "$(find "$VAULT_DIR" -mindepth 1 -maxdepth 1 -print -quit)" ]]; then
    printf '%s\n' 'Vault data exists without its key; restore the original key.' >&2
    exit 1
  fi
  umask 077
  openssl rand -out "$KEY_FILE" 32
fi
[[ -f "$KEY_FILE" && "$(wc -c < "$KEY_FILE")" -eq 32 ]]
chown "$WORKER_UID:$WORKER_GID" "$KEY_FILE"
chmod 0400 "$KEY_FILE"
printf '%s\n' 'Credential vault paths and permissions ready.'
