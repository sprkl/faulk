<script lang="ts">
  // A test surface for the shared time grid (AR-3), until the timeline (AR-4) mounts it. It shows
  // the grid only on the dev server and in the e2e build (`VITE_FAULK_DEV_ROUTES=1`): a production
  // build renders an empty page. Plain English text, not part of the product UI.
  import '../../../lib/styles/app.css'
  import TimeGrid from '../../../lib/components/timegrid/TimeGrid.svelte'
  import { DEFAULT_PX_PER_TICK, SNAP_SETTINGS, type SnapSetting, type View } from '../../../lib/components/timegrid/grid'
  import { DENOMINATORS, formatPosition, type Signature } from '../../../views/transport/time'

  const enabled = import.meta.env.DEV || import.meta.env.VITE_FAULK_DEV_ROUTES === '1'

  let view = $state<View>({ pxPerTick: DEFAULT_PX_PER_TICK, scrollTick: 0 })
  let snap = $state<SnapSetting>('auto')
  let signature = $state<Signature>({ numerator: 4, denominator: 4 })
  let hover = $state<number | null>(null)
  let seeked = $state<number | null>(null)
</script>

<svelte:head>
  <title>Time grid (test surface)</title>
</svelte:head>

{#if enabled}
  <main class="demo" data-theme="dark">
    <header>
      <label>
        Snap
        <select bind:value={snap} data-testid="snap">
          {#each SNAP_SETTINGS as setting (setting)}<option value={setting}>{setting}</option>{/each}
        </select>
      </label>
      <label>
        Beats per bar
        <input type="number" min="1" max="32" bind:value={signature.numerator} data-testid="numerator" />
      </label>
      <label>
        Beat unit
        <select bind:value={signature.denominator} data-testid="denominator">
          {#each DENOMINATORS as denominator (denominator)}<option value={denominator}>{denominator}</option>{/each}
        </select>
      </label>
      <span class="value" data-testid="hover" data-tick={hover ?? ''}>{hover === null ? '-' : formatPosition(hover, signature)}</span>
      <span class="value" data-testid="seeked" data-tick={seeked ?? ''}>{seeked === null ? '-' : formatPosition(seeked, signature)}</span>
    </header>
    <div class="grid">
      <TimeGrid bind:view {signature} {snap} playhead={seeked} label="Time grid" onhover={(tick) => (hover = tick)} onseek={(tick) => (seeked = tick)} />
    </div>
  </main>
{/if}

<style>
  .demo {
    display: grid;
    grid-template-rows: auto 1fr;
    height: 100vh;
    background: var(--fk-bg);
    color: var(--fk-text);
  }
  header {
    display: flex;
    gap: var(--fk-space-4);
    align-items: center;
    padding: var(--fk-space-2) var(--fk-space-4);
  }
  .grid {
    min-height: 0;
  }
</style>
