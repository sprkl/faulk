import { expect, test, type Locator, type Page } from '@playwright/test'
import { fakeWebMidi, watch } from './support'

// #300: the continuous controls in a real browser. One shared behaviour (`Slider`): the magnet on
// the default value, the fine mode (Ctrl), the reset by double click, by the context menu and by
// the keyboard, each as one history entry. Real worklet, engine and project core.

test.describe.configure({ mode: 'serial' })

async function start(page: Page) {
  await fakeWebMidi(page)
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await expect(page.getByTestId('engine-mode')).toContainText('Web')
  await page.getByRole('radio', { name: 'E2E Keys' }).check()
  await expect(page.getByTestId('track-list')).toHaveAttribute('data-track-count', '1')
}

const readout = (page: Page) => page.getByTestId('mixer-gain').first()
const text = async (locator: Locator) => (await locator.textContent())?.trim() ?? ''

/** Drags a vertical fader up (positive) or down (negative) by whole pixels, one pixel per move, reading `gain` after each. */
async function dragFader(page: Page, fader: Locator, pixels: number, readings: string[] = []) {
  const box = (await fader.boundingBox())!
  const x = box.x + box.width / 2
  const y = box.y + box.height / 2
  await page.mouse.move(x, y)
  await page.mouse.down()
  // The first pixels only pass the slop (a click moves nothing).
  await page.mouse.move(x, y - Math.sign(pixels) * 4)
  const sign = Math.sign(pixels)
  for (let px = 1; px <= Math.abs(pixels); px++) {
    await page.mouse.move(x, y - sign * (4 + px))
    readings.push(await text(readout(page)))
  }
  await page.mouse.up()
  return readings
}

test('the mixer fader: the magnet holds 0 dB, Ctrl passes it, reset is one undo step', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await page.getByTestId('nav-mixer').click()
  const fader = page.getByTestId('mixer-strip').first().getByTestId('mixer-fader')
  await expect(fader).toHaveAttribute('role', 'slider')
  await expect(fader).toHaveAttribute('aria-valuetext', '0.0 dB')

  // One step up from 0 dB leaves the default: a little above it.
  await fader.focus()
  await page.keyboard.press('ArrowUp')
  const above = await text(readout(page))
  expect(Number(above.replace(' dB', ''))).toBeGreaterThan(0)

  // Dragging down through 0 dB: the value sticks to 0.0 dB for ten pixels, then goes below.
  const plain = await dragFader(page, fader, -40)
  expect(plain.filter((r) => r === '0.0 dB').length).toBeGreaterThanOrEqual(9)
  expect(plain.at(-1)!.startsWith('−')).toBe(true)

  // The same sweep with Ctrl: ten times finer, no magnet. It crosses 0 dB without stopping on it.
  await fader.focus()
  await page.keyboard.press('Home')
  await page.keyboard.press('Delete')
  await expect(fader).toHaveAttribute('aria-valuetext', '0.0 dB')
  await page.keyboard.press('ArrowUp')
  await page.keyboard.down('Control')
  const fine = await dragFader(page, fader, -40)
  await page.keyboard.up('Control')
  expect(fine.filter((r) => r === '0.0 dB').length).toBeLessThanOrEqual(6)
  expect(Number(fine[0]!.replace('−', '-').replace(' dB', ''))).toBeGreaterThan(0)
  expect(fine.at(-1)!.startsWith('−')).toBe(true)
  // Ctrl + arrows is a fine step, too.
  await fader.focus()
  await page.keyboard.press('Delete')
  await page.keyboard.press('Control+ArrowUp')
  expect(Number((await text(readout(page))).replace(' dB', ''))).toBeLessThan(0.2)
  expect(await text(readout(page))).not.toBe('0.0 dB')
  await page.keyboard.press('Delete')

  // -6 dB with seven steps. A double click puts 0 dB back, and one undo takes that alone back.
  for (let i = 0; i < 7; i++) await page.keyboard.press('ArrowDown')
  await expect(fader).toHaveAttribute('aria-valuetext', '−6.0 dB')
  await fader.dblclick()
  await expect(fader).toHaveAttribute('aria-valuetext', '0.0 dB')
  await page.keyboard.press('Control+z')
  await expect(fader).toHaveAttribute('aria-valuetext', '−6.0 dB')

  // The context menu shows the default value and does the same.
  await fader.click({ button: 'right' })
  const item = page.getByTestId('value-menu-default')
  await expect(item).toContainText('Default value')
  await expect(item).toContainText('0.0 dB')
  await item.click()
  await expect(page.getByTestId('value-menu')).toHaveCount(0)
  await expect(fader).toHaveAttribute('aria-valuetext', '0.0 dB')
  await page.keyboard.press('Control+z')
  await expect(fader).toHaveAttribute('aria-valuetext', '−6.0 dB')

  // Escape closes the menu and changes nothing.
  await fader.click({ button: 'right' })
  await expect(page.getByTestId('value-menu')).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('value-menu')).toHaveCount(0)
  await expect(fader).toHaveAttribute('aria-valuetext', '−6.0 dB')

  // Delete puts it back too.
  await fader.focus()
  await page.keyboard.press('Delete')
  await expect(fader).toHaveAttribute('aria-valuetext', '0.0 dB')
  await page.keyboard.press('Control+z')
  await expect(fader).toHaveAttribute('aria-valuetext', '−6.0 dB')

  // The mixer's pan: centre by double click, one undo.
  const pan = page.getByTestId('mixer-pan').first()
  await expect(pan).toHaveAttribute('role', 'slider')
  await pan.focus()
  for (let i = 0; i < 5; i++) await page.keyboard.press('ArrowLeft')
  await expect(pan).toHaveAttribute('aria-valuetext', 'L20')
  await pan.dblclick()
  await expect(pan).toHaveAttribute('aria-valuetext', 'C')
  await page.keyboard.press('Control+z')
  await expect(pan).toHaveAttribute('aria-valuetext', 'L20')

  expect(errors).toEqual([])
})

