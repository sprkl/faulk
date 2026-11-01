<script lang="ts">
  /**
   * The mixer: one strip per track in the order of the track list, and the master's pinned at the
   * right. The meters come from the shared meter store (`stores/meters.svelte.ts`). Every control is a command (`MixerStrip`).
   */
  import { useStores } from '../../lib/stores'
  import MixerStrip from './MixerStrip.svelte'

  const { project, ui } = useStores()
  const t = ui.t
</script>

<div class="mixer" data-testid="mixer">
  {#if project.tracks.length === 0}
    <p class="empty" data-testid="mixer-empty">{t('mixer.empty')}</p>
  {/if}
  <ul class="strips" aria-label={t('mixer.strips')}>
    {#each project.tracks as entry (entry.id)}
      <MixerStrip {entry} selected={project.selected?.id === entry.id} soloed={project.soloed.includes(entry.id)} armed={project.armed.includes(entry.id)} />
    {/each}
  </ul>
  <ul class="master" aria-label={t('mixer.master')}>
    <MixerStrip />
  </ul>
</div>

<style>
  .mixer {
    display: flex;
    align-items: flex-start;
    gap: var(--fk-space-3);
    height: 100%;
    min-height: 0;
    padding: var(--fk-space-3);
    overflow: auto;
  }
  .strips {
    flex: 1;
    min-width: 0;
    display: flex;
    gap: var(--fk-space-2);
    margin: 0;
    padding: 0;
    overflow-x: auto;
  }
  .master {
    flex: none;
    margin: 0;
    padding: 0;
  }
  .empty {
    position: absolute;
    margin: 0;
    color: var(--fk-text-muted);
  }
</style>
