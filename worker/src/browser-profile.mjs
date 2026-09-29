const viewport = { width: 1024, height: 768 }
const defaultChromiumVersion = '140.0.7339.16'

export function browserLaunchOptions(env = process.env) {
  return {
    headless: env.BROWSER_HEADLESS !== 'false',
    args: ['--disable-blink-features=AutomationControlled'],
  }
}

export function browserContextOptions({ browserVersion = defaultChromiumVersion, proxy } = {}) {
  const options = {
    viewport,
    locale: 'en-US',
    timezoneId: 'UTC',
    colorScheme: 'light',
    userAgent: `Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/${browserVersion} Safari/537.36`,
    acceptDownloads: false,
    serviceWorkers: 'block',
  }
  if (proxy) options.proxy = proxy
  return options
}

export const browserFingerprintInitScript = () => {
  Object.defineProperty(navigator, 'webdriver', { configurable: true, get: () => undefined })
}
