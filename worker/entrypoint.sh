#!/usr/bin/env bash
set -euo pipefail

if [[ "${BROWSER_HEADLESS:-true}" == "false" ]]; then
  exec xvfb-run --auto-servernum --server-args='-screen 0 1024x768x24' node src/server.mjs
fi

exec node src/server.mjs
