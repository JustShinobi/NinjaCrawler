const PROVIDER_COOKIE_DOMAINS = {
  instagram: ['instagram.com'],
  twitter: ['x.com', 'twitter.com'],
  tiktok: ['tiktok.com'],
}

// Tempo máximo que a captura espera pela sonda de página. Sem esse corte, uma
// aba não-responsiva (timeline pesada, renderer ocupado, aba navegando) deixa o
// `chrome.scripting.executeScript` pendente para sempre, mantém o service worker
// vivo e trava o popup em "Capturing…".
const PAGE_PROBE_TIMEOUT_MS = 10_000

const INSTAGRAM_PROBE_FILTER = {
  urls: [
    'https://instagram.com/api/v1/accounts/current_user/*',
    'https://*.instagram.com/api/v1/accounts/current_user/*',
  ],
}

export function cookieToPayload(cookie) {
  return {
    domain: cookie.domain,
    name: cookie.name,
    value: cookie.value,
    path: cookie.path || '/',
    expiresAt: cookie.expirationDate
      ? new Date(cookie.expirationDate * 1000).toISOString()
      : null,
    secure: Boolean(cookie.secure),
    httpOnly: Boolean(cookie.httpOnly),
  }
}

export function providerUserIdFromCookies(provider, cookies) {
  const value = (name) => cookies.find((cookie) => cookie.name === name)?.value?.trim()
  if (provider === 'instagram') return value('ds_user_id')
  if (provider === 'twitter') {
    const twid = value('twid')
    if (!twid) return undefined
    try {
      return decodeURIComponent(twid).replace(/^u=/, '')
    } catch {
      return twid.replace(/^u=/, '')
    }
  }
  if (provider === 'tiktok') return value('uid_tt') || value('uid_tt_ss')
  return undefined
}

export async function captureAccountFromTab(tab, provider) {
  const domains = PROVIDER_COOKIE_DOMAINS[provider]
  if (!tab?.id || !tab.url || !domains) {
    throw new Error('Account import is unavailable for this tab.')
  }

  const storeId = await findCookieStoreId(tab.id)
  const cookieSets = await Promise.all(domains.map((domain) => chrome.cookies.getAll({
    domain,
    ...(storeId ? { storeId } : {}),
  })))
  const rawCookies = Array.from(new Map(
    cookieSets
      .flat()
      .map((cookie) => [`${cookie.domain}\t${cookie.path}\t${cookie.name}`, cookie]),
  ).values())
  const cookies = rawCookies.map(cookieToPayload)
  const csrfToken = rawCookies.find((cookie) => cookie.name === 'csrftoken')?.value
  const cookieProviderUserId = providerUserIdFromCookies(provider, rawCookies)
  const observedHeaders = {}

  const probe = provider === 'instagram'
    ? await runInstagramProbe(tab.id, csrfToken, observedHeaders)
    : await withTimeout(
      runPageProbe(tab.id, provider),
      PAGE_PROBE_TIMEOUT_MS,
      `${provider} account capture timed out. Reload the tab and try again.`,
    )
  const providerUserId = probe.identity?.providerUserId || cookieProviderUserId
  const username = resolveCapturedUsername(probe.identity?.username, providerUserId)
  if (!username) {
    const detail = probe.warning ? ` ${probe.warning}` : ''
    throw new Error(`Could not detect the signed-in account. Confirm that you are logged in and try again.${detail}`)
  }

  return {
    provider,
    currentUrl: tab.url,
    identity: {
      providerUserId: providerUserId || null,
      username,
    },
    cookies,
    authorization: compactValues({
      csrfToken,
      appId: observedHeaders.appId,
      asbdId: observedHeaders.asbdId,
      igWwwClaim: observedHeaders.igWwwClaim,
      userAgent: probe.browser?.userAgent,
      secChUa: probe.browser?.secChUa,
      secChUaFullVersionList: probe.browser?.secChUaFullVersionList,
      secChUaPlatformVersion: probe.browser?.secChUaPlatformVersion,
      lsd: probe.authorization?.lsd,
      dtsg: probe.authorization?.dtsg,
    }),
  }
}

