#!/usr/bin/env bash
set -euo pipefail

ROOT="${CPR_TWOFA_ROOT:-/opt/cpr-twofa}"
COMPOSE_FILE="${CPR_TWOFA_COMPOSE:-$ROOT/release/codex-proxy-twofa-worker/ops/compose.yaml}"
COMPOSE_DIR="$(dirname -- "$COMPOSE_FILE")"
BACKUP_DIR="$ROOT/backup/twofa-worker"
IMAGE_FILE="$BACKUP_DIR/previous-image"
ORIGIN_FILE="$BACKUP_DIR/current-origin"

if [[ "${1:-}" == "--check" ]]; then
  [[ -s "$COMPOSE_FILE" && -s "$IMAGE_FILE" && -s "$ORIGIN_FILE" ]]
  bash -n "$ROOT/release/codex-proxy-twofa-worker/ops/deploy.sh"
  bash -n "$COMPOSE_DIR/provision-vault.sh"
  printf '%s\n' 'Rollback inputs verified.'
  exit 0
fi

[[ -s "$IMAGE_FILE" ]] || { printf '%s\n' 'No previous Worker image recorded.' >&2; exit 1; }
IMAGE="$(<"$IMAGE_FILE")"
ORIGIN="${PUBLIC_ORIGIN:-$(<"$ORIGIN_FILE")}"
case "$IMAGE" in
  *@sha256:* ) ;;
  *) printf '%s\n' 'Previous Worker image is not digest-pinned.' >&2; exit 1 ;;
esac
export WORKER_IMAGE="$IMAGE"
export PUBLIC_ORIGIN="$ORIGIN"
export CPR_TWOFA_ROOT="$ROOT"
docker pull "$IMAGE"
docker compose -p cpr-twofa -f "$COMPOSE_FILE" up -d --no-build worker
for _ in $(seq 1 45); do
  if curl --fail --silent --max-time 3 http://127.0.0.1:28082/health | grep -q '"ready":true'; then
    cp -p "$BACKUP_DIR/current-image" "$BACKUP_DIR/failed-image" 2>/dev/null || true
    printf '%s\n' "$IMAGE" > "$BACKUP_DIR/current-image"
    printf '%s\n' 'Companion Worker rolled back.'
    exit 0
  fi
  sleep 2
done
printf '%s\n' 'Rollback Worker did not become ready.' >&2
exit 1
