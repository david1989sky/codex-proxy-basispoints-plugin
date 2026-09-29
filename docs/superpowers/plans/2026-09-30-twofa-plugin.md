# Codex Proxy RS 2FA Plugin Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build and publish an `x86_64-unknown-linux-gnu` Codex Proxy RS plugin package that exposes the existing 2FA authorization workflow through the official plugin management page while keeping the Playwright Worker as a verified companion container.

**Architecture:** A Rust plugin process uses the v2 SDK management bridge to proxy authenticated management calls to the loopback Worker, and a Vue iframe page uses only `window.codexProxyPlugin.request`. The companion Worker remains a separately built Playwright container with the existing encrypted vault mounts; one GitHub Release publishes the plugin archive, checksum, Worker image reference, and deployment scripts.

**Tech Stack:** Rust 1.97, `gateway-plugin-sdk` pinned to the v3.18.1 tag commit, Tokio, Reqwest with rustls, Vue 3, Vite, `@codex-proxy/ui`, Node 22, Fastify 5, Playwright 1.55, Docker Compose, GitHub Actions.

---

### Task 1: Create the standalone repository skeleton and public contracts

**Files:**
- Create: `plugin.json`
- Create: `README.md`
- Create: `LICENSE`
- Create: `.gitignore`
- Create: `backend/Cargo.toml`
- Create: `backend/src/lib.rs`
- Create: `backend/src/main.rs`
- Create: `backend/src/manifest.rs`
- Create: `backend/src/management/mod.rs`
- Create: `backend/src/management/registration.rs`
- Create: `backend/src/management/response.rs`
- Create: `backend/src/management/validation.rs`
- Test: `backend/tests/manifest.rs`

- [ ] **Step 1: Add the author manifest with only management and state capabilities.**

  Set `publisher` to `david1989sky`, `name` to `codex-proxy-twofa`, `version` to `0.1.0`, `engines.codex-proxy-rs` to `>=3.18.1, <4.0.0`, `main` to `bin/plugin`, `runtime` to `trustedProcess`, one `management` contribution, a `configurationSchema` with loopback Worker URL and expected Worker image digest fields, `resources` for `web/index.html`, `web/app.js`, and `web/app.css`, and a `twofa` state namespace with bounded migration metadata. Do not add a `permissions` field because the v3.18.1 manifest rejects it.

- [ ] **Step 2: Pin the public SDK to the v3.18.1 tag commit and create a warning-free binary crate.**

  Use `gateway-plugin-sdk = { git = "https://github.com/zyycn/codex-proxy-rs.git", rev = "70d1557b55aed871d66dee5db82c2254990d06a6", features = ["io"] }`, Tokio with `io-std`, `macros`, `rt`, and `time`, Reqwest with `json` and `rustls-tls`, Serde, and URL parsing. Deny Rust warnings and unsafe code; configure release stripping, LTO, one codegen unit, and abort-on-panic.

- [ ] **Step 3: Implement the SDK session entry point and manifest helper.**

  `main.rs` accepts the SDK session over stdin/stdout, verifies the handshake plugin id and contributions against `plugin.json`, then runs `PluginSession`. `manifest.rs` exports `PLUGIN_ID` and loads the embedded author manifest. No diagnostic output may be written to stdout.

- [ ] **Step 4: Add the first failing manifest contract test and run it.**

  Parse `plugin.json` with `Manifest::from_author_slice`, assert the derived id, management contribution, engine range, `web/index.html` resource, and `twofa` state namespace. Run `cargo test --manifest-path backend/Cargo.toml --locked`; the test must fail before the manifest and helpers exist.

- [ ] **Step 5: Implement the minimal manifest and run the contract test again.**

  Keep the test focused on package identity and contracts, not UI details. Run `cargo fmt --manifest-path backend/Cargo.toml -- --check` and `cargo test --manifest-path backend/Cargo.toml --locked`; both must pass.

