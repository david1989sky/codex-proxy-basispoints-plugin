# Basis Points Responses Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a protected `api/basispoints/responses` management route that resolves an OpenAI OAuth token from a host account export and forwards a bounded Responses request to the fixed Basis Points endpoint.

**Architecture:** The Rust plugin registers and validates the management route, forwards the authenticated management Cookie and Origin to the loopback Worker, and preserves the upstream response type. The Worker calls the host sensitive export endpoint, validates the selected OAuth document and JWT account claim, then performs the fixed external request entirely in memory. Existing 2FA routes and UI remain unchanged.

**Tech Stack:** Rust 1.97, `gateway-plugin-sdk`, reqwest, Node.js 22+, Fastify 5, Node test runner, native `fetch`, JSON and bounded byte buffers.

---

## File Map

- Create `worker/src/basispoints.mjs`: account export selection, JWT payload decoding, account ID resolution, and fixed Basis Points HTTP request
- Create `worker/test/basispoints.test.mjs`: unit tests with injected host and Basis Points fetch functions
- Modify `worker/src/app.mjs`: register the protected route and inject a fetch function for tests
- Modify `worker/test/api.test.mjs`: management guard, wrapper validation, raw response, and secret non-disclosure tests
- Modify `backend/src/worker_client.rs`: map the new Worker path and preserve raw Basis Points bodies instead of unwrapping Worker envelopes
- Modify `backend/src/management/router.rs`: validate and forward `POST api/basispoints/responses`, including special error mapping
- Modify `backend/src/management/response.rs`: emit raw responses with their allow-listed content type
- Modify `backend/src/management/registration.rs`: register the route with JSON input and JSON/SSE output contracts
- Modify `README.md`: document the route contract and token handling without including credentials

### Task 1: Add the isolated Basis Points client

**Files:**
- Create: `worker/src/basispoints.mjs`
- Create: `worker/test/basispoints.test.mjs`

- [ ] **Step 1: Write failing tests for JWT and account-document validation.**

Add tests for these exported behaviors before creating the module:

```js
function fixtureJwt(payload) {
  const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url')
  return `${encode({ alg: 'none', typ: 'JWT' })}.${encode(payload)}.signature`
}
const fixtureAccessToken = fixtureJwt({ 'https://api.openai.com/auth': {} })

function fixtureOptions({ authentication_kind = 'oauth', access_token, account_id = 'chatgpt-acct', tokenClaim = {} } = {}) {
  const token = access_token ?? fixtureJwt({ 'https://api.openai.com/auth': tokenClaim })
  return {
    accountId: 'acct-1', request: { model: 'gpt-test', input: 'hello' }, cookie: 'cpr_session=admin',
    upstream: async () => ({ documents: [{ provider: 'openai', document: {
      id: 'acct-1', authentication_kind, account_id, access_token: token,
    } }] }),
    fetchImpl: async () => new Response('{"ok":true}', { status: 200, headers: { 'content-type': 'application/json' } }),
  }
}

test('uses the matching OpenAI OAuth export and host account id first', async () => {
  const result = await requestBasispoints({
    accountId: 'acct-1',
    request: { model: 'gpt-test', input: 'hello' },
    cookie: 'cpr_session=admin',
    origin: 'https://fixture.example',
    upstream: async path => {
      assert.equal(path, '/api/admin/accounts/export?accountIds=acct-1&confirm=export_sensitive_accounts')
      return { documents: [{ provider: 'openai', document: {
        id: 'acct-1', authenticationKind: 'oauth', account_id: 'chatgpt-acct',
        access_token: fixtureAccessToken,
      } }] }
    },
    fetchImpl: async (url, init) => {
      assert.equal(url, 'https://bps.openai.com/basispoints/api/responses')
      assert.equal(init.headers.authorization, `Bearer ${fixtureAccessToken}`)
      assert.equal(init.headers['chatgpt-account-id'], 'chatgpt-acct')
      assert.equal(init.headers['x-openai-account-id'], 'chatgpt-acct')
      assert.equal(init.headers['x-basispoints-auth-mode'], 'chatgpt')
      assert.deepEqual(JSON.parse(init.body), { model: 'gpt-test', input: 'hello' })
      return new Response('{"ok":true}', { status: 200, headers: { 'content-type': 'application/json' } })
    },
  })
  assert.equal(result.status, 200)
  assert.equal(result.contentType, 'application/json')
  assert.equal(result.body.toString(), '{"ok":true}')
})

test('rejects API keys, malformed JWTs, missing account ids, and claim mismatches', async () => {
  await assert.rejects(() => requestBasispoints(fixtureOptions({ authentication_kind: 'api_key' })), /OAuth/)
  await assert.rejects(() => requestBasispoints(fixtureOptions({ access_token: 'opaque-token', account_id: 'chatgpt-acct' })), /JWT/)
  await assert.rejects(() => requestBasispoints(fixtureOptions({ account_id: undefined, tokenClaim: {} })), /账号/)
  await assert.rejects(() => requestBasispoints(fixtureOptions({ account_id: 'host-acct', tokenClaim: { chatgpt_account_id: 'other-acct' } })), /账号/)
})
```

