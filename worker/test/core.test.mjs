import { test } from 'node:test'
import assert from 'node:assert/strict'
import { setTimeout as delay } from 'node:timers/promises'
import { parseAccounts, oauthTarget, Jobs } from '../src/core.mjs'

const secret = 'JBSWY3DPEHPK3PXP'
const line = (email = 'fixture@example.com') => `${email}----private----password----${secret}`
const credentials = () => parseAccounts(line())
const owner = 'session-a'
async function settled(jobs, id) {
  for (let n = 0; n < 100; n++) {
    const task = jobs.read(id, owner)
    if (!task.running) return task
    await delay(10)
  }
  throw new Error('task did not settle')
}

test('TXT parses BOM, CRLF and password delimiters without losing password bytes', () => {
  const rows = parseAccounts(`\ufeff ${line()}\r\n\r\n`)
  assert.equal(rows.length, 1)
  assert.equal(rows[0].password, 'private----password')
  assert.equal(rows[0].totpSecret, secret)
})
test('invalid lines, duplicate identities and batch limits fail without echoing credentials', () => {
  for (const text of ['', 'fixture@example.com----private----BAD!', `${line()}\n${line()}`, Array.from({ length: 51 }, (_, i) => line(`${i}@example.com`)).join('\n')]) {
    assert.throws(() => parseAccounts(text), error => !error.message.includes('private') && !error.message.includes(secret))
  }
})
test('OAuth nested desktop URL is validated and callback is fixed', () => {
  const inner = 'https://auth.openai.com/oauth/authorize?state=fixture-state&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback'
  assert.equal(oauthTarget(`https://chatgpt.com/codex/desktop-auth?authorize_url=${encodeURIComponent(inner)}`).state, 'fixture-state')
  assert.throws(() => oauthTarget('https://evil.example/?state=x'))
  assert.throws(() => oauthTarget(inner.replace('localhost', 'evil.example')))
})
test('tasks bind to session, propagate settings, retry only failures, erase success secrets', async () => {
  let attempts = 0
  let captured
  const jobs = new Jobs({ run: async (input) => {
    captured = input.credentials
    assert.equal(input.settings.weight, 2)
    if (++attempts === 1) throw new Error(`raw secret ${secret}`)
    return { accountId: 'fixture-account' }
  } })
  const task = jobs.create({ owner, cookie: 'cookie', submissionId: 'fixture', credentials: credentials(), settings: { weight: 2 } })
  assert.throws(() => jobs.read(task.id, 'session-b'), /404/)
  const failed = await settled(jobs, task.id)
  assert.equal(failed.items[0].status, 'failed')
  assert.ok(!JSON.stringify(failed).includes(secret))
  jobs.retry(task.id, owner, 'cookie')
  const success = await settled(jobs, task.id)
  assert.equal(success.items[0].status, 'succeeded')
  assert.equal(success.items[0].accountId, 'fixture-account')
  assert.equal(captured.password, '')
  assert.equal(captured.totpSecret, '')
  jobs.close()
})
test('cancel erases running and queued secrets; duplicate submissions do not repeat login', async () => {
  let captured
  const jobs = new Jobs({ run: async ({ credentials, signal }) => {
    captured = credentials
    await new Promise(resolve => signal.addEventListener('abort', resolve, { once: true }))
    throw new Error('cancelled')
  } })
  const input = { owner, cookie: 'cookie', submissionId: 'fixture', credentials: credentials(), settings: {} }
  const task = jobs.create(input)
  assert.equal(jobs.create({ ...input, credentials: credentials() }).id, task.id)
  await delay(10)
  jobs.cancel(task.id, owner)
  assert.equal((await settled(jobs, task.id)).items[0].status, 'cancelled')
  assert.equal(captured.password, '')
  jobs.close()
})
test('abandoned task expires and clears retry credentials', async () => {
  let captured
  const jobs = new Jobs({ leaseMs: 60, run: async ({ credentials }) => { captured = credentials; throw new Error('fixture') } })
  const task = jobs.create({ owner, cookie: 'cookie', submissionId: 'fixture', credentials: credentials(), settings: {} })
  await settled(jobs, task.id)
  await delay(80)
  jobs.sweep()
  assert.equal(captured.totpSecret, '')
  assert.throws(() => jobs.read(task.id, owner), /404/)
  jobs.close()
})

test('successful authorization persists credentials before erasing and reports storage failures without repeating OAuth', async t => {
  let saved
  const jobs = new Jobs({ run: async () => ({ accountId: 'original' }), persist: async ({ credentials, accountId }) => {
    saved = { ...credentials, accountId }
    throw new Error('disk failure with secret')
  } })
  t.after(() => jobs.close())
  const task = jobs.create({ owner, cookie: 'cookie', submissionId: 'save', credentials: credentials(), targetAccountId: 'original' })
  const result = await settled(jobs, task.id)
  assert.equal(saved?.password, 'private----password')
  assert.equal(saved.accountId, 'original')
  assert.equal(result.items[0].status, 'succeeded')
  assert.equal(result.items[0].credentialsSaved, false)
  assert.match(result.items[0].message, /保存/)
  assert.throws(() => jobs.retry(task.id, owner, 'cookie'), /409/)
})

test('reauthorization locks the target across sessions and preserves committing credentials on cancel', async t => {
  let finish
  let saved
  const jobs = new Jobs({ run: async ({ targetAccountId, update }) => {
    assert.equal(targetAccountId, 'original')
    update('importing')
    await new Promise(resolve => { finish = resolve })
    return { accountId: 'original' }
  }, persist: async ({ credentials }) => { saved = { ...credentials } } })
  t.after(() => jobs.close())
  const input = { owner, cookie: 'cookie', submissionId: 'same', targetAccountId: 'original', credentials: credentials() }
  const task = jobs.create(input)
  assert.throws(() => jobs.create({ ...input, owner: 'another', credentials: credentials() }), /409/)
  assert.throws(() => jobs.create({ ...input, targetAccountId: 'different', credentials: credentials() }), /409/)
  jobs.cancel(task.id, owner)
  finish()
  const result = await settled(jobs, task.id)
  assert.equal(result.items[0].status, 'succeeded')
  assert.equal(result.items[0].credentialsSaved, true)
  assert.equal(saved.password, 'private----password')
})