async function findCookieStoreId(tabId) {
  const stores = await chrome.cookies.getAllCookieStores()
  return stores.find((store) => store.tabIds.includes(tabId))?.id
}

async function runInstagramProbe(tabId, csrfToken, observedHeaders) {
  const requestListener = (details) => {
    if (details.tabId !== tabId) return
    const headers = headerMap(details.requestHeaders)
    observedHeaders.appId = headers['x-ig-app-id'] || observedHeaders.appId
    observedHeaders.asbdId = headers['x-asbd-id'] || observedHeaders.asbdId
    observedHeaders.igWwwClaim = headers['x-ig-www-claim'] || observedHeaders.igWwwClaim
  }
  const responseListener = (details) => {
    if (details.tabId !== tabId) return
    const headers = headerMap(details.responseHeaders)
    observedHeaders.igWwwClaim = headers['x-ig-set-www-claim']
      || headers['x-ig-www-claim']
      || observedHeaders.igWwwClaim
  }

  chrome.webRequest.onBeforeSendHeaders.addListener(
    requestListener,
    INSTAGRAM_PROBE_FILTER,
    ['requestHeaders', 'extraHeaders'],
  )
  chrome.webRequest.onHeadersReceived.addListener(
    responseListener,
    INSTAGRAM_PROBE_FILTER,
    ['responseHeaders', 'extraHeaders'],
  )

  try {
    return await withTimeout(
      runPageProbe(tabId, 'instagram', { csrfToken }),
      PAGE_PROBE_TIMEOUT_MS,
      'Instagram account capture timed out.',
    )
  } finally {
    chrome.webRequest.onBeforeSendHeaders.removeListener(requestListener)
    chrome.webRequest.onHeadersReceived.removeListener(responseListener)
  }
}

async function runPageProbe(tabId, provider, options = {}) {
  const results = await chrome.scripting.executeScript({
    target: { tabId },
    world: 'ISOLATED',
    func: pageProbe,
    args: [provider, options],
  })
  return unwrapPageProbeResult(results)
}

