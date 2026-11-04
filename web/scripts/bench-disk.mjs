// Usage, from web/: node scripts/bench-disk.mjs "$(pwd)/package.json" <scratch dir> (writes a 256 MiB file there; serves on port 5421).
// Ranged-read throughput in Chromium (AU-3): 256 KiB `File.slice().arrayBuffer()` reads, sequential and
// 8 in flight (the planner's limit), from an OPFS file and from a disk-backed File (input type=file).
import { createServer } from 'node:http'
import { writeFileSync, mkdirSync } from 'node:fs'
import { join } from 'node:path'
import { createRequire } from 'node:module'
const require = createRequire(process.argv[2])
const { chromium } = require('@playwright/test')

const dir = process.argv[3]
mkdirSync(dir, { recursive: true })
const SIZE = 256 * 1024 * 1024
const path = join(dir, 'bench.bin')
const buf = Buffer.alloc(SIZE)
for (let i = 0; i < SIZE; i += 4096) buf[i] = i & 255
writeFileSync(path, buf)

const page = `<!doctype html><input type=file id=f><script>
async function bench(file, label) {
  const CH = 256 * 1024, n = Math.floor(file.size / CH)
  let t = performance.now()
  for (let i = 0; i < n; i++) await file.slice(i * CH, (i + 1) * CH).arrayBuffer()
  const seq = performance.now() - t
  t = performance.now()
  let next = 0
  await Promise.all(Array.from({ length: 8 }, async () => { while (next < n) { const i = next++; await file.slice(i * CH, (i + 1) * CH).arrayBuffer() } }))
  const par = performance.now() - t
  // Random 256 KiB ranges, as jumps do.
  t = performance.now()
  for (let k = 0; k < 200; k++) { const i = Math.floor(Math.random() * n); await file.slice(i * CH, (i + 1) * CH).arrayBuffer() }
  const rnd = performance.now() - t
  const mb = file.size / 1e6
  return { label, mb, seqMBs: mb / (seq / 1000), par8MBs: mb / (par / 1000), randomReadMs: rnd / 200 }
}
window.opfs = async (bytes) => {
  const root = await navigator.storage.getDirectory()
  const h = await root.getFileHandle('bench.bin', { create: true })
  const w = await h.createWritable(); await w.write(new Uint8Array(bytes)); await w.close()
  return bench(await h.getFile(), 'opfs')
}
window.disk = () => bench(document.getElementById('f').files[0], 'disk File')
</script>`
const server = createServer((_, res) => res.writeHead(200, { 'content-type': 'text/html' }).end(page)).listen(5421)
const browser = await chromium.launch()
const p = await browser.newPage()
await p.goto('http://localhost:5421/')
await p.setInputFiles('#f', path)
const results = []
for (let r = 0; r < 3; r++) results.push(await p.evaluate(() => window.disk()))
results.push(await p.evaluate(async () => {
  // OPFS file of the same size, written in the page.
  const bytes = new Uint8Array(256 * 1024 * 1024)
  return window.opfs(bytes.buffer)
}))
for (let r = 0; r < 2; r++) results.push(await p.evaluate(async () => {
  const root = await navigator.storage.getDirectory()
  const h = await root.getFileHandle('bench.bin')
  const file = await h.getFile()
  const CH = 256 * 1024, n = Math.floor(file.size / CH)
  let t = performance.now()
  for (let i = 0; i < n; i++) await file.slice(i * CH, (i + 1) * CH).arrayBuffer()
  const seq = performance.now() - t
  t = performance.now(); let next = 0
  await Promise.all(Array.from({ length: 8 }, async () => { while (next < n) { const i = next++; await file.slice(i * CH, (i + 1) * CH).arrayBuffer() } }))
  const par = performance.now() - t
  t = performance.now()
  for (let k = 0; k < 200; k++) { const i = Math.floor(Math.random() * n); await file.slice(i * CH, (i + 1) * CH).arrayBuffer() }
  const mb = file.size / 1e6
  return { label: 'opfs (again)', mb, seqMBs: mb / (seq / 1000), par8MBs: mb / (par / 1000), randomReadMs: (performance.now() - t) / 200 }
}))
console.log(JSON.stringify(results, null, 1))
console.log(await browser.version())
await browser.close()
server.close()
