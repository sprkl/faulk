import { mkdirSync } from 'node:fs'
import { expect, test, type CDPSession, type Page } from '@playwright/test'
import { best, chooseMidiInput, engineUp, fakeWebMidi, mainThreadMeter, median, watch } from './support'

// MX-3 (#92): the mixer view in a real browser. Real worklet, engine and project core: every
// control goes through the dispatcher as a command, and the meters are the engine's own report.
// The hidden `feedback` element carries that report, per track slot, for the sound checks.
// Besides the functional test, two measurements are printed: the render cost of 32 strips and,
// when FAULK_E2E_SOAK_MIN is set (minutes), a soak that watches the heap, the DOM and the listeners.

test.describe.configure({ mode: 'serial' })

const peaks = async (page: Page) => ((await page.getByTestId('feedback').getAttribute('data-track-peaks')) ?? '').split(',').filter(Boolean).map(Number)
const send = (page: Page, bytes: number[]) => page.evaluate((b) => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send(b), bytes)

async function start(page: Page) {
  await fakeWebMidi(page)
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await engineUp(page, 'Web')
  await chooseMidiInput(page, 'E2E Keys')
  await expect(page.getByTestId('track-list')).toHaveAttribute('data-track-count', '1')
  await page.getByTestId('nav-mixer').click()
  await expect(page.getByTestId('mixer')).toBeVisible()
}

async function addTracks(page: Page, count: number) {
  for (let i = 0; i < count; i++) {
    await page.getByTestId('add-track').click()
    await page.getByTestId('add-track-menu').getByRole('menuitem').first().click()
  }
}

const gain = (page: Page, strip = 0) => page.getByTestId('mixer-gain').nth(strip)

