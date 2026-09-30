import { createHash } from 'node:crypto'
import Fastify from 'fastify'
import { PublicError } from './core.mjs'
import { requestBasispoints } from './basispoints.mjs'

export function makeUpstream(base, origin) {
  return async (path, cookie, body) => {
    const response = await fetch(new URL(path, base), {
      method: body ? 'POST' : 'GET', redirect: 'error', signal: AbortSignal.timeout(30000),
      headers: { cookie, origin, 'content-type': 'application/json' },
      body: body ? JSON.stringify(body) : undefined,
    })
    const envelope = await response.json().catch(() => null)
    if (!response.ok || envelope?.code !== 200) {
      throw new PublicError([401, 403, 404].includes(response.status) ? response.status : 502, 'Codex Proxy 接口请求失败，请检查服务和登录状态')
    }
    return envelope.data
  }
}

export async function createApp({ origin, upstream, basispointsFetch = fetch }) {
  const app = Fastify({ logger: false, bodyLimit: 140000, disableRequestLogging: true, ajv: { customOptions: { removeAdditional: false } } })
  app.setErrorHandler((error, _request, reply) => {
    const status = error instanceof PublicError ? error.statusCode : error.statusCode === 413 ? 413 : error.statusCode === 400 ? 400 : 500
    reply.code(status).send({ code: status, message: error instanceof PublicError ? error.message : status === 400 ? '请求格式错误' : 'Basis Points Worker 请求失败', data: null })
  })
  app.addHook('onRequest', async (request, reply) => {
    reply.header('Cache-Control', 'no-store').header('X-Content-Type-Options', 'nosniff')
    if (request.url === '/health') return
    if (request.headers['x-cpr-basispoints'] !== '1' || (request.headers.origin && request.headers.origin !== origin)
      || (!['GET', 'HEAD'].includes(request.method) && request.headers.origin !== origin)) {
      throw new PublicError(403, '来源验证失败')
    }
    const session = request.headers.cookie?.split(';').map(value => value.trim()).find(value => /^cpr_session=[^;\s]+$/.test(value))
    if (!session) throw new PublicError(401, '请先登录管理员账号')
    const auth = await upstream('/api/auth/status', session)
    if (!auth.authenticated) throw new PublicError(401, '管理员会话已过期')
    if (auth.session?.role !== 'admin') throw new PublicError(403, '此操作需要管理员权限')
    request.sessionCookie = session
    request.owner = createHash('sha256').update(session).digest('hex')
  })
  const ok = data => ({ code: 200, message: 'ok', data })
  app.get('/health', () => ok({ ready: true }))
  app.post('/api/basispoints/responses', { schema: { body: {
    type: 'object', additionalProperties: false, required: ['accountId', 'request'],
    properties: { accountId: { type: 'string', minLength: 1, maxLength: 128 }, request: { type: 'object', additionalProperties: true } },
  } } }, async (request, reply) => {
    const result = await requestBasispoints({
      accountId: request.body.accountId,
      request: request.body.request,
      cookie: request.sessionCookie,
      upstream,
      fetchImpl: basispointsFetch,
    })
    return reply.code(result.status).type(result.contentType).send(result.body)
  })
  return app
}
