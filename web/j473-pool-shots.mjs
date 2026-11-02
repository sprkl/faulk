// Scratch script for #473 PR 2: screenshots of the clip pool, repeats and linked marks (deleted before the PR).
import { chromium } from '@playwright/test'

const out = process.argv[2]
const lang = process.argv[3] ?? 'en'
const browser = await chromium.launch()
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })
const errors = []
page.on('console', (m) => m.type() === 'error' && !m.text().includes('47800') && errors.push(m.text()))
page.on('pageerror', (e) => errors.push(String(e)))
await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
await page.goto('http://localhost:5473/app')
await page.getByTestId('engine-state').getByRole('button').click()
await page.getByTestId('engine-mode').waitFor()
const run = (c) => page.evaluate((c) => window.__faulkRun(c), c)
const view = () => page.evaluate(() => window.__faulkView())
const track = Object.keys((await view()).project.tracks)[0]
const make = async (start, name) => {
  await run({ type: 'clip.create', track, at: { at: 'arrangement', start }, name })
  await page.waitForTimeout(150)
  return Object.entries((await view()).project.tracks[track].clips).find(([, c]) => c.name === name)[0]
}
const riff = await make(0, 'Riff')
await run({ type: 'clip.notes.add', clip: riff, notes: [0, 1, 2, 3].map((i) => ({ start: i * 960, pitch: 60 + i * 2, length: 480, velocity: 100 })) })
const pad = await make(7680, 'Pad')
await run({ type: 'clip.notes.add', clip: pad, notes: [{ start: 0, pitch: 55, length: 3000, velocity: 90 }] })
const aside = await make(11520, 'Aside')
await run({ type: 'clip.notes.add', clip: aside, notes: [{ start: 480, pitch: 67, length: 480, velocity: 90 }] })
await page.waitForTimeout(200)
const placements = (await view()).project.tracks[track].placements
const pOf = (clip) => Object.entries(placements).find(([, p]) => p.clip === clip)[0]
await run({ type: 'arrangement.resize', placement: pOf(riff), edge: 'end', to: 3840 * 2 })
await run({ type: 'arrangement.place', clip: pad, start: 15360 })
await run({ type: 'arrangement.delete', placements: [pOf(aside)] })
await page.getByTestId('arr-zoom-out').click()
await page.getByTestId('panel-clips').click()
await page.waitForTimeout(500)
await page.screenshot({ path: `${out}/pool-${lang}-1440.png` })
await page.getByTestId('clips-panel').screenshot({ path: `${out}/pool-panel-${lang}.png` })
console.log(JSON.stringify({ errors }))
await browser.close()
