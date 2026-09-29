export interface PluginHost {
  readonly version: 2
  request: (input: { method: string, path: string, contentType?: string, body?: string }) => Promise<{ status: number, contentType: string, body: ArrayBuffer }>
}

export function getHost(): PluginHost {
  const host = window.codexProxyPlugin
  if (!host || host.version !== 2)
    throw new Error('请从 RS 插件管理页面打开此页面')
  return host
}
