import test from 'node:test'
import assert from 'node:assert/strict'
import { classify } from '../src/browser.mjs'

test('account deactivated error is not exposed as a manual verification page', async () => {
  const hidden = { first() { return this }, async isVisible() { return false } }
  const page = {
    locator(selector) {
      if (selector.includes('captcha')) return { first() { return this }, async isVisible() { return true } }
      if (selector === 'body') return { innerText: async () => 'Authentication Error account_deactivated Your account has been deleted or deactivated.' }
      return hidden
    },
  }
  await assert.rejects(classify(page), error => error.statusCode === 422 && /停用/.test(error.message))
})
