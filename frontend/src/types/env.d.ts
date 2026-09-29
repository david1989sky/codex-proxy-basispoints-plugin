import type { PluginHost } from '../api/host'

declare global {
  interface Window {
    readonly codexProxyPlugin?: PluginHost
  }
}

export {}
