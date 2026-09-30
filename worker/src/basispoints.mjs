import { PublicError } from './core.mjs'

export const BASISPOINTS_URL = 'https://bps.openai.com/basispoints/api/responses'
export const MAXIMUM_RESPONSE_BYTES = 2 * 1024 * 1024

const HOST_ACCOUNT_ID = /^[A-Za-z0-9_.-]{1,128}$/
const CHATGPT_ACCOUNT_ID = /^[A-Za-z0-9_.-]{1,128}$/
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

export function resolveAccountId(account, token) {
  const claims = decodeJwtPayload(token)
  const authClaims = claims['https://api.openai.com/auth']
  const claimAccountId = authClaims && typeof authClaims === 'object' ? authClaims.chatgpt_account_id : undefined
  const documentAccountId = account.accountId
  if (documentAccountId !== undefined && (typeof documentAccountId !== 'string' || !CHATGPT_ACCOUNT_ID.test(documentAccountId)))
    throw new PublicError(400, '宿主账号 ID 无效')
  if (claimAccountId !== undefined && (typeof claimAccountId !== 'string' || !CHATGPT_ACCOUNT_ID.test(claimAccountId)))
    throw new PublicError(400, 'access token 账号 ID 无效')
  if (documentAccountId && claimAccountId && documentAccountId !== claimAccountId)
    throw new PublicError(400, '宿主账号与 access token 不匹配')
  const result = documentAccountId || claimAccountId
  if (!result) throw new PublicError(400, 'access token 缺少 ChatGPT 账号 ID')
  return result
}

async function readBoundedBody(response) {
  const length = Number(response.headers.get('content-length'))
  if (Number.isFinite(length) && length > MAXIMUM_RESPONSE_BYTES) throw new PublicError(502, 'Basis Points 响应超过大小限制')
  const reader = response.body?.getReader()
  if (!reader) throw new PublicError(502, 'Basis Points 响应正文无效')
  const chunks = []
  let size = 0
  try {
    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      size += value.byteLength
      if (size > MAXIMUM_RESPONSE_BYTES) {
        await reader.cancel()
        throw new PublicError(502, 'Basis Points 响应超过大小限制')
      }
      chunks.push(Buffer.from(value))
    }
    return Buffer.concat(chunks, size)
  } finally {
    reader.releaseLock()
  }
}

export async function requestBasispoints({ accountId, request, cookie, upstream, fetchImpl = fetch }) {
  if (typeof accountId !== 'string' || !HOST_ACCOUNT_ID.test(accountId)) throw new PublicError(400, '宿主账号 ID 无效')
  if (!request || typeof request !== 'object' || Array.isArray(request)) throw new PublicError(400, 'Responses 请求格式无效')
  const exported = await upstream(`/api/admin/accounts/export?accountIds=${encodeURIComponent(accountId)}&confirm=export_sensitive_accounts`, cookie)
  const documents = Array.isArray(exported?.documents) ? exported.documents : []
  const matches = documents.flatMap(item => item?.provider === 'openai' && Array.isArray(item.document?.accounts)
    ? item.document.accounts.filter(account => account?.id === accountId)
    : [])
  if (matches.length !== 1) throw new PublicError(400, '宿主账号不是唯一的 OpenAI OAuth 账号')
  const account = matches[0]
  if (account.apiKey || account.api_key || typeof account.accessToken !== 'string') throw new PublicError(400, '宿主账号不是 OpenAI OAuth 账号')
  let accessToken = account.accessToken
  if (typeof accessToken !== 'string' || !accessToken) throw new PublicError(400, '宿主账号没有可用 access token')
  try {
    const chatgptAccountId = resolveAccountId(account, accessToken)
    let response
    try {
      response = await fetchImpl(BASISPOINTS_URL, {
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
    } catch (error) {
      if (error instanceof PublicError) throw error
      throw new PublicError(502, 'Basis Points 请求失败')
    }
    try {
      const contentType = response.headers.get('content-type')?.split(';', 1)[0].trim().toLowerCase() || ''
      if (!ALLOWED_MEDIA_TYPES.has(contentType)) throw new PublicError(502, 'Basis Points 返回了不支持的内容类型')
      const body = await readBoundedBody(response)
      return { status: response.status, contentType: response.headers.get('content-type') || contentType, body }
    } catch (error) {
      if (error instanceof PublicError) throw error
      throw new PublicError(502, 'Basis Points 响应读取失败')
    }
  } finally {
    accessToken = ''
  }
}