test('the strips drive the tracks, and the meters show what the engine sounds', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await expect(page.getByTestId('mixer-strip')).toHaveCount(1)
  await expect(page.getByTestId('mixer-master')).toHaveCount(1)
  const strip = page.getByTestId('mixer-strip').first()
  await expect(strip.getByTestId('mixer-name')).toHaveText('Synth')
  await expect(strip.getByTestId('mixer-chain')).toContainText('Oscillator')

  // A held note: the track meter at unity.
  await send(page, [0x90, 69, 127])
  await expect.poll(async () => (await peaks(page))[0]).toBeGreaterThan(0.05)
  await page.waitForTimeout(500)
  const unity = (await peaks(page))[0]
  // The strip's own meter shows it (after the fader), and only that strip's.
  await expect.poll(async () => Number(await strip.getByTestId('mixer-meter').getAttribute('aria-valuenow'))).toBeGreaterThan(-40)

  // -6 dB on the fader, measured on the track's own meter (after the fader).
  const fader = strip.getByTestId('mixer-fader')
  // Seven steps down from 0 dB on the fader's curve are -6 dB.
  await fader.focus()
  for (let i = 0; i < 7; i++) await page.keyboard.press('ArrowDown')
  await expect(gain(page)).toHaveText('−6.0 dB')
  await expect.poll(async () => (await peaks(page))[0] / unity, { timeout: 5000 }).toBeGreaterThan(0.45)
  await expect.poll(async () => (await peaks(page))[0] / unity).toBeLessThan(0.56)
  await fader.dblclick()
  await expect(gain(page)).toHaveText('0.0 dB')
  await expect.poll(async () => (await peaks(page))[0] / unity, { timeout: 5000 }).toBeGreaterThan(0.9)

  // The keyboard moves the fader, one command per step.
  await fader.focus()
  await page.keyboard.press('ArrowDown')
  await expect(gain(page)).not.toHaveText('0.0 dB')
  await page.keyboard.press('Home')
  await expect(gain(page)).toHaveText('−∞ dB')
  await expect.poll(async () => (await peaks(page))[0], { timeout: 5000 }).toBeLessThan(0.001)
  await fader.dblclick()
  await expect(gain(page)).toHaveText('0.0 dB')

  // Mute silences the track, unmute brings it back.
  const mute = strip.getByTestId('mixer-mute')
  await mute.click()
  await expect(mute).toHaveAttribute('aria-pressed', 'true')
  await expect.poll(async () => (await peaks(page))[0], { timeout: 5000 }).toBeLessThan(0.001)
  await mute.click()
  await expect.poll(async () => (await peaks(page))[0], { timeout: 5000 }).toBeGreaterThan(0.05)

  // Solo on another track silences this one; solo on this one brings it back (cumulative).
  await addTracks(page, 1)
  await expect(page.getByTestId('mixer-strip')).toHaveCount(2)
  const second = page.getByTestId('mixer-strip').nth(1)
  await second.getByTestId('mixer-solo').click()
  await expect.poll(async () => (await peaks(page))[0], { timeout: 5000 }).toBeLessThan(0.001)
  await strip.getByTestId('mixer-solo').click()
  await expect.poll(async () => (await peaks(page))[0], { timeout: 5000 }).toBeGreaterThan(0.05)
  await strip.getByTestId('mixer-solo').click()
  await second.getByTestId('mixer-solo').click()

  // The master fader moves the master's output (the master meter sums the tracks, after the master).
  const master = page.getByTestId('mixer-master')
  const masterPeak = async () => Number(await page.getByTestId('feedback').getAttribute('data-master-peak'))
  await expect.poll(masterPeak, { timeout: 5000 }).toBeGreaterThan(0.05)
  await page.waitForTimeout(300)
  const masterBefore = Number(await page.getByTestId('feedback').getAttribute('data-master-peak'))
  await master.getByTestId('mixer-master-fader').focus()
  for (let i = 0; i < 7; i++) await page.keyboard.press('ArrowDown')
  await expect(master.getByTestId('mixer-gain')).toHaveText('−6.0 dB')
  await expect
    .poll(async () => Number(await page.getByTestId('feedback').getAttribute('data-master-peak')) / masterBefore, { timeout: 5000 })
    .toBeLessThan(0.56)
  await master.getByTestId('mixer-master-fader').dblclick()

  // The devices button selects the strip's track and shows the Devices panel.
  await second.getByTestId('mixer-devices').click()
  await expect(second).toHaveAttribute('aria-current', 'true')
  await expect(page.getByTestId('bottom-panel')).toHaveAttribute('data-panel', 'devices')

  const shots = process.env.FAULK_E2E_SHOTS
  if (shots) {
    mkdirSync(shots, { recursive: true })
    await page.screenshot({ path: `${shots}/mx-3-mixer.png` })
  }
  await send(page, [0x80, 69, 0])
  expect(errors).toEqual([])
})

test('a drag of the fader is one undo step', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  const fader = page.getByTestId('mixer-strip').first().getByTestId('mixer-fader')
  await expect(gain(page)).toHaveText('0.0 dB')
  const box = (await fader.boundingBox())!
  const x = box.x + box.width / 2
  await page.mouse.move(x, box.y + box.height * 0.22)
  await page.mouse.down()
  for (let i = 1; i <= 12; i++) await page.mouse.move(x, box.y + box.height * (0.22 + i * 0.04))
  // Let go outside the fader: the window still ends the gesture.
  await page.mouse.move(x + 200, box.y + box.height * 0.8)
  await page.mouse.up()
  await expect(gain(page)).not.toHaveText('0.0 dB')
  const dragged = await gain(page).innerText()
  // One undo takes the whole drag back, not one step of it.
  await page.getByRole('button', { name: 'Undo', exact: true }).click()
  await expect(gain(page)).toHaveText('0.0 dB')
  await page.getByRole('button', { name: 'Redo', exact: true }).click()
  await expect(gain(page)).toHaveText(dragged)
  expect(errors).toEqual([])
})

