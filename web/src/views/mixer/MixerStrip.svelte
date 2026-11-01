<script lang="ts">
  /**
   * One strip of the mixer: a track's (colour, name, devices, arm, mute, solo, pan, fader, meter)
   * or the master's (its devices, pan, fader and meter: no command changes the master's mute). It holds
   * no project state: it reads the stores and every control is a command through the dispatcher.
   * The name is read-only here; renaming stays in the track list.
   */
  import { chainOf, masterChainOf } from '../../lib/project/chain'
  import { formatGain, formatPan, instrumentName, NO_COLOR, trackLabel, type TrackEntry } from '../../lib/project/tracks'
  import { useStores } from '../../lib/stores'
  import { MASTER } from '../../lib/stores/automated.svelte'
  import { createLane, createMasterLane } from '../../lib/automation-commands'
  import { armTrack, selectTrack, setMasterVolume, setMute, setSolo, setVolume } from '../../lib/track-commands'
  import type { MeterSnap } from './ballistics'
  import MixerFader from './MixerFader.svelte'
  import PanControl from '../../lib/components/PanControl.svelte'
  import MixerMeter from './MixerMeter.svelte'

  let {
    entry = null,
    snap,
    selected = false,
    soloed = false,
    armed = false,
    onreset,
  }: { entry?: TrackEntry | null; snap: MeterSnap; selected?: boolean; soloed?: boolean; armed?: boolean; onreset: () => void } = $props()

  const { project, ui, dispatcher, automated } = useStores()
  const t = ui.t

  // The controls whose readouts open the field to type a value.
  let panControl: ReturnType<typeof PanControl> | undefined = $state()
  let faderControl: ReturnType<typeof MixerFader> | undefined = $state()

  const name = $derived(entry ? trackLabel(entry, t) : t('mixer.master'))
  const mixer = $derived(entry ? entry.track.mixer : project.view?.project.master)
  // A lane that drives the volume or the pan moves the control with the play position, and marks it (AM-4, #388).
  const shownGain = $derived(automated.shownGain(entry ? entry.id : MASTER, mixer?.gain_db))
  const shownPan = $derived(automated.shownPan(entry ? entry.id : MASTER, mixer?.pan ?? 0))
  const gainMark = $derived(automated.mixerMark(entry ? entry.id : MASTER, 'gain'))
  const panMark = $derived(automated.mixerMark(entry ? entry.id : MASTER, 'pan'))
  const chain = $derived(entry ? chainOf(entry.track) : masterChainOf(project.view?.project.master_devices))
  const chainText = $derived(chain.length === 0 ? t('mixer.noDevice') : chain.map((e) => e.descriptor?.name ?? t('mixer.unknownDevice')).join(' › '))

  /** Shows the track's devices: selects it (the Devices panel follows the selection) and opens that panel. */
  function showDevices() {
    if (!entry) {
      // The master is not a track: the panel shows its chain without touching the selection.
      ui.showDevices(true, project.selected?.id)
      return
    }
    if (project.selected?.id !== entry.id) dispatcher.run(selectTrack(entry.id))
    ui.showDevices(false)
  }

  const select = () => {
    if (entry && !selected) dispatcher.run(selectTrack(entry.id))
    if (entry) ui.masterSelection = null
  }

  /** Whether the strip's owner (the track, or the master) already has a lane on `param` (Automate is then a navigate-only action). */
  function hasLane(param: 'gain' | 'pan'): boolean {
    const lanes = entry ? entry.track.automation : project.view?.project.master_automation
    return Object.values(lanes ?? {}).some((lane) => lane && 'mixer' in lane.target && lane.target.mixer.param === param)
  }

  /** Automate the volume or the pan: create the lane if absent, then navigate to the arrangement and unfold the owner's lanes. */
  function automate(param: 'gain' | 'pan') {
    if (!hasLane(param)) dispatcher.run(entry ? createLane(entry.id, { mixer: { param } }) : createMasterLane({ mixer: { param } }))
    ui.showLanes(entry ? entry.id : MASTER)
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
      <button
        class="devices"
        type="button"
        title={t('mixer.devicesHint', { name })}
        aria-label={t('mixer.devicesHint', { name })}
        data-testid="mixer-devices"
        onclick={showDevices}
      >
        {t('mixer.devices')} ({chain.length})
      </button>
    {:else}
      <p class="name master-name">{name}</p>
      <p class="meta" title={chainText} data-testid="mixer-chain">{chainText}</p>
      <button
        class="devices"
        type="button"
        title={t('mixer.devicesHint', { name })}
        aria-label={t('mixer.devicesHint', { name })}
        data-testid="mixer-master-devices"
        onclick={showDevices}
      >
        {t('mixer.devices')} ({chain.length})
      </button>
    {/if}
  </div>

  <div class="pan">
    <PanControl
      bind:this={panControl}
      track={entry ? entry.id : null}
      pan={shownPan}
      label={entry ? t('tracks.pan', { name }) : t('mixer.masterPan')}
      automated={panMark}
      testid={entry ? 'mixer-pan' : 'mixer-master-pan'}
    />
    <button
      class="readout"
      type="button"
      tabindex="-1"
      title={t('controls.type')}
      data-testid={entry ? 'mixer-pan-readout' : 'mixer-master-pan-readout'}
      onmousedown={(e) => e.preventDefault()}
      onclick={() => panControl?.edit()}>{formatPan(shownPan)}</button
    >
    <button
      class="automate"
      type="button"
      title={t('automation.automateHint')}
      aria-label={t('automation.automateParam', { param: entry ? t('tracks.pan', { name }) : t('mixer.masterPan') })}
      data-testid={entry ? 'mixer-automate-pan' : 'mixer-master-automate-pan'}
      onclick={() => automate('pan')}
    >
      {t('automation.automate')}
    </button>
  </div>

  {#if entry}
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
          onclick={() => dispatcher.run(armTrack(entry.id, !armed))}>In</button
        >
      {/if}
    </div>
  {/if}

  <div class="levels">
    <MixerFader
      bind:this={faderControl}
      gainDb={shownGain}
      label={entry ? t('tracks.volume', { name }) : t('mixer.masterVolume')}
      set={entry ? (gain) => setVolume(entry.id, gain) : setMasterVolume}
      automated={gainMark}
      testid={entry ? 'mixer-fader' : 'mixer-master-fader'}
    />
    <MixerMeter {snap} label={t('mixer.level', { name })} peakLabel={t('mixer.peak', { name })} clipLabel={t('mixer.clip')} {onreset} />
  </div>
  <button
    class="readout gain"
    type="button"
    tabindex="-1"
    title={t('controls.type')}
    data-testid="mixer-gain"
    onmousedown={(e) => e.preventDefault()}
    onclick={() => faderControl?.edit()}>{formatGain(shownGain)} dB</button
  >
  <button
    class="automate"
    type="button"
    title={t('automation.automateHint')}
    aria-label={t('automation.automateParam', { param: entry ? t('tracks.volume', { name }) : t('mixer.masterVolume') })}
    data-testid={entry ? 'mixer-automate-gain' : 'mixer-master-automate-gain'}
    onclick={() => automate('gain')}
  >
    {t('automation.automate')}
  </button>
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
  .name:hover,
  .automate:hover {
    background: var(--fk-surface-2);
  }
  .automate {
    align-self: stretch;
    height: calc(var(--fk-row) - 6px);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-work);
    background: transparent;
    font-size: 10px;
    cursor: pointer;
  }
  .pan {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    width: 100%;
  }
  .readout {
    border: 0;
    padding: 0;
    background: transparent;
    cursor: text;
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
    background: var(--fk-text);
    border-color: var(--fk-text);
    color: var(--fk-bg);
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
