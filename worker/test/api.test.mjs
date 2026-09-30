import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createApp } from '../src/app.mjs'
import { PublicError } from '../src/core.mjs'
import { setTimeout as delay } from 'node:timers/promises'
import { mkdtemp, writeFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { randomBytes } from 'node:crypto'
import { openVault } from '../src/vault.mjs'

const origin = 'https://fixture.example'
const prefix = '/api/admin/twofa'
const payload = { text: 'fixture@example.com----private-password----JBSWY3DPEHPK3PXP', submissionId: 'test', settings: { enabled: true, concurrencyLimit: null, weight: 1, groupIds: [] } }
const headers = { origin, cookie: 'cpr_session=admin', 'x-cpr-twofa': '1' }
const upstream = async (path, cookie) => {
  if (path === '/api/auth/status') return { authenticated: !!cookie, session: { role: cookie.includes('admin') ? 'admin' : 'key' } }
  return {}
}

function fixtureJwt(payload) {
  const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url')
  return `${encode({ alg: 'none', typ: 'JWT' })}.${encode(payload)}.fixture-signature`
}

test('Basis Points route forwards a bounded raw response without exposing the access token', async t => {
  const fixtureToken = fixtureJwt({ 'https://api.openai.com/auth': {} })
  const calls = []
  let basispointsCalls = 0
  const routeUpstream = async (path, cookie) => {
    calls.push({ path, cookie })
    if (path === '/api/auth/status') return upstream(path, cookie)
    if (path === '/api/admin/accounts/export?accountIds=acct-1&confirm=export_sensitive_accounts') {
      return { documents: [{ provider: 'openai', document: { accounts: [{ id: 'acct-1', accountId: 'chatgpt-acct', accessToken: fixtureToken }] } }] }
    }
    throw new Error(`unexpected upstream request: ${path}`)
  }
  const app = await createApp({
    origin, upstream: routeUpstream, run: async () => ({}),
    basispointsFetch: async (url, init) => {
      basispointsCalls++
      assert.equal(url, 'https://bps.openai.com/basispoints/api/responses')
      assert.equal(init.headers.authorization, `Bearer ${fixtureToken}`)
      assert.deepEqual(JSON.parse(init.body), { model: 'gpt-test', input: 'hello' })
      return new Response('{"output":"fixture"}', { status: 201, headers: { 'content-type': 'application/json; charset=utf-8' } })
    },
  })
  t.after(() => app.close())
  const response = await app.inject({
    method: 'POST', url: '/api/basispoints/responses', headers,
    payload: { accountId: 'acct-1', request: { model: 'gpt-test', input: 'hello' } },
  })
  assert.equal(response.statusCode, 201)
  assert.equal(response.body, '{"output":"fixture"}')
  assert.match(response.headers['content-type'], /^application\/json/)
  assert.equal(basispointsCalls, 1)
  assert.deepEqual(calls.filter(call => call.path.includes('/api/admin/accounts/export')), [{
    path: '/api/admin/accounts/export?accountIds=acct-1&confirm=export_sensitive_accounts', cookie: 'cpr_session=admin',
  }])
  assert.ok(!response.body.includes(fixtureToken))
})

test('Basis Points route guards management requests and validates its wrapper before fetching', async t => {
  let basispointsCalls = 0
  const app = await createApp({
    origin, upstream, run: async () => ({}),
    basispointsFetch: async () => { basispointsCalls++; return new Response('{}', { headers: { 'content-type': 'application/json' } }) },
  })
  t.after(() => app.close())
  const validPayload = { accountId: 'acct-1', request: { model: 'gpt-test' } }
  for (const requestHeaders of [
    { origin, 'x-cpr-twofa': '1' },
    { ...headers, cookie: 'cpr_session=key' },
    { ...headers, origin: 'https://other.example' },
    { origin, cookie: headers.cookie },
  ]) {
    const response = await app.inject({ method: 'POST', url: '/api/basispoints/responses', headers: requestHeaders, payload: validPayload })
    assert.ok([401, 403].includes(response.statusCode), `${response.statusCode} for ${JSON.stringify(requestHeaders)}`)
  }
  for (const invalidPayload of [
    { accountId: 'acct-1', request: { model: 'gpt-test' }, extra: true },
    { accountId: 'acct-1', request: null },
    { accountId: 'acct-1', request: 'gpt-test' },
  ]) {
    const response = await app.inject({ method: 'POST', url: '/api/basispoints/responses', headers, payload: invalidPayload })
    assert.equal(response.statusCode, 400)
  }
  assert.equal(basispointsCalls, 0)
})

test('admin guard rejects anonymous/key sessions and cross-origin/missing CSRF headers', async t => {
  const app = await createApp({ origin, upstream, run: async () => ({ accountId: 'ok' }) })
  t.after(() => app.close())
  for (const h of [{ origin, 'x-cpr-twofa': '1' }, { ...headers, cookie: 'cpr_session=key' }, { ...headers, origin: 'https://other.example' }, { origin, cookie: headers.cookie }]) {
    const response = await app.inject({ method: 'POST', url: `${prefix}/tasks`, headers: h, payload })
    assert.ok([401, 403].includes(response.statusCode))
    assert.ok(!response.body.includes('private-password'))
  }
})

test('import saves account credentials; saved reauthorization binds original account, uses current proxy, and returns metadata only', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'cpr-api-vault-test-'))
  t.after(() => rm(directory, { recursive: true, force: true }))
  const vaultOptions = { directory: join(directory, 'data'), keyFile: join(directory, 'key') }
  await writeFile(vaultOptions.keyFile, randomBytes(32))
  const vault = await openVault(vaultOptions)
  let deleted = false
  let account = { id: 'original', email: 'fixture@example.com', provider: 'openai', authenticationKind: 'oauth', outboundProxyEndpoint: null }
  const calls = []
  const official = async (path, cookie) => {
    if (path.startsWith('/api/admin/accounts/detail')) {
      if (deleted) throw new PublicError(404, '账号不存在')
      return { account }
    }
    if (path.startsWith('/api/admin/proxies')) return { items: [{ id: 'current-proxy', endpoint: 'http://127.0.0.1:9999', lastTest: { success: false }, hasAuthentication: false }], page: { totalPages: 1 } }
    return upstream(path, cookie)
  }
  const options = { origin, upstream: official, vault, run: async input => { calls.push({ targetAccountId: input.targetAccountId, proxy: input.proxy, password: input.credentials.password }); return { accountId: 'original' } } }
  let app = await createApp(options)
  t.after(() => app.close())
  async function wait(id) {
    for (let i = 0; i < 50; i++) {
      const result = (await app.inject({ url: `${prefix}/tasks/${id}`, headers })).json().data
      if (!result.running) return result
      await delay(5)
    }
    assert.fail('task did not settle')
  }
  let response = await app.inject({ method: 'POST', url: `${prefix}/tasks`, headers, payload })
  assert.equal(response.statusCode, 200)
  const result = await wait(response.json().data.id)
  assert.equal(result.items[0].credentialsSaved, true)
  assert.equal((await vault.get('original')).credentials.password, 'private-password')
  await app.close()
  app = await createApp({ ...options, vault: await openVault(vaultOptions) })
  response = await app.inject({ url: `${prefix}/accounts/original`, headers })
  assert.equal(response.json().data.saved, true)
  assert.ok(!response.body.includes('private-password'))
  assert.ok(!response.body.includes('JBSWY'))
  account.outboundProxyEndpoint = 'http://127.0.0.1:9999'
  response = await app.inject({ method: 'POST', url: `${prefix}/accounts/original/reauthorize`, headers, payload: { submissionId: 'reauth' } })
  assert.equal(response.statusCode, 200)
  assert.equal((await wait(response.json().data.id)).items[0].status, 'succeeded')
  assert.equal(calls[1].targetAccountId, 'original')
  assert.equal(calls[1].proxy.server, 'http://127.0.0.1:9999')
  assert.equal(calls[1].password, 'private-password')
  response = await app.inject({ method: 'POST', url: `${prefix}/accounts/original/reauthorize`, headers, payload: { submissionId: 'mismatch', text: payload.text.replace('fixture@', 'wrong@') } })
  assert.equal(response.statusCode, 400)
  account.provider = 'xai'
  response = await app.inject({ method: 'POST', url: `${prefix}/accounts/original/reauthorize`, headers, payload: { submissionId: 'wrong-provider' } })
  assert.equal(response.statusCode, 400)
  account.provider = 'openai'
  deleted = true
  response = await app.inject({ url: `${prefix}/accounts/original`, headers })
  assert.equal(response.statusCode, 404)
  assert.equal(await vault.get('original'), null)
})

