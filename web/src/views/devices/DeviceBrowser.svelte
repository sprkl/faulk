<script lang="ts">
  /**
   * The browser of instruments and effects, from the device descriptors (`DEVICES`): a new device
   * is listed with no change here. An instrument replaces the track's instrument
   * (`track.set_instrument`); an effect is added at the end of the chain (`device.insert`).
   */
  import { effectDescriptors, instrumentDescriptors, type DeviceDescriptor } from '../../lib/devices'
  import { setInstrument } from '../../lib/device-commands'
  import { addEffectCommand, type ChainEntry } from '../../lib/project/chain'
  import { useStores } from '../../lib/stores'

  let {
    track,
    midi,
    chain,
    descriptors,
  }: {
    track: string
    /** Whether the track is a MIDI track: an audio track keeps its clip player, no instrument is chosen. */
    midi: boolean
    chain: readonly ChainEntry[]
    /** The devices on offer; the registry's by default. */
    descriptors?: readonly DeviceDescriptor[]
  } = $props()

  const { ui, dispatcher } = useStores()
  const t = ui.t

  const instruments = $derived(instrumentDescriptors(descriptors))
  const effects = $derived(effectDescriptors(descriptors))
  const current = $derived(chain.find((e) => e.effectIndex === null)?.kind ?? null)
</script>

<nav class="browser" aria-label={t('devices.browser')} data-testid="device-browser">
  <h3 class="label">{t('devices.instruments')}</h3>
  {#if midi}
    <ul>
      {#each instruments as d (d.kind)}
        <li>
          <button
            aria-pressed={current === d.kind}
            aria-label={t('devices.use', { name: d.name })}
            title={t('devices.use', { name: d.name })}
            data-testid="browse-instrument"
            data-kind={d.kind}
            onclick={() => dispatcher.run(setInstrument(track, d.kind))}>{d.name}</button
          >
        </li>
      {/each}
    </ul>
  {:else}
    <p class="muted" data-testid="browse-audio">{t('devices.audioTrack')}</p>
  {/if}
  <h3 class="label">{t('devices.effects')}</h3>
  {#if effects.length === 0}
    <p class="muted" data-testid="browse-no-effects">{t('devices.noEffects')}</p>
  {:else}
    <ul>
      {#each effects as d (d.kind)}
        <li>
          <button
            aria-label={t('devices.add', { name: d.name })}
            title={t('devices.add', { name: d.name })}
            data-testid="browse-effect"
            data-kind={d.kind}
            onclick={() => dispatcher.run(addEffectCommand(track, d.kind, chain))}>{d.name}</button
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
</style>
