<script lang="ts">
  /**
   * The Clips panel (#473): the pool of the selected track, every clip it has, wherever it is used (its placements, its
   * launch-grid slot) or nowhere (a clip that lost its last placement stays in its track). A linked clip (used more than
   * once) is marked. A row is dragged onto a lane of the arrangement or a cell of the grid, or sent there by its
   * buttons; every action is a command (`lib/clip-pool.ts` picks which, by the founder's decision 11: a copy of a clip
   * already used, the clip itself with Alt).
   */
  import LinkMark from '../../lib/components/LinkMark.svelte'
  import Waveform from '../../lib/components/Waveform.svelte'
  import { barStart } from '../../lib/arrangement-commands'
  import { CLIP_DRAG_TYPE, clipTrackType, freeScene, gridCommand, placeCommand, poolOf, type PoolClip } from '../../lib/clip-pool'
  import { cancelAudioImport, chooseAudio, deleteClip } from '../../lib/clip-commands'
  import type { ImportFailure } from '../../lib/audio/import/audio-import.svelte'
  import type { MessageKey } from '../../lib/i18n'
  import { useStores } from '../../lib/stores'
  import { barTicks } from '../transport/time'
  import { trackColor } from '../../lib/project/tracks'

  const { project, dispatcher, ui, engine, audioImport, peaks } = useStores()
  const t = ui.t

  /** What a failed import says: by why the file was refused, or that the folder refused it. */
  function failureText(failure: ImportFailure): string {
    const known: Record<string, MessageKey> = {
      unsupported_format: 'clips.importFailed.unsupportedFormat',
      unsupported_codec: 'clips.importFailed.unsupportedCodec',
      corrupt: 'clips.importFailed.corrupt',
      empty: 'clips.importFailed.empty',
      bad_sample_rate: 'clips.importFailed.badSampleRate',
      bad_channels: 'clips.importFailed.badChannels',
      too_long: 'clips.importFailed.tooLong',
      unavailable: 'clips.importFailed.unavailable',
      io: 'clips.importFailed.unreadable',
    }
    return t(known[failure.code] ?? 'clips.importFailed.store', { name: failure.name })
  }

  const trackId = $derived(project.view?.session.selected_track ?? null)
  const track = $derived(trackId ? (project.view?.project.tracks[trackId] ?? null) : null)
  /** The colour a clip without its own shows: its track's, as in the timeline. */
  const color = $derived(project.selected ? trackColor(project.selected.track.color, project.selected.index) : 'currentColor')
  const pool = $derived(project.view && trackId ? poolOf(project.view.project, trackId) : [])
  const free = $derived(project.view && trackId ? freeScene(project.view.project, trackId) : null)
  /** The import under way into this track, and the last failure of one (AU-1). */
  const importing = $derived(audioImport.running && audioImport.running.track === trackId ? audioImport.running : null)
  const failed = $derived(audioImport.failure && audioImport.failure.track === trackId ? audioImport.failure : null)

  function where(entry: PoolClip): string {
    const parts: string[] = []
    if (entry.slot) parts.push(t('clips.grid', { scene: project.view?.project.scenes[entry.slot]?.name || '·' }))
    if (entry.placements.length > 0) parts.push(t('clips.placements', { n: entry.placements.length }))
    return parts.length > 0 ? parts.join(' · ') : t('clips.unused')
  }

  /** The bar the play position is in, as the grid sends its clips. */
  function playheadBar(): number {
    const signature = project.view?.project.meter[0]?.signature ?? { numerator: 4, denominator: 4 }
    return barStart(engine.started ? (engine.feedback?.playheadTick ?? 0) : 0, barTicks(signature))
  }

  function place(entry: PoolClip, linked: boolean) {
    const command = placeCommand(entry, playheadBar(), linked)
    if (command) dispatcher.run(command)
  }

  function toGrid(entry: PoolClip, linked: boolean) {
    if (!free) return
    const command = gridCommand(entry, free, linked)
    if (command) dispatcher.run(command)
  }

  function onDragStart(event: DragEvent, entry: PoolClip) {
    if (!event.dataTransfer || !trackId) return
    // Copy by default, link with Alt: both drop effects are allowed.
    event.dataTransfer.effectAllowed = 'all'
    event.dataTransfer.setData(CLIP_DRAG_TYPE, JSON.stringify({ clip: entry.id, track: trackId }))
    event.dataTransfer.setData(clipTrackType(trackId), entry.id)
    event.dataTransfer.setData('text/plain', entry.clip.name)
  }

  function onKey(event: KeyboardEvent, entry: PoolClip) {
    if (event.target !== event.currentTarget) return
    if (event.key === 'Enter') {
      event.preventDefault()
      place(entry, event.altKey)
    } else if (event.key === 'Delete' || event.key === 'Backspace') {
      event.preventDefault()
      dispatcher.run(deleteClip(entry.id))
    }
  }