- [ ] **Step 6: Commit the repository skeleton.**

  Commit with `git add plugin.json README.md LICENSE .gitignore backend && git commit -m "feat: scaffold twofa gateway plugin"`.

### Task 2: Add the Rust management bridge to the companion Worker

**Files:**
- Create: `backend/src/worker_client.rs`
- Modify: `backend/src/management/registration.rs`
- Modify: `backend/src/management/router.rs`
- Modify: `backend/src/management/mod.rs`
- Modify: `backend/src/management/response.rs`
- Test: `backend/tests/worker_client.rs`
- Test: `backend/tests/management.rs`

- [ ] **Step 1: Write forwarding tests against a local mock HTTP server.**

  Cover `GET api/status`, credential lookup/deletion, task creation/read/control, screenshot retrieval, and manual input. Assert that the client sends `Cookie`, `Origin`, and `X-CPR-TwoFA: 1`, never logs or returns credential bodies, maps Worker `{code,message,data}` responses to plugin JSON, and turns loopback connection failures into HTTP 503. Add a test that rejects non-admin or missing session headers before forwarding.

- [ ] **Step 2: Implement a bounded Worker client.**

  Store the base URL from plugin configuration, reject non-loopback URLs, use a `reqwest::Client` with a 30-second timeout and no proxy environment, copy only the incoming `Cookie` and `Origin` headers, add `X-CPR-TwoFA: 1`, cap request bodies at 140 KiB, cap response bodies at 2 MiB, and map only the allowed Worker routes. Never print request or response bodies.

- [ ] **Step 3: Register the plugin management routes.**

  Register only the static relative routes `api/status`, `api/migration`, `api/migration/import`, and `api/request` POST, plus the three page resources and one management page. The request body carries a closed `operation` enum and bounded IDs/data because the RS management contract does not support dynamic `:id` route declarations.

- [ ] **Step 4: Implement route validation and error mapping.**

  Reject query parameters, unsupported content types, unknown operations, unbounded IDs, control characters, bodies above the limit, and requests without the authenticated management headers. Translate Worker 401/403/404/409/413/5xx responses to stable plugin errors. Return `Cache-Control: no-store` and JSON content type for API responses; screenshot data remains a JSON data URL inside the bounded response.

- [ ] **Step 5: Implement migration/status semantics without moving the vault in the first release.**

  `api/status` calls Worker `/health` and reports `ready`, Worker version, and a boolean `legacyVaultMounted`. `api/migration` reports that the companion deployment uses the existing encrypted vault mount and returns a migration marker; `POST api/migration/import` is idempotent and records the marker through `host.state` after verifying Worker readiness. It must never read or return vault contents.

- [ ] **Step 6: Run bridge tests and commit.**

  Run `cargo fmt`, `cargo clippy --all-targets --all-features --locked -- -D warnings`, and `cargo test --manifest-path backend/Cargo.toml --locked`. Commit with `git add backend && git commit -m "feat: proxy twofa management through plugin bridge"`.

### Task 3: Port the verified Worker into the companion source tree

**Files:**
- Create: `worker/package.json`
- Create: `worker/package-lock.json`
- Create: `worker/src/app.mjs`
- Create: `worker/src/browser-profile.mjs`
- Create: `worker/src/browser.mjs`
- Create: `worker/src/core.mjs`
- Create: `worker/src/server.mjs`
- Create: `worker/src/vault.mjs`
- Create: `worker/test/*.test.mjs`
- Create: `worker/Dockerfile`
- Create: `worker/entrypoint.sh`
- Create: `worker/ops/compose.yaml`
- Create: `worker/ops/provision-vault.sh`
- Create: `worker/ops/deploy.sh`
- Create: `worker/ops/rollback.sh`
- Modify: `worker/README.md`

