#!/usr/bin/env bash
set -euo pipefail

ROOT="${CPR_BASISPOINTS_ROOT:-/opt/cpr-basispoints}"
COMPOSE_FILE="${CPR_BASISPOINTS_COMPOSE:-$ROOT/release/codex-proxy-basispoints-worker/ops/compose.yaml}"
COMPOSE_DIR="$(dirname -- "$COMPOSE_FILE")"
IMAGE="${WORKER_IMAGE:?WORKER_IMAGE must be an immutable GHCR image reference}"
ORIGIN="${PUBLIC_ORIGIN:?PUBLIC_ORIGIN is required}"

case "$IMAGE" in
  *@sha256:* ) ;;
  *) printf '%s\n' 'WORKER_IMAGE must include an immutable @sha256 digest.' >&2; exit 1 ;;
esac
[[ -f "$COMPOSE_FILE" ]] || { printf 'Compose file not found: %s\n' "$COMPOSE_FILE" >&2; exit 1; }
export WORKER_IMAGE="$IMAGE" PUBLIC_ORIGIN="$ORIGIN"
docker pull "$IMAGE"
docker compose -p cpr-basispoints -f "$COMPOSE_FILE" up -d --no-build worker

ready=0
for _ in $(seq 1 45); do
  if curl --fail --silent --max-time 3 http://127.0.0.1:28082/health | grep -q '"ready":true'; then
    ready=1
    break
  fi
  sleep 2
done
if [[ "$ready" -ne 1 ]]; then
  printf '%s\n' 'Basis Points Worker did not become ready.' >&2
  exit 1
fi
printf '%s\n' 'Basis Points Worker is ready.'
