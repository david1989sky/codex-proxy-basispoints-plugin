#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "$0")/.." && pwd)"
TARGET="${1:-x86_64-unknown-linux-gnu}"
PLUGIN_CLI="${PLUGIN_CLI:-cpr-plugin}"
VERSION="$(node -e "process.stdout.write(JSON.parse(require('fs').readFileSync(process.argv[1])).version)" "$ROOT/plugin.json")"

"$ROOT/scripts/build-linux-x64.sh" "$TARGET"
command -v "$PLUGIN_CLI" >/dev/null 2>&1 || { printf '%s\n' 'cpr-plugin was not found; install the pinned Codex Proxy plugin CLI first.' >&2; exit 1; }
rm -rf "$ROOT/dist"
mkdir -p "$ROOT/dist"
"$PLUGIN_CLI" package \
  --manifest "$ROOT/plugin.json" \
  --binary "$ROOT/backend/target/$TARGET/release/codex-proxy-twofa-plugin" \
  --target "$TARGET" \
  --resource-map web=frontend/dist \
  --output-dir "$ROOT/dist"

PLUGIN_ARCHIVE="$(find "$ROOT/dist" -maxdepth 1 -name '*.tar.gz' -print -quit)"
[[ -n "$PLUGIN_ARCHIVE" ]] || { printf '%s\n' 'Plugin CLI did not produce an archive.' >&2; exit 1; }
ARCHIVE_NAME="$(basename "$PLUGIN_ARCHIVE")"
ARCHIVE_SHA="$(awk '{print $1}' "$PLUGIN_ARCHIVE.sha256")"
[[ "$ARCHIVE_SHA" == "$(shasum -a 256 "$PLUGIN_ARCHIVE" | awk '{print $1}')" ]] || { printf '%s\n' 'Plugin checksum verification failed.' >&2; exit 1; }

WORKER_BUNDLE="$ROOT/dist/codex-proxy-twofa-worker-${VERSION}.tar.gz"
tar -czf "$WORKER_BUNDLE" --exclude='worker/node_modules' --exclude='worker/.env*' --exclude='worker/data' --exclude='worker/secrets' -C "$ROOT" worker
printf '%s  %s\n' "$(shasum -a 256 "$WORKER_BUNDLE" | awk '{print $1}')" "$(basename "$WORKER_BUNDLE")" > "$WORKER_BUNDLE.sha256"

WORKER_IMAGE="${WORKER_IMAGE:-ghcr.io/david1989sky/codex-proxy-twofa-worker:${VERSION}}"
WORKER_IMAGE_DIGEST="${WORKER_IMAGE_DIGEST:-unpublished}"
SOURCE_COMMIT="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || printf '%s' unknown)"
node -e 'const fs=require("fs"); const out={pluginVersion:process.argv[1], pluginArchive:process.argv[2], pluginSha256:process.argv[3], workerImage:process.argv[4], workerImageDigest:process.argv[5], workerBundle:process.argv[6], workerBundleSha256:process.argv[7], sourceCommit:process.argv[8]}; fs.writeFileSync(process.argv[9], JSON.stringify(out,null,2)+"\n")' \
  "$VERSION" "$ARCHIVE_NAME" "$ARCHIVE_SHA" "$WORKER_IMAGE" "$WORKER_IMAGE_DIGEST" \
  "$(basename "$WORKER_BUNDLE")" "$(awk '{print $1}' "$WORKER_BUNDLE.sha256")" "$SOURCE_COMMIT" "$ROOT/dist/companion-manifest.json"

archive_members="$(tar -tzf "$PLUGIN_ARCHIVE")"
if printf '%s\n' "$archive_members" | rg -n '(^|/)(\.env|node_modules|secrets|credentials|screenshots?)(/|$)'; then
  printf '%s\n' 'Plugin archive contains a forbidden sensitive path.' >&2
  exit 1
fi
printf '%s\n' "Packaged $ARCHIVE_NAME"