</script>

<div class="clips" data-testid="clips-panel">
  {#if track && trackId && track.kind === 'audio'}
    <div class="import" data-testid="audio-import">
      <button
        class="ghost"
        type="button"
        title={t('clips.importAudioHint')}
        data-testid="audio-import-button"
        onclick={() => dispatcher.run(chooseAudio(trackId))}>{t('clips.importAudio')}</button
      >
      {#if importing}
        <span class="progress" role="status" data-testid="audio-import-progress" data-fraction={importing.fraction}>
          <span class="bar" aria-hidden="true"><span style:width="{Math.round(importing.fraction * 100)}%"></span></span>
          {importing.count > 1
            ? t('clips.importingMany', { name: importing.name, index: importing.index, count: importing.count, percent: Math.round(importing.fraction * 100) })
            : t('clips.importing', { name: importing.name, percent: Math.round(importing.fraction * 100) })}
        </span>
        <button class="ghost" type="button" data-testid="audio-import-cancel" onclick={() => dispatcher.run(cancelAudioImport())}
          >{t('clips.importCancel')}</button
        >
      {:else if failed}
        <span class="failure" role="alert" data-testid="audio-import-failure" data-code={failed.code}>{failureText(failed)}</span>
        <button class="ghost" type="button" onclick={() => audioImport.dismiss()}>{t('clips.dismiss')}</button>
      {:else if audioImport.needsClick}
        <span class="failure" role="status" data-testid="audio-import-needs-click">{t('clips.importNeedsClick')}</span>
        <button class="ghost" type="button" onclick={() => audioImport.dismiss()}>{t('clips.dismiss')}</button>
      {:else if engine.mode === 'native'}
        <!-- The helper reads no project folder yet (AU-7): its engine keeps audio clips silent. -->
        <span class="note" role="status" data-testid="audio-boost-silent">{t('clips.boostSilent')}</span>
      {/if}
    </div>
  {/if}
  {#if !track}
    <p class="empty">{t('clips.noTrack')}</p>
  {:else if pool.length === 0}
    <p class="empty">{t('clips.empty', { track: track.name })}</p>
  {:else}
    <ul class="list" aria-label={t('clips.label', { track: track.name })}>
      {#each pool as entry (entry.id)}
        <li class="row" class:unused={entry.uses === 0} data-testid="pool-clip" data-clip={entry.id} data-uses={entry.uses}>
          <span
            class="handle"
            role="button"
            tabindex="0"
            draggable={entry.midi}
            title={t('clips.dragHint')}
            aria-label={t('clips.row', { name: entry.clip.name, where: where(entry) }) + (entry.uses > 1 ? `, ${t('clips.linked')}` : '')}
            data-testid="pool-handle"
            ondragstart={(event) => onDragStart(event, entry)}
            onkeydown={(event) => onKey(event, entry)}
          >
            <span class="swatch" style:background={entry.clip.color ?? color} aria-hidden="true"></span>
            <span class="name">{entry.clip.name || t('clips.audio')}</span>
            {#if 'audio' in entry.clip.content}
              {@const audio = entry.clip.content.audio}
              <!-- The part of its file the clip plays (AU-2), drawn from the file's peaks. -->
              <span class="wave" data-testid="pool-waveform" data-media={audio.media.hash}>
                <Waveform peaks={peaks.of(audio.media).peaks} from={audio.start} frames={audio.frames} color={entry.clip.color ?? color} />
              </span>
            {/if}
          </span>
          {#if entry.uses > 1}<span class="linked" title={t('clips.linkedHint')}><LinkMark />{t('clips.linked')}</span>{/if}
          <span class="where">{where(entry)}</span>
          <span class="actions">
            <button
              class="ghost"
              type="button"
              disabled={!entry.midi}
              title={t('clips.placeHint')}
              data-testid="pool-place"
              onclick={(event) => place(entry, event.altKey)}>{t('clips.place')}</button
            >
            <button
              class="ghost"
              type="button"
              disabled={!free || (!entry.midi && entry.uses > 0)}
              title={free ? t('clips.toGridHint') : t('clips.noFreeSlot')}
              data-testid="pool-grid"
              onclick={(event) => toGrid(entry, event.altKey)}>{t('clips.toGrid')}</button
            >
            <button class="ghost" type="button" title={t('clips.deleteHint')} data-testid="pool-delete" onclick={() => dispatcher.run(deleteClip(entry.id))}
              >{t('clips.delete')}</button
            >
          </span>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .clips {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .list {
    flex: 1;
    min-height: 0;
    margin: 0;
    padding: 0;
    list-style: none;
    overflow: auto;
  }
  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 72px minmax(9rem, auto) auto;
    align-items: center;
    gap: var(--fk-space-3);
    padding: var(--fk-space-1) var(--fk-space-3);
    border-bottom: 1px solid var(--fk-line);
  }
  .handle {
    display: flex;
    align-items: center;
    gap: var(--fk-space-2);
    min-width: 0;
    cursor: grab;
  }
  .handle:focus-visible {
    outline: 2px solid var(--fk-accent);
    outline-offset: -2px;
  }
  .swatch {
    flex: none;
    width: 10px;
    height: 10px;
    border-radius: 2px;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wave {
    flex: 1 1 64px;
    min-width: 32px;
    max-width: 240px;
    height: 20px;
  }
  .row.unused .name {
    font-style: italic;
  }
  .linked {
    grid-column: 2;
    display: inline-flex;
    align-items: center;
    gap: var(--fk-space-1);
    font-size: 0.85em;
  }
  .where {
    grid-column: 3;
    text-align: right;
    color: var(--fk-text-muted);
    font-size: 0.85em;
    font-variant-numeric: tabular-nums;
  }
  .actions {
    grid-column: 4;
    display: flex;
    gap: var(--fk-space-1);
  }
  .empty {
    margin: 0;
    padding: var(--fk-space-4);
    color: var(--fk-text-muted);
  }
  .import {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--fk-space-2);
    padding: var(--fk-space-1) var(--fk-space-3);
    border-bottom: 1px solid var(--fk-line);
  }
  .progress {
    display: inline-flex;
    align-items: center;
    gap: var(--fk-space-2);
    min-width: 0;
    color: var(--fk-text-muted);
    font-size: 0.85em;
    font-variant-numeric: tabular-nums;
  }
  .bar {
    flex: none;
    width: 96px;
    height: 4px;
    border-radius: 2px;
    background: var(--fk-line);
    overflow: hidden;
  }
  .bar > span {
    display: block;
    height: 100%;
    background: var(--fk-accent);
  }
  .failure {
    min-width: 0;
    font-size: 0.85em;
  }
</style>