test('clear cannot race an in-flight saved-credential read and silently resurrect credentials', async t => {
  let release
  let started
  const reading = new Promise(resolve => { started = resolve })
  const gate = new Promise(resolve => { release = resolve })
  let deletes = 0
  const app = await createApp({ origin, upstream: async (path, cookie) => path.startsWith('/api/admin/accounts/detail')
    ? { account: { id: 'original', email: 'fixture@example.com', provider: 'openai', authenticationKind: 'oauth' } } : upstream(path, cookie),
  run: async () => ({ accountId: 'original' }), vault: {
    get: async () => { started(); await gate; return { credentials: { email: 'fixture@example.com', password: 'fixture-password', totpSecret: 'JBSWY3DPEHPK3PXP' } } },
    put: async () => {}, delete: async () => { deletes++ },
  } })
  t.after(() => app.close())
  const pending = app.inject({ method: 'POST', url: `${prefix}/accounts/original/reauthorize`, headers, payload: { submissionId: 'read-race' } }).then(result => result)
  await reading
  const result = await app.inject({ method: 'DELETE', url: `${prefix}/accounts/original`, headers: { ...headers, cookie: 'cpr_session=admin-other' } })
  release()
  await pending
  assert.equal(result.statusCode, 409)
  assert.equal(deletes, 0)
})

