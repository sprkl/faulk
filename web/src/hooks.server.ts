import type { Handle } from '@sveltejs/kit/hooks'
import { themeScript } from './lib/theme'

/**
 * Fills the `<html>` attributes of `app.html`: dark theme for the DAW, light for the landing page. The DAW's head
 * also gets the script that applies the theme the user chose (#687) before the first paint.
 */
export const handle: Handle = ({ event, resolve }) => {
  const app = event.route.id?.startsWith('/app') ?? false
  const theme = app ? 'dark' : 'light'
  const lang = event.params.lang === 'fr' ? 'fr' : 'en'
  const head = app ? `<script>${themeScript()}</script>` : ''
  return resolve(event, {
    transformPageChunk: ({ html }) =>
      html
        .replace('%faulk.theme%', theme)
        .replace('%faulk.lang%', lang)
        // A function: the script is inserted as it is, never read as a replacement pattern (`$&`).
        .replace('%faulk.head%', () => head),
  })
}