- [ ] **Step 1: Copy the production Worker source without node_modules, credentials, keys, or deployment snapshots.**

  Preserve the current Fastify routes and Playwright behavior from the read-only production copy. Keep `package-lock.json` and all tests; do not copy `/opt/cpr-twofa/data`, `/opt/cpr-twofa/secrets`, logs, or screenshots containing real sessions.

- [ ] **Step 2: Add the companion image metadata contract.**

  Pin the Playwright base image `mcr.microsoft.com/playwright:v1.55.0-noble`, Node engine `>=22`, and the Worker image name `ghcr.io/david1989sky/codex-proxy-twofa-worker`. The compose file must use host networking, read-only root, no-new-privileges, dropped capabilities, bounded memory/PIDs, and the existing encrypted vault mounts.

- [ ] **Step 3: Make deployment scripts digest-aware and idempotent.**

  `deploy.sh` must verify the old Nginx and vault backup checksums, build/pull the requested image digest, run `provision-vault.sh` only when the vault is empty, wait for `/health` to return `data.ready=true`, and leave the old image available for rollback. `rollback.sh` must restore only the Worker image and compose state, never the RS core container or database.

- [ ] **Step 4: Run all Worker tests with fake credentials only.**

  Run `npm ci --ignore-scripts --no-audit --no-fund`, `npm test`, and the existing UI test in a controlled environment. Record browser-dependent tests separately if Chromium is unavailable locally.

- [ ] **Step 5: Commit the companion Worker.**

  Commit with `git add worker && git commit -m "feat: add twofa companion worker"`.

### Task 4: Build the isolated plugin management page

**Files:**
- Create: `frontend/package.json`
- Create: `frontend/pnpm-lock.yaml`
- Create: `frontend/index.html`
- Create: `frontend/vite.config.ts`
- Create: `frontend/tsconfig*.json`
- Create: `frontend/eslint.config.ts`
- Create: `frontend/src/main.ts`
- Create: `frontend/src/App.vue`
- Create: `frontend/src/api/host.ts`
- Create: `frontend/src/api/request.ts`
- Create: `frontend/src/api/modules/twofa.ts`
- Create: `frontend/src/components/TwoFaImport.vue`
- Create: `frontend/src/components/TwoFaTask.vue`
- Create: `frontend/src/styles/index.css`

- [ ] **Step 1: Copy the official workbench Vite and host bridge setup.**

  Use `base: './'`, classic deferred output, a single `app.js`, a single `app.css`, and `@codex-proxy/ui` from the pinned public release. The page must work only inside the RS plugin iframe and must show a clear error in standalone preview mode.

- [ ] **Step 2: Add API modules with typed request/response contracts.**

  Use relative paths only. Implement `status`, `migration`, `startTask`, `readTask`, `cancelTask`, `retryTask`, `deleteTask`, `readCredentials`, `deleteCredentials`, `reauthorize`, `readScreen`, and `sendInput`. Decode JSON envelopes and redact any credential fields before putting data into component state.

- [ ] **Step 3: Implement the batch import and task state UI.**

  Support TXT upload and paste, 128 KiB client limit, account settings, task polling every 1.5 seconds, waiting-screen display, manual input, retry, cancel, delete, saved-credential state, and reauthorization. Use the existing account wording and 2FA behavior; do not duplicate the host settings form or expose implementation details.

- [ ] **Step 4: Add loading, empty, failure, and theme states.**

  Use the UI package primitives and Lucide icons, preserve natural document flow, inherit the host page height, and ensure controls remain usable at narrow widths. Do not use direct `fetch`, host cookies, or hard-coded host API URLs.

- [ ] **Step 5: Run frontend checks and commit.**

  Run `pnpm install --frozen-lockfile`, `pnpm run typecheck`, `pnpm run lint`, and `pnpm run build`. Commit with `git add frontend && git commit -m "feat: add twofa plugin management page"`.

### Task 5: Add packaging, companion release, and repository documentation

