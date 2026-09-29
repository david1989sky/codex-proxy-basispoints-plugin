import { randomUUID } from 'node:crypto'

export class PublicError extends Error {
  constructor(statusCode, message) {
    super(`${statusCode}: ${message}`)
    this.statusCode = statusCode
  }
}

export function parseAccounts(text) {
  if (typeof text !== 'string' || Buffer.byteLength(text) > 131072)
    throw new PublicError(400, '账号内容超出限制')
  const lines = text.replace(/^\uFEFF/, '').split(/\r?\n/).filter(line => line.trim())
  if (!lines.length || lines.length > 50)
    throw new PublicError(400, '每批账号数量须为 1 至 50')
  const seen = new Set()
  return lines.map((line, index) => {
    const first = line.indexOf('----')
    const last = line.lastIndexOf('----')
    const email = line.slice(0, first).trim()
    const password = line.slice(first + 4, last)
    const totpSecret = line.slice(last + 4).replace(/\s/g, '').toUpperCase()
    if (first < 0 || last === first || !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)
      || email.length > 254 || !password || password.length > 1024
      || !/^[A-Z2-7]{16,128}={0,6}$/.test(totpSecret))
      throw new PublicError(400, `第 ${index + 1} 行格式错误：邮箱----密码----2FA密钥`)
    if (seen.has(email.toLowerCase()))
      throw new PublicError(400, `第 ${index + 1} 行账号重复`)
    seen.add(email.toLowerCase())
    return { email, password, totpSecret }
  })
}

export function oauthTarget(value, allowedOrigin = 'https://auth.openai.com') {
  let url = new URL(value)
  if (url.origin === 'https://chatgpt.com' && url.pathname === '/codex/desktop-auth')
    url = new URL(url.searchParams.get('authorize_url'))
  const state = url.searchParams.get('state')
  const callback = url.searchParams.get('redirect_uri')
  if (url.origin !== allowedOrigin || !state || callback !== 'http://localhost:1455/auth/callback')
    throw new PublicError(502, 'OAuth 授权地址校验失败')
  return { url: url.href, state, callback }
}

function erase(credentials) {
  credentials.password = ''
  credentials.totpSecret = ''
}

export class Jobs {
  #jobs = new Map()
  #timer
  constructor({ run, persist, leaseMs = 90000, maxAgeMs = 1800000 }) {
    this.run = run
    this.persist = persist
    this.leaseMs = leaseMs
    this.maxAgeMs = maxAgeMs
    this.#timer = setInterval(() => this.sweep(), Math.min(leaseMs, 10000))
    this.#timer.unref()
  }

