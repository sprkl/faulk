<script lang="ts">
  /**
   * The top bar: home link, view switcher (the registry's `main` views), transport
   * (`src/views/transport/`), engine state (mode, latency, voices; or the start button), language switch.
   */
  import { resolve } from '$app/paths'
  import Monogram from '../../lib/components/Monogram.svelte'
  import { useStores } from '../../lib/stores'
  import { allNotesOff } from '../../lib/synth-commands'
  import { viewsIn } from '../registry'
  import Transport from '../transport/Transport.svelte'
  import ProjectMenu from './ProjectMenu.svelte'

  const { engine, ui, dispatcher, keys } = useStores()
  const t = ui.t

  const latency = $derived(
    engine.status?.estimatedLatencyMs != null ? t('engine.latency', { ms: Math.round(engine.status.estimatedLatencyMs) }) : t('engine.latencyUnknown'),
  )
  // The engine's load, when the host measures it (neither the worklet nor the helper does yet: it stays hidden until it is).
  const load = $derived(engine.feedback?.cpuLoad != null ? t('transport.load', { pct: Math.round(engine.feedback.cpuLoad * 100) }) : null)
</script>

<header class="top">
  <h1 class="sr-only">Faulk</h1>
  <!-- Full page load: the landing page has its own theme and no client-side router. -->
  <a class="home" href={resolve('/')} data-sveltekit-reload aria-label={t('app.home')}><Monogram size={20} /></a>

  <ProjectMenu />

  <nav class="views" aria-label={t('views.nav')}>
    {#each viewsIn('main') as view (view.id)}
      <button aria-current={ui.view === view.id ? 'page' : undefined} data-testid="nav-{view.id}" onclick={() => ui.show(view.id)}>{t(view.label)}</button>
    {/each}
  </nav>

  <div class="spacer"></div>

  <Transport />

  <div class="engine" data-testid="engine-state">
    {#if engine.started}
      <span class="mode" class:boost={engine.mode === 'native'} data-testid="engine-mode">
        {#if engine.mode === 'native'}
          <svg width="12" height="12" viewBox="0 0 16 16" aria-hidden="true"><path d="M9 1 L3 9 H8 L7 15 L13 7 H8 Z" fill="currentColor" /></svg>
          {t('engine.boost')}
        {:else}
          <svg width="12" height="12" viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.4"
            ><circle cx="8" cy="8" r="6.5" /><path d="M1.5 8 H14.5 M8 1.5 C 5 5, 5 11, 8 14.5 M8 1.5 C 11 5, 11 11, 8 14.5" /></svg
          >
          {t('engine.web')}
        {/if}
        <span class="value" data-testid="latency">{latency}</span>
      </span>
      <span class="value muted" data-testid="voices">{t('engine.voices', { n: engine.voices })}</span>
      {#if load}<span class="value muted" data-testid="engine-load">{load}</span>{/if}
      <!-- The computer keyboard plays in every view (#361): the instrument view shows its octave and its panic button itself. -->
      {#if ui.view !== 'play'}
        <span class="value muted" data-testid="keys-octave" title={keys.targetHint}>{t('keys.octave', { n: keys.octave })}</span>
        <button class="ghost" data-testid="panic" onclick={() => dispatcher.run(allNotesOff())}>{t('panic')}</button>
      {/if}
      <!-- The engine's report as data, for the browser checks: the mixer and the transport (MX-3, TR-4) will draw it. -->
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
      <button class="primary" disabled={engine.starting} onclick={() => dispatcher.start()}>{t('start.button')}</button>
    {/if}
  </div>

  <button class="ghost" onclick={() => ui.setLang(ui.lang === 'fr' ? 'en' : 'fr')}>{t('lang.switch')}</button>
</header>

<style>
  .top {
    flex-shrink: 0;
    min-height: 52px;
    display: flex;
    align-items: center;
    gap: var(--fk-space-2);
    padding: var(--fk-space-2) var(--fk-space-3);
    border-bottom: 1px solid var(--fk-line);
    flex-wrap: wrap;
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
    display: inline-flex;
    padding: var(--fk-space-1);
    color: var(--fk-text);
  }
  .views {
    display: inline-flex;
    gap: 2px;
    padding: 3px;
    border-radius: var(--fk-radius-pill);
    background: var(--fk-bg);
    border: 1px solid var(--fk-line);
  }
  .views button {
    height: 26px;
    padding: 0 var(--fk-space-2);
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
  }
  /* The bar is full at this width: a little less air between and inside its controls. */
  @media (max-width: 1399px) {
    .top {
      gap: 4px;
    }
    .views button {
      padding: 0 6px;
    }
    button.ghost {
      padding: 0 var(--fk-space-3);
    }
  }
  .engine {
    display: flex;
    align-items: center;
    gap: var(--fk-space-2);
  }
  .mode {
    display: inline-flex;
    align-items: center;
    gap: var(--fk-space-2);
    height: 30px;
    padding: 0 var(--fk-space-2);
    border-radius: var(--fk-radius-pill);
    background: var(--fk-surface-2);
    font-weight: 600;
    font-size: 12px;
    white-space: nowrap;
  }
  .mode.boost {
    background: var(--fk-accent);
    color: #fff;
  }
  .muted {
    color: var(--fk-text-muted);
    white-space: nowrap;
  }
  button.primary,
  button.ghost {
    height: 30px;
    padding: 0 var(--fk-space-4);
    border-radius: var(--fk-radius-pill);
    cursor: pointer;
    white-space: nowrap;
    transition: background var(--fk-dur-fast) ease-out;
  }
  button.primary {
    border: 1px solid var(--fk-accent);
    background: var(--fk-accent);
    color: #fff;
    font-weight: 600;
  }
  button.primary:hover {
    background: var(--fk-accent-strong);
  }
  button.primary:disabled {
    opacity: 0.5;
    cursor: default;
  }
  button.ghost {
    border: 1px solid var(--fk-line);
    background: transparent;
  }
  button.ghost:hover {
    background: var(--fk-surface-2);
  }
</style>
