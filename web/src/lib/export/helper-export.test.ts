// What the page sends the helper for an export (#319): the job's own bytes in standard base64, and a file
// name the helper accepts for every name the dialog can hold.
import { describe, expect, it } from 'vitest'
import { EXPORT_NAME } from '../generated/protocol'
import { helperFileName, toBase64 } from './helper-export'

/** The helper's own check of a stem (`crates/helper/src/disk.rs`, `export_path`), as far as text goes. */
function helperAccepts(stem: string): boolean {
  if (stem === '' || new TextEncoder().encode(stem).length > EXPORT_NAME.max_bytes) return false
  // eslint-disable-next-line no-control-regex
  if (/[/\\:<>"|?*\u0000-\u001f\u007f-\u009f]/.test(stem)) return false
  if (stem === '.' || stem === '..' || /[. ]$/.test(stem)) return false
  return !(EXPORT_NAME.devices as readonly string[]).includes(stem.split('.')[0]!.trimEnd().toLowerCase())
}

describe("the helper's export", () => {
  it('encodes only the bytes of the view, wherever it sits in its buffer', () => {
    const memory = new Uint8Array(70_000).map((_, i) => i % 251)
    const view = memory.subarray(1000, 1000 + 66_000)
    const decoded = Uint8Array.from(atob(toBase64(view)), (c) => c.charCodeAt(0))
    expect(decoded).toEqual(view.slice())
    expect(toBase64(new Uint8Array())).toBe('')
    expect(toBase64(Uint8Array.of(0xfb, 0xff))).toBe('+/8=')
  })

  it('gives a name the helper accepts for every name the dialog can hold', () => {
    const names = [
      'Song',
      'CON',
      'nul.txt',
      'com1',
      'Lpt3 ',
      '..',
      '.',
      '',
      '   ',
      '../../etc/passwd',
      'C:\\Windows\\x',
      'a:b',
      'what?*<>|"',
      'tab\tname',
      'é'.repeat(100),
      '日本語'.repeat(40),
      '🎵'.repeat(60),
      'a'.repeat(99) + '.',
      'con.' + 'x'.repeat(200),
      // cleanName cuts at 100 UTF-16 units: here inside the emoji's pair.
      'a'.repeat(99) + '\u{1F3B5}',
      'x\u0085y',
      '\u007f',
      'lone \uD83D surrogate',
    ]
    for (const name of names) {
      const stem = helperFileName(name)
      // What the helper parses: no surrogate left alone.
      expect(/[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/.test(stem), JSON.stringify(stem)).toBe(false)
      expect(helperAccepts(stem), `${JSON.stringify(name)} gave ${JSON.stringify(stem)}`).toBe(true)
    }
    expect(helperFileName('Song')).toBe('Song')
    expect(helperFileName('CON')).toBe('_CON')
    expect(new TextEncoder().encode(helperFileName('é'.repeat(100))).length).toBe(EXPORT_NAME.max_bytes)
  })
})
