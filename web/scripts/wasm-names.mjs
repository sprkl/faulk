// The "name" custom section of a WebAssembly module holds the function and local names that make a stack trace
// readable (`project_wasm::pw_apply` instead of `wasm-function[1234]`). The release build keeps it; the production
// site does not need it (#248): `stripNames` rewrites a module without that one section, the rest byte for byte.

/** @param {Uint8Array} bytes */
function readSections(bytes) {
  if (bytes.length < 8 || bytes[0] !== 0 || bytes[1] !== 0x61 || bytes[2] !== 0x73 || bytes[3] !== 0x6d) throw new Error('not a WebAssembly module')
  const sections = []
  let at = 8
  while (at < bytes.length) {
    const start = at
    const id = bytes[at++]
    let size = 0
    for (let shift = 0; ; shift += 7) {
      if (at >= bytes.length || shift > 28) throw new Error('truncated WebAssembly section')
      const byte = bytes[at++]
      size += (byte & 0x7f) * 2 ** shift
      if (!(byte & 0x80)) break
    }
    const body = at
    const end = body + size
    if (end > bytes.length) throw new Error('truncated WebAssembly section')
    let name = null
    if (id === 0) {
      // A custom section starts with its name: a length (LEB128), then UTF-8 bytes.
      let length = 0
      let p = body
      for (let shift = 0; ; shift += 7) {
        if (p >= end || shift > 28) throw new Error('bad custom section name')
        const byte = bytes[p++]
        length += (byte & 0x7f) * 2 ** shift
        if (!(byte & 0x80)) break
      }
      if (p + length > end) throw new Error('bad custom section name')
      name = new TextDecoder().decode(bytes.subarray(p, p + length))
    }
    sections.push({ start, end, id, name })
    at = end
  }
  return sections
}

/** Whether the module carries a "name" section. @param {Uint8Array} bytes */
export function hasNames(bytes) {
  return readSections(bytes).some((s) => s.id === 0 && s.name === 'name')
}

/** The module without its "name" section; every other byte is kept. @param {Uint8Array} bytes @returns {Uint8Array} */
export function stripNames(bytes) {
  const drop = readSections(bytes).filter((s) => s.id === 0 && s.name === 'name')
  if (drop.length === 0) return bytes
  const parts = [bytes.subarray(0, 8)]
  let at = 8
  for (const s of drop) {
    parts.push(bytes.subarray(at, s.start))
    at = s.end
  }
  parts.push(bytes.subarray(at))
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0))
  let o = 0
  for (const p of parts) {
    out.set(p, o)
    o += p.length
  }
  return out
}