test('clear invalidates failed reauthorization retry credentials before asynchronous disk deletion', async t => {
  let release
  let started
  const deleting = new Promise(resolve => { started = resolve })
  const gate = new Promise(resolve => { release = resolve })
  let runs = 0
  const app = await createApp({ origin, upstream: async (path, cookie) => path.startsWith('/api/admin/accounts/detail')
    ? { account: { id: 'original', email: 'fixture@example.com', provider: 'openai', authenticationKind: 'oauth' } } : upstream(path, cookie),
  run: async () => { runs++; throw new Error('fixture login failure') }, vault: {
    get: async () => ({ credentials: { email: 'fixture@example.com', password: 'fixture-password', totpSecret: 'JBSWY3DPEHPK3PXP' } }),
    put: async () => {}, delete: async () => { started(); await gate },
  } })
  t.after(() => app.close())
  const task = (await app.inject({ method: 'POST', url: `${prefix}/accounts/original/reauthorize`, headers, payload: { submissionId: 'retry-race' } })).json().data
  for (let i = 0; i < 50; i++) {
    const status = (await app.inject({ url: `${prefix}/tasks/${task.id}`, headers })).json().data
    if (!status.running) break
    await delay(5)
  }
  const removal = app.inject({ method: 'DELETE', url: `${prefix}/accounts/original`, headers: { ...headers, cookie: 'cpr_session=admin-other' } }).then(result => result)
  await deleting
  const retry = await app.inject({ method: 'POST', url: `${prefix}/tasks/${task.id}/retry`, headers })
  release()
  assert.equal((await removal).statusCode, 200)
  assert.equal(retry.statusCode, 409)
  assert.equal(runs, 1)
})

test('saved credential endpoints enforce admin and CSRF and allow removal without exposing secrets', async t => {
  let removed = false
  const app = await createApp({ origin, upstream, run: async () => ({}), vault: { get: async () => ({ credentials: { password: 'private-password' } }), delete: async () => { removed = true } } })
  t.after(() => app.close())
  for (const method of ['GET', 'DELETE', 'POST']) {
    const url = `${prefix}/accounts/original${method === 'POST' ? '/reauthorize' : ''}`
    for (const h of [{}, { ...headers, origin: 'https://other.example' }, { ...headers, cookie: 'cpr_session=key' }]) {
      const result = await app.inject({ method, url, headers: h, ...(method === 'POST' ? { payload: { submissionId: 'x' } } : {}) })
      assert.ok([401, 403].includes(result.statusCode))
      assert.ok(!result.body.includes('private-password'))
    }
  }
  const result = await app.inject({ method: 'DELETE', url: `${prefix}/accounts/original`, headers })
  assert.equal(result.statusCode, 200)
  assert.equal(removed, true)
})
test('JSON errors never echo secrets and tasks are session-isolated', async t => {
  const app = await createApp({ origin, upstream, run: async () => ({ accountId: 'ok' }) })
  t.after(() => app.close())
  const bad = await app.inject({ method: 'POST', url: `${prefix}/tasks`, headers: { ...headers, 'content-type': 'application/json' }, payload: '{"private-password"' })
  assert.equal(bad.statusCode, 400)
  assert.ok(!bad.body.includes('private-password'))
  const response = await app.inject({ method: 'POST', url: `${prefix}/tasks`, headers, payload })
  assert.equal(response.statusCode, 200)
  const id = response.json().data.id
  assert.ok(!response.body.includes('JBSWY'))
  const foreign = await app.inject({ method: 'GET', url: `${prefix}/tasks/${id}`, headers: { ...headers, cookie: 'cpr_session=admin-other' } })
  assert.equal(foreign.statusCode, 404)
  const status = await app.inject({ method: 'GET', url: `${prefix}/tasks/${id}`, headers })
  assert.equal(status.headers['cache-control'], 'no-store')
})
