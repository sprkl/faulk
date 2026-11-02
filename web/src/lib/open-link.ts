/**
 * Opening links (#497): `/app?open=<link to a .faulkpack>` opens a hosted project. The link comes from
 * anywhere, so the page asks before it downloads anything (`OpenLinkDialog.svelte`), then runs
 * `project.open_url`. The parameter leaves the address as soon as the page has read it, so a reload
 * never opens the project again.
 */

const PARAM = 'open'

/**
 * Takes the link out of the current address: returns it (`null` when there is none, or it is empty) and,
 * when the address had the parameter, has `clear` replace the address with the same path, the other
 * parameters and the fragment, without any `open`. Under SvelteKit `clear` goes through its router
 * (`goto` with `shallow` and `replace`), as for the pairing links.
 */
export function takeOpenLink(location: Pick<Location, 'hash' | 'pathname' | 'search'>, clear: (url: string) => void): string | null {
  const params = new URLSearchParams(location.search)
  if (!params.has(PARAM)) return null
  const link = params.get(PARAM)?.trim() ?? ''
  params.delete(PARAM)
  const rest = params.toString()
  clear(location.pathname + (rest ? `?${rest}` : '') + location.hash)
  return link === '' ? null : link
}

/** The address that opens `link` in the DAW at `appPath` (`/app` under the site's base path). */
export function openLinkHref(appPath: string, link: string): string {
  return `${appPath}?${PARAM}=${encodeURIComponent(link)}`
}