`fixtureJwt` must create a three-part token with a base64url JSON payload and all token strings must be fixture-only values. The test must run with `node --test worker/test/basispoints.test.mjs` and fail because the module does not exist.

- [ ] **Step 2: Implement bounded JWT decoding and export selection.**

Create `worker/src/basispoints.mjs` with these contracts:

```js
import { PublicError } from './core.mjs'

export const BASISPOINTS_URL = 'https://bps.openai.com/basispoints/api/responses'
export const MAXIMUM_RESPONSE_BYTES = 2 * 1024 * 1024

const ACCOUNT_ID = /^[A-Za-z0-9_.:-]{1,128}$/
const ALLOWED_MEDIA_TYPES = new Set(['application/json', 'text/event-stream'])

export function decodeJwtPayload(token) {
  if (typeof token !== 'string' || token.length > 16384) throw new PublicError(400, 'access token JWT 格式无效')
  const parts = token.split('.')
  if (parts.length !== 3 || !parts[1]) throw new PublicError(400, 'access token JWT 格式无效')
  let value
  try {
    const encoded = parts[1].replace(/-/g, '+').replace(/_/g, '/')
    value = JSON.parse(Buffer.from(encoded, 'base64').toString('utf8'))
  } catch {
    throw new PublicError(400, 'access token JWT 格式无效')
  }
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new PublicError(400, 'access token JWT payload 无效')
  return value
}

export function resolveAccountId(document, token) {
  const claims = decodeJwtPayload(token)
  const authClaims = claims['https://api.openai.com/auth']
  const claimAccountId = authClaims && typeof authClaims === 'object' ? authClaims.chatgpt_account_id : undefined
  const documentAccountId = document.account_id
  if (documentAccountId !== undefined && (typeof documentAccountId !== 'string' || !ACCOUNT_ID.test(documentAccountId)))
    throw new PublicError(400, '宿主账号 ID 无效')
  if (claimAccountId !== undefined && (typeof claimAccountId !== 'string' || !ACCOUNT_ID.test(claimAccountId)))
    throw new PublicError(400, 'access token 账号 ID 无效')
  if (documentAccountId && claimAccountId && documentAccountId !== claimAccountId)
    throw new PublicError(400, '宿主账号与 access token 不匹配')
  const result = documentAccountId || claimAccountId
  if (!result) throw new PublicError(400, 'access token 缺少 ChatGPT 账号 ID')
  return result
}

export async function requestBasispoints({ accountId, request, cookie, upstream, fetchImpl = fetch }) {
  if (typeof accountId !== 'string' || !ACCOUNT_ID.test(accountId)) throw new PublicError(400, '宿主账号 ID 无效')
  if (!request || typeof request !== 'object' || Array.isArray(request)) throw new PublicError(400, 'Responses 请求格式无效')
  const exported = await upstream(`/api/admin/accounts/export?accountIds=${encodeURIComponent(accountId)}&confirm=export_sensitive_accounts`, cookie)
  const documents = Array.isArray(exported?.documents) ? exported.documents : []
  const matches = documents.filter(item => item?.provider === 'openai' && item?.document?.id === accountId)
  if (matches.length !== 1) throw new PublicError(400, '宿主账号不是唯一的 OpenAI OAuth 账号')
  const document = matches[0].document
  if (document.authentication_kind !== 'oauth' && document.authenticationKind !== 'oauth') throw new PublicError(400, '宿主账号不是 OpenAI OAuth 账号')
  let accessToken = document.access_token
  if (typeof accessToken !== 'string' || !accessToken) throw new PublicError(400, '宿主账号没有可用 access token')
  try {
    const chatgptAccountId = resolveAccountId(document, accessToken)
    const response = await fetchImpl(BASISPOINTS_URL, {
      method: 'POST', redirect: 'error', signal: AbortSignal.timeout(30000),
      headers: {
        authorization: `Bearer ${accessToken}`,
        'chatgpt-account-id': chatgptAccountId,
        'x-openai-account-id': chatgptAccountId,
        'x-basispoints-auth-mode': 'chatgpt',
        'content-type': 'application/json',
      },
      body: JSON.stringify(request),
    })
    const contentType = response.headers.get('content-type')?.split(';', 1)[0].trim().toLowerCase() || ''
    if (!ALLOWED_MEDIA_TYPES.has(contentType)) throw new PublicError(502, 'Basis Points 返回了不支持的内容类型')
    const body = Buffer.from(await response.arrayBuffer())
    if (body.length > MAXIMUM_RESPONSE_BYTES) throw new PublicError(502, 'Basis Points 响应超过大小限制')
    return { status: response.status, contentType: response.headers.get('content-type') || contentType, body }
  } finally {
    accessToken = ''
  }
}
```

