import { chromium } from 'playwright'
import { createApp, makeUpstream } from './app.mjs'
import { createBrowserRunner } from './browser.mjs'
import { browserLaunchOptions } from './browser-profile.mjs'
import { openVault } from './vault.mjs'

const origin = process.env.PUBLIC_ORIGIN
if (!origin || new URL(origin).origin !== origin) throw new Error('PUBLIC_ORIGIN is required')
const vault = await openVault({ directory: process.env.CREDENTIALS_DIR || '/data/credentials', keyFile: process.env.CREDENTIALS_KEY_FILE || '/run/secrets/twofa-key' })
const browser = await chromium.launch(browserLaunchOptions())
const upstream = makeUpstream(process.env.CPR_BASE_URL || 'http://127.0.0.1:28080', origin)
const app = await createApp({ origin, upstream, vault, run: createBrowserRunner({ browser, upstream }), proxyMap: JSON.parse(process.env.BROWSER_PROXY_MAP || '{}') })
await app.listen({ host: '127.0.0.1', port: Number(process.env.PORT || 28082) })
console.info('CPR 2FA worker listening on loopback')
for (const signal of ['SIGTERM', 'SIGINT']) process.once(signal, async () => { await app.close(); await browser.close(); process.exit(0) })
