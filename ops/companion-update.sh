#!/usr/bin/env bash
set -euo pipefail

ROOT="${CPR_TWOFA_ROOT:-/opt/cpr-twofa}"
RELEASE_DIR="${CPR_TWOFA_RELEASE_DIR:-$ROOT/release/codex-proxy-twofa-worker}"
export CPR_TWOFA_ROOT="$ROOT" CPR_TWOFA_COMPOSE="$RELEASE_DIR/ops/compose.yaml"
if [[ "${1:-}" == "--rollback" ]]; then
  bash "$RELEASE_DIR/ops/rollback.sh"
else
  WORKER_IMAGE="${WORKER_IMAGE:?WORKER_IMAGE must be an immutable GHCR image reference}"
  PUBLIC_ORIGIN="${PUBLIC_ORIGIN:?PUBLIC_ORIGIN is required}"
  export WORKER_IMAGE PUBLIC_ORIGIN
  bash "$RELEASE_DIR/ops/deploy.sh"
fi
