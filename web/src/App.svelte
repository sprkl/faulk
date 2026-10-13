<script lang="ts">
  import { onMount } from 'svelte'
  import Knob from './lib/components/Knob.svelte'
  import Meter from './lib/components/Meter.svelte'
  import PianoStrip from './lib/components/PianoStrip.svelte'
  import type { MessageKey } from './lib/i18n'
  import { clampOctave, DEFAULT_OCTAVE, ignoreKeyEvent, noteForKey, octaveStep } from './lib/keyboard'
  import { formatValue, KNOB_PARAMS, positionOf, valueAt } from './lib/params'
  import { Session } from './lib/session.svelte'

  const session = new Session()
  const t = session.t

  let octave = $state(DEFAULT_OCTAVE)
  let code = $state('')
  /** Notes held on the computer keyboard, by physical key, so octave changes release the right note. */
  // Not reactive on purpose: nothing renders from it.
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const held = new Map<string, number>()

  const WAVEFORMS: { value: number; key: MessageKey; path: string }[] = [
    { value: 0, key: 'synth.sine', path: 'M2 10 C 6 0, 10 0, 12 10 S 18 20, 22 10' },
    { value: 1, key: 'synth.saw', path: 'M2 16 L 12 4 L 12 16 L 22 4' },
    { value: 2, key: 'synth.square', path: 'M2 16 L 2 4 L 12 4 L 12 16 L 22 16 L 22 4' },
    { value: 3, key: 'synth.triangle', path: 'M2 16 L 7 4 L 12 16 L 17 4 L 22 16' },
  ]

  function releaseAllKeys() {
    for (const note of held.values()) session.run({ type: 'note.off', note })
    held.clear()
  }

  function onKeyDown(event: KeyboardEvent) {
    if (!session.started || ignoreKeyEvent(event)) return
    const step = octaveStep(event.code)
    if (step !== 0) {
      releaseAllKeys()
      octave = clampOctave(octave + step)
      return
    }
    const note = noteForKey(event.code, octave)
    if (note === null || held.has(event.code)) return
    held.set(event.code, note)
    session.run({ type: 'note.on', note, velocity: 100 })
  }

  function onKeyUp(event: KeyboardEvent) {
    const note = held.get(event.code)
    if (note === undefined) return
    held.delete(event.code)
    session.run({ type: 'note.off', note })
  }

  onMount(() => {
    void session.checkHelper()
    const release = () => releaseAllKeys()
    const onVisibility = () => document.hidden && releaseAllKeys()
    window.addEventListener('blur', release)
    document.addEventListener('visibilitychange', onVisibility)
    return () => {
      window.removeEventListener('blur', release)
      document.removeEventListener('visibilitychange', onVisibility)
    }
  })

  $effect(() => {
    document.documentElement.lang = session.lang
  })

  const latency = $derived(
    session.status?.estimatedLatencyMs != null
      ? t('engine.latency', { ms: Math.round(session.status.estimatedLatencyMs) })
      : t('engine.latencyUnknown'),
  )
</script>

<svelte:window onkeydown={onKeyDown} onkeyup={onKeyUp} />

