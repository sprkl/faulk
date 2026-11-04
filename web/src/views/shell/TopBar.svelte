<script lang="ts">
  /**
   * The top bar: home link, view switcher (the registry's `main` views), transport
   * (`src/views/transport/`), engine state (mode, latency, load; or the start button).
   */
  import { resolve } from '$app/paths'
  import { flushSync, onMount } from 'svelte'
  import LevelMeter from '../../lib/components/LevelMeter.svelte'
  import Monogram from '../../lib/components/Monogram.svelte'
  import { MASTER_METER, useStores } from '../../lib/stores'
  import { viewsIn } from '../registry'
  import { setLaunchQuantize } from '../../lib/transport-commands'
  import Transport from '../transport/Transport.svelte'
  import { QUANTIZE_ORDER } from '../transport/time'
  import ProjectMenu from './ProjectMenu.svelte'

  const { engine, project, ui, dispatcher, keys, meters, storage } = useStores()
  const t = ui.t

  /**
   * How full the bar is (#604). The bar is one row that never overlaps: when its contents do not fit, `fit` goes up one
   * step at a time until they do. The order is what the producer reads least while working, first to go first:
   *   1  the master meter gets shorter (64 px, still readable),
   *   2  the keyboard octave readout moves to the "more" menu (it plays in every view, so it keeps a place), the project
   *      name's room and the tabs' padding get tighter,
   *   3  the launch quantization moves to that menu too (the command is the same, the menu runs it),
   *   4  the idle take target text hides (the record button's title and a status line for screen readers keep it; the live
   *      count-in and a lost-note notice never hide), and the master meter shrinks again to 40 px. The target goes last: it
   *      says where the next take lands (spec § 4.1).
   * The project name keeps its minimum room at every step (`ProjectMenu.svelte`). Never hidden: the project, the views, the
   * transport's main controls, the engine pill with its latency and load, the help.
   */
  const LAST_FIT = 4
  let fit = $state(0)
  let bar = $state<HTMLElement | null>(null)
  let moreButton = $state<HTMLElement | null>(null)

  /** Raises `fit` from 0 to the first step at which the bar's contents fit, reading the layout between the steps. */
  function measure() {
    if (!bar) return
    const fits = () => bar!.scrollWidth <= bar!.clientWidth
    // A control that folds has the focus (the select being used, the menu's button): the folded parts stay in the page
    // (hidden, never removed), the search only goes up from the current step, and a bar that fits is left as it is.
    // Any other control (Record, a view tab) keeps its focus whatever is folded, so the bar measures from the start.
    const active = document.activeElement
    const keepsFocus = active instanceof HTMLElement && active.closest('[data-folds]') !== null
    if (keepsFocus && fits()) return
    for (let step = keepsFocus ? fit : 0; step <= LAST_FIT; step++) {
      fit = step
      flushSync()
      if (fits()) break
    }
    // A trial step blurs the control that had the focus: give it back, or, when the control is now folded away
    // (it cannot take the focus), hand it to the "more" button that holds what it did.
    if (keepsFocus) {
      // The browser moves the focus off a hidden control later, not at once: decide from the control, not from `activeElement`.
      const folded = (active as HTMLElement).closest('[hidden]') !== null || (active as HTMLElement).offsetParent === null
      if (folded) moreButton?.focus({ preventScroll: true })
      else if (document.activeElement !== active) (active as HTMLElement).focus({ preventScroll: true })
    }
  }

  onMount(() => {
    if (!bar || typeof ResizeObserver === 'undefined') return
    // The bar and every one of its children: a longer text (a language, a take's target, a tempo tap) is a change of width too.
    // `measure` changes the layout (it folds parts and flushes the DOM), so it never runs inside the observer's callback: the
    // browser would then report "ResizeObserver loop completed with undelivered notifications" (#674). The callback asks for
    // one measure on the next frame, before the next layout; the observer sees the folded bar then and asks again until it fits.
    let frame = 0
    const observer = new ResizeObserver(() => {
      if (frame !== 0) return
      frame = requestAnimationFrame(() => {
        frame = 0
        measure()
      })
    })
    observer.observe(bar)
    for (const child of bar.children) observer.observe(child)
    measure()
    return () => {
      observer.disconnect()
      cancelAnimationFrame(frame)
    }
  })

  /** The menu of what the bar folded: the launch quantization (a radio group) and the keyboard octave. */
  function openMore(event: MouseEvent) {
    const box = (event.currentTarget as HTMLElement).getBoundingClientRect()
    const current = project.view?.project.transport.launch_quantize ?? 'bar'
    const ready = engine.started && project.view !== null
    ui.openMenuAt(
      box.left,
      box.bottom + 2,
      [
        // The octave readout, folded from the bar at step 2 (the keys play in every view, even with the keyboard panel closed).
        { id: 'more-octave', label: t('keys.octaveShort', { n: keys.octave }), disabled: true, run: () => {} },
        ...(fit >= 3
          ? [
              { id: 'more-quantize', label: t('transport.launchQuantize'), separator: true, disabled: true, run: () => {} },
              ...QUANTIZE_ORDER.map((q) => ({
                id: `more-quantize-${q}`,
                label: t(`transport.quantize.${q}`),
                detail: q === current ? '✓' : undefined,
                disabled: !ready,
                run: () => {
                  if (q !== current) void dispatcher.run(setLaunchQuantize(q))
                },
              })),
            ]
          : []),
      ],
      t('transport.more'),
    )
  }

  const latency = $derived(
    engine.status?.estimatedLatencyMs != null ? t('engine.latency', { ms: Math.round(engine.status.estimatedLatencyMs) }) : t('engine.latencyUnknown'),
  )
  // In the pill the figure is short; "estimated" is in the tooltip and the dialog.
  const latencyShort = $derived(
    engine.status?.estimatedLatencyMs != null ? t('engine.latencyShort', { ms: Math.round(engine.status.estimatedLatencyMs) }) : t('engine.latencyUnknown'),
  )
  // The engine's load (#486): the smoothed figure in the pill (the bar has no room for a second one at 1280 px), with its
  // peak of the last seconds in the tooltip and the accessible name; hidden while the engine does not measure it.
  const load = $derived(engine.load ? t('engine.loadShort', { pct: Math.round(engine.load.smoothed * 100) }) : null)
  const loadPeak = $derived(
    engine.load ? t('engine.loadPeak', { pct: Math.round(engine.load.smoothed * 100), peak: Math.round(engine.load.peak * 100) }) : null,
  )
  // The helper runs without an audio output (#525): the page plays in Web mode until it has one.
  const waiting = $derived(engine.mode === 'web' && engine.awaitingOutput ? t('engine.boostWaiting') : null)
  // The output protection silenced a non-finite signal (#667): a red dot in the pill and the words in its tooltip and
  // accessible name, for as long as this engine plays; the dialog says how much.
  const silenced = $derived(engine.silencedFrames > 0 ? t('engine.silenced') : null)
  const notes = $derived([loadPeak, waiting, silenced].filter((n): n is string => n !== null))
