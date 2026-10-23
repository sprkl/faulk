<script lang="ts">
  /**
   * A vertical fader in dB, on the taper of `fader.ts`, with its marks (0 dB is the one drawn
   * strong). A native range input, so the keyboard (arrows, Page keys, Home, End) and the screen
   * reader come with it. One drag is one history group (`gesture.ts`); a double click puts it back
   * at 0 dB. It sends the command `set` builds and holds no project state.
   */
  import { onDestroy } from 'svelte'
  import { createGesture } from '../../lib/gesture'
  import type { Command } from '../../lib/generated/protocol'
  import { formatGain } from '../../lib/project/tracks'
  import { useStores } from '../../lib/stores'
  import { FADER_MARKS, gainAt, positionOf } from './fader'

  let { gainDb, label, set, testid }: { gainDb: number | null | undefined; label: string; set: (gainDb: number | null) => Command; testid: string } = $props()

  const { dispatcher } = useStores()
  const gesture = createGesture((command) => dispatcher.run(command))
  onDestroy(() => gesture.end())

  const sameGain = (a: number | null | undefined, b: number | null) => (a ?? null) === b

  function onInput(event: Event & { currentTarget: HTMLInputElement }) {
    const gain = gainAt(Number(event.currentTarget.value))
    if (!sameGain(gainDb, gain)) dispatcher.run(set(gain))
  }
</script>

<div class="fader">
  <input
    type="range"
    min="0"
    max="1"
    step="0.005"
    value={positionOf(gainDb)}
    aria-label={label}
    aria-orientation="vertical"
    aria-valuetext={`${formatGain(gainDb)} dB`}
    data-testid={testid}
    oninput={onInput}
    onpointerdown={gesture.begin}
    onpointerup={gesture.end}
    onpointercancel={gesture.end}
    onchange={gesture.end}
    onblur={gesture.end}
    ondblclick={() => dispatcher.run(set(0))}
  />
  <div class="marks" aria-hidden="true">
    {#each FADER_MARKS as db (db)}
      <span class="mark" class:zero={db === 0} style:--at={positionOf(db)}>{db > 0 ? `+${db}` : db < 0 ? `−${-db}` : '0'}</span>
    {/each}
  </div>
</div>

<style>
  .fader {
    position: relative;
    display: flex;
    align-items: stretch;
    gap: var(--fk-space-1);
    height: 168px;
  }
  input {
    width: 22px;
    height: 100%;
    margin: 0;
    writing-mode: vertical-lr;
    direction: rtl;
    accent-color: var(--fk-accent-fg);
    cursor: pointer;
  }
  .marks {
    position: relative;
    width: 3ch;
  }
  .mark {
    position: absolute;
    left: 0;
    bottom: calc(var(--at) * (100% - 16px) + 8px);
    transform: translateY(50%);
    font-family: var(--fk-font-mono);
    font-size: 9px;
    line-height: 1;
    color: var(--fk-text-muted);
  }
  .mark.zero {
    color: var(--fk-text);
    font-weight: 700;
  }
  .mark.zero::before {
    content: '';
    position: absolute;
    right: 100%;
    top: 50%;
    width: 6px;
    height: 2px;
    margin-right: 2px;
    background: var(--fk-text);
  }
</style>
