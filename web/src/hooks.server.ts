import type { Handle } from '@sveltejs/kit/hooks'

/** Fills the `<html>` attributes of `app.html`: dark theme for the DAW, light for the landing page. */
export const handle: Handle = ({ event, resolve }) => {
  const theme = event.route.id?.startsWith('/app') ? 'dark' : 'light'
  const lang = event.params.lang === 'fr' ? 'fr' : 'en'
  return resolve(event, {
    transformPageChunk: ({ html }) => html.replace('%faulk.theme%', theme).replace('%faulk.lang%', lang),
  })
}
