import { setTimeout as delay } from 'node:timers/promises'
import { authenticator } from 'otplib'
import { oauthTarget, PublicError } from './core.mjs'
import { browserContextOptions, browserFingerprintInitScript } from './browser-profile.mjs'

const viewport = { width: 1024, height: 768 }
const keys = new Set(['Enter', 'Tab', 'Shift+Tab', 'Backspace', 'Delete', 'Escape', 'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Space'])

async function visible(page, selectors) {
  for (const selector of selectors) {
    const locator = page.locator(selector).first()
    if (await locator.isVisible().catch(() => false)) return locator
  }
}

export async function classify(page) {
  const body = await page.locator('body').innerText().catch(() => '')
  if (/account_deactivated|authentication error|account has been (?:deleted|deactivated)/i.test(body))
    throw new PublicError(422, 'OpenAI 账号已被删除或停用，请更换账号')
  if (await visible(page, ['iframe[src*="captcha"]', '[data-testid="captcha"]', 'iframe[src*="challenges.cloudflare.com"]'])) return { stage: 'waiting' }
  const error = await visible(page, ['[role="alert"]', '[data-error-code]'])
  if (error && (await error.textContent())?.trim()) throw new PublicError(422, '登录页面返回验证错误，请检查账号信息或手动授权')
  const password = await visible(page, ['input[type="password"]'])
  if (password) return { stage: 'password', input: password }
  const email = await visible(page, ['input[type="email"]', 'input[name="username"]'])
  if (email) return { stage: 'login', input: email }
  const totp = await visible(page, ['input[name="totp"]', 'input[autocomplete="one-time-code"]', 'input[name="code"]', 'input[inputmode="numeric"]'])
  // 邮箱验证码不使用 TOTP 猜测；只有验证器页面才自动填写动态码。
  if (totp && /authenticator|authentication app|multi.factor authentication|验证器|身份验证应用/i.test(body))
    return { stage: 'totp', input: totp }
  const consent = await visible(page, ['button[name="authorize"]', '[data-testid="consent"]', 'button:has-text("Allow")', 'button:has-text("Authorize")'])
  if (consent) return { stage: 'consent', button: consent }
  if (/\/consent(?:\?|$|\/)/.test(page.url())) {
    const button = await visible(page, ['button[type="submit"]', 'button:has-text("Continue")'])
    if (button) return { stage: 'consent', button }
  }
  return { stage: 'unknown' }
}

function takeover(page, signal, resume) {
  let chain = Promise.resolve()
  let count = 0
  let started = Date.now()
  return {
    screen: () => page.screenshot({ type: 'jpeg', quality: 65, scale: 'css', mask: [page.locator('input, textarea')] }),
    input(value) {
      if (signal.aborted) throw new PublicError(409, '任务已结束')
      if (Date.now() - started > 1000) { count = 0; started = Date.now() }
      if (++count > 20) throw new PublicError(429, '操作过于频繁')
      if (!value || typeof value !== 'object') throw new PublicError(400, '验证操作格式错误')
      const valid = value.kind === 'resume'
        || (value.kind === 'click' && Number.isFinite(value.x) && Number.isFinite(value.y) && value.x >= 0 && value.x < viewport.width && value.y >= 0 && value.y < viewport.height)
        || (value.kind === 'text' && typeof value.text === 'string' && value.text.length <= 4096)
        || (value.kind === 'key' && keys.has(value.key))
        || (value.kind === 'scroll' && Number.isFinite(value.delta) && Math.abs(value.delta) <= 2000)
      if (!valid) throw new PublicError(400, '验证操作格式错误')
      chain = chain.catch(() => {}).then(async () => {
        if (value.kind === 'resume') resume()
        if (value.kind === 'click') await page.mouse.click(value.x, value.y)
        if (value.kind === 'text') await page.keyboard.insertText(value.text)
        if (value.kind === 'key') await page.keyboard.press(value.key)
        if (value.kind === 'scroll') await page.mouse.wheel(0, value.delta)
      })
      return chain
    },
  }
}

