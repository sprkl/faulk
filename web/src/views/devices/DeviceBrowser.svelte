<script lang="ts">
  /**
   * The browser of instruments and effects, from the device descriptors (`DEVICES`): a new device
   * is listed with no change here. An instrument replaces the track's instrument
   * (`track.set_instrument`); an effect is added at the end of the chain (`device.insert`).
   */
  import { effectDescriptors, instrumentDescriptors, type DeviceDescriptor } from '../../lib/devices'
  import { setInstrument } from '../../lib/device-commands'
  import { addEffectCommand, addMasterEffectCommand, type ChainEntry } from '../../lib/project/chain'
  import { useStores } from '../../lib/stores'

  let {
    track,
    midi,
    chain,
    descriptors,
    master = false,
  }: {
    /** The track whose chain it is; ignored for the master. */
    track: string
    /** The browser of the master's chain: effects only, added with `master: true`. */
    master?: boolean
    /** Whether the track is a MIDI track: an audio track keeps its clip player, no instrument is chosen. */
    midi: boolean
    chain: readonly ChainEntry[]
    /** The devices on offer; the registry's by default. */
    descriptors?: readonly DeviceDescriptor[]
  } = $props()

  const { project, ui, dispatcher } = useStores()
  const t = ui.t

  const instruments = $derived(instrumentDescriptors(descriptors))
  const effects = $derived(effectDescriptors(descriptors))
  const current = $derived(chain.find((e) => e.effectIndex === null)?.kind ?? null)

  // A new instrument takes the automation lanes of the old one with it (`track.set_instrument`; undo brings them back): the warning is
  // shown before the swap, and the count of what it took after it. Both come from the project at the moment of the click.
  const instrumentId = $derived(chain.find((e) => e.effectIndex === null)?.id ?? null)
  const lanesOnInstrument = $derived.by(() => {
    const lanes = project.tracks.find((e) => e.id === track)?.track.automation ?? {}
    return Object.values(lanes).filter((lane) => lane && 'device' in lane.target && lane.target.device.device === instrumentId).length
  })
  let removed = $state({ track: '', n: 0 })
  function pick(kind: string) {
    const n = kind === current ? 0 : lanesOnInstrument
    dispatcher.run(setInstrument(track, kind))
    removed = { track, n }
  }
</script>

<nav class="browser" aria-label={master ? t('devices.browserMaster') : t('devices.browser')} data-testid="device-browser">
  {#if master}
    <p class="muted" data-testid="browse-master-hint">{t('devices.masterHint')}</p>
  {:else}
    <h3 class="label">{t('devices.instruments')}</h3>
  {/if}
  {#if master}
    <!-- the master holds effects only -->
  {:else if midi}
    <ul>
      {#each instruments as d (d.kind)}
        <li>
          <button
            aria-pressed={current === d.kind}
            aria-label={t('devices.use', { name: d.name })}
            title={t('devices.use', { name: d.name })}
            data-testid="browse-instrument"
            data-kind={d.kind}
            onclick={() => pick(d.kind)}>{d.name}</button
          >
        </li>
      {/each}
    </ul>
  {:else}
    <p class="muted" data-testid="browse-audio">{t('devices.audioTrack')}</p>
  {/if}
  {#if !master && midi && lanesOnInstrument > 0}
    <p class="warn" role="note" data-testid="instrument-lanes-warning">{t('automation.instrumentLanes', { n: lanesOnInstrument })}</p>
  {:else if !master && midi && removed.track === track && removed.n > 0}
    <p class="warn" role="status" data-testid="instrument-lanes-removed">
      {t('automation.instrumentLanesRemoved', { n: removed.n })}
      <button type="button" class="dismiss" onclick={() => (removed = { track: '', n: 0 })}>{t('automation.dismiss')}</button>
    </p>
  {/if}
  <h3 class="label">{t('devices.effects')}</h3>
  {#if effects.length === 0}
    <p class="muted" data-testid="browse-no-effects">{t('devices.noEffects')}</p>
  {:else}
    <ul>
      {#each effects as d (d.kind)}
        <li>
          <button
            aria-label={master ? t('devices.addMaster', { name: d.name }) : t('devices.add', { name: d.name })}
            title={master ? t('devices.addMaster', { name: d.name }) : t('devices.add', { name: d.name })}
            data-testid="browse-effect"
            data-kind={d.kind}
            onclick={() => dispatcher.run(master ? addMasterEffectCommand(d.kind, chain) : addEffectCommand(track, d.kind, chain))}>{d.name}</button
          >
        </li>
      {/each}
    </ul>
  {/if}
</nav>

<style>
  .browser {
    flex: none;
    width: 168px;
    overflow-y: auto;
    padding: var(--fk-space-3);
    border-right: 1px solid var(--fk-line);
    display: flex;
    flex-direction: column;
    gap: var(--fk-space-2);
  }
  h3 {
    margin: 0;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  li button {
    width: 100%;
    min-height: var(--fk-row);
    padding: 0 var(--fk-space-3);
    border: 0;
    border-radius: var(--fk-radius-pill);
    background: transparent;
    color: var(--fk-text);
    text-align: left;
    cursor: pointer;
  }
  li button:hover {
    background: var(--fk-surface-2);
  }
  li button[aria-pressed='true'] {
    background: var(--fk-accent-tint);
    color: var(--fk-accent-fg);
  }
  .muted {
    margin: 0;
    color: var(--fk-text-muted);
  }
  .warn {
    margin: 0;
    padding: var(--fk-space-2);
    border: 1px solid var(--fk-accent-fg);
    border-radius: var(--fk-radius-work);
    background: var(--fk-accent-tint);
    font-size: 12px;
  }
  .dismiss {
    display: block;
    margin-top: var(--fk-space-1);
    padding: 0 var(--fk-space-2);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-work);
    background: transparent;
    color: inherit;
    cursor: pointer;
  }
</style>