</script>

<header class="top" bind:this={bar} data-fit={fit} style:--fk-project-max={fit >= 2 ? '128px' : '160px'}>
  <h1 class="sr-only">Faulk</h1>
  <!-- Full page load: the landing page has its own theme and no client-side router. -->
  <a class="home" href={resolve('/')} data-sveltekit-reload aria-label={t('app.home')}><Monogram size={20} /></a>

  <ProjectMenu />

  <nav class="views" aria-label={t('views.nav')}>
    {#each viewsIn('main') as view, i (view.id)}
      <button
        aria-current={ui.view === view.id ? 'page' : undefined}
        title={`${t(view.label)} (Alt+${i + 1})`}
        aria-keyshortcuts={`Alt+${i + 1}`}
        data-testid="nav-{view.id}"
        onclick={() => ui.show(view.id)}>{t(view.label)}</button
      >
    {/each}
  </nav>

  <div class="spacer"></div>

  <Transport hideTarget={fit >= 4} hideQuantize={fit >= 3} />
  <button
    class="more"
    bind:this={moreButton}
    hidden={fit < 2}
    data-folds
    type="button"
    aria-haspopup="menu"
    aria-expanded={ui.contextMenu?.label === t('transport.more')}
    aria-label={t('transport.more')}
    title={t('transport.moreHint')}
    data-testid="bar-more"
    onclick={openMore}>⋯</button
  >

  <div class="engine" data-testid="engine-state">
    {#if engine.started}
      <!-- The engine pill is a button: it opens the engine dialog (audio, helper, MIDI). -->
      <button
        class="mode"
        class:boost={engine.mode === 'native'}
        type="button"
        aria-haspopup="dialog"
        aria-label={`${[engine.mode === 'native' ? t('engine.boost') : t('engine.web'), latency, ...notes].join(', ')}. ${t('engine.settings')}`}
        title={`${t('engine.settings')}: ${[engine.mode === 'native' ? t('engine.boost') : t('engine.web'), latency, loadPeak].filter(Boolean).join(', ')}${[
          waiting,
          silenced,
        ]
          .filter(Boolean)
          .map((n) => `. ${n}`)
          .join('')}`}
        data-testid="engine-mode"
        onclick={() => (ui.engineOpen = true)}
      >
        {#if engine.mode === 'native'}
          <svg width="12" height="12" viewBox="0 0 16 16" aria-hidden="true"><path d="M9 1 L3 9 H8 L7 15 L13 7 H8 Z" fill="currentColor" /></svg>
          <span class="mode-name">{t('engine.boost')}</span>
        {:else}
          <svg width="12" height="12" viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.4"
            ><circle cx="8" cy="8" r="6.5" /><path d="M1.5 8 H14.5 M8 1.5 C 5 5, 5 11, 8 14.5 M8 1.5 C 11 5, 11 11, 8 14.5" /></svg
          >
          <span class="mode-name">{t('engine.web')}</span>
        {/if}
        <span class="value" data-testid="latency">{latencyShort}</span>
        {#if load}<span class="value load" data-testid="engine-load">{load}</span>{/if}
        {#if waiting}<span class="waiting" aria-hidden="true" data-testid="engine-waiting"></span>{/if}
        {#if silenced}<span class="silenced" aria-hidden="true" data-testid="engine-silenced"></span>{/if}
      </button>
      <!-- The master's level, in every view: the output is what the listener hears, whatever the view shows. -->
      <div class="master-meter" class:short={fit >= 1} class:shortest={fit >= 4}>
        <LevelMeter
          snap={meters.snap(MASTER_METER)}
          orientation="horizontal"
          label={t('meter.master')}
          clipLabel={t('meter.clip')}
          resetLabel={t('meter.reset', { name: t('mixer.master') })}
          onreset={() => meters.reset(MASTER_METER)}
          testid="top-meter"
        />
      </div>
      <!-- The octave of the computer keyboard, which plays in every view (#361); the keyboard tab of the bottom panel has the keys. -->
      <span class="value muted" hidden={fit >= 2} data-folds data-testid="keys-octave-bar" title={keys.targetHint}
        >{t('keys.octaveShort', { n: keys.octave })}</span
      >
      <!-- The voices playing: for the browser checks (the dialog shows them to the eye), out of the screen reader's way. -->
      <span class="sr-only" aria-hidden="true" data-testid="voices">{t('engine.voices', { n: engine.voices })}</span>
      <!-- The engine's report as data, for the browser checks. -->
      <span
        hidden
        data-testid="feedback"
        data-seq={engine.feedback?.seq ?? 0}
        data-transport={engine.feedback?.transport ?? 'stopped'}
        data-tick={engine.feedback?.playheadTick ?? 0}
        data-master-peak={engine.peak}
        data-track-peaks={engine.feedback?.tracks.map((m) => Math.max(m.peakL, m.peakR).toFixed(4)).join(',') ?? ''}
      ></span>
    {:else}
      <!-- One main action before the audio runs (#474): while a project waits for its click, that click is the main one. -->
      <button class:primary={storage.banner !== 'reopen'} disabled={engine.starting} title={t('start.body')} onclick={() => dispatcher.start()}
        >{t('start.button')}</button
      >
    {/if}
  </div>

  <!-- A real button, in the tab order: the list of every shortcut (#474). `?` opens it too. -->
  <button
    class="help"
    type="button"
    aria-haspopup="dialog"
    aria-keyshortcuts="?"
    aria-label={t('shortcut.open')}
    title={t('shortcut.openHint')}
    data-testid="shortcuts-open"
    onclick={() => (ui.helpOpen = true)}>?</button
  >
</header>

<style>
  /* One row from 1280 px up, in every view and both languages: nothing wraps and nothing overlaps (#604). The spacer gives way
     first, then the project name down to its minimum, then `fit` folds the least-read parts (script). */
  .top {
    flex-shrink: 0;
    min-height: 52px;
    display: flex;
    align-items: center;
    gap: var(--fk-space-2);
    padding: var(--fk-space-2) var(--fk-space-3);
    border-bottom: 1px solid var(--fk-line);
    flex-wrap: nowrap;
    min-width: 0;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  .home {
    flex: none;
    display: inline-flex;
    padding: var(--fk-space-1);
    color: var(--fk-text);
  }
  .views {
    flex: none;
    display: inline-flex;
    gap: 2px;
    padding: 3px;
    border-radius: var(--fk-radius-pill);
    background: var(--fk-bg);
    border: 1px solid var(--fk-line);
  }
  .views button {
    height: 26px;
    padding: 0 var(--fk-space-3);
    border: 0;
    border-radius: var(--fk-radius-pill);
    background: transparent;
    color: var(--fk-text-muted);
    font-weight: 500;
    font-size: 12px;
    cursor: pointer;
    transition: background var(--fk-dur-fast) ease-out;
  }
  .views button:hover {
    color: var(--fk-text);
  }
  .views button[aria-current='page'] {
    background: var(--fk-text);
    color: var(--fk-bg);
  }
  .spacer {
    flex: 1 1 0;
    min-width: 0;
  }
  .engine {
    flex: none;
    display: flex;
    align-items: center;
    gap: var(--fk-space-2);
  }
  /* Below 1440 px the bar is full (#486): the pill yields first, before the project name. The mode's word goes
     (its icon stays, the word is in the tooltip and the accessible name) and the pill's gaps tighten. */
  @media (max-width: 1439px) {
    .mode-name {
      display: none;
    }
    .top .mode {
      gap: var(--fk-space-1);
    }
  }
  .mode {
    display: inline-flex;
    align-items: center;
    gap: var(--fk-space-2);
    height: 30px;
    padding: 0 var(--fk-space-3);
    border: 1px solid transparent;
    border-radius: var(--fk-radius-pill);
    background: var(--fk-surface-2);
    color: inherit;
    font: inherit;
    font-weight: 600;
    font-size: 12px;
    white-space: nowrap;
    cursor: pointer;
    transition: background var(--fk-dur-fast) ease-out;
  }
  .mode:hover {
    border-color: var(--fk-line);
  }
  .mode.boost {
    background: var(--fk-accent);
    color: #fff;
  }
  /* The load changes about 30 times a second: fixed-width digits keep the pill still while its digit count stays. */
  .mode .load {
    font-variant-numeric: tabular-nums;
  }
  /* Boost waits for the helper's audio output (#525). */
  .waiting {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--fk-solo);
  }
  /* The output protection silenced a non-finite signal (#667): red, apart from the yellow wait; ringed so that it
     reads on the Boost pill's accent too. */
  .silenced {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--fk-rec);
    box-shadow: 0 0 0 1px var(--fk-surface-2);
  }
  .help {
    flex: none;
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    padding: 0;
    border: 1px solid var(--fk-line);
    border-radius: 50%;
    background: transparent;
    color: var(--fk-text-muted);
    font: inherit;
    font-weight: 600;
    font-size: 12px;
    cursor: pointer;
    transition: background var(--fk-dur-fast) ease-out;
  }
  .help:hover {
    background: var(--fk-surface-2);
    color: var(--fk-text);
  }
  .muted {
    color: var(--fk-text-muted);
    white-space: nowrap;
  }
  .master-meter {
    flex: none;
  }
  .master-meter :global(.meter) {
    --len: 96px;
  }
  .master-meter.short :global(.meter) {
    --len: 64px;
  }
  /* Last step of the fold: a short bar, the clip light stays. */
  .master-meter.shortest :global(.meter) {
    --len: 40px;
  }
  /* The overflow button: as wide as a transport tool, so it costs one slot of the bar. */
  .more {
    flex: none;
    width: 28px;
    height: 30px;
    padding: 0;
    border: 1px solid transparent;
    border-radius: var(--fk-radius-panel);
    background: var(--fk-surface-2);
    color: var(--fk-text);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }
  .more[hidden],
  .muted[hidden] {
    display: none;
  }
  .more:hover,
  .more[aria-expanded='true'] {
    border-color: var(--fk-line);
  }
  .top:is([data-fit='2'], [data-fit='3'], [data-fit='4']) .views button {
    padding: 0 var(--fk-space-2);
  }
  button.primary {
    height: 30px;
    padding: 0 var(--fk-space-4);
    border: 1px solid var(--fk-accent);
    border-radius: var(--fk-radius-pill);
    background: var(--fk-accent);
    color: #fff;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
    transition: background var(--fk-dur-fast) ease-out;
  }
  button.primary:hover {
    background: var(--fk-accent-strong);
  }
  button.primary:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
