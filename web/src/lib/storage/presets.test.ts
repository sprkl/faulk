import { describe, expect, it } from 'vitest'
import { MemoryDir } from './memory-dir'
import { PresetLibrary, type LibraryDir } from './presets'

const ID = '01HZY00000000000000000AAAA'
const OTHER = '01HZY00000000000000000BBBB'
const text = (s: string) => new TextEncoder().encode(s)

function library(root = new MemoryDir('presets')) {
  const dir: LibraryDir = { open: async () => root as unknown as FileSystemDirectoryHandle }
  return { root, lib: new PresetLibrary(dir, true) }
}

describe('PresetLibrary', () => {
  it('reads the preset files and leaves every other file alone', async () => {
    const { root, lib } = library()
    root.put(`${ID}.json`, '{"a":1}').put('notes.txt', 'mine').put('../x.json', '{}').put('lower.json', '{}')
    expect(await lib.read()).toEqual([{ id: ID, text: '{"a":1}' }])
    expect(root.text('notes.txt')).toBe('mine')
  })

  it('writes a file whole, replaces it, and removes it', async () => {
    const { root, lib } = library()
    await lib.write(ID, text('one'))
    await lib.write(ID, text('two'))
    expect(root.text(`${ID}.json`)).toBe('two')
    await lib.remove(ID)
    expect(root.text(`${ID}.json`)).toBeNull()
    // Removing what is not there is nothing.
    await lib.remove(OTHER)
  })

  it('keeps the old file when the write fails before the file is closed (a tab closed meanwhile)', async () => {
    const { root, lib } = library()
    await lib.write(ID, text('kept'))
    root.hooks.failClose = () => true
    await expect(lib.write(ID, text('lost'))).rejects.toThrow()
    expect(root.text(`${ID}.json`)).toBe('kept')
    root.hooks.failClose = undefined
    root.hooks.failWrite = () => true
    await expect(lib.write(ID, text('lost'))).rejects.toThrow()
    expect(root.text(`${ID}.json`)).toBe('kept')
  })

  it('refuses a name that is not a preset id', async () => {
    const { lib } = library()
    await expect(lib.write('../escape', text('x'))).rejects.toThrow()
  })

  it('does nothing where the browser has no storage', async () => {
    const lib = new PresetLibrary({ open: async () => null }, false)
    expect(await lib.read()).toEqual([])
    await lib.write(ID, text('x'))
    await lib.remove(ID)
  })
})
