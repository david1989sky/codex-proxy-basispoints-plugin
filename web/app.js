(() => {
  'use strict'

  const refreshButton = document.getElementById('refresh')
  const refreshLabel = document.querySelector('[data-refresh-label]')
  const statusNode = document.getElementById('status')
  const updatedAtNode = document.getElementById('updated-at')
  const values = {
    totalRequests: document.getElementById('total-requests'),
    successfulRequests: document.getElementById('successful-requests'),
    failedRequests: document.getElementById('failed-requests'),
    lastRequest: document.getElementById('last-request'),
  }
  const dateFormatter = new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'medium',
  })
  let refreshInFlight = false
  let refreshTimer

  function bridge() {
    const host = window.codexProxyPlugin
    if (!host || typeof host.request !== 'function')
      throw new Error('请从 RS 插件管理页面打开此页面')
    return host
  }

  function decodeBody(body) {
    if (!(body instanceof ArrayBuffer))
      throw new Error('插件返回了无效响应')
    return new TextDecoder().decode(body)
  }

  function errorMessage(value, status) {
    if (value && typeof value === 'object' && value.error
      && typeof value.error === 'object' && typeof value.error.message === 'string')
      return value.error.message
    return `插件请求失败（HTTP ${status}）`
  }

  async function readUsage() {
    const reply = await bridge().request({ method: 'GET', path: 'api/usage' })
    const text = decodeBody(reply.body)
    let value
    try {
      value = text ? JSON.parse(text) : null
    }
    catch {
      throw new Error('插件返回了无效响应')
    }
    if (reply.status < 200 || reply.status >= 300)
      throw new Error(errorMessage(value, reply.status))
    if (!value || typeof value !== 'object')
      throw new Error('插件返回了无效统计数据')

    const fields = ['totalRequests', 'successfulRequests', 'failedRequests']
    if (!fields.every(field => Number.isSafeInteger(value[field]) && value[field] >= 0))
      throw new Error('插件返回了无效统计数据')
    if (value.lastRequestAtMs !== null
      && (!Number.isSafeInteger(value.lastRequestAtMs) || value.lastRequestAtMs < 0))
      throw new Error('插件返回了无效统计数据')
    return value
  }

  function formatRequestTime(timestamp) {
    if (timestamp === null)
      return '暂无请求'
    const date = new Date(timestamp)
    return Number.isNaN(date.getTime()) ? '暂无请求' : dateFormatter.format(date)
  }

  function render(value) {
    values.totalRequests.textContent = value.totalRequests.toLocaleString()
    values.successfulRequests.textContent = value.successfulRequests.toLocaleString()
    values.failedRequests.textContent = value.failedRequests.toLocaleString()
    values.lastRequest.textContent = formatRequestTime(value.lastRequestAtMs)
    const now = new Date()
    updatedAtNode.textContent = dateFormatter.format(now)
    updatedAtNode.dateTime = now.toISOString()
  }

  function setStatus(message, tone) {
    statusNode.textContent = message
    statusNode.dataset.tone = tone
  }

  async function refresh() {
    if (refreshInFlight)
      return
    refreshInFlight = true
    refreshButton.disabled = true
    refreshButton.setAttribute('aria-busy', 'true')
    refreshLabel.textContent = '刷新中…'
    try {
      render(await readUsage())
      setStatus('数据已更新', 'success')
    }
    catch (cause) {
      setStatus(cause instanceof Error ? cause.message : '读取统计数据失败', 'error')
    }
    finally {
      refreshInFlight = false
      refreshButton.disabled = false
      refreshButton.removeAttribute('aria-busy')
      refreshLabel.textContent = '刷新'
    }
  }

  function applyTheme() {
    const theme = window.codexProxyPlugin && window.codexProxyPlugin.theme
    if (theme === 'light' || theme === 'dark')
      document.documentElement.dataset.theme = theme
  }

  function stop() {
    window.clearInterval(refreshTimer)
  }

  refreshButton.addEventListener('click', () => void refresh())
  window.addEventListener('pagehide', stop)
  window.addEventListener('codex-proxy-themechange', applyTheme)
  applyTheme()
  void refresh()
  refreshTimer = window.setInterval(() => void refresh(), 5000)
})()
