import { createHash } from 'node:crypto'
import Fastify from 'fastify'
import { Jobs, parseAccounts, PublicError } from './core.mjs'

const prefix = '/api/admin/twofa'
const settingsSchema = {
  type: 'object', additionalProperties: false, required: ['enabled', 'concurrencyLimit', 'weight', 'groupIds'],
  properties: {
    enabled: { type: 'boolean' }, concurrencyLimit: { anyOf: [{ type: 'null' }, { type: 'integer', minimum: 1, maximum: 100000 }] },
    weight: { type: 'number', minimum: 0, maximum: 100000 }, notes: { type: 'string', maxLength: 500 },
    groupIds: { type: 'array', maxItems: 100, items: { type: 'string', maxLength: 128 } },
    modelAccess: { type: 'object', additionalProperties: false, required: ['mode', 'models'], properties: {
      mode: { enum: ['all', 'allowlist', 'denylist'] }, models: { type: 'array', maxItems: 1000, items: { type: 'string', maxLength: 256 } },
    } },
  },
}

export function makeUpstream(base, origin) {
  return async (path, cookie, body) => {
    const response = await fetch(new URL(path, base), {
      method: body ? 'POST' : 'GET', redirect: 'error', signal: AbortSignal.timeout(30000),
      headers: { cookie, origin, 'content-type': 'application/json' },
      body: body ? JSON.stringify(body) : undefined,
    })
    const envelope = await response.json().catch(() => null)
    if (!response.ok || envelope?.code !== 200)
      throw new PublicError([401, 403, 404].includes(response.status) ? response.status : 502, 'Codex Proxy 接口请求失败，请检查服务和登录状态')
    return envelope.data
  }
}

async function resolveProxy(id, cookie, upstream, proxyMap, requireTest = true) {
  if (!id) return undefined
  for (let page = 1; page <= 100; page++) {
    const data = await upstream(`/api/admin/proxies?page=${page}&pageSize=100`, cookie)
    const record = data.items.find(item => item.id === id)
    if (record) {
      if (requireTest && !record.lastTest?.success) throw new PublicError(400, '所选代理尚未通过测试')
      const value = proxyMap[id] || (!record.hasAuthentication && record.endpoint)
      if (!value) throw new PublicError(400, '所选代理需要在登录服务配置对应认证信息')
      const url = new URL(value)
      if (!['http:', 'https:', 'socks5:'].includes(url.protocol)) throw new PublicError(400, '浏览器暂不支持此代理协议')
      if (url.protocol === 'socks5:' && url.username) throw new PublicError(400, '浏览器 SOCKS5 代理须使用免认证入口')
      return { server: `${url.protocol}//${url.host}`, username: decodeURIComponent(url.username) || undefined, password: decodeURIComponent(url.password) || undefined }
    }
    if (page >= data.page.totalPages) break
  }
  throw new PublicError(400, '所选代理不存在')
}

