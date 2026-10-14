import { describe, expect, it } from 'vitest'
import { LANG_STORAGE_KEY } from '../i18n/lang'
import { animBootScript, detectScript, inlineScript, rememberScript } from './scripts'

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

describe('animBootScript', () => {
  /** Runs the generated bootstrap against fake globals; returns what it observed and imported. */
  function boot(opts: { webgl2?: boolean; saveData?: boolean; elements?: number; intersecting?: boolean }) {
    const observed: unknown[] = []
    const imported: string[] = []
    let callback: ((entries: { isIntersecting: boolean }[]) => void) | undefined
    let disconnected = 0
    class IntersectionObserver {
      constructor(cb: typeof callback, readonly options: unknown) {
        callback = cb
      }
      observe(el: unknown) {
        observed.push(el)
      }
      disconnect() {
        disconnected++
      }
    }
    const window = {
      WebGL2RenderingContext: opts.webgl2 === false ? undefined : class {},
      IntersectionObserver,
    }
    const navigator = { connection: opts.saveData ? { saveData: true } : undefined }
    const els = Array.from({ length: opts.elements ?? 2 }, (_, i) => ({ i }))
    const document = { querySelectorAll: () => els }
    const source = animBootScript('/landing/anim.abc.js').replace(/import\(/g, '__import(')
    const __import = (url: string) => {
      imported.push(url)
      return { then: () => ({ catch: () => {} }) }
    }
    new Function('window', 'navigator', 'document', 'IntersectionObserver', '__import', source)(window, navigator, document, IntersectionObserver, __import)
    if (callback && opts.intersecting !== undefined) callback([{ isIntersecting: opts.intersecting }])
    return { observed, imported, disconnected, options: callback ? 'set' : 'unset' }
  }

  it('imports the module once an element intersects, then stops observing', () => {
    const r = boot({ intersecting: true })
    expect(r.observed.length).toBe(2)
    expect(r.imported).toEqual(['/landing/anim.abc.js'])
    expect(r.disconnected).toBe(1)
  })

  it('does nothing before an element intersects', () => {
    const r = boot({ intersecting: false })
    expect(r.imported).toEqual([])
    expect(r.disconnected).toBe(0)
  })

  it('never loads without WebGL2, with data saver, or without animated elements', () => {
    expect(boot({ webgl2: false, intersecting: true }).imported).toEqual([])
    expect(boot({ saveData: true, intersecting: true }).imported).toEqual([])
    expect(boot({ elements: 0, intersecting: true }).observed).toEqual([])
  })

  it('cannot close its script tag', () => {
    expect(animBootScript('/x</script><script>alert(1)//')).not.toContain('</script>')
    expect(inlineScript('x', 'module')).toBe('<script type="module">x</script>')
  })
})
