<script lang="ts">
  /**
   * One device of the chain: its name, bypass, and for an effect (audio or MIDI) its place in the chain and its
   * removal, its presets (#191), then one control per parameter of its descriptor (`ParamControl`). Generic: no device
   * has a panel of its own. Every action is a command (`device-commands.ts`).
   */
  import { onDestroy } from 'svelte'
  import ParamControl from '../../lib/components/ParamControl.svelte'
  import ReductionMeter from '../../lib/components/ReductionMeter.svelte'
  import PresetBar from './PresetBar.svelte'
  import { setDeviceParam } from '../../lib/device-commands'
  import { createLane, createMasterLane } from '../../lib/automation-commands'
  import { createGesture } from '../../lib/gesture'
  import { cardActions, type ChainEntry } from '../../lib/project/chain'
  import { deviceText, labelKey, shownParams } from '../../lib/devices'
  import { MASTER_METER, useStores } from '../../lib/stores'
  import type { MenuItem } from '../../lib/context-menu'

  let { entry, chain, master = false, track }: { entry: ChainEntry; chain: readonly ChainEntry[]; master?: boolean; track?: string } = $props()

  const { ui, dispatcher, automated, project, meters } = useStores()
  const t = ui.t

  const descriptor = $derived(entry.descriptor)
  const name = $derived(descriptor?.name ?? entry.kind ?? t('devices.unknown'))
  const bypassed = $derived(!entry.device.enabled)
  // An audio or a MIDI effect moves (within its section) and is removed; the instrument does neither.
  const isEffect = $derived(entry.section !== 'instrument')
  const actions = $derived(cardActions(chain, entry))
  // The Compressor shows the gain it takes off (the engine reports the largest reduction of the strip's enabled compressors).
  const reduces = $derived(entry.kind === 'faulk.compressor')
  const reduction = $derived(bypassed ? 0 : meters.snap(master ? MASTER_METER : (track ?? MASTER_METER)).reductionDb)

  // A knob drag is one history group: one undo takes it back whole.
  const gesture = createGesture((command, options) => dispatcher.run(command, options))
  onDestroy(() => gesture.end())

  /** The right click on the card (#524): bypass, and for an effect its place in the chain and its removal. A control inside answers for itself. */
  function onContextMenu(event: MouseEvent) {
    if (event.defaultPrevented) return
    const items: MenuItem[] = [
      { id: 'bypass', label: t('devices.bypass', { name }), detail: bypassed ? '✓' : undefined, run: () => dispatcher.run(actions.bypass) },
    ]
    if (isEffect) {
      items.push(
        {
          id: 'move-left',
          label: t('devices.moveLeft', { name }),
          disabled: actions.moveLeft === null,
          separator: true,
          run: () => actions.moveLeft && dispatcher.run(actions.moveLeft),
        },
        {
          id: 'move-right',
          label: t('devices.moveRight', { name }),
          disabled: actions.moveRight === null,
          run: () => actions.moveRight && dispatcher.run(actions.moveRight),
        },
        { id: 'remove', label: t('devices.remove', { name }), separator: true, run: () => dispatcher.run(actions.remove) },
      )
    }
    ui.openMenu(event, items, name)
  }

  /** Whether the owner (the master, or the track) already has a lane on a device parameter (the Automate button is then a navigate-only action). */
  function hasDeviceLane(paramKey: string): boolean {
    const lanes = master ? project.view?.project.master_automation : track ? project.tracks.find((e) => e.id === track)?.track.automation : undefined
    return Object.values(lanes ?? {}).some(
      (lane) => lane && 'device' in lane.target && lane.target.device.device === entry.id && lane.target.device.param === paramKey,
    )
  }

  /** Automate a device parameter: create the lane if absent, then navigate to the arrangement and unfold the owner's lanes. */
  function automateParam(paramKey: string) {
    const target = { device: { device: entry.id, param: paramKey } }
    if (master) {
      if (!hasDeviceLane(paramKey)) dispatcher.run(createMasterLane(target))
      ui.showMasterLanes()
    } else if (track) {
      if (!hasDeviceLane(paramKey)) dispatcher.run(createLane(track, target))
      ui.showLanes(track)
    }
  }
</script>

<article
  class="card"
  class:bypassed
  oncontextmenu={onContextMenu}
  data-testid="device-card"
  data-kind={entry.kind}
  data-role={entry.section === 'midi' ? 'midi_effect' : entry.section}
  data-bypassed={bypassed}
