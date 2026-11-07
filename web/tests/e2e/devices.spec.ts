import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// DV-9 (#70): the device panel in a real browser. Real worklet, engine and project core: every
// action goes through the dispatcher as a DV-8 command. Echo (DV-3) is the first effect: the
// browser lists it with no change to the panel, and it is inserted, bypassed, removed and undone here.

const TEXTS = {
  en: {
    bypass: (n: string) => `Bypass ${n}`,
    use: (n: string) => `Use ${n} as the instrument`,
    add: (n: string) => `Add ${n} to the track`,
    remove: (n: string) => `Remove ${n}`,
    hint: 'Output level of the synth, from 0 to 1 (linear gain).',
    bare: (u: string) => `A bare number is read in ${u}.`,
  },
  fr: {
    bypass: (n: string) => `Contourner ${n}`,
    use: (n: string) => `Utiliser ${n} comme instrument`,
    add: (n: string) => `Ajouter ${n} à la piste`,
    remove: (n: string) => `Retirer ${n}`,
    hint: 'Niveau de sortie du synthé, de 0 à 1 (gain linéaire).',
    bare: (u: string) => `Un nombre seul = ${u}.`,
  },
} as const

const card = (page: Page, name: string) => page.getByTestId('device-card').filter({ has: page.getByRole('heading', { name, exact: true }) })

async function addTrack(page: Page, item: string) {
  await page.getByTestId('add-track').click()
  await page.getByRole('menuitem', { name: item, exact: true }).click()
}