test('a track added with the mixer open gets a strip and a meter; stopped reports silence the meters', async ({ page }) => {
  const { errors } = watch(page)
  // Keeps the AudioContext so that the test can stop the reports the way an engine that goes away does.
  await page.addInitScript(() => {
    const Native = window.AudioContext
    const contexts: AudioContext[] = []
    window.AudioContext = class extends Native {
      constructor(options?: AudioContextOptions) {
        super(options)
        contexts.push(this)
      }
    }
    Object.assign(window, { __contexts: contexts })
  })
  await start(page)
  await addTracks(page, 2)
  await expect(page.getByTestId('mixer-strip')).toHaveCount(3)
  // The new track took the keyboard: give it back to the first strip.
  await page.getByTestId('mixer-strip').first().getByTestId('mixer-name').click()
  await send(page, [0x90, 69, 127])
  await expect.poll(async () => (await peaks(page))[0]).toBeGreaterThan(0.05)
  const meter = page.getByTestId('mixer-strip').first().getByTestId('mixer-meter')
  await expect.poll(async () => Number(await meter.getAttribute('aria-valuenow'))).toBeGreaterThan(-40)
  // The other strips stay at the floor: the slot lookup lights the right strip.
  for (const other of [1, 2]) await expect(page.getByTestId('mixer-strip').nth(other).getByTestId('mixer-meter')).toHaveAttribute('aria-valuenow', '-60')
  await page.evaluate(() => (window as unknown as { __contexts: AudioContext[] }).__contexts.forEach((c) => void c.suspend()))
  // No report any more: the bars fall to the floor instead of staying lit.
  await expect.poll(async () => Number(await meter.getAttribute('aria-valuenow')), { timeout: 8000 }).toBe(-60)
  await page.evaluate(() => (window as unknown as { __contexts: AudioContext[] }).__contexts.forEach((c) => void c.resume()))
  await send(page, [0x80, 69, 0])
  expect(errors).toEqual([])
})

interface Counters {
  Nodes: number
  JSEventListeners: number
  JSHeapUsedSize: number
  ScriptDuration: number
  LayoutDuration: number
  RecalcStyleDuration: number
  TaskDuration: number
}

async function counters(cdp: CDPSession, collect = false): Promise<Counters> {
  if (collect) await cdp.send('HeapProfiler.collectGarbage')
  const { metrics } = await cdp.send('Performance.getMetrics')
  const by = Object.fromEntries(metrics.map((m) => [m.name, m.value]))
  return {
    Nodes: by.Nodes,
    JSEventListeners: by.JSEventListeners,
    JSHeapUsedSize: by.JSHeapUsedSize,
    ScriptDuration: by.ScriptDuration,
    LayoutDuration: by.LayoutDuration,
    RecalcStyleDuration: by.RecalcStyleDuration,
    TaskDuration: by.TaskDuration,
  }
}

/** Frame intervals (ms) and long tasks over a window of the page. */
function frames(page: Page, ms: number) {
  return page.evaluate(
    (window_ms) =>
      new Promise<{ p50: number; p95: number; max: number; frames: number; longTasks: number; writes: number }>((resolve) => {
        const intervals: number[] = []
        let longTasks = 0
        const observer = new PerformanceObserver((list) => (longTasks += list.getEntries().length))
        observer.observe({ entryTypes: ['longtask'] })
        // The style writes the page makes inside this very window (the observer of the render-cost test, if it is installed), so that they are counted over the same frames.
        const written = () => (window as unknown as { __writes?: number }).__writes ?? 0
        const writes0 = written()
        let last = performance.now()
        const end = last + window_ms
        const tick = (now: number) => {
          intervals.push(now - last)
          last = now
          if (now < end) requestAnimationFrame(tick)
          else {
            observer.disconnect()
            intervals.shift()
            intervals.sort((a, b) => a - b)
            const at = (q: number) => intervals[Math.min(intervals.length - 1, Math.floor(q * intervals.length))]
            resolve({ p50: at(0.5), p95: at(0.95), max: intervals[intervals.length - 1], frames: intervals.length, longTasks, writes: written() - writes0 })
          }
        }
        requestAnimationFrame(tick)
      }),
    ms,
  )
}

