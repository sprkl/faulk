<script lang="ts">
  /**
   * One device of the chain: its name, bypass, and for an effect its place in the chain and its
   * removal, then one control per parameter of its descriptor (`ParamControl`). Generic: no device
   * has a panel of its own. Every action is a command (`device-commands.ts`).
   */
  import { onDestroy } from 'svelte'
  import ParamControl from '../../lib/components/ParamControl.svelte'
  import { setDeviceParam } from '../../lib/device-commands'
  import { createLane, createMasterLane } from '../../lib/automation-commands'
  import { createGesture } from '../../lib/gesture'
  import { cardActions, type ChainEntry } from '../../lib/project/chain'
  import { useStores } from '../../lib/stores'

  let { entry, chain, master = false, track }: { entry: ChainEntry; chain: readonly ChainEntry[]; master?: boolean; track?: string } = $props()

  const { ui, dispatcher, automated, project } = useStores()
  const t = ui.t

  const descriptor = $derived(entry.descriptor)
  const name = $derived(descriptor?.name ?? entry.kind ?? t('devices.unknown'))
  const bypassed = $derived(!entry.device.enabled)
  const isEffect = $derived(entry.effectIndex !== null)
  const actions = $derived(cardActions(chain, entry))

  // A knob drag is one history group: one undo takes it back whole.
  const gesture = createGesture((command) => dispatcher.run(command))
  onDestroy(() => gesture.end())

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

<article class="card" class:bypassed data-testid="device-card" data-kind={entry.kind} data-role={isEffect ? 'effect' : 'instrument'} data-bypassed={bypassed}>
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
    <div class="params">
      {#each descriptor.params as param (param.key)}
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
              aria-label={t('automation.automate')}
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
    flex: none;
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
    background: var(--fk-accent-tint);
    color: var(--fk-accent-fg);
  }
  .params {
    display: flex;
    flex-wrap: wrap;
    gap: var(--fk-space-4);
    align-items: flex-end;
    max-width: 640px;
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
    font-size: 10px;
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
