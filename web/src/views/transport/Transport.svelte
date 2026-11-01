<script lang="ts">
  /**
   * The transport bar: play, stop, back to start, record (off until recording exists), loop,
   * metronome, position in bars.beats.ticks and minutes:seconds, tempo and time signature, and the
   * launch quantization (`transport.set_launch_quantize`). It holds no state of its own
   * but the text being typed: it reads the stores and every control is a command (`transport-commands.ts`)
   * through the dispatcher.
   */
  import { onMount } from 'svelte'
  import { ignoreKeyEvent, spaceTogglesTransport, trackFocusOrigin } from '../../lib/keyboard'
  import type { MessageKey } from '../../lib/i18n/index'
  import { useStores } from '../../lib/stores'
  import { beginGroup, endGroup } from '../../lib/track-commands'
  import { offTimeline } from '../../lib/off-timeline'
  import { backToArrangement } from '../../lib/scene-commands'
  import { pause, play, seek, setLaunchQuantize, setLoopRegion, setMetronome, setTempo, setTimeSignature, stop, toggleLoop } from '../../lib/transport-commands'
  import { allSoundOff } from '../../lib/synth-commands'
  import {
    barTicks,
    DEFAULT_LOOP_BARS,
    DENOMINATORS,
    formatPosition,
    formatTempo,
    formatTime,
    parseNumerator,
    parseTempo,
    secondsOf,
    stepTempo,
    type Signature,
  } from './time'

  const { engine, project, ui, dispatcher } = useStores()
  const t = ui.t

  const QUANTIZE_ORDER = ['none', 'sixteenth', 'beat', 'bar', 'bars_2', 'bars_4', 'bars_8'] as const

  const settings = $derived(project.view?.project.transport ?? null)
  const bpm = $derived(project.view?.project.tempo[0]?.bpm ?? null)
  const meter = $derived(project.view?.project.meter[0]?.signature ?? null)
  // Before the project exists the bar shows the defaults it would start from, disabled.
  const signature = $derived<Signature>(meter ?? { numerator: 4, denominator: 4 })
  const ready = $derived(engine.started && project.view !== null)
  const playing = $derived(engine.feedback?.transport === 'playing' || engine.feedback?.transport === 'recording')
  const tick = $derived(engine.feedback?.playheadTick ?? 0)
  // Lit while at least one track plays from the launch grid instead of its timeline.
  const detached = $derived(engine.started && offTimeline(engine.feedback, project.view?.slots).size > 0)
  const position = $derived(formatPosition(tick, signature))
  const clock = $derived(bpm === null ? formatTime(0) : formatTime(secondsOf(tick, bpm)))

  let tempoDraft = $state('')
  let numeratorDraft = $state('')
  // Follow the project (undo, another author) except while the user is typing in the field.
  let tempoFocus = $state(false)
  let numeratorFocus = $state(false)
  $effect(() => {
    if (!tempoFocus) tempoDraft = bpm === null ? '' : formatTempo(bpm)
  })
  $effect(() => {
    if (!numeratorFocus) numeratorDraft = meter === null ? '' : String(meter.numerator)
  })

  const run = dispatcher.run.bind(dispatcher)

  function commitTempo() {
    // Only a text the user changed counts: leaving the field untouched must not round a tempo with more than two decimals.
    const value = bpm !== null && tempoDraft === formatTempo(bpm) ? null : parseTempo(tempoDraft)
    if (value !== null && value !== bpm) run(setTempo(value))
    // A refused text, or an applied one, shows what the project holds.
    tempoDraft = bpm === null ? '' : formatTempo(bpm)
  }

  function onTempoKey(event: KeyboardEvent) {
    if (event.key === 'Enter') commitTempo()
    else if (event.key === 'Escape') {
      tempoDraft = bpm === null ? '' : formatTempo(bpm)
      ;(event.currentTarget as HTMLElement).blur()
    } else if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
      event.preventDefault()
      const base = parseTempo(tempoDraft) ?? bpm
      if (base === null) return
      tempoDraft = formatTempo(stepTempo(base, event.key === 'ArrowUp' ? 1 : -1))
      commitTempo()
    }
  }

  function commitNumerator() {
    const value = parseNumerator(numeratorDraft)
    if (value !== null && meter && value !== meter.numerator) run(setTimeSignature(value, meter.denominator))
    numeratorDraft = meter === null ? '' : String(meter.numerator)
  }

  function onNumeratorKey(event: KeyboardEvent) {
    if (event.key === 'Enter') commitNumerator()
    else if (event.key === 'Escape') {
      numeratorDraft = meter === null ? '' : String(meter.numerator)
      ;(event.currentTarget as HTMLElement).blur()
    }
  }

  function onDenominator(event: Event) {
    const value = Number((event.currentTarget as HTMLSelectElement).value)
    if (meter && value !== meter.denominator) run(setTimeSignature(meter.numerator, value))
  }

  function onQuantize(event: Event) {
    const value = (event.currentTarget as HTMLSelectElement).value
    const quantize = QUANTIZE_ORDER.find((q) => q === value)
    if (quantize && quantize !== settings?.launch_quantize) run(setLaunchQuantize(quantize))
  }

  /** The loop needs a region: a project without one gets the first bars, in the same history group. */
  function onLoop() {
    if (!settings) return
    if (settings.loop_region) {
      run(toggleLoop())
      return
    }
    run(beginGroup())
    run(setLoopRegion(0, DEFAULT_LOOP_BARS * barTicks(signature)))
    run(toggleLoop())
    run(endGroup())
  }

  /**
   * A button's click handler. After a mouse click the button lets go of the focus, so the space
   * bar keeps toggling the transport instead of pressing the same button again; a key press
   * (`detail` 0) keeps the focus.
   */
  const press = (action: () => void) => (event: MouseEvent) => {
    action()
    if (event.detail > 0) (event.currentTarget as HTMLElement).blur()
  }

  // `:focus-visible` is not enough (it turns on at the first key press), so the origin of the focus is followed.
  onMount(() => trackFocusOrigin())

  function onSpace(event: KeyboardEvent) {
    if (event.code !== 'Space' || !ready || ignoreKeyEvent(event)) return
    // A text field or a keyboard-focused control keeps the space bar (`spaceTogglesTransport`); a clicked one does not.
    if (!spaceTogglesTransport(event.target)) return
    event.preventDefault()
    if (playing) {
      // Pause: stops where the playhead is and keeps the marker, so the next Space resumes from here and Stop still goes back to the marker (#360).
      run(pause(tick))
    } else run(play())
  }
