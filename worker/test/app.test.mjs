import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createApp } from '../src/app.mjs'

function fixtureJwt() {
  const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url')
  return `${encode({ alg: 'none', typ: 'JWT' })}.${encode({ 'https://api.openai.com/auth': { chatgpt_account_id: 'chatgpt-acct' } })}.signature`
}

test('accepts host-resolved credentials on the internal plugin path', async () => {
  const app = await createApp({
    origin: 'https://cx.subarx.com',
    upstream: async () => { throw new Error('admin session must not be used') },
    basispointsFetch: async (_url, init) => {
      assert.equal(init.headers['chatgpt-account-id'], 'chatgpt-acct')
      return new Response('{}', { status: 200, headers: { 'content-type': 'application/json' } })
    },
  })
  const response = await app.inject({
    method: 'POST',
    url: '/api/basispoints/responses',
    headers: { 'x-cpr-basispoints': '1', 'x-cpr-basispoints-internal': '1' },
    payload: { accountId: 'acct-1', chatgptAccountId: 'chatgpt-acct', accessToken: fixtureJwt(), request: { model: 'gpt-test' } },
  })
  assert.equal(response.statusCode, 200)
  await app.close()
})
