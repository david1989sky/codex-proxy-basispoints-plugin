import test from 'node:test'
import assert from 'node:assert/strict'
import { browserContextOptions, browserLaunchOptions } from '../src/browser-profile.mjs'

test('production browser launch uses headed Chromium with automation controls disabled', () => {
  const options = browserLaunchOptions({ BROWSER_HEADLESS: 'false' })
  assert.equal(options.headless, false)
  assert.ok(options.args.includes('--disable-blink-features=AutomationControlled'))
})

test('account contexts use isolated storage settings and a normal Chrome user agent', () => {
  const options = browserContextOptions({ browserVersion: '140.0.7339.16', proxy: undefined })
  assert.deepEqual(options.viewport, { width: 1024, height: 768 })
  assert.equal(options.locale, 'en-US')
  assert.equal(options.timezoneId, 'UTC')
  assert.match(options.userAgent, /Chrome\/140\.0\.7339\.16/)
  assert.doesNotMatch(options.userAgent, /HeadlessChrome/)
})
