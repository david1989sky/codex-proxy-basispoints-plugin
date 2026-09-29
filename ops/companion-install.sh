#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "$0")" && pwd)"
ROOT="${CPR_TWOFA_ROOT:-/opt/cpr-twofa}"
RELEASE_DIR="${CPR_TWOFA_RELEASE_DIR:-$ROOT/release/codex-proxy-twofa-worker}"
PUBLIC_ORIGIN="${PUBLIC_ORIGIN:?PUBLIC_ORIGIN is required}"
WORKER_IMAGE="${WORKER_IMAGE:?WORKER_IMAGE must be an immutable GHCR image reference}"

case "$WORKER_IMAGE" in *@sha256:* ) ;; *) printf '%s\n' 'WORKER_IMAGE must include @sha256.' >&2; exit 1 ;; esac

SOURCE_DIR="${WORKER_SOURCE_DIR:-$SCRIPT_DIR/../worker}"
TEMP_DIR=""
if [[ ! -d "$SOURCE_DIR" ]]; then
  BUNDLE="${WORKER_BUNDLE:?WORKER_BUNDLE is required when the worker source directory is unavailable}"
  TEMP_DIR="$(mktemp -d)"
  tar -xzf "$BUNDLE" -C "$TEMP_DIR"
  SOURCE_DIR="$TEMP_DIR/worker"
fi
trap '[[ -n "$TEMP_DIR" ]] && rm -rf "$TEMP_DIR"' EXIT
[[ -f "$SOURCE_DIR/ops/compose.yaml" && -f "$SOURCE_DIR/ops/provision-vault.sh" ]] || { printf '%s\n' 'Worker bundle is incomplete.' >&2; exit 1; }

mkdir -p "$RELEASE_DIR/ops"
cp -p "$SOURCE_DIR/Dockerfile" "$SOURCE_DIR/entrypoint.sh" "$SOURCE_DIR/package.json" "$SOURCE_DIR/package-lock.json" "$RELEASE_DIR/"
cp -p "$SOURCE_DIR/ops/compose.yaml" "$SOURCE_DIR/ops/provision-vault.sh" "$SOURCE_DIR/ops/deploy.sh" "$SOURCE_DIR/ops/rollback.sh" "$RELEASE_DIR/ops/"
chmod 0555 "$RELEASE_DIR/entrypoint.sh" "$RELEASE_DIR/ops/"*.sh
export CPR_TWOFA_ROOT="$ROOT" CPR_TWOFA_COMPOSE="$RELEASE_DIR/ops/compose.yaml"
bash "$RELEASE_DIR/ops/deploy.sh"