test('the track list: volume on the fader curve, reset by double click and menu, pan reset', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  const header = page.getByTestId('track-inspector')
  const volume = header.getByRole('slider', { name: /^Volume of/ })
  const pan = header.getByRole('slider', { name: /^Pan of/ })
  await expect(volume).toHaveAttribute('aria-valuetext', '0.0 dB')
  // The same curve as the mixer: seven steps down from 0 dB are -6 dB.
  await volume.focus()
  for (let i = 0; i < 7; i++) await page.keyboard.press('ArrowDown')
  await expect(volume).toHaveAttribute('aria-valuetext', '−6.0 dB')
  await volume.dblclick()
  await expect(volume).toHaveAttribute('aria-valuetext', '0.0 dB')
  await page.keyboard.press('Control+z')
  await expect(volume).toHaveAttribute('aria-valuetext', '−6.0 dB')
  // A drag to the right through 0 dB sticks to it.
  const box = (await volume.boundingBox())!
  const y = box.y + box.height / 2
  await page.mouse.move(box.x + 20, y)
  await page.mouse.down()
  await page.mouse.move(box.x + 24, y)
  const seen: string[] = []
  for (let px = 1; px <= 40; px++) {
    await page.mouse.move(box.x + 24 + px, y)
    seen.push((await volume.getAttribute('aria-valuetext')) ?? '')
  }
  await page.mouse.up()
  expect(seen.filter((v) => v === '0.0 dB').length).toBeGreaterThanOrEqual(3)

  await pan.focus()
  await page.keyboard.press('ArrowRight')
  await expect(pan).not.toHaveAttribute('aria-valuetext', 'C')
  await pan.click({ button: 'right' })
  await page.getByTestId('value-menu-default').click()
  await expect(pan).toHaveAttribute('aria-valuetext', 'C')
  await page.keyboard.press('Control+z')
  await expect(pan).not.toHaveAttribute('aria-valuetext', 'C')
  expect(errors).toEqual([])
})