for (const lang of ['en', 'fr'] as const) {
  test(`devices, ${lang}: the panel is generated for Oscillator and Drums, switches instrument, bypasses and undoes`, async ({ page }) => {
    const x = TEXTS[lang]
    const { errors } = watch(page)
    await page.setViewportSize({ width: 1280, height: 900 })
    await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
    await page.goto('/app')
    await expect(page.getByTestId('devices-empty')).toBeVisible()
    await page.getByTestId('engine-state').getByRole('button').click()
    await engineUp(page)

    // The Oscillator track: one card, a control per parameter of its descriptor.
    const panel = page.getByTestId('bottom-panel')
    await expect(page.getByTestId('device-card')).toHaveCount(1)
    const osc = card(page, 'Oscillator')
    await expect(osc.locator('[data-testid^=param-]')).toHaveCount(6)
    await expect(osc.getByTestId('device-remove')).toHaveCount(0)
    await expect(osc.getByTestId('device-left')).toHaveCount(0)
    await expect(panel.getByTestId('browse-no-effects')).toHaveCount(0)
    // The tooltip of a parameter is its description, in the page's language.
    await expect(osc.getByTestId('param-gain').locator('.knob').first()).toHaveAttribute('title', `${x.hint} ${x.bare('dB')}`)

    // A choice and a knob reach the project.
    await osc.getByRole('radio', { name: lang === 'en' ? 'Square' : 'Carré' }).click()
    await expect(osc.getByRole('radio', { name: lang === 'en' ? 'Square' : 'Carré' })).toHaveAttribute('aria-checked', 'true')
    const attack = osc.getByTestId('param-attack').getByRole('slider')
    const before = await attack.getAttribute('aria-valuetext')

    // A drag of a knob is one history entry: one undo takes it back whole.
    const box = (await attack.boundingBox())!
    await page.mouse.move(box.x + box.width * 0.1, box.y + box.height / 2)
    await page.mouse.down()
    await page.mouse.move(box.x + box.width * 0.5, box.y + box.height / 2, { steps: 6 })
    await page.mouse.move(box.x + box.width * 0.9, box.y + box.height / 2, { steps: 6 })
    await page.mouse.up()
    await expect.poll(() => attack.getAttribute('aria-valuetext')).not.toBe(before)
    await page.keyboard.press('Control+z')
    await expect.poll(() => attack.getAttribute('aria-valuetext')).toBe(before)
    await expect(osc.getByRole('radio', { name: lang === 'en' ? 'Square' : 'Carré' })).toHaveAttribute('aria-checked', 'true')

    // Bypass the instrument, then undo.
    const bypass = osc.getByRole('button', { name: x.bypass('Oscillator'), exact: true })
    await bypass.click()
    await expect(bypass).toHaveAttribute('aria-pressed', 'true')
    await expect(osc).toHaveAttribute('data-bypassed', 'true')
    await page.keyboard.press('Control+z')
    await expect(bypass).toHaveAttribute('aria-pressed', 'false')

    // Echo, the first effect: inserted from the browser, one control per parameter, bypassed,
    // removed and brought back by undo.
    await panel.getByRole('button', { name: x.add('Echo'), exact: true }).click()
    await expect(page.getByTestId('device-card')).toHaveCount(2)
    const echo = card(page, 'Echo')
    // Synced, its default: the note value shows, the free time does not (#472), so five of the six parameters.
    await expect(echo.locator('[data-testid^=param-]')).toHaveCount(5)
    // Its controls: the mode and the note value are choices, the feedback is shown as a percentage.
    const quarter = echo.getByRole('radio', { name: '1/4', exact: true })
    await quarter.click()
    await expect(quarter).toHaveAttribute('aria-checked', 'true')
    const free = echo.getByRole('radio', { name: lang === 'en' ? 'Free' : 'Libre', exact: true })
    await free.click()
    await expect(free).toHaveAttribute('aria-checked', 'true')
    await expect(echo.getByTestId('param-time')).toBeVisible()
    await expect(echo.locator('[data-testid^=param-]')).toHaveCount(5)
    // The note value is back with the synced mode, and kept what was chosen.
    await echo.getByRole('radio', { name: lang === 'en' ? 'Synced' : 'Au tempo', exact: true }).click()
    await expect(quarter).toHaveAttribute('aria-checked', 'true')
    await expect(echo.getByTestId('param-feedback').getByRole('slider')).toHaveAttribute('aria-valuetext', /%/)
    await expect(echo.getByTestId('param-division').getByRole('radio')).toHaveCount(10)
    // What a new time does while the echo rings: Glide is the default, Fade is a click away.
    const glide = echo.getByRole('radio', { name: lang === 'en' ? 'Glide' : 'Glissé', exact: true })
    await expect(glide).toHaveAttribute('aria-checked', 'true')
    const fade = echo.getByRole('radio', { name: lang === 'en' ? 'Fade' : 'Fondu', exact: true })
    await fade.click()
    await expect(fade).toHaveAttribute('aria-checked', 'true')
    await expect(echo.getByTestId('param-time_change').getByRole('radio')).toHaveCount(3)
    const echoBypass = echo.getByRole('button', { name: x.bypass('Echo'), exact: true })
    await echoBypass.click()
    await expect(echo).toHaveAttribute('data-bypassed', 'true')
    await page.keyboard.press('Control+z')
    await expect(echo).toHaveAttribute('data-bypassed', 'false')
    await echo.getByRole('button', { name: x.remove('Echo'), exact: true }).click()
    await expect(page.getByTestId('device-card')).toHaveCount(1)
    await page.keyboard.press('Control+z')
    await expect(page.getByTestId('device-card')).toHaveCount(2)
    // Back through the four control changes, then the insertion.
    for (let i = 0; i < 5; i++) await page.keyboard.press('Control+z')
    await expect(page.getByTestId('device-card')).toHaveCount(1)

    // Reverb (DV-4): inserted from the browser, one control per parameter, a knob moves, removed,
    // then undone back to the insertion and its removal.
    await panel.getByRole('button', { name: x.add('Reverb'), exact: true }).click()
    const reverb = card(page, 'Reverb')
    await expect(reverb.locator('[data-testid^=param-]')).toHaveCount(5)
    const size = reverb.getByTestId('param-size').getByRole('slider')
    const sizeBefore = await size.getAttribute('aria-valuetext')
    await size.focus()
    await page.keyboard.press('ArrowRight')
    await expect.poll(() => size.getAttribute('aria-valuetext')).not.toBe(sizeBefore)
    await expect(reverb.getByTestId('param-mix').getByRole('slider')).toHaveAttribute('aria-valuetext', /%/)
    // The pre-delay follows the same time-change setting as the Echo: Glide by default.
    const reverbGlide = reverb.getByRole('radio', { name: lang === 'en' ? 'Glide' : 'Glissé', exact: true })
    await expect(reverbGlide).toHaveAttribute('aria-checked', 'true')
    await expect(reverb.getByTestId('param-time_change').getByRole('radio')).toHaveCount(3)
    // Choosing Fade is a control change like any other: it is checked, then undone below.
    const reverbFade = reverb.getByRole('radio', { name: lang === 'en' ? 'Fade' : 'Fondu', exact: true })
    await reverbFade.click()
    await expect(reverbFade).toHaveAttribute('aria-checked', 'true')
    await expect(reverbGlide).toHaveAttribute('aria-checked', 'false')
    await reverb.getByRole('button', { name: x.remove('Reverb'), exact: true }).click()
    await expect(page.getByTestId('device-card')).toHaveCount(1)
    await page.keyboard.press('Control+z')
    await expect(reverb).toHaveCount(1)
    await page.keyboard.press('Control+z')
    await expect(reverbGlide).toHaveAttribute('aria-checked', 'true')
    await page.keyboard.press('Control+z')
    await expect(size).toHaveAttribute('aria-valuetext', sizeBefore!)
    await page.keyboard.press('Control+z')
    await expect(page.getByTestId('device-card')).toHaveCount(1)
    await expect(reverb).toHaveCount(0)

    // Filter (DV-2, #686): inserted from the browser, one control per parameter shown in its LFO mode (synced:
    // fourteen of sixteen), the mode is a choice of
    // four, the cutoff shows hertz and the resonance a percentage, removed, then undone.
    await panel.getByRole('button', { name: x.add('Filter'), exact: true }).click()
    const filter = card(page, 'Filter')
    await expect(filter.locator('[data-testid^=param-]')).toHaveCount(14)
    await expect(filter.getByTestId('param-mode').getByRole('radio')).toHaveCount(4)
    const lowpass = filter.getByRole('radio', { name: lang === 'en' ? 'Low-pass' : 'Passe-bas', exact: true })
    await expect(lowpass).toHaveAttribute('aria-checked', 'true')
    const bandpass = filter.getByRole('radio', { name: lang === 'en' ? 'Band-pass' : 'Passe-bande', exact: true })
    await bandpass.click()
    await expect(bandpass).toHaveAttribute('aria-checked', 'true')
    const cutoff = filter.getByTestId('param-cutoff').getByRole('slider')
    await expect(cutoff).toHaveAttribute('aria-valuetext', /kHz|Hz/)
    const cutoffBefore = await cutoff.getAttribute('aria-valuetext')
    await cutoff.press('End')
    await expect.poll(() => cutoff.getAttribute('aria-valuetext')).not.toBe(cutoffBefore)
    await expect(filter.getByTestId('param-resonance').getByRole('slider')).toHaveAttribute('aria-valuetext', /%/)
    await filter.getByRole('button', { name: x.remove('Filter'), exact: true }).click()
    await expect(filter).toHaveCount(0)
    await page.keyboard.press('Control+z')
    await expect(filter).toHaveCount(1)
    for (let i = 0; i < 3; i++) await page.keyboard.press('Control+z')
    await expect(page.getByTestId('device-card')).toHaveCount(1)
    await expect(filter).toHaveCount(0)

    // Compressor (#157): inserted from the browser, its reduction meter, one control per parameter with the
    // release time shown only in manual release and the sidechain frequency only with the sidechain high-pass
    // on, its switches, removed, then undone.
    await panel.getByRole('button', { name: x.add('Compressor'), exact: true }).click()
    const comp = card(page, 'Compressor')
    await expect(comp.getByTestId('gain-reduction')).toBeVisible()
    // Manual release and no high-pass, the defaults: eleven of the twelve parameters.
    await expect(comp.locator('[data-testid^=param-]')).toHaveCount(11)
    await expect(comp.getByTestId('param-sidechain_freq')).toHaveCount(0)
    await expect(comp.getByTestId('param-release')).toBeVisible()
    await expect(comp.getByTestId('param-knee').getByRole('slider')).toHaveAttribute('aria-valuetext', /dB/)
    await expect(comp.getByTestId('param-lookahead').getByRole('slider')).toHaveAttribute('aria-valuetext', /ms/)
    await expect(comp.getByTestId('param-mix').getByRole('slider')).toHaveAttribute('aria-valuetext', /100\s?%/)
    const hpfOn = comp.getByTestId('param-sidechain_hpf').getByRole('radio', { name: lang === 'en' ? 'On' : 'Oui', exact: true })
    await hpfOn.click()
    await expect(hpfOn).toHaveAttribute('aria-checked', 'true')
    await expect(comp.getByTestId('param-sidechain_freq').getByRole('slider')).toHaveAttribute('aria-valuetext', /Hz/)
    await expect(comp.locator('[data-testid^=param-]')).toHaveCount(12)
    const rms = comp.getByTestId('param-detection').getByRole('radio', { name: 'RMS', exact: true })
    await rms.click()
    await expect(rms).toHaveAttribute('aria-checked', 'true')
    await comp.getByTestId('param-release_mode').getByRole('radio', { name: 'Auto', exact: true }).click()
    await expect(comp.getByTestId('param-release')).toHaveCount(0)
    await expect(comp.locator('[data-testid^=param-]')).toHaveCount(11)
    const mix = comp.getByTestId('param-mix').getByRole('slider')
    await mix.press('Home')
    await expect(mix).toHaveAttribute('aria-valuetext', /^0\s?%/)
    await comp.getByRole('button', { name: x.remove('Compressor'), exact: true }).click()
    await expect(comp).toHaveCount(0)
    await page.keyboard.press('Control+z')
    await expect(comp).toHaveCount(1)
    await expect(comp.getByTestId('param-sidechain_freq')).toBeVisible()
    // The mix, the release mode, the detection, the high-pass, then the insertion.
    for (let i = 0; i < 5; i++) await page.keyboard.press('Control+z')
    await expect(page.getByTestId('device-card')).toHaveCount(1)
    await expect(comp).toHaveCount(0)

    // Ceiling (#159): inserted from the browser, its reduction meter, one control per parameter
    // with the release time shown only in manual release, its switches, removed, then undone.
    await panel.getByRole('button', { name: x.add('Ceiling'), exact: true }).click()
    const ceiling = card(page, 'Ceiling')
    await expect(ceiling.getByTestId('gain-reduction')).toBeVisible()
    // Auto release, its default: six of the seven parameters.
    await expect(ceiling.locator('[data-testid^=param-]')).toHaveCount(6)
    await expect(ceiling.getByTestId('param-release')).toHaveCount(0)
    await expect(ceiling.getByTestId('param-ceiling').getByRole('slider')).toHaveAttribute('aria-valuetext', /dB/)
    await expect(ceiling.getByTestId('param-gain').getByRole('slider')).toHaveAttribute('aria-valuetext', /dB/)
    await expect(ceiling.getByTestId('param-lookahead').getByRole('slider')).toHaveAttribute('aria-valuetext', /ms/)
    await ceiling.getByRole('radio', { name: lang === 'en' ? 'Manual' : 'Manuel', exact: true }).click()
    await expect(ceiling.getByTestId('param-release')).toBeVisible()
    await expect(ceiling.locator('[data-testid^=param-]')).toHaveCount(7)
    const truePeak = ceiling.getByTestId('param-true_peak').getByRole('radio', { name: lang === 'en' ? 'On' : 'Oui', exact: true })
    await truePeak.click()
    await expect(truePeak).toHaveAttribute('aria-checked', 'true')
    const gain = ceiling.getByTestId('param-gain').getByRole('slider')
    await gain.press('End')
    await expect(gain).toHaveAttribute('aria-valuetext', /24/)
    await ceiling.getByRole('button', { name: x.remove('Ceiling'), exact: true }).click()
    await expect(ceiling).toHaveCount(0)
    await page.keyboard.press('Control+z')
    await expect(ceiling).toHaveCount(1)
    await expect(ceiling.getByTestId('param-release')).toBeVisible()
    for (let i = 0; i < 4; i++) await page.keyboard.press('Control+z')
    await expect(page.getByTestId('device-card')).toHaveCount(1)
    await expect(ceiling).toHaveCount(0)

    // Stack (#690), the first MIDI effect: listed apart, inserted before the instrument, its card
    // generated from the descriptor (a voice's settings show while it is on), removed and undone.
    await panel.getByTestId('browse-midi-effect').filter({ hasText: 'Stack' }).click()
    await expect(page.getByTestId('device-card')).toHaveCount(2)
    await expect(page.getByTestId('device-card').first()).toHaveAttribute('data-kind', 'faulk.stack')
    await expect(page.getByTestId('device-card').first()).toHaveAttribute('data-role', 'midi_effect')
    const stack = card(page, 'Stack')
    // Its default adds the fifth: voice 1 shows its three settings, voices 2 to 6 only their switch.
    await expect(stack.locator('[data-testid^=param-]')).toHaveCount(15)
    await expect(stack.getByTestId('param-strum')).toContainText(lang === 'en' ? 'Strum' : 'Égrenage')
    await expect(stack.getByTestId('param-voice3_interval')).toHaveCount(0)
    await stack
      .getByTestId('param-voice3')
      .getByRole('radio', { name: lang === 'en' ? 'On' : 'Marche', exact: true })
      .click()
    await expect(stack.getByTestId('param-voice3_interval')).toBeVisible()
    await expect(stack.locator('[data-testid^=param-]')).toHaveCount(18)
    // Alone among the MIDI effects: it moves neither way, and never past the instrument.
    await expect(stack.getByTestId('device-left')).toBeDisabled()
    await expect(stack.getByTestId('device-right')).toBeDisabled()
    await stack.getByRole('button', { name: x.remove('Stack'), exact: true }).click()
    await expect(stack).toHaveCount(0)
    await page.keyboard.press('Control+z')
    await expect(stack).toHaveCount(1)
    await page.keyboard.press('Control+z')
    await expect(stack.getByTestId('param-voice3_interval')).toHaveCount(0)
    await page.keyboard.press('Control+z')
    await expect(stack).toHaveCount(0)

    // Switch the instrument from the browser: the panel is Drums' own, generated the same way, then back.
    await panel.getByRole('button', { name: x.use('Drums'), exact: true }).click()
    await expect(page.getByTestId('device-card')).toHaveCount(1)
    const drums = card(page, 'Drums')
    await expect(drums.locator('[data-testid^=param-]')).toHaveCount(16)
    await expect(panel.getByRole('button', { name: x.use('Drums'), exact: true })).toHaveAttribute('aria-pressed', 'true')
    await page.keyboard.press('Control+z')
    await expect(card(page, 'Oscillator')).toHaveCount(1)

    // A Drums track of its own: its panel on selection, a knob changes.
    await addTrack(page, 'Drums')
    const kit = card(page, 'Drums')
    await expect(kit.locator('[data-testid^=param-]')).toHaveCount(16)
    const tune = kit.getByTestId('param-kick_tune').getByRole('slider')
    const tuneBefore = await tune.getAttribute('aria-valuetext')
    await tune.press('End')
    await expect.poll(() => tune.getAttribute('aria-valuetext')).not.toBe(tuneBefore)

    // Nothing sticks out sideways.
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)
    expect(overflow).toBe(false)
    expect(errors).toEqual([])
  })
}
