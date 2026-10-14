/**
 * Pairing links: the helper's tray opens the page with `#pair=<code>`. The code sits in
 * the fragment, which the browser never sends to a server, and leaves the address bar
 * as soon as the page has read it.
 */

const PREFIX = '#pair='

export function isPairHash(hash: string): boolean {
  return hash.startsWith(PREFIX)
}

/** The 8-digit code of a pairing link, or null for any other fragment. */
export function pairCodeFromHash(hash: string): string | null {
  if (!isPairHash(hash)) return null
  const code = hash.slice(PREFIX.length)
  return /^\d{8}$/.test(code) ? code : null
}

/**
 * Takes the code out of the current address: returns it (null when the fragment is not
 * a valid pairing link) and has `clear` replace the address with the same path and
 * query without the fragment, for a pairing fragment, valid or not. Under SvelteKit,
 * `clear` goes through its router (`goto` with `shallow` and `replace`): a raw
 * `history.replaceState` conflicts with it.
 */
export function takePairLink(
  location: Pick<Location, 'hash' | 'pathname' | 'search'>,
  clear: (url: string) => void,
): string | null {
  const hash = location.hash
  if (!isPairHash(hash)) return null
  clear(location.pathname + location.search)
  return pairCodeFromHash(hash)
}