async function pageProbe(provider, options) {
  const browser = { userAgent: navigator.userAgent }
  // Os client-hints (Sec-CH-UA*) só são consumidos pelo connector do Instagram.
  // Twitter/TikTok baixam com impersonation própria (que já emite um conjunto
  // coerente de UA+CH+TLS), então coletar esse fingerprint do navegador para eles
  // só exporia dado sensível sem uso. O userAgent, esse sim, os três usam.
  if (provider === 'instagram' && navigator.userAgentData) {
    const brands = navigator.userAgentData.brands ?? []
    browser.secChUa = brands.map((brand) => `"${brand.brand}";v="${brand.version}"`).join(', ')
    try {
      const highEntropy = await navigator.userAgentData.getHighEntropyValues([
        'fullVersionList',
        'platformVersion',
      ])
      browser.secChUaFullVersionList = (highEntropy.fullVersionList ?? [])
        .map((brand) => `"${brand.brand}";v="${brand.version}"`)
        .join(', ')
      browser.secChUaPlatformVersion = highEntropy.platformVersion
        ? `"${highEntropy.platformVersion}"`
        : undefined
    } catch {
      // High entropy client hints are optional.
    }
  }

  if (provider === 'instagram') {
    const html = document.documentElement?.innerHTML ?? ''
    const documentToken = (patterns) => {
      for (const pattern of patterns) {
        const value = html.match(pattern)?.[1]
        if (value) return value
      }
      return undefined
    }
    const profileImage = Array.from(document.querySelectorAll('a[href^="/"] img[alt]'))
      .find((image) => /profile picture/i.test(image.getAttribute('alt') ?? ''))
    const domUsername = profileImage
      ?.closest('a[href^="/"]')
      ?.getAttribute('href')
      ?.match(/^\/([^/?#]+)\/?/)?.[1]
    const partial = {
      identity: { username: domUsername },
      browser,
      authorization: {
        lsd: documentToken([/"LSD",\[\],{"token":"([^"]+)"/, /"lsd":"([^"]+)"/]),
        dtsg: documentToken([/"DTSGInitialData",\[\],{"token":"([^"]+)"/, /"fb_dtsg":"([^"]+)"/]),
      },
    }

    try {
      const headers = {
        'x-ig-app-id': '936619743392459',
        'x-asbd-id': '129477',
        ...(options.csrfToken ? { 'x-csrftoken': options.csrfToken } : {}),
      }
      const response = await fetch('/api/v1/accounts/current_user/?edit=true', {
        credentials: 'include',
        headers,
        cache: 'no-store',
      })
      if (!response.ok) throw new Error(`Instagram identity request failed (${response.status}).`)
      const payload = await response.json()
      const user = payload.user ?? payload
      return {
        ok: true,
        value: {
          ...partial,
          identity: {
            providerUserId: user.pk_id ?? user.pk,
            username: user.username ?? domUsername,
          },
        },
      }
    } catch (error) {
      return {
        ok: false,
        error: error?.message || String(error),
        partial,
      }
    }
  }

  if (provider === 'twitter') {
    // Deriva o handle apenas de elementos que pertencem à conta logada. Uma
    // varredura global de `a[href^="/"]` percorreria toda a timeline (caro em
    // páginas longas) e capturaria o perfil de terceiros — o que gravaria a
    // conta com o username errado.
    const handleFromPath = (value) => value?.match(/^\/([A-Za-z0-9_]{1,15})$/)?.[1]
    const profileLink = document.querySelector('a[data-testid="AppTabBar_Profile_Link"]')
    const switcher = document.querySelector('[data-testid="SideNav_AccountSwitcher_Button"]')
    const username = handleFromPath(profileLink?.getAttribute('href'))
      ?? switcher?.textContent?.match(/@([A-Za-z0-9_]{1,15})/)?.[1]
      ?? handleFromPath(switcher?.querySelector('a[href^="/"]')?.getAttribute('href'))
    return { ok: true, value: { identity: { username }, browser } }
  }

  if (provider === 'tiktok') {
    const accountAnchor = document.querySelector(
      'a[data-e2e="profile-icon"], a[href^="/@"][data-e2e*="profile"], header a[href^="/@"]',
    )
    const username = accountAnchor?.getAttribute('href')?.match(/^\/@([^/?#]+)/)?.[1]
    return { ok: true, value: { identity: { username }, browser } }
  }

  return { ok: true, value: { identity: {}, browser } }
}

export function resolveCapturedUsername(username, providerUserId) {
  return String(username || providerUserId || '').trim().replace(/^@/, '')
}

export function unwrapPageProbeResult(results) {
  const result = results?.[0]?.result
  if (!result) {
    throw new Error('The provider page did not return account information. Reload the tab and try again.')
  }
  if (result.ok === false) {
    if (result.partial) {
      return {
        ...result.partial,
        warning: result.error || 'The provider identity endpoint failed.',
      }
    }
    throw new Error(result.error || 'The provider page probe failed.')
  }
  return result.value ?? result
}

function headerMap(headers = []) {
  return Object.fromEntries(
    headers
      .filter((header) => header.name && header.value)
      .map((header) => [header.name.toLowerCase(), header.value]),
  )
}

function compactValues(values) {
  return Object.fromEntries(
    Object.entries(values).filter(([, value]) => typeof value === 'string' && value.trim()),
  )
}

function withTimeout(promise, timeoutMs, message) {
  let timer
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(message)), timeoutMs)
  })
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer))
}
