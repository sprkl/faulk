<script lang="ts">
  /**
   * The device panel: the selected track's chain (instrument first, then the effects) as cards
   * generated from the descriptors, and the browser to add devices. It writes nothing per device.
   * A device or a preset dragged out of the device browser (#190) is dropped on the chain: an effect lands where the
   * mark shows, an instrument takes the instrument's place.
   */
  import { dropTarget, insertionIndex, type CardBox } from '../../lib/browser/dnd'
  import { putItem } from '../../lib/browser/put'
  import { effectCount, chainOf, masterChainOf } from '../../lib/project/chain'
  import { isMidi, trackLabel } from '../../lib/project/tracks'
  import { useStores } from '../../lib/stores'
  import DeviceBrowser from './DeviceBrowser.svelte'
  import DeviceCard from './DeviceCard.svelte'

  /** `compact`: a narrow column beside the piano roll: the browser on top, the cards stacked. */
  let { compact = false }: { compact?: boolean } = $props()

  const stores = useStores()
  const { project, ui } = stores
  const t = ui.t

  const selected = $derived(project.selected)
  // The master's chain shows until another track is selected (from the list, the grid or the mixer).
  const master = $derived(ui.masterSelection !== null && ui.masterSelection === (project.selected?.id ?? ''))
  const chain = $derived(master ? masterChainOf(project.view?.project.master_devices) : selected ? chainOf(selected.track) : [])

  let chainEl = $state<HTMLElement | null>(null)
  /** Among the effects, where the dragged effect would land; null when nothing is dragged over the chain. */
  let insertAt = $state<number | null>(null)

  /** The boxes of the effect cards, in chain order. */
  function boxes(): CardBox[] {
    const cards = chainEl?.querySelectorAll<HTMLElement>('[data-testid="device-card"][data-role="effect"]') ?? []
    return Array.from(cards, (card, index) => {
      const r = card.getBoundingClientRect()
      return { index, left: r.left, top: r.top, right: r.right, bottom: r.bottom }
    })
  }

  const where = (event: DragEvent) => insertionIndex(boxes(), event.clientX, event.clientY)

  function dropOnChain(item: Parameters<typeof putItem>[1], event: DragEvent) {
    const index = item.category === 'instrument' ? undefined : where(event)
    insertAt = null
    putItem(stores, item, master ? { type: 'master', index } : { type: 'track', track: selected!.id, index })
  }
</script>

{#snippet cards(track: string | undefined)}
  {#each chain as entry (entry.id)}
    {#if insertAt !== null && entry.effectIndex === insertAt}<span class="mark" aria-hidden="true" data-testid="insert-mark"></span>{/if}
    <DeviceCard {entry} {chain} {track} {master} />
  {/each}
  {#if insertAt !== null && insertAt >= effectCount(chain)}<span class="mark" aria-hidden="true" data-testid="insert-mark"></span>{/if}
{/snippet}

{#if master}
  <div class="devices" class:compact data-testid="devices" data-target="master">
    <DeviceBrowser track="" midi={false} {chain} master />
    <div
      class="chain"
      role="group"
      aria-label={t('devices.chain', { name: t('mixer.master') })}
      data-testid="device-chain"
      bind:this={chainEl}
      use:dropTarget={{
        ui,
        accepts: (item) => item.category !== 'instrument',
        move: (event) => (insertAt = where(event)),
        over: (over) => !over && (insertAt = null),
        drop: dropOnChain,
      }}
    >
      {@render cards(undefined)}
    </div>
  </div>
{:else if selected}
  <div class="devices" class:compact data-testid="devices">
    <DeviceBrowser track={selected.id} midi={isMidi(selected.track)} {chain} />
    <div
      class="chain"
      role="group"
      aria-label={t('devices.chain', { name: trackLabel(selected, t) })}
      data-testid="device-chain"
      bind:this={chainEl}
      use:dropTarget={{
        ui,
        accepts: () => true,
        move: (event) => (insertAt = ui.dragged?.category === 'instrument' ? null : where(event)),
        over: (over) => !over && (insertAt = null),
        drop: dropOnChain,
      }}
    >
      {@render cards(selected.id)}
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
  /* A device or a preset of the browser over the chain (#190), and where an effect would land. */
  .chain:global([data-drop-over]) {
    outline: 2px solid var(--fk-accent-fg);
    outline-offset: -2px;
  }
  .mark {
    align-self: stretch;
    flex: none;
    width: 3px;
    min-height: 48px;
    margin: 0 calc(var(--fk-space-2) * -1);
    border-radius: 2px;
    background: var(--fk-accent-fg);
  }
  .compact.devices {
    flex-direction: column;
    height: auto;
    min-height: 100%;
  }
  .compact .mark {
    width: auto;
    min-height: 0;
    height: 3px;
    margin: calc(var(--fk-space-2) * -1) 0;
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