>
  <header>
    <h3 title={name}>{name}</h3>
    <div class="actions">
      <button
        aria-label={t('devices.bypass', { name })}
        aria-pressed={bypassed}
        title={t('devices.bypass', { name })}
        data-testid="device-bypass"
        onclick={() => dispatcher.run(actions.bypass)}
      >
        <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true"
          ><path d="M7 1 V 7 M3.8 3.2 A 5 5 0 1 0 10.2 3.2" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg
        >
      </button>
      {#if isEffect}
        <button
          aria-label={t('devices.moveLeft', { name })}
          title={t('devices.moveLeft', { name })}
          data-testid="device-left"
          disabled={actions.moveLeft === null}
          onclick={() => actions.moveLeft && dispatcher.run(actions.moveLeft)}
        >
          <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true"
            ><path d="M9 2 L 4 7 L 9 12" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg
          >
        </button>
        <button
          aria-label={t('devices.moveRight', { name })}
          title={t('devices.moveRight', { name })}
          data-testid="device-right"
          disabled={actions.moveRight === null}
          onclick={() => actions.moveRight && dispatcher.run(actions.moveRight)}
        >
          <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true"
            ><path d="M5 2 L 10 7 L 5 12" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg
          >
        </button>
        <button
          aria-label={t('devices.remove', { name })}
          title={t('devices.remove', { name })}
          data-testid="device-remove"
          onclick={() => dispatcher.run(actions.remove)}
        >
          <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true"
            ><path d="M3 3 L 11 11 M11 3 L 3 11" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg
          >
        </button>
      {/if}
    </div>
  </header>
  {#if descriptor}
    <PresetBar device={entry.id} {name} />
  {/if}
  {#if reduces}
    <ReductionMeter reductionDb={reduction} label={t('devices.gainReduction')} />
  {/if}
  {#if descriptor}
    <div class="params">
      {#each shownParams(descriptor, (key) => automated.device(entry.id, key) ?? entry.device.params[key]) as param (param.key)}
        <div class="param-wrap">
          <ParamControl
            device={descriptor}
            {param}
            value={automated.device(entry.id, param.key) ?? entry.device.params[param.key] ?? param.default}
            automated={automated.deviceMark(entry.id, param.key)}
            onchange={(value) => dispatcher.run(setDeviceParam(entry.id, param.key, value))}
            onbegin={gesture.begin}
            onend={gesture.end}
          />
          {#if master || track}
            <button
              class="param-automate"
              type="button"
              title={t('automation.automateHint')}
              aria-label={t('automation.automateParam', { param: deviceText(ui.lang, labelKey(descriptor, param.key)) ?? param.key })}
              data-testid="device-param-automate"
              onclick={() => automateParam(param.key)}
            >
              {t('automation.automate')}
            </button>
          {/if}
        </div>
      {/each}
    </div>
  {:else}
    <p class="muted">{t('devices.noParams')}</p>
  {/if}
</article>

<style>
  .card {
    flex: 0 1 auto;
    min-width: 0;
    max-width: 100%;
    display: flex;
    flex-direction: column;
    gap: var(--fk-space-3);
    padding: var(--fk-space-3) var(--fk-space-4);
    border-radius: var(--fk-radius-panel);
    border: 1px solid var(--fk-line);
    background: var(--fk-bg);
  }
  .card.bypassed .params {
    opacity: 0.45;
  }
  header {
    display: flex;
    align-items: center;
    gap: var(--fk-space-2);
  }
  h3 {
    margin: 0;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
    font-weight: 600;
  }
  .actions {
    display: flex;
    gap: 2px;
  }
  .actions button {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    padding: 0;
    border: 0;
    border-radius: var(--fk-radius-pill);
    background: transparent;
    color: var(--fk-text-muted);
    cursor: pointer;
  }
  .actions button:hover:not(:disabled) {
    background: var(--fk-surface-2);
    color: var(--fk-text);
  }
  .actions button:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .actions button[aria-pressed='true'] {
    background: var(--fk-text);
    color: var(--fk-bg);
  }
  .params {
    display: flex;
    flex-wrap: wrap;
    gap: var(--fk-space-4);
    align-items: flex-end;
  }
  .param-wrap {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
  }
  .param-automate {
    height: 18px;
    padding: 0 var(--fk-space-1);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-pill);
    background: transparent;
    color: var(--fk-text-muted);
    font-size: 11px;
    cursor: pointer;
  }
  .param-automate:hover {
    background: var(--fk-surface-2);
    color: var(--fk-text);
  }
  .muted {
    margin: 0;
    color: var(--fk-text-muted);
  }
</style>
