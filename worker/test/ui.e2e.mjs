import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { readFile, mkdir } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { chromium } from 'playwright'

const dist = process.env.UI_DIST || fileURLToPath(new URL('../../frontend/dist/', import.meta.url))
const output = fileURLToPath(new URL('../../../verification/', import.meta.url))
test('batch 2FA UI: desktop/mobile, upload, start, retry, cancel and existing OAuth', async t => {
  const server = createServer(async (req, res) => {
    const pathname = new URL(req.url, 'http://local').pathname
    const path = pathname.startsWith('/assets/') ? pathname.slice(1) : 'index.html'
    try {
      res.setHeader('content-type', path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css' : path.endsWith('.woff2') ? 'font/woff2' : 'text/html')
      res.end(await readFile(dist + path))
    } catch { res.writeHead(404).end() }
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  t.after(() => new Promise(resolve => server.close(resolve)))
  const browser = await chromium.launch()
  t.after(() => browser.close())
  await mkdir(output, { recursive: true })
  for (const width of [1440, 390]) {
    const page = await browser.newPage({ viewport: { width, height: width === 390 ? 844 : 1000 }, reducedMotion: 'reduce' })
    const errors = []
    page.on('pageerror', error => errors.push(error.message))
    let task
    let submitted
    let cancelled = false
    let saved = true
    let reauthorized
    let manualStart
    const account = { id: 'fixture-account', name: 'Fixture', email: 'fixture@example.com', provider: 'openai', authenticationKind: 'oauth', status: 'error', errorReason: 'credential_invalid', errorMessage: '401 Unauthorized', planType: 'free', planTypeDisplay: 'Free', enabled: true, weight: 2, concurrencyLimit: 3, groups: [], modelAccess: { mode: 'all', models: [] }, quota: { windows: [], limitReached: false, recoveryProbeRequired: false }, usage: { costs: [], models: [], requestCount: 0, requestCountDisplay: '0', totalTokensDisplay: '0' } }
    await page.route('**/api/**', async route => {
      const req = route.request()
      const url = new URL(req.url())
      let data = { items: [], page: { page: 1, pageSize: 20, total: 0, totalPages: 0 } }
      if (url.pathname === '/api/auth/status') data = { authenticated: true, session: { role: 'admin', expiresAt: '2099-01-01T00:00:00Z' } }
      if (url.pathname === '/api/admin/accounts') data = { items: [account], summary: { total: 1, normal: 0, error: 1, disabled: 0, quotaExhausted: 0, rateLimited: 0 }, page: { page: 1, pageSize: 20, total: 1, totalPages: 1 } }
      if (url.pathname.includes('/oauth/start')) {
        manualStart = req.postDataJSON()
        data = { flowId: 'manual', authorizationUrl: 'https://auth.openai.com/fixture' }
      }
      if (url.pathname.includes('/twofa/accounts/')) {
        if (req.method() === 'DELETE') saved = false
        data = { saved }
        if (url.pathname.endsWith('/reauthorize')) {
          reauthorized = req.postDataJSON()
          task = { id: 'reauth-task', running: false, cancelled: false, items: [{ id: 'one', email: account.email, status: 'succeeded', credentialsSaved: true, attempts: 1, accountId: account.id }] }
          data = task
        }
      }
      if (url.pathname.includes('/twofa/tasks')) {
        if (req.method() === 'POST' && url.pathname.endsWith('/tasks')) {
          submitted = req.postDataJSON()
          task = { id: 'fixture-task', running: false, cancelled: false, items: [
            { id: 'one', email: 'first@example.com', status: 'succeeded', attempts: 1, accountId: 'fixture' },
            { id: 'two', email: 'second@example.com', status: 'failed', attempts: 1, message: '验证失败' },
          ] }
        }
        if (url.pathname.endsWith('/retry')) task = { ...task, running: true, items: task.items.map(i => i.status === 'failed' ? { ...i, status: 'login', attempts: 2 } : i) }
        if (url.pathname.endsWith('/cancel')) { cancelled = true; task = { ...task, cancelled: true, running: false, items: task.items.map(i => i.status === 'login' ? { ...i, status: 'cancelled' } : i) } }
        data = task || {}
      }
      await route.fulfill({ json: { code: 200, message: 'ok', data } })
    })
    await page.goto(`http://127.0.0.1:${server.address().port}/accounts`)
    await page.getByRole('button', { name: '导入账号', exact: true }).click()
    await page.getByRole('radio', { name: /批量/ }).click()
    await page.getByRole('button', { name: '继续导入', exact: true }).click()
    if (process.env.BASELINE) await page.screenshot({ path: `${output}baseline-${width}.png` })
    await page.getByLabel('2FA 账号内容', { exact: true }).waitFor({ timeout: 3000 })
    await page.screenshot({ path: `${output}twofa-empty-${width}.png` })
    await page.locator('input[type=file]').setInputFiles({ name: 'fixture.txt', mimeType: 'text/plain', buffer: Buffer.from('first@example.com----fixture-password----JBSWY3DPEHPK3PXP\nsecond@example.com----fixture-password----JBSWY3DPEHPK3PXP') })
    await page.getByRole('button', { name: '开始授权登录', exact: true }).click()
    await page.getByText('second@example.com', { exact: true }).waitFor()
    assert.equal(submitted.settings.weight, 1)
    assert.equal(await page.locator('textarea').count(), 0)
    await page.screenshot({ path: `${output}twofa-results-${width}.png` })
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
    await page.getByRole('button', { name: '重试失败账号', exact: true }).click()
    await page.getByRole('button', { name: '取消任务', exact: true }).click()
    assert.equal(cancelled, true)
    await page.getByRole('button', { name: '完成', exact: true }).click()
    await page.getByRole('button', { name: '导入账号', exact: true }).click()
    await page.getByRole('radiogroup', { name: '选择账号平台' }).getByRole('radio', { name: 'OpenAI', exact: true }).click()
    await page.getByRole('button', { name: '继续导入', exact: true }).click()
    await page.getByText('OpenAI OAuth 授权', { exact: true }).waitFor()
    await page.getByRole('button', { name: '关闭', exact: true }).click()
    await page.getByRole('button', { name: '更多操作', exact: true }).click()
    await page.getByRole('button', { name: '重新授权', exact: true }).click()
    await page.getByRole('radio', { name: '2FA 登录', exact: true }).waitFor({ timeout: 3000 })
    await page.getByText('已保存 2FA 信息', { exact: true }).waitFor()
    assert.equal(await page.getByLabel('2FA 账号内容', { exact: true }).count(), 0)
    await page.getByRole('button', { name: '更新 2FA 信息', exact: true }).click()
    await page.getByLabel('2FA 账号内容', { exact: true }).waitFor()
    await page.getByRole('button', { name: '使用已保存信息', exact: true }).click()
    await page.screenshot({ path: `${output}twofa-saved-${width}.png` })
    await page.getByRole('button', { name: '使用已保存 2FA 登录', exact: true }).click()
    await page.getByText('重新授权成功', { exact: true }).waitFor()
    assert.ok(reauthorized.submissionId)
    assert.equal(reauthorized.text, undefined)
    assert.equal(reauthorized.settings, undefined)
    await page.screenshot({ path: `${output}twofa-reauthorized-${width}.png` })
    await page.getByRole('button', { name: '完成', exact: true }).click()
    await page.getByRole('button', { name: '更多操作', exact: true }).click()
    await page.getByRole('button', { name: '重新授权', exact: true }).click()
    await page.getByRole('button', { name: '清除已保存信息', exact: true }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: '清除', exact: true }).click()
    assert.equal(saved, false)
    await page.getByLabel('2FA 账号内容', { exact: true }).waitFor()
    await page.screenshot({ path: `${output}twofa-missing-${width}.png` })
    await page.getByLabel('2FA 账号内容', { exact: true }).fill('fixture@example.com----fixture-password----JBSWY3DPEHPK3PXP')
    await page.getByRole('button', { name: '保存并重新授权', exact: true }).click()
    await page.getByText('重新授权成功', { exact: true }).waitFor()
    assert.ok(reauthorized.text.startsWith('fixture@example.com----'))
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
    await page.getByRole('button', { name: '完成', exact: true }).click()
    await page.getByRole('button', { name: '更多操作', exact: true }).click()
    await page.getByRole('button', { name: '重新授权', exact: true }).click()
    await page.getByRole('radio', { name: '授权链接', exact: true }).click()
    await page.getByRole('button', { name: '生成授权链接', exact: true }).click()
    assert.equal(manualStart.accountId, account.id)
    assert.deepEqual(errors, [])
    await page.close()
  }
})