<header>
  <div class="brand">
    <h1>FAULK</h1>
    <span class="ref">FLK-0.1</span>
    <span class="tagline">{t('app.tagline')}</span>
  </div>
  <div class="status">
    {#if session.started}
      <span class="mode" class:boost={session.mode === 'native'} data-testid="engine-mode">
        {#if session.mode === 'native'}
          <svg width="14" height="14" viewBox="0 0 16 16" aria-hidden="true"><path d="M9 1 L3 9 H8 L7 15 L13 7 H8 Z" fill="currentColor" /></svg>
          {t('engine.boost')}
        {:else}
          <svg width="14" height="14" viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.4"><circle cx="8" cy="8" r="6.5" /><path d="M1.5 8 H14.5 M8 1.5 C 5 5, 5 11, 8 14.5 M8 1.5 C 11 5, 11 11, 8 14.5" /></svg>
          {t('engine.web')}
        {/if}
      </span>
      <span class="value" data-testid="latency">{latency}</span>
      <span class="value muted">{t('engine.voices', { n: session.voices })}</span>
    {/if}
    <button class="ghost" onclick={() => session.setLang(session.lang === 'fr' ? 'en' : 'fr')}>{t('lang.switch')}</button>
  </div>
</header>

{#if session.notice}
  <p class="notice" role="status">{t(session.notice)}</p>
{/if}

{#if !session.started}
  <main class="start">
    <section class="card">
      <h2>{t('start.title')}</h2>
      <p>{t('start.body')}</p>
      <button class="primary" disabled={session.starting} onclick={() => session.start()}>{t('start.button')}</button>
    </section>
  </main>
{:else}
  <main class="grid">
    <section class="panel helper">
      <h2 class="label">{t('helper.title')}</h2>
      {#if session.helper === 'checking'}
        <p class="muted">{t('helper.checking')}</p>
      {:else if session.helper === 'absent'}
        <p class="muted">{t('helper.absent')}</p>
        <button class="ghost" onclick={() => session.checkHelper()}>{t('helper.recheck')}</button>
      {:else if session.helper === 'incompatible'}
        <p class="muted">{t('helper.incompatible', { v: session.helperProtocol ?? '?' })}</p>
      {:else if session.mode === 'native'}
        <button class="ghost" onclick={() => session.enterWeb()}>{t('helper.useWeb')}</button>
      {:else if session.paired}
        <button class="primary" disabled={session.helper === 'connecting'} onclick={() => session.enterBoost()}>{t('helper.useBoost')}</button>
      {:else}
        <form
          class="pair"
          onsubmit={(e) => {
            e.preventDefault()
            void session.pair(code)
          }}
        >
          <label>
            <span class="muted">{t('helper.pairPrompt')}</span>
            <input class="value" bind:value={code} inputmode="numeric" autocomplete="off" placeholder="0000 0000" aria-label={t('helper.code')} />
          </label>
          <button class="primary" type="submit">{t('helper.pair')}</button>
        </form>
        {#if session.pairingMessage}
          <p class="muted" role="status">{t(session.pairingMessage.key, { n: session.pairingMessage.n ?? 0 })}</p>
        {/if}
      {/if}
    </section>

    <section class="panel midi">
      <h2 class="label">{t('midi.title')}</h2>
      {#if session.mode === 'web' && session.midiState === 'unsupported'}
        <p class="muted">{t('midi.unsupported')}</p>
      {:else if session.mode === 'web' && session.midiState === 'denied'}
        <p class="muted">{t('midi.denied')}</p>
      {:else if session.inputs.length === 0}
        <p class="muted" data-testid="midi-empty">{t('midi.empty')}</p>
      {:else}
        <ul class="inputs" data-testid="midi-inputs">
          <li>
            <label><input type="radio" name="midi" checked={session.selectedInput === null} onchange={() => session.run({ type: 'midi.select_input', id: null })} /> {t('midi.none')}</label>
          </li>
          {#each session.inputs as input (input.id)}
            <li>
              <label>
                <input type="radio" name="midi" checked={session.selectedInput === input.id} onchange={() => session.run({ type: 'midi.select_input', id: input.id })} />
                {input.name}
                {#if session.selectedInput === input.id && session.activeNotes.length > 0}<span class="led" aria-hidden="true"></span>{/if}
              </label>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <section class="panel synth">
      <h2 class="label">{t('synth.title')}</h2>
      <div class="row">
        <div role="radiogroup" aria-label={t('synth.waveform')} class="segmented">
          {#each WAVEFORMS as w (w.value)}
            <button
              role="radio"
              aria-checked={session.params.waveform === w.value}
              aria-label={t(w.key)}
              title={t(w.key)}
              onclick={() => session.run({ type: 'synth.set_param', param: 'waveform', value: w.value })}
            >
              <svg width="24" height="20" viewBox="0 0 24 20" aria-hidden="true"><path d={w.path} fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" /></svg>
            </button>
          {/each}
        </div>
      </div>
      <div class="synth-body">
      <div class="knobs">
        {#each KNOB_PARAMS as param (param)}
          <Knob
            label={t(`synth.${param}` as MessageKey)}
            position={positionOf(param, session.params[param])}
            display={formatValue(param, session.params[param])}
            onchange={(p) => session.run({ type: 'synth.set_param', param, value: valueAt(param, p) })}
          />
        {/each}
      </div>
      <div class="meter-box">
        <Meter peak={session.peak} label={t('meter.label')} />
      </div>
      </div>
    </section>

    <section class="panel keys">
      <div class="keys-head">
        <h2 class="label">{t('keys.title')}</h2>
        <span class="value">{t('keys.octave', { n: octave })}</span>
        <button class="ghost" onclick={() => session.run({ type: 'note.all_off' })}>{t('panic')}</button>
      </div>
      <PianoStrip low={(octave + 1) * 12} active={session.activeNotes} />
      <p class="muted">{t('keys.help')}</p>
    </section>
  </main>
{/if}

<style>
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: var(--fk-space-4);
    flex-wrap: wrap;
    padding: var(--fk-space-4) var(--fk-space-6);
    border-bottom: 1px solid var(--fk-line);
  }
  .brand {
    display: flex;
    align-items: baseline;
    gap: var(--fk-space-3);
  }
  h1 {
    margin: 0;
    font-family: var(--fk-font-display);
    font-stretch: 125%;
    font-weight: 800;
    font-size: 28px;
    letter-spacing: -0.02em;
  }
  .ref {
    font-family: var(--fk-font-mono);
    font-size: 12px;
    color: var(--fk-text-muted);
  }
  .tagline {
    color: var(--fk-text-muted);
  }
  .status {
    display: flex;
    align-items: center;
    gap: var(--fk-space-3);
  }
  .mode {
    display: inline-flex;
    align-items: center;
    gap: var(--fk-space-1);
    padding: 2px var(--fk-space-3);
    border-radius: var(--fk-radius-pill);
    background: var(--fk-surface-2);
    font-weight: 600;
  }
  .mode.boost {
    background: var(--fk-accent);
    color: #fff;
  }
  .muted {
    color: var(--fk-text-muted);
  }
  .notice {
    margin: var(--fk-space-4) var(--fk-space-6) 0;
    padding: var(--fk-space-3) var(--fk-space-4);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-surface);
  }
  .start {
    display: grid;
    place-items: center;
    padding: var(--fk-space-12) var(--fk-space-4);
  }
  .card {
    max-width: 420px;
    padding: var(--fk-space-8);
    border-radius: var(--fk-radius-card);
    background: var(--fk-surface);
    border: 1px solid var(--fk-line);
  }
  .card h2 {
    margin: 0 0 var(--fk-space-2);
    font-size: 15px;
  }
  .grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 2fr);
    grid-template-areas:
      'helper synth'
      'midi synth'
      'keys keys';
    gap: var(--fk-space-4);
    padding: var(--fk-space-6);
  }
  @media (max-width: 760px) {
    .grid {
      grid-template-columns: minmax(0, 1fr);
      grid-template-areas: 'helper' 'midi' 'synth' 'keys';
      padding: var(--fk-space-4);
    }
  }
  .panel {
    padding: var(--fk-space-4);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-surface);
    border: 1px solid var(--fk-line);
    display: flex;
    flex-direction: column;
    gap: var(--fk-space-3);
  }
  .panel h2 {
    margin: 0;
  }
  .helper {
    grid-area: helper;
  }
  .midi {
    grid-area: midi;
  }
  .synth {
    grid-area: synth;
  }
  .keys {
    grid-area: keys;
  }
  .synth-body {
    display: flex;
    gap: var(--fk-space-6);
    align-items: flex-end;
  }
  .keys-head {
    display: flex;
    align-items: center;
    gap: var(--fk-space-3);
  }
  .keys-head button {
    margin-left: auto;
  }
  p {
    margin: 0;
  }
  button {
    min-height: var(--fk-control);
    padding: 0 var(--fk-space-4);
    border-radius: var(--fk-radius-pill);
    border: 1px solid var(--fk-line);
    background: transparent;
    cursor: pointer;
    transition: background var(--fk-dur-fast) ease-out;
  }
  button:hover {
    background: var(--fk-surface-2);
  }
  button.primary {
    background: var(--fk-accent);
    border-color: var(--fk-accent);
    color: #fff;
    font-weight: 600;
  }
  button.primary:hover {
    background: var(--fk-accent-strong);
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .pair {
    display: flex;
    gap: var(--fk-space-2);
    align-items: flex-end;
    flex-wrap: wrap;
  }
  .pair label {
    display: flex;
    flex-direction: column;
    gap: var(--fk-space-1);
  }
  .pair input {
    height: var(--fk-control);
    width: 140px;
    padding: 0 var(--fk-space-3);
    border-radius: var(--fk-radius-panel);
    border: 1px solid var(--fk-line);
    background: var(--fk-bg);
    font-size: 15px;
    letter-spacing: 0.1em;
  }
  .inputs {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--fk-space-1);
  }
  .inputs label {
    display: flex;
    align-items: center;
    gap: var(--fk-space-2);
    min-height: var(--fk-row);
  }
  .led {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--fk-play);
  }
  .segmented {
    display: inline-flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--fk-radius-pill);
    background: var(--fk-bg);
    border: 1px solid var(--fk-line);
  }
  .segmented button {
    border: 0;
    padding: 0 var(--fk-space-3);
    color: var(--fk-text-muted);
  }
  .segmented button[aria-checked='true'] {
    background: var(--fk-accent-tint);
    color: var(--fk-accent-fg);
  }
  .knobs {
    display: flex;
    flex-wrap: wrap;
    gap: var(--fk-space-4);
    align-items: flex-end;
  }
  .meter-box {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--fk-space-1);
    margin-left: auto;
  }
</style>
