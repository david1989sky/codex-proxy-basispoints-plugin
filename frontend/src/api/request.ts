import { getHost } from './host'

export async function request<T>(body: Record<string, unknown>): Promise<T> {
  const reply = await getHost().request({
    method: 'POST',
    path: 'api/request',
    contentType: 'application/json',
    body: JSON.stringify(body),
  })
  const text = new TextDecoder().decode(reply.body)
  let value: unknown
  try {
    value = text ? JSON.parse(text) : null
  }
  catch {
    throw new Error('插件返回了无效响应')
  }
  if (reply.status < 200 || reply.status >= 300) {
    const message = typeof value === 'object' && value !== null
      && 'error' in value && typeof value.error === 'object' && value.error !== null
      && 'message' in value.error && typeof value.error.message === 'string'
      ? value.error.message
      : `插件请求失败（HTTP ${reply.status}）`
    throw new Error(message)
  }
  return value as T
}

export async function getJson<T>(path: 'api/status' | 'api/migration'): Promise<T> {
  const reply = await getHost().request({ method: 'GET', path })
  const text = new TextDecoder().decode(reply.body)
  if (reply.status < 200 || reply.status >= 300)
    throw new Error(`插件请求失败（HTTP ${reply.status}）`)
  return JSON.parse(text) as T
}

export async function postJson<T>(path: 'api/migration/import', body: Record<string, unknown>): Promise<T> {
  const reply = await getHost().request({
    method: 'POST',
    path,
    contentType: 'application/json',
    body: JSON.stringify(body),
  })
  const text = new TextDecoder().decode(reply.body)
  if (reply.status < 200 || reply.status >= 300)
    throw new Error(`插件请求失败（HTTP ${reply.status}）`)
  return JSON.parse(text) as T
}