export function createBrowserRunner({ browser, upstream, allowedOrigin = 'https://auth.openai.com', timeoutMs = 300000 }) {
  return async ({ credentials, cookie, settings, targetAccountId, outboundProxyId, signal, update }) => {
    signal.throwIfAborted()
    const flow = await upstream('/api/admin/accounts/oauth/start', cookie, { provider: 'openai', name: credentials.email, ...(targetAccountId ? { accountId: targetAccountId } : { outboundProxyId }) })
    signal.throwIfAborted()
    const target = oauthTarget(flow.authorizationUrl, allowedOrigin)
    const browserVersion = typeof browser.version === 'function' ? browser.version() : undefined
    // 第三方 OAuth 页面走 Worker 所在环境的网络，账号出站代理只由服务端交换与业务请求使用。
    const context = await browser.newContext(browserContextOptions({ browserVersion }))
    let callback
    let callbackError
    let timedOut = false
    const abort = () => { void context.close().catch(() => {}) }
    signal.addEventListener('abort', abort, { once: true })
    const timeout = setTimeout(() => { timedOut = true; abort() }, timeoutMs)
    timeout.unref()
    try {
      signal.throwIfAborted()
      if (typeof context.addInitScript === 'function') await context.addInitScript(browserFingerprintInitScript)
      const page = await context.newPage()
      signal.throwIfAborted()
      page.setDefaultTimeout(15000)
      // 每次主框架重定向都经过网络层校验，不能只校验最初的授权 URL。
      const cdp = await context.newCDPSession(page)
      const { frameTree } = await cdp.send('Page.getFrameTree')
      await cdp.send('Fetch.enable', { patterns: [{ urlPattern: '*', resourceType: 'Document', requestStage: 'Request' }] })
      cdp.on('Fetch.requestPaused', async event => {
        const url = new URL(event.request.url)
        if (event.frameId === frameTree.frame.id) {
          if (`${url.origin}${url.pathname}` === target.callback) {
            if (url.searchParams.getAll('state').length !== 1 || url.searchParams.get('state') !== target.state
              || url.searchParams.getAll('code').length !== 1 || !url.searchParams.get('code'))
              callbackError = new PublicError(422, 'OAuth 回调校验失败')
            else callback = url.href
            await cdp.send('Fetch.fulfillRequest', { requestId: event.requestId, responseCode: 200,
              responseHeaders: [{ name: 'Content-Type', value: 'text/html' }], body: Buffer.from('<h1>OAuth complete</h1>').toString('base64') }).catch(() => {})
            return
          }
          if (![allowedOrigin, 'https://chatgpt.com'].includes(url.origin)) {
            callbackError = new PublicError(422, '登录跳转到额外身份验证站点，请使用单账号授权')
            await cdp.send('Fetch.failRequest', { requestId: event.requestId, errorReason: 'BlockedByClient' }).catch(() => {})
            return
          }
        }
        await cdp.send('Fetch.continueRequest', { requestId: event.requestId }).catch(() => {})
      })
      await page.goto(target.url, { waitUntil: 'domcontentloaded', timeout: 45000 })
      let lastAction = ''
      let lastActionAt = 0
      let unknownSince = Date.now()
      while (!callback) {
        signal.throwIfAborted()
        if (timedOut) throw new PublicError(408, '登录超时，请重试')
        if (callbackError) throw callbackError
        const state = await classify(page)
        const action = `${page.url()}|${state.stage}`
        if (state.stage === 'unknown' && Date.now() - unknownSince < 4000) { await delay(250); continue }
        if (state.stage === 'waiting' || state.stage === 'unknown' || (action === lastAction && Date.now() - lastActionAt > 20000)) {
          let resumed = false
          update('waiting', takeover(page, signal, () => { resumed = true }))
          while (!resumed && !callback && !callbackError && !timedOut && !signal.aborted) await delay(200)
          update('starting')
          lastAction = ''
          unknownSince = Date.now()
          continue
        }
        if (lastAction === action) { await delay(250); continue }
        unknownSince = Date.now()
        lastAction = action
        lastActionAt = Date.now()
        update(state.stage)
        if (['login', 'password', 'totp'].includes(state.stage) && new URL(page.url()).origin !== allowedOrigin)
          throw new PublicError(422, '登录跳转来源校验失败')
        if (state.stage === 'login') await state.input.fill(credentials.email)
        if (state.stage === 'password') await state.input.fill(credentials.password)
        if (state.stage === 'totp') {
          // 避免在时间窗口即将结束时提交已失效的动态码。
          if (authenticator.timeRemaining() <= 3) await delay(3500, undefined, { signal })
          await state.input.fill(authenticator.generate(credentials.totpSecret.replace(/=+$/, '')))
        }
        const button = state.button || await visible(page, ['button[type="submit"]', 'button:has-text("Continue")', 'button:has-text("继续")'])
        if (!button) { lastActionAt = 0; continue }
        await button.click()
        await delay(200)
      }
      if (callbackError) throw callbackError
      signal.throwIfAborted()
      update('importing')
      // 提交已开始时保留真实导入结果，取消只影响尚未提交的账号。
      return await upstream('/api/admin/accounts/oauth/complete', cookie, { provider: 'openai', flowId: flow.flowId, callbackUrl: callback, ...(targetAccountId ? {} : { settings }) })
    } catch (error) {
      if (callbackError) throw callbackError
      if (timedOut) throw new PublicError(408, '登录超时，请重试')
      if (error instanceof PublicError || signal.aborted) throw error
      throw new PublicError(502, '浏览器登录失败，请检查网络、代理或使用单账号授权')
    } finally {
      clearTimeout(timeout)
      signal.removeEventListener('abort', abort)
      await context.close().catch(() => {})
    }
  }
}