</script>

<svelte:window onkeydown={onSpace} />

<div class="transport" role="group" aria-label={t('transport.label')} data-testid="transport" title={ready ? undefined : t('transport.needsAudio')}>
  <button
    class="tool"
    disabled={!ready}
    aria-label={t('transport.rewind')}
    title={t('transport.rewind')}
    data-testid="transport-rewind"
    onclick={press(() => run(seek(0)))}
  >
    <svg width="12" height="12" viewBox="0 0 14 14" aria-hidden="true"
      ><path d="M2 1.5 V12.5 M12 1.5 L5 7 L12 12.5 Z" fill="currentColor" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" /></svg
    >
  </button>
  <button
    class="tool"
    class:playing
    disabled={!ready}
    aria-pressed={playing}
    aria-label={t('transport.play')}
    title={t('transport.play')}
    data-testid="transport-play"
    onclick={press(() => run(play()))}
  >
    <svg width="13" height="13" viewBox="0 0 14 14" aria-hidden="true"
      ><path d="M3 1.5 L12 7 L3 12.5 Z" fill={playing ? 'var(--fk-play)' : 'currentColor'} /></svg
    >
  </button>
  <button
    class="tool"
    disabled={!ready}
    aria-label={t('transport.stop')}
    title={t('transport.stopHint')}
    data-testid="transport-stop"
    onclick={press(() => run(stop()))}
    ondblclick={() => run(allSoundOff())}
  >
    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><rect width="10" height="10" rx="1" fill="currentColor" /></svg>
  </button>
  <button
    class="tool"
    class:on={detached}
    disabled={!ready}
    aria-pressed={detached}
    aria-label={t('transport.backToArrangement')}
    title={detached ? t('transport.backToArrangementHint') : t('transport.backToArrangementIdle')}
    data-testid="transport-back-to-arrangement"
    onclick={press(() => run(backToArrangement()))}
  >
    <svg
      width="15"
      height="15"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      stroke-width="1.4"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"><path d="M2 4 H14 M2 8 H14 M2 12 H8 M12 14 L9 11 L12 8" /></svg
    >
  </button>
  <button class="tool" disabled aria-label={t('transport.record')} title={t('transport.recordSoon')} data-testid="transport-record">
    <svg width="11" height="11" viewBox="0 0 12 12" aria-hidden="true"><circle cx="6" cy="6" r="5.5" fill="var(--fk-rec)" /></svg>
  </button>
  <button
    class="tool"
    class:on={settings?.loop_enabled}
    disabled={!ready}
    aria-pressed={settings?.loop_enabled ?? false}
    aria-label={t('transport.loop')}
    title={t('transport.loop')}
    data-testid="transport-loop"
    onclick={press(onLoop)}
  >
    <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" aria-hidden="true"
      ><path d="M3 6 V5 a2 2 0 0 1 2-2 h7 M10 1 l2 2 -2 2 M13 10 v1 a2 2 0 0 1-2 2 H4 M6 15 l-2-2 2-2" /></svg
    >
  </button>
  <button
    class="tool"
    class:on={settings?.metronome.enabled}
    disabled={!ready}
    aria-pressed={settings?.metronome.enabled ?? false}
    aria-label={t('transport.metronome')}
    title={t('transport.metronome')}
    data-testid="transport-metronome"
    onclick={press(() => run(setMetronome(!settings?.metronome.enabled)))}
  >
    <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" aria-hidden="true"
      ><path d="M5.5 14.5 L7 2 H9 L10.5 14.5 Z M8 11 L11.5 4.5" /></svg
    >
  </button>

  <dl class="readouts">
    <div>
      <dt class="sr-only">{t('transport.position')}</dt>
      <dd class="value pos" data-testid="transport-position">{position}</dd>
    </div>
    <div>
      <dt class="sr-only">{t('transport.time')}</dt>
      <dd class="value muted" data-testid="transport-time">{clock}</dd>
    </div>
  </dl>

  <label class="field">
    <span class="sr-only">{t('transport.tempo')}</span>
    <input
      class="value tempo"
      type="text"
      inputmode="decimal"
      autocomplete="off"
      size="6"
      disabled={!ready}
      aria-label={t('transport.tempo')}
      title={t('transport.tempo')}
      data-testid="transport-tempo"
      bind:value={tempoDraft}
      onfocus={() => (tempoFocus = true)}
      onblur={() => {
        commitTempo()
        tempoFocus = false
      }}
      onkeydown={onTempoKey}
    />
  </label>

  <div class="field" role="group" aria-label={t('transport.signature')}>
    <input
      class="value sig"
      type="text"
      inputmode="numeric"
      autocomplete="off"
      size="2"
      disabled={!ready}
      aria-label={t('transport.numerator')}
      title={t('transport.numerator')}
      data-testid="transport-numerator"
      bind:value={numeratorDraft}
      onfocus={() => (numeratorFocus = true)}
      onblur={() => {
        commitNumerator()
        numeratorFocus = false
      }}
      onkeydown={onNumeratorKey}
    />
    <span class="unit" aria-hidden="true">/</span>
    <select
      class="value"
      disabled={!ready}
      aria-label={t('transport.denominator')}
      title={t('transport.denominator')}
      data-testid="transport-denominator"
      value={String(signature.denominator)}
      onchange={onDenominator}
    >
      {#each DENOMINATORS as d (d)}<option value={String(d)}>{d}</option>{/each}
    </select>
  </div>

  <label class="field">
    <span class="sr-only">{t('transport.launchQuantize')}</span>
    <select
      class="value quant"
      disabled={!ready}
      aria-label={t('transport.launchQuantize')}
      title={t('transport.launchQuantizeHint')}
      data-testid="transport-quantize"
      value={settings?.launch_quantize ?? 'bar'}
      onchange={onQuantize}
    >
      {#each QUANTIZE_ORDER as q (q)}<option value={q}>{t(`transport.quantize.${q}` satisfies MessageKey)}</option>{/each}
    </select>
  </label>
</div>

<style>
  .transport {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--fk-space-1);
  }
  .tool {
    width: 30px;
    height: 30px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid transparent;
    border-radius: var(--fk-radius-panel);
    background: var(--fk-surface-2);
    color: var(--fk-text);
    cursor: pointer;
  }
  .tool:hover:not(:disabled) {
    border-color: var(--fk-line);
  }
  .tool {
    position: relative;
  }
  /*
   * One voice: the on state of a toggle is a fill and a frame in the text colour with a dot in the
   * corner (never a hue); playback running is the one green, with its triangle; violet stays for
   * selection, focus, the main action and Boost.
   */
  .tool.on,
  .tool.on:hover:not(:disabled) {
    background: var(--fk-text);
    border-color: var(--fk-text);
    color: var(--fk-bg);
  }
  .tool.on::after {
    content: '';
    position: absolute;
    top: 3px;
    right: 3px;
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: currentColor;
  }
  .tool.playing,
  .tool.playing:hover:not(:disabled) {
    background: color-mix(in srgb, var(--fk-play) 22%, transparent);
    border-color: var(--fk-play);
  }
  .tool.playing::after {
    content: none;
  }
  .tool:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .readouts {
    display: flex;
    flex-direction: column;
    margin: 0 var(--fk-space-1) 0 var(--fk-space-2);
    line-height: 1.15;
    color: var(--fk-text-muted);
  }
  .readouts dd {
    margin: 0;
    white-space: nowrap;
  }
  .pos {
    font-size: 14px;
    font-weight: 500;
    color: var(--fk-text);
  }
  .muted {
    font-size: 11px;
  }
  .field {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    margin-left: var(--fk-space-1);
  }
  select.quant {
    width: 100px;
    padding: 0 var(--fk-space-1);
    font-size: 11px;
  }
  .unit {
    font-size: 11px;
    color: var(--fk-text-muted);
  }
  input.value,
  select.value {
    height: 28px;
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-bg);
    font-size: 13px;
    padding: 0 var(--fk-space-2);
    text-align: center;
  }
  input.tempo {
    width: 52px;
    font-weight: 500;
  }
  input.sig {
    width: 30px;
    padding: 0;
  }
  select.value:disabled,
  input.value:disabled {
    opacity: 0.45;
  }
  /* The top bar is full at this width (the back-to-arrangement button joined the tools): tighter tools. */
  @media (max-width: 1399px) {
    .tool {
      width: 28px;
    }
    select.quant {
      width: 84px;
    }
    .field {
      margin-left: 2px;
    }
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
</style>
