import { expect, test } from '@playwright/test'
import { LANG_STORAGE_KEY } from '../../src/lib/i18n/lang'
import { watch } from './support'

// The browser's local network permission (`loopback-network`) is set through the DevTools protocol.
// No helper listens in this file: the probe is refused, as when the user never ran it, and the page
// explains what the permission says. The expected texts are the founder's (#175), written out here on purpose.

const PERMISSION = 'loopback-network'

const TEXTS = {
  en: {
    pending: 'Your browser may ask to let Faulk reach apps on this device. Allow it so the page can find the helper.',
    blocked:
      'Your browser blocks access to apps on this device, so the page cannot reach the helper. Allow it in the site settings (icon left of the address), then check again.',
  },
  fr: {
    pending: 'Ton navigateur peut te demander d’autoriser Faulk à joindre les applications de cet appareil. Autorise-le pour que la page trouve le helper.',
    blocked:
      'Ton navigateur bloque l’accès aux applications de cet appareil : la page ne peut pas joindre le helper. Autorise-le dans les paramètres du site (icône à gauche de l’adresse), puis clique sur « Chercher à nouveau ».',
  },
} as const

for (const lang of ['en', 'fr'] as const) {
  for (const [state, shown, hidden] of [
    ['prompt', 'helper.localAccessPending', 'helper.localAccessBlocked'],
    ['denied', 'helper.localAccessBlocked', 'helper.localAccessPending'],
  ] as const) {
    test(`explains the ${state} local network permission in ${lang}`, async ({ browser, baseURL, page }) => {
      const { errors } = watch(page)
      await page.addInitScript(([key, value]) => localStorage.setItem(key, value), [LANG_STORAGE_KEY, lang])
      await page.goto('/app')
      // The setting applies to this page's browser context, found through its own target.
      const cdp = await browser.newBrowserCDPSession()
      const { targetInfos } = await cdp.send('Target.getTargets')
      const browserContextId = targetInfos.find((target) => target.type === 'page' && target.url === page.url())?.browserContextId
      await cdp.send('Browser.setPermission', {
        permission: { name: PERMISSION },
        setting: state === 'denied' ? 'denied' : 'prompt',
        origin: new URL(baseURL!).origin,
        browserContextId,
      })
      expect(browserContextId, 'browser context of the page').toBeTruthy()
      await page.reload()
      // The setting took effect for the page (a permission left at its default would pass `prompt` by itself).
      expect(await page.evaluate(async (name) => (await navigator.permissions.query({ name: name as PermissionName })).state, PERMISSION)).toBe(state)
      await page.getByTestId('engine-state').getByRole('button').click()
      await page.getByTestId('engine-mode').click()
      await expect(page.getByTestId(shown)).toHaveText(TEXTS[lang][state === 'denied' ? 'blocked' : 'pending'])
      await expect(page.getByTestId(hidden)).toHaveCount(0)
      // Blocked replaces "No helper found"; pending comes with it.
      await expect(page.getByTestId('helper.absent')).toHaveCount(state === 'denied' ? 0 : 1)
      expect(errors).toEqual([])
    })
  }
}