Validate `accountId` with the same ASCII identifier rule used by the Rust bridge. The host path must be constructed with `encodeURIComponent(accountId)` and the fixed confirmation string. Select exactly one document whose `document.id` equals the requested ID, whose provider is `openai`, whose authentication kind is `oauth`, and whose `access_token` is a non-empty string. Do not return the export document.

Decode only the JWT payload; accept an account ID from `document.account_id` or from `claims['https://api.openai.com/auth'].chatgpt_account_id`. If both exist they must match. Reject malformed tokens, non-object payloads, missing IDs, and oversized response bodies with `PublicError` messages that contain no token data.

- [ ] **Step 3: Implement the fixed Basis Points request and response cap.**

Use `fetchImpl(BASISPOINTS_URL, { method: 'POST', redirect: 'error', signal: AbortSignal.timeout(30000), headers, body: JSON.stringify(request) })` with only these headers:

```js
{
  authorization: `Bearer ${accessToken}`,
  'chatgpt-account-id': resolvedAccountId,
  'x-openai-account-id': resolvedAccountId,
  'x-basispoints-auth-mode': 'chatgpt',
  'content-type': 'application/json',
}
```

Read the response as an `ArrayBuffer`, reject bodies above `MAXIMUM_RESPONSE_BYTES`, allow only `application/json` and `text/event-stream` content types, and return `{ status, contentType, body: Buffer }`. Use a mutable local token reference and clear it in `finally`; never include it in thrown errors. Run the focused test again; it must pass.

- [ ] **Step 4: Commit the isolated client.**

Run `npm --prefix worker test -- --test-name-pattern='Basis Points|JWT|export'` and commit:

```bash
git add worker/src/basispoints.mjs worker/test/basispoints.test.mjs
git commit -m "feat: add basispoints request client"
```

### Task 2: Expose the Worker route

**Files:**
- Modify: `worker/src/app.mjs`
- Modify: `worker/test/api.test.mjs`

- [ ] **Step 1: Add failing Fastify route tests.**

Extend `createApp` test options with an injected `basispointsFetch`. Add tests that POST `/api/basispoints/responses` with `{ accountId: 'acct-1', request: { model: 'gpt-test', input: 'hello' } }` and the existing admin headers. Assert the host export is called, the returned status/content type/body are preserved, and the body never contains the fixture token. Also assert that anonymous, key-role, cross-origin, missing-header, malformed-wrapper, and non-object `request` calls return 4xx before the Basis Points fetch runs.

Run `npm --prefix worker test -- --test-name-pattern='Basis Points route'`; the new tests must fail before the route exists.

- [ ] **Step 2: Register the route with an explicit schema.**

Import `requestBasispoints` and add an `app.post('/api/basispoints/responses', { schema: { body: { type: 'object', additionalProperties: false, required: ['accountId', 'request'], properties: { accountId: { type: 'string', minLength: 1, maxLength: 128 }, request: { type: 'object', additionalProperties: true } } } } }, handler)` route. The handler must call `requestBasispoints` with `request.sessionCookie`, the configured origin, the existing `upstream`, and the injected fetch function, then send `reply.code(result.status).type(result.contentType).send(result.body)`.

Keep the existing `onRequest` admin and CSRF guard in front of the route. Do not add an access-token field to the schema or to any response. Run the focused route tests and then `npm --prefix worker test`; all Worker tests must pass.

- [ ] **Step 3: Commit the Worker route.**

```bash
git add worker/src/app.mjs worker/test/api.test.mjs
git commit -m "feat: expose basispoints management route"
```

### Task 3: Forward raw responses through the Rust bridge

**Files:**
- Modify: `backend/src/worker_client.rs`
- Modify: `backend/src/management/router.rs`
- Modify: `backend/src/management/response.rs`

- [ ] **Step 1: Add failing Rust unit tests for raw path mapping and content types.**

Extend the `worker_client` tests to assert `worker_path("POST", "api/basispoints/responses")` maps to `/api/basispoints/responses`, that a successful raw JSON body containing `{ "code": 200, "data": ... }` is not envelope-unwrapped, and that `text/event-stream` is accepted by the response helper. Add a router test that a malformed wrapper is rejected before forwarding.

Run `cargo test --manifest-path backend/Cargo.toml --locked`; these tests must fail until the route and raw response path are implemented.

- [ ] **Step 2: Preserve raw Worker bodies for the Basis Points path.**

