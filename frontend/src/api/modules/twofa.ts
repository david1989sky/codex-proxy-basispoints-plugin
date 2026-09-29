import { getJson, postJson, request } from '../request'

export interface TwoFaTask {
  id: string
  running: boolean
  cancelled: boolean
  items: Array<{ id: string, email: string, status: string, attempts: number, message?: string, accountId?: string, credentialsSaved?: boolean }>
}

export interface TwoFaStatus {
  ready: boolean
  workerImageDigest: string
  legacyVaultMounted: boolean
}

export interface TwoFaSettings {
  enabled: boolean
  concurrencyLimit: number | null
  weight: number
  groupIds: string[]
  notes?: string
  modelAccess?: { mode: 'all' | 'allowlist' | 'denylist', models: string[] }
}

export function getTwoFaStatus() {
  return getJson<TwoFaStatus>('api/status')
}

export function getMigration() {
  return getJson<{ legacyVaultMounted: boolean, imported: boolean, marker?: { source: string, recordCount: number, updatedAt: string } }>('api/migration')
}

export function importMigration() {
  return postJson<{ imported: boolean }>('api/migration/import', {})
}

export function startTwoFaTask(data: { text: string, submissionId: string, settings: TwoFaSettings, outboundProxyId?: string }) {
  return request<TwoFaTask>({ operation: 'startTask', ...data })
}

export function getTwoFaCredentials(accountId: string) {
  return request<{ saved: boolean, updatedAt?: string }>({ operation: 'getCredentials', accountId })
}

export function deleteTwoFaCredentials(accountId: string) {
  return request<{ deleted: boolean }>({ operation: 'deleteCredentials', accountId })
}

export function reauthorizeTwoFaAccount(accountId: string, data: { submissionId: string, text?: string }) {
  return request<TwoFaTask>({ operation: 'reauthorize', accountId, ...data })
}

export function getTwoFaTask(taskId: string) {
  return request<TwoFaTask>({ operation: 'getTask', taskId })
}

export function controlTwoFaTask(taskId: string, action: 'cancel' | 'retry' | 'delete') {
  const operation = action === 'cancel' ? 'cancelTask' : action === 'retry' ? 'retryTask' : 'deleteTask'
  return request<TwoFaTask>({ operation, taskId })
}

export function getTwoFaScreen(taskId: string, itemId: string) {
  return request<{ image: string }>({ operation: 'getScreen', taskId, itemId })
}

export function sendTwoFaInput(taskId: string, itemId: string, data: Record<string, unknown>) {
  return request<{ accepted: boolean }>({ operation: 'sendInput', taskId, itemId, data })
}
