<script lang="ts">
  /**
   * The top bar: home link, view switcher (the registry's `main` views), transport
   * (`src/views/transport/`), engine state (mode, latency, load; or the start button).
   */
  import { resolve } from '$app/paths'
  import LevelMeter from '../../lib/components/LevelMeter.svelte'
  import Monogram from '../../lib/components/Monogram.svelte'
  import { MASTER_METER, useStores } from '../../lib/stores'
  import { viewsIn } from '../registry'
  import Transport from '../transport/Transport.svelte'
  import ProjectMenu from './ProjectMenu.svelte'

  const { engine, ui, dispatcher, keys, meters, storage } = useStores()
  const t = ui.t

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
</script>

<header class="top">
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

  <Transport />

  <div class="engine" data-testid="engine-state">
    {#if engine.started}
      <!-- The engine pill is a button: it opens the engine dialog (audio, helper, MIDI). -->
      <button
        class="mode"
        class:boost={engine.mode === 'native'}
        type="button"
        aria-haspopup="dialog"
        aria-label={`${engine.mode === 'native' ? t('engine.boost') : t('engine.web')}, ${latency}${loadPeak ? `, ${loadPeak}` : ''}${waiting ? `, ${waiting}` : ''}. ${t('engine.settings')}`}
        title={`${t('engine.settings')}: ${engine.mode === 'native' ? t('engine.boost') : t('engine.web')}, ${latency}${loadPeak ? `, ${loadPeak}` : ''}${waiting ? `. ${waiting}` : ''}`}
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
      </button>
      <!-- The master's level, in every view: the output is what the listener hears, whatever the view shows. -->
      <div class="master-meter">
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
      <span class="value muted" data-testid="keys-octave-bar" title={keys.targetHint}>{t('keys.octaveShort', { n: keys.octave })}</span>
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
  /* One row from 1280 px up, in every view and both languages: nothing wraps, the spacer gives way first. */
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
    flex: 1;
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