/**
 * Two ceilings on the mixer view with 32 strips whose meters move (#739). A time cannot tell a regression from a busy
 * machine closely enough (the ratios of runs under a parallel build overlapped those of a mixer that did three times
 * the meter work), so the ceiling on time is wide and the work is counted.
 *
 * `MIXER_CEILING`: CPU time the page's renderer process spent per frame in the mixer view, over the same in the grid
 * view with the same tracks (alternately, in the same page, the best round of each). 2.5 to 5.0 clean, on a quiet
 * machine and under a cargo build; about 5 with the meters rendered three times. It catches a gross regression.
 *
 * `MIXER_WRITES_CEILING`: style writes per frame in the mixer view, counted by a MutationObserver. 29 to 30 clean
 * (the median of 7 rounds, quiet or under a cargo build); a meter rendered twice reads 58 to 60.
 */
const MIXER_CEILING = 8
const MIXER_WRITES_CEILING = 45
const ROUNDS = 7

test('32 strips with every meter moving: render cost @full', async ({ page }) => {
  test.setTimeout(240_000)
  const { errors } = watch(page)
  await start(page)
  await addTracks(page, 31)
  await expect(page.getByTestId('mixer-strip')).toHaveCount(32)
  // Every MIDI track is armed: the one note reaches all 32 instruments, so all 32 meters move.
  for (const arm of await page.getByTestId('mixer-arm').all()) await arm.click()
  await send(page, [0x90, 69, 127])
  await expect.poll(async () => (await peaks(page)).filter((p) => p > 0.01).length, { timeout: 15000 }).toBe(32)
  const cdp = await page.context().newCDPSession(page)
  await cdp.send('Performance.enable')

  // Three windows, the best one counts: noise on a shared runner only adds time.
  const WINDOW_MS = 3000
  const WINDOWS = 3
  const meter = await mainThreadMeter(page)
  const browserCdp = await page.context().browser()!.newBrowserCDPSession()
  const rendererCpu = async () => {
    const { processInfo } = await browserCdp.send('SystemInfo.getProcessInfo')
    return processInfo.filter((p) => p.type === 'renderer').reduce((a, p) => a + p.cpuTime, 0)
  }
  await cdp.send('Profiler.enable')
  /** Share of the main thread the page's JavaScript used (CPU profile: samples that are not idle or the browser's own). */
  const busyShare = async () => {
    await cdp.send('Profiler.start')
    await page.waitForTimeout(WINDOW_MS)
    const { profile } = await cdp.send('Profiler.stop')
    const idle = new Set(profile.nodes.filter((n) => ['(idle)', '(program)', '(garbage collector)'].includes(n.callFrame.functionName)).map((n) => n.id))
    let busy = 0
    let total = 0
    profile.samples!.forEach((id, i) => {
      const dt = profile.timeDeltas![i]
      total += dt
      if (!idle.has(id)) busy += dt
    })
    return busy / total
  }
  const measure = async (label: string, moving: boolean, windowCount = WINDOWS) => {
    await page.waitForTimeout(500)
    // The note is pulsed (on, off, on...) all through the windows, so that the meters keep moving whatever the
    // engine's level settles to; the style writes the page makes meanwhile are counted, to prove the windows measure a moving mixer.
    await page.evaluate(() => {
      const w = window as unknown as { __fakeMidi: { send(b: number[]): void }; __pulse?: number; __writes?: number; __observer?: MutationObserver }
      w.__writes = 0
      w.__observer?.disconnect()
      w.__observer = new MutationObserver((list) => (w.__writes! += list.length))
      w.__observer.observe(document.body, { attributes: true, attributeFilter: ['style'], subtree: true })
      let on = false
      w.__pulse = window.setInterval(() => {
        w.__fakeMidi.send(on ? [0x80, 69, 0] : [0x90, 69, 127])
        on = !on
      }, 120)
    })
    const started = Date.now()
    const before = await counters(cdp)
    const windows = []
    const cpus: number[] = []
    try {
      for (let i = 0; i < windowCount; i++) {
        const c0 = await rendererCpu()
        const w = await meter.measure(() => Promise.all([frames(page, WINDOW_MS), busyShare()]))
        const c1 = await rendererCpu()
        cpus.push(((c1 - c0) * 1000) / w.result[0].frames)
        windows.push(w)
      }
    } catch (error) {
      // A failed window must not leave the pulse and the observer running for the rest of the page's life.
      await page.evaluate(() => {
        const w = window as unknown as { __pulse?: number; __observer?: MutationObserver }
        clearInterval(w.__pulse)
        w.__observer?.disconnect()
      })
      throw error
    }
    const after = await counters(cdp)
    const elapsed = Date.now() - started
    const writes = await page.evaluate(() => {
      const w = window as unknown as { __pulse: number; __writes: number; __observer: MutationObserver }
      clearInterval(w.__pulse)
      w.__observer.disconnect()
      return w.__writes
    })
    await send(page, [0x80, 69, 0])
    if (moving) expect(writes, 'style writes while the meters pulse').toBeGreaterThan(100)
    const perSecond = (key: keyof Counters) => ((after[key] - before[key]) * 1_000_000) / elapsed
    // The cost: main-thread time per frame in reference units (see `mainThreadMeter`), best window.
    const units = windows.map((w) => w.cost.task / w.result[0].frames / w.cost.unit)
    const cost = best(units)
    const [f, share] = windows[units.indexOf(cost)]!.result
    console.log(
      `mixer-cost ${label}: frames p50 ${f.p50.toFixed(1)} ms, p95 ${f.p95.toFixed(1)} ms, max ${f.max.toFixed(1)} ms over ${f.frames} frames, long tasks ${f.longTasks}; ` +
        `JavaScript busy share ${(share * 100).toFixed(2)} %; main thread per frame ${cost.toFixed(4)} reference units (best of ${windowCount}); per second: script ${perSecond('ScriptDuration').toFixed(1)} ms, layout ${perSecond('LayoutDuration').toFixed(1)} ms, style ${perSecond('RecalcStyleDuration').toFixed(1)} ms; ` +
        `style writes ${writes}; nodes ${after.Nodes}, listeners ${after.JSEventListeners}`,
    )
    // Style writes per frame, over the frames of each window (not over the setup around them): the median window.
    const writesPerFrame = median(windows.map((w) => w.result[0].writes / w.result[0].frames))
    return { f, share, cost, cpu: best(cpus), writesPerFrame }
  }

  // Two measurements (#739; the absolute main-thread figure moved from 1.3 to 2.0 reference units under a parallel
  // build). The time is a ratio to the same page's grid view with the same 32 tracks, rounds alternating the two views,
  // the best round of each view counting (noise only ever adds time): it follows the machine less than an absolute
  // figure but still moves, so its ceiling is wide. The work is counted: style writes per frame of the moving mixer.
  const rounds: { mixer: number; baseline: number; mixerUnits: number; baselineUnits: number; share: number; baselineShare: number; writes: number }[] = []
  for (let round = 0; round < ROUNDS; round++) {
    await page.getByTestId('nav-mixer').click()
    await expect(page.getByTestId('mixer')).toBeVisible()
    const mixer = await measure(`mixer view, 32 strips (round ${round + 1}/${ROUNDS})`, true, 1)
    await page.getByTestId('nav-grid').click()
    const baseline = await measure(`grid view, 32 tracks (baseline, round ${round + 1}/${ROUNDS})`, false, 1)
    rounds.push({
      mixer: mixer.cpu,
      baseline: baseline.cpu,
      mixerUnits: mixer.cost,
      baselineUnits: baseline.cost,
      share: mixer.share,
      writes: mixer.writesPerFrame,
      baselineShare: baseline.share,
    })
  }
  const ratio = best(rounds.map((r) => r.mixer)) / best(rounds.map((r) => r.baseline))
  console.log(
    `mixer-cost ratio to the grid view: ${ratio.toFixed(2)} = best of ${ROUNDS} rounds ${best(rounds.map((r) => r.mixer)).toFixed(2)} ms of renderer CPU per frame in the mixer view over ${best(rounds.map((r) => r.baseline)).toFixed(2)} in the grid view; ` +
      `per round, mixer: ${rounds.map((r) => r.mixer.toFixed(1)).join(', ')}, grid: ${rounds.map((r) => r.baseline.toFixed(1)).join(', ')}; ` +
      `main-thread reference units, best mixer ${best(rounds.map((r) => r.mixerUnits)).toFixed(3)}, best grid ${best(rounds.map((r) => r.baselineUnits)).toFixed(3)}; extra JavaScript busy share ${((best(rounds.map((r) => r.share)) - best(rounds.map((r) => r.baselineShare))) * 100).toFixed(2)} points`,
  )

  // The time ceiling catches a gross regression only (a mixer 2.5 to 3 times heavier); a doubling that adds no style
  // write (canvas, layout, JavaScript) passes it. The frame intervals are printed, not asserted: they follow the
  // runner (vsync, neighbours), not the code.
  expect(ratio).toBeLessThan(MIXER_CEILING)
  // The work, counted: a busy machine changes how long a frame takes, not how many style writes the meters make in
  // it, so a render done twice doubles it. Counted over the frames of the window, median of the rounds. It sees
  // `style` attribute writes only: a meter moved to a canvas or to classes would need its own count.
  const writes = median(rounds.map((r) => r.writes))
  console.log(`mixer-cost style writes per frame in the mixer view: ${writes.toFixed(1)} (rounds: ${rounds.map((r) => r.writes.toFixed(1)).join(', ')})`)
  expect(writes, 'style writes per frame, mixer view').toBeLessThan(MIXER_WRITES_CEILING)
  await send(page, [0x80, 69, 0])
  expect(errors).toEqual([])
})

