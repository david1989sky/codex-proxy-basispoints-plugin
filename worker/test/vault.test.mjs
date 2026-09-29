import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, readFile, readdir, writeFile, stat, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { randomBytes } from 'node:crypto'

const credentials = { email: 'fixture@example.com', password: 'fixture-private-password', totpSecret: 'JBSWY3DPEHPK3PXP' }
export async function fixtureVault(t) {
  const root = await mkdtemp(join(tmpdir(), 'cpr-vault-test-'))
  t.after(() => rm(root, { recursive: true, force: true }))
  const options = { directory: join(root, 'data'), keyFile: join(root, 'key') }
  await writeFile(options.keyFile, randomBytes(32), { mode: 0o600 })
  const module = await import('../src/vault.mjs').catch(() => ({}))
  assert.equal(typeof module.openVault, 'function', 'encrypted account vault must be implemented')
  return { root, options, openVault: module.openVault, vault: await module.openVault(options) }
}
test('vault encrypts credentials, persists across restart, and deletes only the selected account', async t => {
  const { options, openVault, vault } = await fixtureVault(t)
  await vault.put('account-one', credentials)
  await vault.put('account-two', credentials)
  assert.deepEqual((await (await openVault(options)).get('account-one')).credentials, credentials)
  for (const file of await readdir(options.directory)) {
    const contents = await readFile(join(options.directory, file), 'utf8')
    for (const value of Object.values(credentials)) assert.ok(!contents.includes(value))
    assert.equal((await stat(join(options.directory, file))).mode & 0o777, 0o600)
  }
  assert.equal((await stat(options.directory)).mode & 0o777, 0o700)
  await vault.delete('account-one')
  assert.equal(await vault.get('account-one'), null)
  assert.ok(await vault.get('account-two'))
})
test('vault rejects wrong or missing keys and tampered or swapped account records', async t => {
  const { options, openVault, vault } = await fixtureVault(t)
  await vault.put('account-one', credentials)
  await vault.put('account-two', credentials)
  const files = (await readdir(options.directory)).filter(file => file.endsWith('.json'))
  assert.equal(files.length, 2)
  await writeFile(join(options.directory, files[1]), await readFile(join(options.directory, files[0])))
  await assert.rejects(async () => { await vault.get('account-one'); await vault.get('account-two') })
  await writeFile(options.keyFile, randomBytes(32))
  await assert.rejects(openVault(options))
  await rm(options.keyFile)
  await assert.rejects(openVault(options))
})
