import { test } from 'node:test'
import assert from 'node:assert/strict'
import { MAXIMUM_RESPONSE_BYTES, requestBasispoints } from '../src/basispoints.mjs'

function fixtureJwt(payload) {
  const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url')
  return `${encode({ alg: 'none', typ: 'JWT' })}.${encode(payload)}.signature`
}

const fixtureAccessToken = fixtureJwt({ 'https://api.openai.com/auth': {} })

function fixtureOptions({ accessToken, accountId = 'chatgpt-acct', apiKey, api_key, tokenClaim = {} } = {}) {
  const token = accessToken ?? fixtureJwt({ 'https://api.openai.com/auth': tokenClaim })
  return {
    accountId: 'acct-1', request: { model: 'gpt-test', input: 'hello' }, cookie: 'cpr_session=admin',
    upstream: async () => ({ documents: [{ provider: 'openai', document: { accounts: [{
      id: 'acct-1', accountId, accessToken: token,
      ...(apiKey === undefined ? {} : { apiKey }), ...(api_key === undefined ? {} : { api_key }),
    }] } }] }),
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
      return { documents: [{ provider: 'openai', document: { accounts: [{
        id: 'acct-1', accountId: 'chatgpt-acct', accessToken: fixtureAccessToken,
      }] } }] }
    },
    fetchImpl: async (url, init) => {
      assert.equal(url, 'https://bps.openai.com/basispoints/api/responses')
      assert.equal(init.headers.authorization, `Bearer ${fixtureAccessToken}`)
      assert.equal(init.headers['chatgpt-account-id'], 'chatgpt-acct')
      assert.equal(init.headers['x-openai-account-id'], 'chatgpt-acct')
      assert.equal(init.headers['x-basispoints-auth-mode'], 'chatgpt')
      assert.equal(init.headers.origin, 'https://bps.openai.com')
      assert.equal(init.headers['x-openai-internal-basispoints-client-product'], 'basispoints-excel-plugin')
      const body = JSON.parse(init.body)
      assert.deepEqual(body, {
        model: 'gpt-test', input: 'hello', model_selection: 'explicit', stream: false, store: false,
        reasoning_effort: 'medium', metadata: body.metadata,
      })
      assert.match(body.metadata.task_id, /^[0-9a-f-]{36}$/)
      assert.match(body.metadata.turn_id, /^[0-9a-f-]{36}$/)
      assert.equal(body.metadata.agent_iteration, '0')
      return new Response('{"ok":true}', { status: 200, headers: { 'content-type': 'application/json' } })
    },
  })
  assert.equal(result.status, 200)
  assert.equal(result.contentType, 'application/json')
  assert.equal(result.body.toString(), '{"ok":true}')
})

test('falls back to the JWT ChatGPT account claim when export metadata is absent', async () => {
  const token = fixtureJwt({ 'https://api.openai.com/auth': { chatgpt_account_id: 'claim-acct' } })
  const options = fixtureOptions({ accessToken: token })
  options.upstream = async () => ({ documents: [{ provider: 'openai', document: { accounts: [{ id: 'acct-1', accessToken: token }] } }] })
  options.fetchImpl = async (_url, init) => {
    assert.equal(init.headers['chatgpt-account-id'], 'claim-acct')
    return new Response('{}', { status: 200, headers: { 'content-type': 'application/json' } })
  }
  const result = await requestBasispoints(options)
  assert.equal(result.status, 200)
})

test('preserves request context while normalizing BPS contract fields', async () => {
  const options = fixtureOptions()
  options.request = {
    model: 'gpt-test', stream: true, reasoning: { effort: 'x-high' },
    metadata: { task_id: 'task-1', turn_id: 'turn-1', agent_iteration: 2, custom: 'keep' },
    input: [
      { type: 'message', role: 'user', content: [{ type: 'input_text', text: 'first' }] },
      { type: 'message', role: 'assistant', content: [{ type: 'output_text', text: 'second' }] },
    ],
  }
  options.fetchImpl = async (_url, init) => {
    const body = JSON.parse(init.body)
    assert.equal(body.stream, true)
    assert.equal(body.store, false)
    assert.equal(body.model_selection, 'explicit')
    assert.equal(body.reasoning_effort, 'xhigh')
    assert.deepEqual(body.metadata, { task_id: 'task-1', turn_id: 'turn-1', agent_iteration: '2', custom: 'keep' })
    assert.equal(body.input.length, 2)
    return new Response('data: ok\n\n', { status: 200, headers: { 'content-type': 'text/event-stream' } })
  }
  const result = await requestBasispoints(options)
  assert.equal(result.contentType, 'text/event-stream')
})

test('rejects API keys, malformed JWTs, missing account ids, and claim mismatches', async () => {
  await assert.rejects(() => requestBasispoints(fixtureOptions({ apiKey: 'fixture-api-key' })), /OAuth/)
  await assert.rejects(() => requestBasispoints(fixtureOptions({ api_key: 'fixture-api-key' })), /OAuth/)
  await assert.rejects(() => requestBasispoints(fixtureOptions({ accessToken: 'opaque-token' })), /JWT/)
  await assert.rejects(() => requestBasispoints(fixtureOptions({ accountId: null, tokenClaim: {} })), /账号/)
  await assert.rejects(() => requestBasispoints(fixtureOptions({ accountId: 'host-acct', tokenClaim: { chatgpt_account_id: 'other-acct' } })), /账号/)
})

test('rejects a legacy snake_case token field', async () => {
  const options = fixtureOptions()
  options.upstream = async () => ({ documents: [{ provider: 'openai', document: { accounts: [{
    id: 'acct-1', accountId: 'chatgpt-acct', access_token: fixtureAccessToken,
  }] } }] })
  await assert.rejects(() => requestBasispoints(options), /OAuth/)
})

test('uses the bridge account id character set', async () => {
  await assert.rejects(() => requestBasispoints({ ...fixtureOptions(), accountId: 'acct:1' }), /账号 ID 无效/)
  await assert.rejects(() => requestBasispoints(fixtureOptions({ accountId: 'chat:gpt' })), /账号 ID 无效/)
})

test('stops reading an oversized response stream at the size limit', async () => {
  let cancelled = false
  const options = fixtureOptions()
  options.fetchImpl = async () => ({
    status: 200,
    headers: new Headers({ 'content-type': 'application/json' }),
    body: {
      getReader: () => ({
        read: async () => ({ done: false, value: new Uint8Array(MAXIMUM_RESPONSE_BYTES + 1) }),
        cancel: async () => { cancelled = true },
        releaseLock: () => {},
      }),
    },
    arrayBuffer: async () => { throw new Error('full body read is forbidden') },
  })
  await assert.rejects(() => requestBasispoints(options), /大小限制/)
  assert.equal(cancelled, true)
})

test('maps Basis Points transport failures without exposing credentials', async () => {
  await assert.rejects(
    () => requestBasispoints({ ...fixtureOptions(), fetchImpl: async () => { throw new Error('fixture token should not escape') } }),
    error => error?.statusCode === 502 && !error.message.includes('fixture token'),
  )
})
