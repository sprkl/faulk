<script lang="ts">
  /**
   * The device panel: the selected track's chain (instrument first, then the effects) as cards
   * generated from the descriptors, and the browser to add devices. It writes nothing per device.
   */
  import { chainOf, masterChainOf } from '../../lib/project/chain'
  import { isMidi, trackLabel } from '../../lib/project/tracks'
  import { useStores } from '../../lib/stores'
  import DeviceBrowser from './DeviceBrowser.svelte'
  import DeviceCard from './DeviceCard.svelte'

  /** `compact`: a narrow column beside the piano roll: the browser on top, the cards stacked. */
  let { compact = false }: { compact?: boolean } = $props()

  const { project, ui } = useStores()
  const t = ui.t

  const selected = $derived(project.selected)
  // The master's chain shows until another track is selected (from the list, the grid or the mixer).
  const master = $derived(ui.masterSelection !== null && ui.masterSelection === (project.selected?.id ?? ''))
  const chain = $derived(master ? masterChainOf(project.view?.project.master_devices) : selected ? chainOf(selected.track) : [])
</script>

{#if master}
  <div class="devices" class:compact data-testid="devices" data-target="master">
    <DeviceBrowser track="" midi={false} {chain} master />
    <div class="chain" role="group" aria-label={t('devices.chain', { name: t('mixer.master') })} data-testid="device-chain">
      {#each chain as entry (entry.id)}
        <DeviceCard {entry} {chain} master />
      {/each}
    </div>
  </div>
{:else if selected}
  <div class="devices" class:compact data-testid="devices">
    <DeviceBrowser track={selected.id} midi={isMidi(selected.track)} {chain} />
    <div class="chain" role="group" aria-label={t('devices.chain', { name: trackLabel(selected, t) })} data-testid="device-chain">
      {#each chain as entry (entry.id)}
        <DeviceCard {entry} {chain} track={selected.id} />
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
    flex-wrap: wrap;
    align-items: flex-start;
    align-content: flex-start;
    padding: var(--fk-space-3);
    overflow: auto;
  }
  .compact.devices {
    flex-direction: column;
    height: auto;
    min-height: 100%;
  }
  .compact :global(.browser) {
    display: none;
  }
  .compact .chain {
    flex: none;
    flex-direction: column;
    flex-wrap: nowrap;
    align-items: stretch;
    overflow: visible;
  }
  .empty {
    margin: 0;
    padding: var(--fk-space-4);
    color: var(--fk-text-muted);
  }
</style>
