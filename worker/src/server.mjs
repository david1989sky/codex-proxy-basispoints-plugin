import { createApp, makeUpstream } from './app.mjs'

const origin = process.env.PUBLIC_ORIGIN
if (!origin || new URL(origin).origin !== origin) throw new Error('PUBLIC_ORIGIN is required')
const upstream = makeUpstream(process.env.CPR_BASE_URL || 'http://127.0.0.1:28080', origin)
const app = await createApp({ origin, upstream })
await app.listen({ host: '127.0.0.1', port: Number(process.env.PORT || 28082) })
console.info('CPR Basis Points worker listening on loopback')
for (const signal of ['SIGTERM', 'SIGINT']) process.once(signal, async () => { await app.close(); process.exit(0) })
