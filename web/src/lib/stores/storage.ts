/** Where the page keeps its helper token and language: `localStorage`, when the browser allows it. */
export function storage(): Storage | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage
  } catch {
    return null
  }
}

export const TOKEN_KEY = 'faulk.helperToken'
