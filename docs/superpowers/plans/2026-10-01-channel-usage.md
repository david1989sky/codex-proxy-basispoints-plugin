# BPS Channel Usage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an RS management page that displays in-process BPS channel request totals, successes, failures, and the latest request time.

**Architecture:** Share an `Arc<UsageStats>` between the existing middleware and management handlers. Middleware records one attempt immediately before the BPS HTTP call and classifies the resulting status or error; `GET api/usage` returns an atomic snapshot. A packaged HTML/CSS/JavaScript management page calls that route through `window.codexProxyPlugin.request` and refreshes every five seconds. Statistics reset when the plugin process restarts.

**Tech Stack:** Rust 1.97, gateway-plugin-sdk management routes/pages, `AtomicU64`, static HTML/CSS/JavaScript resources, Node test runner, RS plugin CLI.

---

### Task 1: Add the usage snapshot unit

**Files:**
- Create: `backend/src/usage.rs`
- Modify: `backend/src/lib.rs`
- Test: `backend/src/usage.rs` unit tests

- [ ] **Step 1: Write failing tests**

Add tests for a zero snapshot, one successful request, one failed request, latest timestamp replacement, and concurrent updates. The tests must assert `total_requests == successful_requests + failed_requests` after all updates.

- [ ] **Step 2: Run the focused tests and verify they fail**

Run `cargo +1.97.0 test --manifest-path backend/Cargo.toml usage::tests`. Expected failure: the `usage` module and `UsageStats` type do not exist.

- [ ] **Step 3: Implement the minimal counter**

Define:

```rust
pub(crate) struct UsageStats {
    total_requests: AtomicU64,
    successful_requests: AtomicU64,
    failed_requests: AtomicU64,
    last_request_at_ms: AtomicU64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageSnapshot {
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub last_request_at_ms: Option<u64>,
}
```

Expose `begin(now_ms)`, `record_success()`, `record_failure()`, and `snapshot()`. Use `Ordering::Relaxed`; update the latest timestamp with a compare-exchange loop so concurrent requests cannot move it backwards. Keep the type free of request bodies, account IDs, model names, and credentials.

- [ ] **Step 4: Run the focused tests and verify they pass**

Run `cargo +1.97.0 test --manifest-path backend/Cargo.toml usage::tests` and expect all usage tests to pass.

- [ ] **Step 5: Commit**

Run `git add backend/src/usage.rs backend/src/lib.rs && git commit -m "feat: add BPS usage counters"`.

### Task 2: Wire counters into middleware state and classification

**Files:**
- Modify: `backend/src/management/router.rs`
- Modify: `backend/src/middleware.rs`
- Modify: `backend/src/lib.rs`
- Test: `backend/src/middleware.rs` and `backend/src/usage.rs`

- [ ] **Step 1: Write failing classification tests**

Add tests that prove an unselected model leaves counters unchanged and that response statuses `200`, `403`, `400`, and `500` classify as success, failure, failure, and failure. Add a test that a middleware error records failure exactly once.

- [ ] **Step 2: Run the focused tests and verify the new assertions fail**

Run `cargo +1.97.0 test --manifest-path backend/Cargo.toml middleware::tests usage::tests`. Expected failure: `PluginState` has no usage field and middleware does not record classifications.

- [ ] **Step 3: Share `Arc<UsageStats>` through `PluginState`**

Add `pub(crate) usage: Arc<UsageStats>` to `PluginState`, initialize it in `PluginState::new`, and keep the existing `Arc<BpsConfig>` behavior unchanged. Export the module from `lib.rs` without exposing credentials or mutable configuration.

- [ ] **Step 4: Record one BPS attempt and one terminal result**

In `middleware::handle`, after request parsing and credential resolution but immediately before `basispoints::request`, call `state.usage.begin(now_ms())`. On `Err`, call `record_failure` before mapping the error. On a response, call `record_success` only for `200..300`; call `record_failure` for every other status, including the existing 403 native-provider fallback. Leave non-intercepted requests untouched.

- [ ] **Step 5: Run the focused tests and verify they pass**

Run `cargo +1.97.0 test --manifest-path backend/Cargo.toml middleware::tests usage::tests` and expect all assertions to pass.

- [ ] **Step 6: Commit**

Run `git add backend/src/lib.rs backend/src/management/router.rs backend/src/middleware.rs backend/src/usage.rs && git commit -m "feat: track BPS middleware usage"`.

### Task 3: Expose the usage management route and page registration

**Files:**
- Modify: `backend/src/management/registration.rs`
- Modify: `backend/src/management/router.rs`
- Modify: `backend/tests/manifest.rs`
- Modify: `plugin.json`
- Test: `backend/src/management/registration.rs` and `backend/src/management/router.rs`