**Files:**
- Create: `scripts/build-linux-x64.sh`
- Create: `scripts/package.sh`
- Create: `ops/companion-install.sh`
- Create: `ops/companion-update.sh`
- Create: `.github/workflows/release.yml`
- Modify: `README.md`
- Create: `docs/install.md`
- Create: `docs/migration.md`

- [ ] **Step 1: Add deterministic build scripts.**

  Build the frontend, build the Rust binary for `x86_64-unknown-linux-gnu`, run the target `cpr-plugin package` from the v3.18.1 source, map `web=frontend/dist`, write output under `dist/`, and create a manifest containing the plugin archive SHA-256, Worker image digest, and source commit. Fail if the expanded plugin package would exceed 128 MiB.

- [ ] **Step 2: Add companion install/update scripts.**

  Require an explicit image digest, verify the digest before starting, preserve the current compose/image/vault backups, run health checks, and support a `--rollback` path. Never accept a mutable `latest` tag for production.

- [ ] **Step 3: Add the GitHub Actions release workflow.**

  Trigger only on tags matching `v*`, run Worker tests, frontend checks, Rust checks, package the plugin, build/push the companion image to GHCR, generate checksums and release notes, then publish a non-draft non-prerelease Release with the plugin archive, `.sha256`, companion manifest, and install scripts. Do not print secrets or request bodies.

- [ ] **Step 4: Write install and migration documentation.**

  Document the repository URL, supported platform, required RS `>=3.18.1`, GitHub Release attachment selection, Worker digest verification, vault backup, first enable, status check, update, rollback, and uninstall behavior. Include no real host paths containing credentials beyond the documented mount names.

- [ ] **Step 5: Commit packaging and docs.**

  Commit with `git add scripts ops .github README.md docs && git commit -m "build: add plugin packaging and release workflow"`.

### Task 6: Verify locally, publish, and install in production

**Files:**
- Modify: `docs/install.md` with the final published version and checksums after release
- Create: `verification/release-checks.mjs`

- [ ] **Step 1: Run the complete local verification matrix.**

  Run Rust fmt/clippy/test, frontend frozen install/typecheck/lint/build, Worker tests, plugin CLI package inspection, archive size checks, and `verification/release-checks.mjs`. The script must verify manifest identity, target, resources, archive checksum, and absence of credential-like files.

- [ ] **Step 2: Create the public GitHub repository and push the reviewed commits.**

  Create `david1989sky/codex-proxy-twofa-plugin` as a public repository, set it as `origin`, push `main`, and verify the remote tree contains no `.env`, vault, key, screenshot, or node_modules files.

- [ ] **Step 3: Build and publish the first stable Release.**

  Tag `v0.1.0`, run the workflow, verify the public plugin archive and checksum can be downloaded, and verify the companion image digest. Do not call the result published until the public assets are fetched and rehashed.

- [ ] **Step 4: Install the plugin in the production RS instance.**

  Back up the current plugin state, Worker compose file, Nginx include, encrypted credential directory, and key metadata. Install the public GitHub Release through `/plugins`, accept the package, configure the loopback Worker endpoint and verified digest, and enable the plugin. Leave the legacy frontend/Worker path available until the plugin page and status checks pass.

- [ ] **Step 5: Execute controlled production smoke tests.**

  Verify plugin page load, Worker `ready=true`, migration marker, task creation with fake input, task cancellation, screenshot/input route authorization, and no credential leakage in logs. Do not run a real OAuth authorization without an explicitly supplied test account and budget.

- [ ] **Step 6: Verify update and rollback, then document the result.**

  Install a second locally built package in a separate test instance or use the plugin version plan without switching production. Verify the public release metadata, backup paths, current plugin version, and rollback command; update `docs/install.md` with actual checksums and any unverified production scenarios.

- [ ] **Step 7: Commit final verification evidence.**

  Commit only source, documentation, checksums, and redacted verification output with `git add docs verification && git commit -m "chore: record plugin release verification"`.
