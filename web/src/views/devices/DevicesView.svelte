<script lang="ts">
  /**
   * The device panel: the selected track's chain (instrument first, then the effects) as cards
   * generated from the descriptors, and the browser to add devices. It writes nothing per device.
   */
  import { chainOf } from '../../lib/project/chain'
  import { isMidi, trackLabel } from '../../lib/project/tracks'
  import { useStores } from '../../lib/stores'
  import DeviceBrowser from './DeviceBrowser.svelte'
  import DeviceCard from './DeviceCard.svelte'

  const { project, ui } = useStores()
  const t = ui.t

  const selected = $derived(project.selected)
  const chain = $derived(selected ? chainOf(selected.track) : [])
</script>

{#if selected}
  <div class="devices" data-testid="devices">
    <DeviceBrowser track={selected.id} midi={isMidi(selected.track)} {chain} />
    <div class="chain" role="group" aria-label={t('devices.chain', { name: trackLabel(selected, t) })} data-testid="device-chain">
      {#each chain as entry (entry.id)}
        <DeviceCard {entry} {chain} />
      {/each}
    </div>
  </div>
{:else}
  <p class="empty" data-testid="devices-empty">{t('devices.noTrack')}</p>
{/if}

<style>
  .devices {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .chain {
    flex: 1;
    min-width: 0;
    display: flex;
    gap: var(--fk-space-3);
    align-items: flex-start;
    padding: var(--fk-space-3);
    overflow: auto;
  }
  .empty {
    margin: 0;
    padding: var(--fk-space-4);
    color: var(--fk-text-muted);
  }
</style>