const SOAK_MIN = Number(process.env.FAULK_E2E_SOAK_MIN) || 0

test('soak: no growth of the heap, the DOM or the listeners', async ({ page }) => {
  test.skip(SOAK_MIN <= 0, 'opt-in: FAULK_E2E_SOAK_MIN=<minutes>')
  test.setTimeout((SOAK_MIN + 5) * 60_000)
  const { errors } = watch(page)
  await start(page)
  await addTracks(page, 7)
  for (const arm of await page.getByTestId('mixer-arm').all()) await arm.click()
  await send(page, [0x90, 69, 127])
  await expect.poll(async () => (await peaks(page)).filter((p) => p > 0.01).length, { timeout: 15000 }).toBe(8)
  const cdp = await page.context().newCDPSession(page)
  await cdp.send('Performance.enable')
  await cdp.send('HeapProfiler.enable')
  await page.waitForTimeout(10_000)

  const samples: Counters[] = []
  const started = Date.now()
  const reports = async () => Number(await page.getByTestId('feedback').getAttribute('data-seq'))
  const seq0 = await reports()
  while (Date.now() - started < SOAK_MIN * 60_000) {
    samples.push(await counters(cdp, true))
    const s = samples[samples.length - 1]
    console.log(
      `soak t=${Math.round((Date.now() - started) / 1000)}s heap ${(s.JSHeapUsedSize / 1e6).toFixed(2)} MB, nodes ${s.Nodes}, listeners ${s.JSEventListeners}`,
    )
    await page.waitForTimeout(30_000)
  }
  samples.push(await counters(cdp, true))
  const seq1 = await reports()
  // The report's sequence number moves by 2 for each report (odd while it is written).
  const count = (seq1 - seq0) / 2
  const first = samples[1]
  const last = samples[samples.length - 1]
  console.log(
    `soak: ${SOAK_MIN} min, ${count} reports (${(count / (SOAK_MIN * 60)).toFixed(1)} per second); heap ${(first.JSHeapUsedSize / 1e6).toFixed(2)} -> ${(last.JSHeapUsedSize / 1e6).toFixed(2)} MB, nodes ${first.Nodes} -> ${last.Nodes}, listeners ${first.JSEventListeners} -> ${last.JSEventListeners}`,
  )
  expect(count / (SOAK_MIN * 60)).toBeGreaterThan(20)
  expect(last.Nodes - first.Nodes).toBeLessThanOrEqual(20)
  expect(last.JSEventListeners - first.JSEventListeners).toBeLessThanOrEqual(20)
  expect(last.JSHeapUsedSize - first.JSHeapUsedSize).toBeLessThan(2_000_000)
  await send(page, [0x80, 69, 0])
  expect(errors).toEqual([])
})
