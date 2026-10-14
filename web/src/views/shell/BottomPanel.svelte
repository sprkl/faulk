<script lang="ts">
  // The bottom panel: shows the registry's `panel` views (devices, piano roll), switched by its tabs.
  import { useStores } from '../../lib/stores'
  import { viewById, viewsIn } from '../registry'

  const { ui } = useStores()
  const tabs = viewsIn('panel')
  const current = $derived(viewById(ui.panel) ?? tabs[0])
</script>

<section class="panel" aria-label={ui.t('panel.label')} data-testid="bottom-panel" data-panel={current.id}>
  <div class="tabs">
    {#each tabs as tab (tab.id)}
      <button aria-pressed={ui.panel === tab.id} data-testid="panel-{tab.id}" onclick={() => ui.showPanel(tab.id)}>{ui.t(tab.label)}</button>
    {/each}
  </div>
  <div class="body">
    <current.component />
  </div>
</section>

<style>
  .panel {
    grid-area: panel;
    min-height: 0;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--fk-line);
    background: var(--fk-surface);
  }
  .tabs {
    display: flex;
    gap: var(--fk-space-1);
    padding: var(--fk-space-2) var(--fk-space-3);
    border-bottom: 1px solid var(--fk-line);
  }
  button {
    height: 26px;
    padding: 0 var(--fk-space-3);
    border: 0;
    border-radius: var(--fk-radius-pill);
    background: transparent;
    color: var(--fk-text-muted);
    font-weight: 500;
    font-size: 12px;
    cursor: pointer;
  }
  button:hover {
    color: var(--fk-text);
  }
  button[aria-pressed='true'] {
    background: var(--fk-surface-2);
    color: var(--fk-text);
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
</style>