In `WorkerClient::forward`, mark `/api/basispoints/responses` as raw before calling `unwrap_worker_body`; keep the existing envelope behavior for all 2FA paths. Add the exact path to `worker_path` only for `POST`. Keep the existing 140 KiB request and 2 MiB response caps.

- [ ] **Step 3: Add raw response construction with an allow-list.**

Change `raw_json` to call a new `raw_response(status, content_type, body)` helper. The helper must preserve only `application/json` or `text/event-stream` media types (including parameters), always add `Cache-Control: no-store` and `X-Content-Type-Options: nosniff`, and reject every other content type with `ApiError::new(502, "worker_response", "Worker 返回了不支持的内容类型")`.

- [ ] **Step 4: Add the direct management handler and strict wrapper validation.**

In `router.rs`, match `("POST", "api/basispoints/responses")` before the generic operation route. Decode a local `BasispointsRequest { account_id: String, request: Value }` with `deny_unknown_fields`, require `bounded_id(&account_id)` and `request.is_object()`, serialize the wrapper back to JSON, and forward a synthetic `ManagementRequest` with path `api/basispoints/responses`, JSON content type, and the original management headers. Map `WorkerError::RequestTooLarge` to 413, `ResponseTooLarge` to 502, and transport/invalid response failures to 503 without exposing error details.

- [ ] **Step 5: Run Rust formatting, tests, and lints.**

Run:

```bash
cargo +1.97.0 fmt --manifest-path backend/Cargo.toml -- --check
cargo +1.97.0 test --manifest-path backend/Cargo.toml --locked
cargo +1.97.0 clippy --manifest-path backend/Cargo.toml --all-targets --all-features --locked -- -D warnings
```

Expected result: formatting, unit tests, and clippy all pass with no token-bearing output.

- [ ] **Step 6: Commit the Rust bridge.**

```bash
git add backend/src/worker_client.rs backend/src/management/router.rs backend/src/management/response.rs
git commit -m "feat: forward basispoints responses through plugin bridge"
```

### Task 4: Register and document the public plugin contract

**Files:**
- Modify: `backend/src/management/registration.rs`
- Modify: `README.md`

- [ ] **Step 1: Register JSON input and JSON/SSE output.**

Add one route entry:

```rust
ManagementRoute {
    method: "POST".to_owned(),
    path: "api/basispoints/responses".to_owned(),
    request_content_types: vec![JSON_CONTENT_TYPE.to_owned()],
    response_content_types: vec![JSON_CONTENT_TYPE.to_owned(), "text/event-stream".to_owned()],
}
```

Do not add permissions, new resources, or a new page.

- [ ] **Step 2: Document the route without credentials.**

Add a concise README section showing the `{ accountId, request }` wrapper, the fixed behavior, the admin-session requirement, the JSON/SSE response allowance, and the fact that the token is resolved from the host export and never returned. Do not include a real URL with a token, a sample JWT, or an instruction to paste credentials into logs.

- [ ] **Step 3: Verify the manifest and documentation diff.**

Run `git diff --check`, inspect the complete diff, and confirm that `plugin.json` remains unchanged because the existing management contribution already covers route registration. Commit:

```bash
git add backend/src/management/registration.rs README.md
git commit -m "docs: register basispoints responses contract"
```

### Task 5: Full verification and handoff

**Files:**
- No new files; verify all changed files and package inputs

- [ ] **Step 1: Run the complete Worker and Rust checks.**

```bash
npm --prefix worker ci --ignore-scripts --no-audit --no-fund
npm --prefix worker test
cargo +1.97.0 fmt --manifest-path backend/Cargo.toml -- --check
cargo +1.97.0 test --manifest-path backend/Cargo.toml --locked
cargo +1.97.0 clippy --manifest-path backend/Cargo.toml --all-targets --all-features --locked -- -D warnings
```

- [ ] **Step 2: Inspect the final diff for secret handling.**

Run `git diff origin/main...HEAD --stat` and `git diff origin/main...HEAD -- worker backend README.md`. Confirm no access token, refresh token, id token, account export body, real cookie, or real endpoint response is present in tracked files, fixtures, snapshots, or logs.

- [ ] **Step 3: Run the package contract check.**

Run `bash scripts/package.sh` only when the pinned `cpr-plugin` CLI and frontend dependencies are available. If packaging is unavailable, report that exact missing dependency; do not call the real Basis Points endpoint. Confirm the package manifest still validates and the archive contains no credentials or test fixtures.

- [ ] **Step 4: Commit any verification-only documentation fix and report results.**

Use a Conventional Commit only if a documentation or test fixture correction is required. Final handoff must name the route, changed components, commits, commands that passed, and any packaging or live-upstream verification not performed.
