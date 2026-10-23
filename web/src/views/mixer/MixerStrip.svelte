<script lang="ts">
  /**
   * One strip of the mixer: a track's (colour, name, devices, arm, mute, solo, pan, fader, meter)
   * or the master's (fader and meter only: no command changes the master's pan or mute). It holds
   * no project state: it reads the stores and every control is a command through the dispatcher.
   * The name is read-only here; renaming stays in the track list.
   */
  import { onDestroy } from 'svelte'
  import { createGesture } from '../../lib/gesture'
  import { chainOf } from '../../lib/project/chain'
  import { formatGain, formatPan, instrumentName, NO_COLOR, trackLabel, type TrackEntry } from '../../lib/project/tracks'
  import { useStores } from '../../lib/stores'
  import { armTrack, selectTrack, setMasterVolume, setMute, setPan, setSolo, setVolume } from '../../lib/track-commands'
  import type { MeterSnap } from './ballistics'
  import MixerFader from './MixerFader.svelte'
  import MixerMeter from './MixerMeter.svelte'

  let {
    entry = null,
    snap,
    selected = false,
    soloed = false,
    armed = false,
    onreset,
  }: { entry?: TrackEntry | null; snap: MeterSnap; selected?: boolean; soloed?: boolean; armed?: boolean; onreset: () => void } = $props()

  const { project, ui, dispatcher } = useStores()
  const t = ui.t

  const name = $derived(entry ? trackLabel(entry, t) : t('mixer.master'))
  const mixer = $derived(entry ? entry.track.mixer : project.view?.project.master)
  const chain = $derived(entry ? chainOf(entry.track) : [])
  const chainText = $derived(
    chain.length === 0 ? t('mixer.noDevice') : chain.map((e) => e.descriptor?.name ?? t('mixer.unknownDevice')).join(' › '),
  )

  const gesture = createGesture((command) => dispatcher.run(command))
  onDestroy(() => gesture.end())

  /** Shows the track's devices: selects it (the Devices panel follows the selection) and opens that panel. */
  function showDevices() {
    if (!entry) return
    if (project.selected?.id !== entry.id) dispatcher.run(selectTrack(entry.id))
    ui.showPanel('devices')
  }

  const select = () => {
    if (entry && !selected) dispatcher.run(selectTrack(entry.id))
  }
</script>

<li
  class="strip"
  class:master={!entry}
  class:selected
  class:muted={mixer?.mute}
  style:--track-color={entry ? (entry.track.color ?? NO_COLOR) : 'var(--fk-text)'}
  aria-label={t('mixer.strip', { name })}
  aria-current={selected ? 'true' : undefined}
  data-testid={entry ? 'mixer-strip' : 'mixer-master'}
  data-track={entry?.id}
