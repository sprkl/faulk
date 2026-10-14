// Generates faulk.ico (the helper's executable and tray icon) from the Faulk monogram.
//
// Usage: node crates/helper/assets/make-icon.mjs [preview-dir]
// With a preview directory, also writes one PNG per size there, to look at.
//
// No dependency: the two monogram paths only use M, H, C and Z, so the script flattens
// them and rasterizes them itself. The mark is white on the accent colour, on a rounded
// square, so it reads on light and dark taskbars.

import { writeFileSync, mkdirSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { crc32, deflateSync } from 'node:zlib'

// Logo track A, viewBox 0 0 430 435.
const VIEW = { w: 430, h: 435 }
const MARK = ['M0 0 H430 C392 82 330 157 245 157 H0 Z', 'M0 185 H338 C228 245 92 330 4 435 H0 Z']
// The 16 px variant widens the gap between the two shapes.
const MARK_16 = ['M0 0 H430 C392 82 330 157 245 157 H0 Z', 'M0 220 H338 C228 270 92 340 4 435 H0 Z']

const ACCENT = [0x66, 0x33, 0xf4]
const SIZES = [16, 20, 24, 32, 48, 256]
/** Share of the icon the mark's height takes (larger when small, to stay legible), and the corner radius. */
const markScale = (size) => (size <= 24 ? 0.7 : 0.62)
const CORNER = 0.22
const SUBSAMPLES = 8

/** Parses an absolute M/H/C/Z path into a polygon. */
function polygon(d) {
  const tokens = d.match(/[MHCZ]|-?\d+(\.\d+)?/g)
  const points = []
  let x = 0
  let y = 0
  let i = 0
  const num = () => Number(tokens[i++])
  while (i < tokens.length) {
    const cmd = tokens[i++]
    if (cmd === 'M') {
      x = num()
      y = num()
      points.push([x, y])
    } else if (cmd === 'H') {
      x = num()
      points.push([x, y])
    } else if (cmd === 'C') {
      const [x1, y1, x2, y2, x3, y3] = [num(), num(), num(), num(), num(), num()]
      for (let s = 1; s <= 32; s++) {
        const t = s / 32
        const u = 1 - t
        points.push([
          u * u * u * x + 3 * u * u * t * x1 + 3 * u * t * t * x2 + t * t * t * x3,
          u * u * u * y + 3 * u * u * t * y1 + 3 * u * t * t * y2 + t * t * t * y3,
        ])
      }
      x = x3
      y = y3
    } else if (cmd === 'Z') {
      // Closed implicitly by the rasterizer.
    } else throw new Error(`unsupported path command ${cmd}`)
  }
  return points
}

function roundedSquare(size, radius) {
  const points = []
  const corners = [
    [size - radius, radius, -Math.PI / 2],
    [size - radius, size - radius, 0],
    [radius, size - radius, Math.PI / 2],
    [radius, radius, Math.PI],
  ]
  for (const [cx, cy, start] of corners) {
    for (let s = 0; s <= 8; s++) {
      const a = start + (s / 8) * (Math.PI / 2)
      points.push([cx + radius * Math.cos(a), cy + radius * Math.sin(a)])
    }
  }
  return points
}

/** Coverage (0..1) of each pixel by the polygons (even-odd), with exact horizontal spans. */
function coverage(polygons, size) {
  const out = new Float32Array(size * size)
  for (let py = 0; py < size; py++) {
    for (let sy = 0; sy < SUBSAMPLES; sy++) {
      const y = py + (sy + 0.5) / SUBSAMPLES
      const xs = []
      for (const poly of polygons) {
        for (let k = 0; k < poly.length; k++) {
          const [x0, y0] = poly[k]
          const [x1, y1] = poly[(k + 1) % poly.length]
          if (y0 <= y !== y1 <= y) xs.push(x0 + ((y - y0) / (y1 - y0)) * (x1 - x0))
        }
      }
      xs.sort((a, b) => a - b)
      for (let k = 0; k + 1 < xs.length; k += 2) {
        const a = Math.max(0, xs[k])
        const b = Math.min(size, xs[k + 1])
        for (let px = Math.floor(a); px < Math.ceil(b); px++) {
          const overlap = Math.min(b, px + 1) - Math.max(a, px)
          if (overlap > 0) out[py * size + px] += overlap / SUBSAMPLES
        }
      }
    }
  }
  return out
}

/** RGBA pixels, top row first. */
function render(size) {
  const markHeight = size * markScale(size)
  const scale = markHeight / VIEW.h
  const ox = (size - VIEW.w * scale) / 2
  const oy = (size - markHeight) / 2
  const mark = (size <= 16 ? MARK_16 : MARK).map((d) =>
    polygon(d).map(([x, y]) => [ox + x * scale, oy + y * scale]),
  )
  const bg = coverage([roundedSquare(size, size * CORNER)], size)
  const fg = coverage(mark, size)
  const rgba = Buffer.alloc(size * size * 4)
  for (let i = 0; i < size * size; i++) {
    const m = Math.min(1, fg[i])
    for (let c = 0; c < 3; c++) rgba[i * 4 + c] = Math.round(ACCENT[c] * (1 - m) + 255 * m)
    rgba[i * 4 + 3] = Math.round(Math.min(1, bg[i]) * 255)
  }
  return rgba
}

function png(rgba, size) {
  const chunk = (type, data) => {
    const len = Buffer.alloc(4)
    len.writeUInt32BE(data.length)
    const body = Buffer.concat([Buffer.from(type, 'ascii'), data])
    const crc = Buffer.alloc(4)
    crc.writeUInt32BE(crc32(body))
    return Buffer.concat([len, body, crc])
  }
  const header = Buffer.alloc(13)
  header.writeUInt32BE(size, 0)
  header.writeUInt32BE(size, 4)
  header[8] = 8 // bit depth
  header[9] = 6 // RGBA
  const raw = Buffer.alloc(size * (size * 4 + 1))
  for (let y = 0; y < size; y++) rgba.copy(raw, y * (size * 4 + 1) + 1, y * size * 4, (y + 1) * size * 4)
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', header),
    chunk('IDAT', deflateSync(raw)),
    chunk('IEND', Buffer.alloc(0)),
  ])
}

