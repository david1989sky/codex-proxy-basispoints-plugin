#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "$0")/.." && pwd)"
TARGET="${1:-x86_64-unknown-linux-gnu}"
[[ "$TARGET" == "x86_64-unknown-linux-gnu" ]] || { printf '%s\n' 'Only x86_64-unknown-linux-gnu is supported by this release.' >&2; exit 1; }

pnpm --dir "$ROOT/frontend" install --frozen-lockfile
pnpm --dir "$ROOT/frontend" run typecheck
pnpm --dir "$ROOT/frontend" run lint
pnpm --dir "$ROOT/frontend" run build

if [[ "${BUILD_CONTAINER:-0}" == "1" ]] || [[ "$(rustc -vV | awk '/^host: / {print $2}')" != "$TARGET" ]]; then
  command -v docker >/dev/null 2>&1 || { printf '%s\n' 'Docker is required for a non-host Rust target.' >&2; exit 1; }
  BUILD_ROOT="$(mktemp -d /tmp/cpr-twofa-build.XXXXXX)"
  trap 'rm -rf "$BUILD_ROOT"' EXIT
  mkdir -p "$BUILD_ROOT/backend"
  cp -R "$ROOT/backend/src" "$BUILD_ROOT/backend/"
  cp "$ROOT/backend/Cargo.toml" "$ROOT/backend/Cargo.lock" "$BUILD_ROOT/backend/"
  cp "$ROOT/plugin.json" "$BUILD_ROOT/"
  CONTAINER="$(docker create --platform linux/amd64 rust:1.97-bookworm bash -c "export PATH=/usr/local/cargo/bin:\$PATH; /usr/local/cargo/bin/rustup target add '$TARGET'; cd /tmp; cargo build --release --locked --target '$TARGET' --manifest-path backend/Cargo.toml")"
  trap 'docker rm -f "$CONTAINER" >/dev/null 2>&1 || true; rm -rf "$BUILD_ROOT"' EXIT
  docker cp "$BUILD_ROOT/backend" "$CONTAINER:/tmp/"
  docker cp "$ROOT/plugin.json" "$CONTAINER:/tmp/plugin.json"
  if ! docker start -a "$CONTAINER"; then
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
    exit 1
  fi
  mkdir -p "$ROOT/backend/target/$TARGET/release"
  docker cp "$CONTAINER:/tmp/backend/target/$TARGET/release/codex-proxy-twofa-plugin" "$ROOT/backend/target/$TARGET/release/codex-proxy-twofa-plugin"
  docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
else
  cargo +1.97.0 build --release --locked --target "$TARGET" --manifest-path "$ROOT/backend/Cargo.toml"
fi