>
  <div class="head">
    {#if entry}
      <button class="name" type="button" title={name} data-testid="mixer-name" onclick={select}>{name}</button>
      <p class="meta" title={chainText} data-testid="mixer-chain">{instrumentName(entry.track) ?? chainText}</p>
      <button class="devices" type="button" title={t('mixer.devicesHint', { name })} aria-label={t('mixer.devicesHint', { name })} data-testid="mixer-devices" onclick={showDevices}>
        {t('mixer.devices')} ({chain.length})
      </button>
    {:else}
      <p class="name master-name">{name}</p>
    {/if}
  </div>

  {#if entry}
    <div class="pan">
      <input
        type="range"
        min="-1"
        max="1"
        step="0.01"
        value={entry.track.mixer.pan}
        aria-label={t('tracks.pan', { name })}
        aria-valuetext={formatPan(entry.track.mixer.pan)}
        data-testid="mixer-pan"
        oninput={(e) => dispatcher.run(setPan(entry.id, Number(e.currentTarget.value)))}
        onpointerdown={gesture.begin}
        onpointerup={gesture.end}
        onpointercancel={gesture.end}
        onchange={gesture.end}
        onblur={gesture.end}
        ondblclick={() => dispatcher.run(setPan(entry.id, 0))}
      />
      <span class="readout">{formatPan(entry.track.mixer.pan)}</span>
    </div>
    <div class="toggles">
      <button
        class="toggle mute"
        type="button"
        aria-pressed={mixer?.mute ?? false}
        aria-label={t('tracks.mute', { name })}
        title={t('tracks.mute', { name })}
        data-testid="mixer-mute"
        onclick={() => dispatcher.run(setMute(entry.id, !mixer?.mute))}>M</button
      >
      <button
        class="toggle solo"
        type="button"
        aria-pressed={soloed}
        aria-label={t('tracks.solo', { name })}
        title={t('tracks.solo', { name })}
        data-testid="mixer-solo"
        onclick={() => dispatcher.run(setSolo(entry.id, !soloed))}>S</button
      >
      {#if entry.track.kind === 'midi'}
        <button
          class="toggle arm"
          type="button"
          aria-pressed={armed}
          aria-label={t('tracks.arm', { name })}
          title={t('tracks.armHint', { name })}
          data-testid="mixer-arm"
          onclick={() => dispatcher.run(armTrack(entry.id, !armed))}>R</button
        >
      {/if}
    </div>
  {/if}

  <div class="levels">
    <MixerFader
      gainDb={mixer?.gain_db}
      label={entry ? t('tracks.volume', { name }) : t('mixer.masterVolume')}
      set={entry ? (gain) => setVolume(entry.id, gain) : setMasterVolume}
      testid={entry ? 'mixer-fader' : 'mixer-master-fader'}
    />
    <MixerMeter {snap} label={t('mixer.level', { name })} peakLabel={t('mixer.peak', { name })} clipLabel={t('mixer.clip')} {onreset} />
  </div>
  <span class="readout gain" data-testid="mixer-gain">{formatGain(mixer?.gain_db)} dB</span>
</li>

<style>
  .strip {
    list-style: none;
    flex: none;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--fk-space-2);
    width: 112px;
    padding: var(--fk-space-2);
    border: 1px solid var(--fk-line);
    border-top: 4px solid var(--track-color);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-bg);
  }
  .strip.selected {
    background: var(--fk-accent-tint);
    border-color: var(--fk-accent-fg);
    border-top-color: var(--track-color);
  }
  .strip.master {
    width: 128px;
    background: var(--fk-surface);
  }
  .head {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 2px;
    width: 100%;
    min-width: 0;
  }
  .name {
    height: var(--fk-row);
    padding: 0 var(--fk-space-1);
    border: 1px solid transparent;
    border-radius: var(--fk-radius-work);
    background: transparent;
    font-weight: 600;
    text-align: center;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
  }
  .master-name {
    margin: 0;
    line-height: var(--fk-row);
    cursor: default;
  }
  .strip.muted .name,
  .strip.muted .meta {
    opacity: 0.55;
  }
  .meta {
    margin: 0;
    color: var(--fk-text-muted);
    font-size: 11px;
    text-align: center;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .devices {
    height: var(--fk-row);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-work);
    background: transparent;
    font-size: 11px;
    cursor: pointer;
  }
  .devices:hover,
  .name:hover {
    background: var(--fk-surface-2);
  }
  .pan {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    width: 100%;
  }
  .pan input {
    width: 100%;
    min-width: 0;
    margin: 0;
    accent-color: var(--fk-accent-fg);
  }
  .readout {
    color: var(--fk-text-muted);
    font-family: var(--fk-font-mono);
    font-size: 11px;
    text-align: center;
  }
  .toggles {
    display: flex;
    gap: var(--fk-space-1);
  }
  .toggle {
    width: var(--fk-row);
    height: var(--fk-row);
    padding: 0;
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-work);
    background: transparent;
    font-weight: 700;
    font-size: 11px;
    cursor: pointer;
  }
  .toggle:hover {
    background: var(--fk-surface-2);
  }
  .mute[aria-pressed='true'] {
    background: var(--fk-mute);
    border-color: var(--fk-mute);
    color: #131211;
  }
  .solo[aria-pressed='true'] {
    background: var(--fk-solo);
    border-color: var(--fk-solo);
    color: #131211;
  }
  .arm[aria-pressed='true'] {
    background: var(--fk-rec);
    border-color: var(--fk-rec);
    color: #131211;
  }
  .levels {
    display: flex;
    align-items: flex-start;
    gap: var(--fk-space-2);
  }
  .gain {
    min-width: 8ch;
  }
</style>