/** A 32-bit DIB as stored in an .ico: BGRA rows bottom-up, then an empty AND mask. */
function dib(rgba, size) {
  const header = Buffer.alloc(40)
  header.writeUInt32LE(40, 0)
  header.writeInt32LE(size, 4)
  header.writeInt32LE(size * 2, 8) // colour + mask
  header.writeUInt16LE(1, 12)
  header.writeUInt16LE(32, 14)
  const pixels = Buffer.alloc(size * size * 4)
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const s = (y * size + x) * 4
      const d = ((size - 1 - y) * size + x) * 4
      pixels[d] = rgba[s + 2]
      pixels[d + 1] = rgba[s + 1]
      pixels[d + 2] = rgba[s]
      pixels[d + 3] = rgba[s + 3]
    }
  }
  const maskRow = Math.ceil(size / 32) * 4
  return Buffer.concat([header, pixels, Buffer.alloc(maskRow * size)])
}

function ico(images) {
  const head = Buffer.alloc(6)
  head.writeUInt16LE(1, 2) // icon
  head.writeUInt16LE(images.length, 4)
  const entries = []
  let offset = 6 + 16 * images.length
  for (const { size, data } of images) {
    const e = Buffer.alloc(16)
    e[0] = size >= 256 ? 0 : size
    e[1] = size >= 256 ? 0 : size
    e.writeUInt16LE(1, 4) // planes
    e.writeUInt16LE(32, 6) // bits per pixel
    e.writeUInt32LE(data.length, 8)
    e.writeUInt32LE(offset, 12)
    offset += data.length
    entries.push(e)
  }
  return Buffer.concat([head, ...entries, ...images.map((i) => i.data)])
}

const here = dirname(fileURLToPath(import.meta.url))
const preview = process.argv[2]
if (preview) mkdirSync(preview, { recursive: true })
const images = SIZES.map((size) => {
  const rgba = render(size)
  if (preview) writeFileSync(join(preview, `faulk-${size}.png`), png(rgba, size))
  // Large images are stored as PNG, as Windows expects; small ones as DIB.
  return { size, data: size >= 256 ? png(rgba, size) : dib(rgba, size) }
})
writeFileSync(join(here, 'faulk.ico'), ico(images))
console.log(`wrote ${join(here, 'faulk.ico')} (${SIZES.join(', ')} px)`)