- [ ] **Step 1: Write failing route and manifest assertions**

Extend registration tests to require `GET api/usage`, one `usage` page with entry `web/index.html`, and three resource declarations. Add a route test that serializes the zero snapshot with the exact camelCase fields.

- [ ] **Step 2: Run focused tests and verify they fail**

Run `cargo +1.97.0 test --manifest-path backend/Cargo.toml management::tests`. Expected failure: the registration still has two routes and no pages/resources.

- [ ] **Step 3: Register resources, page, and route**

Declare `web/index.html`, `web/app.js`, and `web/app.css` in `plugin.json` with their MIME types. Register the same resources as private `ManagementResource` values, add a `ManagementPage` with id `usage`, and add `GET api/usage` returning `state.usage.snapshot()` through the existing JSON response helper. Keep `api/status` and the Responses route unchanged.

- [ ] **Step 4: Run focused tests and verify they pass**

Run `cargo +1.97.0 test --manifest-path backend/Cargo.toml management::tests` and expect the new registration and JSON contract tests to pass.

- [ ] **Step 5: Commit**

Run `git add backend/src/management/registration.rs backend/src/management/router.rs backend/tests/manifest.rs plugin.json && git commit -m "feat: expose BPS usage management route"`.

### Task 4: Build the isolated RS management page

**Files:**
- Create: `web/index.html`
- Create: `web/app.js`
- Create: `web/app.css`

- [ ] **Step 1: Add the HTML structure**

Create a semantic page with a heading, four labelled values (`总请求数`, `成功`, `失败`, `最近请求`), a `刷新` button, a live status region, and a note that the counters cover the current plugin process. Reference `web/app.css` and `web/app.js` using relative paths.

- [ ] **Step 2: Add the bridge client and rendering logic**

In `web/app.js`, call `window.codexProxyPlugin.request({ path: 'api/usage', method: 'GET' })`, decode the returned `ArrayBuffer`, parse JSON, and update only text nodes. Format `lastRequestAtMs` with `Intl.DateTimeFormat`; render `暂无请求` for `null`. Show an error in the status region while retaining the last successful values. Wire the button and a five-second interval, and clear the interval on page unload. Do not use `fetch`, external scripts, module imports, or secret-bearing fields.

- [ ] **Step 3: Add restrained responsive styling**

Use the host-friendly light/dark color variables with a simple responsive grid; keep the page usable in the RS iframe at narrow widths. Avoid network fonts and external assets.

- [ ] **Step 4: Run static resource checks**

Run `node --check web/app.js` and `rg -n "api/usage|codexProxyPlugin|fetch\(" web`. Expected: syntax passes, the bridge route is present, and there are no direct network calls.

- [ ] **Step 5: Commit**

Run `git add web && git commit -m "feat: add BPS usage management page"`.

### Task 5: Documentation, full verification, and package checks

**Files:**
- Modify: `README.md`
- Modify: `backend/src/management/registration.rs` tests if coverage gaps remain

- [ ] **Step 1: Document the page and reset behavior**

Document `GET api/usage`, the displayed fields, the in-process reset behavior, and the RS navigation path under the existing management section.

- [ ] **Step 2: Run the full local verification**

Run:

```bash
cargo +1.97.0 fmt --manifest-path backend/Cargo.toml -- --check
cargo +1.97.0 clippy --manifest-path backend/Cargo.toml --all-targets --all-features --locked -- -D warnings
cargo +1.97.0 test --manifest-path backend/Cargo.toml --locked
npm test --prefix worker
node --check web/app.js
```

Expected: every command exits 0; Rust reports 58+ tests plus the manifest test, and Worker reports all existing tests passing.

- [ ] **Step 3: Build and inspect the archive**

Build the Linux x86_64 binary with `BUILD_CONTAINER=1 CARGO_BUILD_JOBS=1 bash scripts/build-linux-x64.sh`, package with `PLUGIN_CLI="$PWD/.tools/bin/cpr-plugin" bash scripts/package.sh`, and verify `tar -tzf dist/*.tar.gz` contains the three web resources and no `.env`, `secrets`, `credentials`, or `node_modules` paths.

- [ ] **Step 4: Commit documentation and package-ready source**

Run `git add README.md && git commit -m "docs: describe BPS usage display"`.

- [ ] **Step 5: Install and verify in RS**

Publish the independent branch/tag release, install the new archive in RS, open 插件管理 → 扩展页 → BPS 通道使用情况, and verify the page loads zero values, refreshes after a real configured-model request, and shows the request result without exposing credentials. Keep the BPS plugin separate from the twofa plugin.
