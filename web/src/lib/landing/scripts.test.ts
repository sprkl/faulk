import { describe, expect, it } from 'vitest'
import { LANG_STORAGE_KEY } from '../i18n/lang'
import { detectScript, inlineScript, rememberScript } from './scripts'

/** Runs the generated detection script, as a browser would, against fake globals. */
function runDetect(stored: string | null | 'throws', language: string | undefined) {
  const replaced: string[] = []
  const localStorage = {
    getItem(key: string) {
      if (stored === 'throws') throw new Error('storage disabled')
      return key === LANG_STORAGE_KEY ? stored : null
    },
  }
  const navigator = { language }
  const location = { replace: (href: string) => replaced.push(href) }
  new Function('localStorage', 'navigator', 'location', detectScript('/fr'))(localStorage, navigator, location)
  return replaced
}

describe('detectScript', () => {
  it('moves a French browser to /fr', () => {
    expect(runDetect(null, 'fr-FR')).toEqual(['/fr'])
    expect(runDetect(null, 'fr-BE')).toEqual(['/fr'])
  })

  it('keeps other browsers on the English page', () => {
    expect(runDetect(null, 'en-US')).toEqual([])
    expect(runDetect(null, 'de-DE')).toEqual([])
    expect(runDetect(null, undefined)).toEqual([])
  })

  it('follows the stored choice over the browser language', () => {
    expect(runDetect('en', 'fr-FR')).toEqual([])
    expect(runDetect('fr', 'en-US')).toEqual(['/fr'])
  })

  it('ignores an invalid stored value', () => {
    expect(runDetect('xx', 'fr-FR')).toEqual(['/fr'])
    expect(runDetect('xx', 'en-GB')).toEqual([])
  })

  it('falls back to the browser language when storage throws', () => {
    expect(runDetect('throws', 'fr-CA')).toEqual(['/fr'])
    expect(runDetect('throws', 'en-US')).toEqual([])
  })

  it('cannot close its script tag', () => {
    expect(detectScript('/fr</script><script>alert(1)//')).not.toContain('</script>')
    expect(inlineScript('x')).toBe('<script>x</script>')
  })
})

describe('rememberScript', () => {
  function setup(setItem: (key: string, value: string) => void) {
    let listener: ((e: { target: unknown }) => void) | undefined
    const document = { addEventListener: (_: string, fn: typeof listener) => (listener = fn) }
    new Function('document', 'localStorage', rememberScript())(document, { setItem })
    return (target: unknown) => listener!({ target })
  }
  const link = (lang: string | null) => ({ closest: () => (lang ? { getAttribute: () => lang } : null) })

  it('stores the language of a clicked language link', () => {
    const stored: [string, string][] = []
    const click = setup((k, v) => stored.push([k, v]))
    click(link('fr'))
    click(link(null))
    click(null)
    expect(stored).toEqual([[LANG_STORAGE_KEY, 'fr']])
  })

  it('still follows the link when storage throws', () => {
    const click = setup(() => {
      throw new Error('quota')
    })
    expect(() => click(link('en'))).not.toThrow()
  })
})
