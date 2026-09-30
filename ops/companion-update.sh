#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "$0")" && pwd)"
ROOT="${CPR_BASISPOINTS_ROOT:-/opt/cpr-basispoints}"
RELEASE_DIR="${CPR_BASISPOINTS_RELEASE_DIR:-$ROOT/release/codex-proxy-basispoints-worker}"
export CPR_BASISPOINTS_ROOT="$ROOT" CPR_BASISPOINTS_COMPOSE="$RELEASE_DIR/ops/compose.yaml"

WORKER_IMAGE="${WORKER_IMAGE:?WORKER_IMAGE must be an immutable GHCR image reference}"
PUBLIC_ORIGIN="${PUBLIC_ORIGIN:?PUBLIC_ORIGIN is required}"
export WORKER_IMAGE PUBLIC_ORIGIN
bash "$RELEASE_DIR/ops/deploy.sh"