  create(input) {
    for (const job of this.#jobs.values()) {
      if (job.owner === input.owner && job.submissionId === input.submissionId) {
        input.credentials.forEach(erase)
        if (job.targetAccountId !== input.targetAccountId) throw new PublicError(409, '提交标识已被其他任务使用')
        return this.read(job.id, input.owner)
      }
    }
    if (this.#jobs.size >= 8 || [...this.#jobs.values()].filter(j => j.running).length >= 2)
      throw new PublicError(429, '登录任务繁忙，请稍后重试')
    if ([...this.#jobs.values()].some(j => j.owner === input.owner && j.running))
      throw new PublicError(409, '已有登录任务正在执行')
    this.assertTargetIdle(input.targetAccountId)
    const job = {
      id: randomUUID(), owner: input.owner, cookie: input.cookie, submissionId: input.submissionId,
      settings: input.settings, proxy: input.proxy, outboundProxyId: input.outboundProxyId,
      targetAccountId: input.targetAccountId,
      createdAt: Date.now(), touchedAt: Date.now(), running: false, cancelled: false,
      items: input.credentials.map(c => ({ id: randomUUID(), email: c.email, status: 'queued', credentials: c, attempts: 0 })),
    }
    this.#jobs.set(job.id, job)
    this.#start(job)
    return this.read(job.id, input.owner)
  }

  #owned(id, owner) {
    const job = this.#jobs.get(id)
    if (!job || job.removed || job.owner !== owner) throw new PublicError(404, '任务不存在或已过期')
    return job
  }

  read(id, owner) {
    const job = this.#owned(id, owner)
    job.touchedAt = Date.now()
    return {
      id: job.id, running: job.running, cancelled: job.cancelled,
      items: job.items.map(({ id, email, status, message, accountId, attempts, credentialsSaved }) => ({ id, email, status, message, accountId, attempts, credentialsSaved })),
    }
  }

  #start(job) {
    job.running = true
    job.abort = new AbortController()
    job.promise = (async () => {
      for (const item of job.items) {
        if (job.cancelled || item.status !== 'queued') continue
        item.attempts++
        item.status = 'starting'
        item.message = undefined
        try {
          const result = await this.run({
            credentials: item.credentials, settings: job.settings, cookie: job.cookie,
            targetAccountId: job.targetAccountId,
            proxy: job.proxy, outboundProxyId: job.outboundProxyId, signal: job.abort.signal,
            update: (status, controls) => { item.status = status; item.controls = controls },
          })
          item.accountId = result.accountId
          if (this.persist) {
            // OAuth 已经提交，保存失败只能提示补录，不能将账号标为可重试导入。
            item.status = 'importing'
            try {
              await this.persist({ credentials: item.credentials, accountId: result.accountId, cookie: job.cookie })
              item.credentialsSaved = true
            } catch {
              item.credentialsSaved = false
              item.message = '授权成功，但 2FA 信息保存失败，请在重新授权中补录'
            }
          }
          item.status = 'succeeded'
          erase(item.credentials)
        } catch (error) {
          item.status = job.cancelled ? 'cancelled' : 'failed'
          item.message = job.cancelled ? '已取消' : error instanceof PublicError ? error.message : '登录失败，请重试或使用单账号授权'
        } finally {
          item.controls = undefined
        }
      }
    })().finally(() => {
      job.running = false
      job.cookie = ''
      if (job.cancelled) job.items.forEach(i => erase(i.credentials))
      if (job.removed) this.#jobs.delete(job.id)
    })
  }

  retry(id, owner, cookie) {
    const job = this.#owned(id, owner)
    if (job.running || job.cancelled) throw new PublicError(409, '当前任务状态不支持重试')
    const failed = job.items.filter(i => i.status === 'failed' && i.credentials.password && i.attempts < 3)
    if (!failed.length) throw new PublicError(409, '没有可重试的账号（每个账号最多 3 次）')
    this.assertTargetIdle(job.targetAccountId)
    if ([...this.#jobs.values()].some(j => j.owner === owner && j.running))
      throw new PublicError(409, '已有登录任务正在执行')
    if ([...this.#jobs.values()].filter(j => j.running).length >= 2)
      throw new PublicError(429, '登录任务繁忙，请稍后重试')
    failed.forEach(i => { i.status = 'queued' })
    job.cookie = cookie
    this.#start(job)
    return this.read(id, owner)
  }

  cancel(id, owner) {
    const job = this.#owned(id, owner)
    job.cancelled = true
    job.abort.abort()
    job.items.forEach(item => {
      if (item.status !== 'importing') erase(item.credentials)
      if (item.status === 'queued') item.status = 'cancelled'
    })
    return this.read(id, owner)
  }

  remove(id, owner) {
    this.cancel(id, owner)
    const job = this.#jobs.get(id)
    job.removed = true
    if (!job.running) this.#jobs.delete(id)
  }

  assertTargetIdle(accountId) {
    if (accountId && [...this.#jobs.values()].some(job => job.targetAccountId === accountId && job.running))
      throw new PublicError(409, '此账号正在重新授权，请等待任务结束')
  }

  forgetTarget(accountId) {
    this.assertTargetIdle(accountId)
    for (const job of this.#jobs.values()) {
      if (job.targetAccountId === accountId && !job.removed) this.cancel(job.id, job.owner)
    }
  }

  controls(id, owner, itemId) {
    const job = this.#owned(id, owner)
    job.touchedAt = Date.now()
    const item = job.items.find(i => i.id === itemId)
    if (item?.status !== 'waiting' || !item.controls) throw new PublicError(409, '账号当前没有等待人工验证')
    return item.controls
  }

  sweep() {
    for (const job of this.#jobs.values()) {
      if (!job.removed && (Date.now() - job.touchedAt > this.leaseMs || Date.now() - job.createdAt > this.maxAgeMs))
        this.remove(job.id, job.owner)
    }
  }

  async close() {
    clearInterval(this.#timer)
    const jobs = [...this.#jobs.values()]
    jobs.filter(job => !job.removed).forEach(job => this.remove(job.id, job.owner))
    await Promise.all(jobs.map(job => job.promise))
  }
}