export async function createApp({ origin, upstream, run, vault, proxyMap = {}, leaseMs }) {
  const app = Fastify({ logger: false, bodyLimit: 140000, disableRequestLogging: true, ajv: { customOptions: { removeAdditional: false } } })
  const accountOperations = new Set()
  async function withAccountLock(id, action) {
    if (accountOperations.has(id)) throw new PublicError(409, '此账号的 2FA 信息正在使用，请稍后重试')
    accountOperations.add(id)
    try { return await action() } finally { accountOperations.delete(id) }
  }
  async function accountDetail(id, cookie) {
    let account
    try { ({ account } = await upstream(`/api/admin/accounts/detail?accountId=${encodeURIComponent(id)}`, cookie)) }
    catch (error) {
      if (error instanceof PublicError && error.statusCode === 404) await vault?.delete(id)
      throw error
    }
    if (account?.id !== id || account.provider !== 'openai' || account.authenticationKind !== 'oauth')
      throw new PublicError(400, '仅支持 OpenAI OAuth 账号')
    return account
  }
  function checkEmail(account, credentials) {
    if (!account.email || account.email.toLowerCase() !== credentials.email.toLowerCase())
      throw new PublicError(400, '2FA 邮箱必须与原账号一致')
  }
  async function currentProxy(account, cookie) {
    if (!account.outboundProxyEndpoint) return undefined
    const matches = []
    for (let page = 1; page <= 100; page++) {
      const data = await upstream(`/api/admin/proxies?page=${page}&pageSize=100`, cookie)
      matches.push(...data.items.filter(item => item.endpoint === account.outboundProxyEndpoint))
      if (page >= data.page.totalPages) break
    }
    // 官方详情仅返回脱敏端点，重复端点不能猜测其认证信息。
    if (matches.length !== 1) throw new PublicError(400, '原账号代理无法唯一匹配，请检查代理配置或使用授权链接')
    return resolveProxy(matches[0].id, cookie, upstream, proxyMap, false)
  }
  const jobs = new Jobs({ leaseMs, run: async input => {
    if (input.targetAccountId) {
      const account = await accountDetail(input.targetAccountId, input.cookie)
      checkEmail(account, input.credentials)
      input.proxy = await currentProxy(account, input.cookie)
    }
    return run(input)
  }, persist: vault ? async ({ credentials, accountId, cookie }) => {
    const account = await accountDetail(accountId, cookie)
    checkEmail(account, credentials)
    await vault.put(accountId, credentials)
  } : undefined })
  const requireVault = () => { if (!vault) throw new PublicError(503, '2FA 信息保存服务未配置') }
  app.setErrorHandler((error, _request, reply) => {
    const status = error instanceof PublicError ? error.statusCode : error.statusCode === 413 ? 413 : error.statusCode === 400 ? 400 : 500
    reply.code(status).send({ code: status, message: error instanceof PublicError ? error.message : status === 400 ? '请求格式错误' : '登录服务请求失败', data: null })
  })
  app.addHook('onRequest', async (request, reply) => {
    reply.header('Cache-Control', 'no-store').header('X-Content-Type-Options', 'nosniff')
    if (request.url === '/health') return
    if (request.headers['x-cpr-twofa'] !== '1' || (request.headers.origin && request.headers.origin !== origin)
      || (!['GET', 'HEAD'].includes(request.method) && request.headers.origin !== origin))
      throw new PublicError(403, '来源验证失败')
    const session = request.headers.cookie?.split(';').map(s => s.trim()).find(s => /^cpr_session=[^;\s]+$/.test(s))
    if (!session) throw new PublicError(401, '请先登录管理员账号')
    const auth = await upstream('/api/auth/status', session)
    if (!auth.authenticated) throw new PublicError(401, '管理员会话已过期')
    if (auth.session?.role !== 'admin') throw new PublicError(403, '此操作需要管理员权限')
    request.sessionCookie = session
    request.owner = createHash('sha256').update(session).digest('hex')
  })
  const ok = data => ({ code: 200, message: 'ok', data })
  app.get('/health', () => ok({ ready: true }))
  const accountParams = { type: 'object', required: ['accountId'], properties: { accountId: { type: 'string', minLength: 1, maxLength: 128 } } }
  app.get(`${prefix}/accounts/:accountId`, { schema: { params: accountParams } }, async request => {
    requireVault()
    const account = await accountDetail(request.params.accountId, request.sessionCookie)
    const saved = await vault.get(account.id)
    if (saved) checkEmail(account, saved.credentials)
    return ok({ saved: !!saved, updatedAt: saved?.updatedAt })
  })
  app.delete(`${prefix}/accounts/:accountId`, { schema: { params: accountParams } }, async request => withAccountLock(request.params.accountId, async () => {
    requireVault()
    jobs.forgetTarget(request.params.accountId)
    await vault.delete(request.params.accountId)
    return ok({ deleted: true })
  }))
  app.post(`${prefix}/accounts/:accountId/reauthorize`, { schema: { params: accountParams, body: {
    type: 'object', additionalProperties: false, required: ['submissionId'],
    properties: { submissionId: { type: 'string', minLength: 1, maxLength: 128 }, text: { type: 'string', maxLength: 4096 } },
  } } }, async request => withAccountLock(request.params.accountId, async () => {
    requireVault()
    const account = await accountDetail(request.params.accountId, request.sessionCookie)
    let credentials
    try {
      if (request.body.text !== undefined) {
        const parsed = parseAccounts(request.body.text)
        request.body.text = ''
        if (parsed.length !== 1) throw new PublicError(400, '重新授权只能提交一个账号')
        credentials = parsed[0]
      } else credentials = (await vault.get(account.id))?.credentials
      if (!credentials) throw new PublicError(404, '此账号尚未保存 2FA 信息，请先补录')
      checkEmail(account, credentials)
      return ok(jobs.create({ targetAccountId: account.id, credentials: [credentials], submissionId: request.body.submissionId, owner: request.owner, cookie: request.sessionCookie }))
    } catch (error) {
      if (credentials) { credentials.password = ''; credentials.totpSecret = '' }
      throw error
    }
  }))
  app.post(`${prefix}/tasks`, { schema: { body: {
    type: 'object', additionalProperties: false, required: ['text', 'settings', 'submissionId'],
    properties: { text: { type: 'string', maxLength: 131072 }, settings: settingsSchema,
      submissionId: { type: 'string', minLength: 1, maxLength: 128 }, outboundProxyId: { type: 'string', maxLength: 128 } },
  } } }, async request => {
    const credentials = parseAccounts(request.body.text)
    request.body.text = ''
    try {
      const proxy = await resolveProxy(request.body.outboundProxyId, request.sessionCookie, upstream, proxyMap)
      return ok(jobs.create({ ...request.body, credentials, proxy, owner: request.owner, cookie: request.sessionCookie }))
    } catch (error) {
      credentials.forEach(c => { c.password = ''; c.totpSecret = '' })
      throw error
    }
  })
  app.get(`${prefix}/tasks/:id`, request => ok(jobs.read(request.params.id, request.owner)))
  app.post(`${prefix}/tasks/:id/cancel`, request => ok(jobs.cancel(request.params.id, request.owner)))
  app.post(`${prefix}/tasks/:id/retry`, request => ok(jobs.retry(request.params.id, request.owner, request.sessionCookie)))
  app.delete(`${prefix}/tasks/:id`, request => { jobs.remove(request.params.id, request.owner); return ok({ deleted: true }) })
  app.get(`${prefix}/tasks/:id/items/:itemId/screen`, async request => {
    const image = await jobs.controls(request.params.id, request.owner, request.params.itemId).screen()
    return ok({ image: `data:image/jpeg;base64,${image.toString('base64')}` })
  })
  app.post(`${prefix}/tasks/:id/items/:itemId/input`, async request => {
    await jobs.controls(request.params.id, request.owner, request.params.itemId).input(request.body)
    return ok({ accepted: true })
  })
  app.addHook('onClose', () => jobs.close())
  return app
}
