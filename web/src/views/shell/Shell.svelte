<script lang="ts">
  /**
   * The DAW's layout: top bar (views, transport, engine state), track list, main area showing the
   * registry's current view, bottom panel (devices, piano roll). It creates the stores once and
   * provides them to every view.
   */
  import { goto } from '$app/navigation'
  import { onMount } from 'svelte'
  import { watchLocalAccess } from '../../lib/engine/local-access'
  import { createStores, provideStores, type Stores } from '../../lib/stores'
  import { DEFAULT_PANEL, DEFAULT_VIEW, viewById } from '../registry'
  import BottomPanel from './BottomPanel.svelte'
  import TopBar from './TopBar.svelte'
  import TrackList from './TrackList.svelte'

  // `stores` is for tests; the page lets the shell create its own.
  let { stores = createStores(DEFAULT_VIEW, DEFAULT_PANEL) }: { stores?: Stores } = $props()
  // svelte-ignore state_referenced_locally
  const { engine, ui, dispatcher } = provideStores(stores)

  const current = $derived(viewById(ui.view) ?? viewById(DEFAULT_VIEW)!)

  onMount(() => {
    void dispatcher.checkHelper()
    // The first probe may wait behind the browser's local network prompt: probe again on Allow.
    const stopLocalAccess = watchLocalAccess(() => void dispatcher.onLocalAccessGranted())
    // Through the router: a raw history.replaceState conflicts with SvelteKit's.
    // `url` is the current path and query, already resolved against the base path.
    // eslint-disable-next-line svelte/no-navigation-without-resolve
    const clearAddress = (url: string) => void goto(url, { shallow: true, replace: true })
    dispatcher.takePairLink(location, clearAddress)
    const onHashChange = () => dispatcher.takePairLink(location, clearAddress)
    window.addEventListener('hashchange', onHashChange)
    return () => {
      stopLocalAccess()
      window.removeEventListener('hashchange', onHashChange)
    }
  })

  $effect(() => {
    document.documentElement.lang = ui.lang
  })
</script>

<div class="shell">
  <TopBar />
  {#if engine.notice}
    <p class="notice" role="status">{ui.t(engine.notice)}</p>
  {/if}
  <!-- The instrument view shows pairing results in its helper panel; elsewhere, here. -->
  {#if engine.pairingMessage && ui.view !== 'play'}
    <p class="notice" role="status" data-testid="pairing-notice">{ui.t(engine.pairingMessage.key, { n: engine.pairingMessage.n ?? 0 })}</p>
  {/if}
  <div class="body">
    <TrackList />
    <main class="main" aria-label={ui.t(current.label)} data-testid="view" data-view={current.id}>
      <current.component />
    </main>
    <BottomPanel />
  </div>
</div>

<style>
  .shell {
    height: 100vh;
    height: 100dvh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .notice {
    flex-shrink: 0;
    margin: 0;
    padding: var(--fk-space-2) var(--fk-space-4);
    border-bottom: 1px solid var(--fk-line);
    background: var(--fk-surface);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 240px minmax(0, 1fr);
    grid-template-rows: minmax(0, 1fr) minmax(120px, 28%);
    grid-template-areas:
      'tracks main'
      'tracks panel';
  }
  .main {
    grid-area: main;
    min-width: 0;
    overflow: auto;
  }
  /* Narrow screens: one column that scrolls as a whole. */
  @media (max-width: 760px) {
    .shell {
      height: auto;
      min-height: 100vh;
      overflow: visible;
    }
    .body {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: auto auto auto;
      grid-template-areas: 'main' 'panel' 'tracks';
    }
    .main {
      overflow: visible;
    }
  }
</style>