test('a device knob: reset by double click, by the menu and by Delete, each one undo step', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  const attack = page.getByTestId('param-attack').getByRole('slider')
  await expect(attack).toBeVisible()
  const factory = (await attack.getAttribute('aria-valuetext'))!
  await attack.focus()
  await page.keyboard.press('End')
  const end = (await attack.getAttribute('aria-valuetext'))!
  expect(end).not.toBe(factory)

  await attack.dblclick()
  await expect(attack).toHaveAttribute('aria-valuetext', factory)
  await page.keyboard.press('Control+z')
  await expect(attack).toHaveAttribute('aria-valuetext', end)

  await attack.click({ button: 'right' })
  await expect(page.getByTestId('value-menu-default')).toContainText(factory)
  await page.getByTestId('value-menu-default').click()
  await expect(attack).toHaveAttribute('aria-valuetext', factory)
  await page.keyboard.press('Control+z')
  await expect(attack).toHaveAttribute('aria-valuetext', end)

  await attack.focus()
  await page.keyboard.press('Backspace')
  await expect(attack).toHaveAttribute('aria-valuetext', factory)
  await page.keyboard.press('Control+z')
  await expect(attack).toHaveAttribute('aria-valuetext', end)

  // #415 option A: non-musical key (Delete/arrows) on a mouse-focused slider switches it to keyboard
  // origin. Consequence: letters no longer play a note on that slider until the next click restores
  // pointer origin. Explicit test of the behavior change (not hidden by replacing focus() with click()).
  //
  // Step 1 — mouse-focused slider: a held letter plays, and keeps a voice alive while held.
  // This also measures, on this shard, how long the engine takes to show the voice: we time it
  // and reuse that measured bound in step 2 instead of guessing a latency (CLAUDE.md: never state
  // an unmeasured figure).
  await attack.click() // explicit pointer press: pointer origin
  const t0 = Date.now()
  await page.keyboard.down('KeyA')
  await expect(page.getByTestId('voices')).toContainText(/\b1\b/)
  const startMs = Date.now() - t0 // measured time for a held KeyA to raise the voice count on this shard
  await page.keyboard.up('KeyA')
  // Wait for the voice to fully release before step 2, so that step 2's assertion
  // is only about whether KeyA starts a new voice, not a releasing one from step 1.
  await expect(page.getByTestId('voices')).not.toContainText(/\b1\b/)
  //
  // Step 2 — non-musical key → keyboard origin → a held letter no longer plays.
  // KeyA is held down for the whole window: if playsNote had regressed to true, the voice would be
  // present for the entire hold (a held key sustains its voice), so at least one sample would catch
  // it. The window is a generous multiple (>=10x, floored at 1 s) of the start time measured in
  // step 1 on this same shard, so a slow shard widens it rather than racing it.
  await page.keyboard.press('Delete') // non-musical key on already-pointer-focused slider
  await page.keyboard.down('KeyA')
  const windowMs = Math.max(1000, startMs * 10)
  const step = Math.max(100, Math.round(windowMs / 8))
  const samples: string[] = []
  const deadline = Date.now() + windowMs
  while (Date.now() < deadline) {
    samples.push((await page.getByTestId('voices').textContent()) ?? '')
    await page.waitForTimeout(step)
  }
  expect(samples.length).toBeGreaterThanOrEqual(5) // enough samples to span the hold
  expect(samples.some((s) => /\b1\b/.test(s))).toBe(false) // no voice ever appeared: playsNote stayed false
  await page.keyboard.up('KeyA')
  // Space must not toggle the transport either (keyboard-focused slider keeps Space).
  await page.keyboard.press('Space')
  await expect(page.getByRole('button', { name: 'Play', exact: true })).toHaveAttribute('aria-pressed', 'false')
  //
  // Step 3 — click restores pointer origin: letter plays again.
  await attack.click()
  await page.keyboard.down('KeyA')
  await expect(page.getByTestId('voices')).toContainText(/\b1\b/)
  await page.keyboard.up('KeyA')

  // Regression #415: context-menu click then key — Space must not toggle the transport.
  // The real path: right-click on attack (already focused) fires pointerdown → the 'down' handler sets
  // pointerFocused = attack (the already-active element). The menu item click then closes the menu;
  // closeMenu() calls root.focus() with lastInput === 'pointer', firing a focusin that confirms
  // pointerFocused = attack. A subsequent attack.focus() is a no-op (no focusin). Without the fix,
  // pointerFocused stays stale and Space toggles the transport; the fix clears it on the first
  // non-musical UI key on the focused slider.
  await attack.click()
  await page.keyboard.press('End')
  await attack.click({ button: 'right' })
  await page.getByTestId('value-menu-default').click()
  // After closeMenu, attack is focused with pointer origin. attack.focus() is a no-op.
  await attack.focus()
  await page.keyboard.press('Backspace') // first non-musical key: clears stale pointerFocused
  await attack.focus()
  await page.keyboard.press('Space') // Space must not toggle the transport (#415)
  await expect(page.getByRole('button', { name: 'Play', exact: true })).toHaveAttribute('aria-pressed', 'false')
  expect(errors).toEqual([])
})
