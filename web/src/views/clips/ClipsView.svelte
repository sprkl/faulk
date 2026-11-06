<script lang="ts">
  /**
   * The Clips panel (#473): the pool of the selected track, every clip it has, wherever it is used (its placements, its
   * launch-grid slot) or nowhere (a clip that lost its last placement stays in its track). A linked clip (used more than
   * once) is marked. A row is dragged onto a lane of the arrangement or a cell of the grid, or sent there by its
   * buttons; every action is a command (`lib/clip-pool.ts` picks which: Place sends the clip itself (linked), Alt a copy (#624); the grid and the drag keep the
   * founder's decision 11: a copy of a clip already used, the clip itself with Alt).
   */
  import LinkMark from '../../lib/components/LinkMark.svelte'
  import Waveform from '../../lib/components/Waveform.svelte'
  import { barStart } from '../../lib/arrangement-commands'
  import { CLIP_DRAG_TYPE, clipTrackType, freeScene, gridCommand, placeCommand, poolNames, poolOf, type PoolClip } from '../../lib/clip-pool'
  import { cancelAudioImport, chooseAudio, deleteClip } from '../../lib/clip-commands'
  import { boostFolderNotice, chooseHelperFolder } from '../../lib/audio/boost-folder'
  import type { DownloadProgress, ImportFailure } from '../../lib/audio/import/audio-import.svelte'
  import { MAX_DECODED_BYTES } from '../../lib/audio/import/abi'
  import { MAX_AUDIO_URL_MB } from '../../lib/agents/confirm-text'
  import type { MessageKey } from '../../lib/i18n'
  import { useStores } from '../../lib/stores'
  import { barTicks } from '../transport/time'
  import { trackColor, trackLabel } from '../../lib/project/tracks'

  const { project, dispatcher, ui, engine, audioImport, peaks, storage } = useStores()
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
      too_large: 'clips.importFailed.tooLarge',
      unavailable: 'clips.importFailed.unavailable',
      io: 'clips.importFailed.unreadable',
      download_unreachable: 'clips.importFailed.downloadUnreachable',
      download_too_big: 'clips.importFailed.downloadTooBig',
      // The helper's import in Boost (#778).
      no_folder: 'clips.importFailed.noFolder',
      path: 'clips.importFailed.outsideFolder',
      unreadable: 'clips.importFailed.unreadable',
      folder_changed: 'clips.importFailed.folderChanged',
      busy: 'clips.importFailed.busy',
      dialog_unavailable: 'clips.importFailed.dialogUnavailable',
      helper_lost: 'clips.importFailed.helperLost',
    }
    return t(known[failure.code] ?? 'clips.importFailed.store', { name: failure.name, max: MAX_AUDIO_URL_MB, gb: MAX_DECODED_BYTES / 2 ** 30 })
  }

  const trackId = $derived(project.view?.session.selected_track ?? null)
  const track = $derived(trackId ? (project.view?.project.tracks[trackId] ?? null) : null)
  /** The track as the track list names it, a name or its default (#781). */
  const trackTitle = $derived(project.selected ? trackLabel(project.selected, t) : '')
  /** The colour a clip without its own shows: its track's, as in the timeline. */
  const color = $derived(project.selected ? trackColor(project.selected.track.color, project.selected.index) : 'currentColor')
  const pool = $derived(project.view && trackId ? poolOf(project.view.project, trackId) : [])
  const free = $derived(project.view && trackId ? freeScene(project.view.project, trackId) : null)
  /** The import under way into this track, and the last failure of one (AU-1). */
  const importing = $derived(audioImport.running && audioImport.running.track === trackId ? audioImport.running : null)
  const failed = $derived(audioImport.failure && audioImport.failure.track === trackId ? audioImport.failure : null)
  /** In Boost, what the helper needs to play the audio clips from the project's folder (AU-7). */
  const boostNotice = $derived(engine.mode === 'native' ? boostFolderNotice(storage.inBrowser, engine.helperFolder, engine.helperLinkFailed) : null)
  /** A file an agent's link brings into this track (#664), while it downloads. */
  const download = $derived(audioImport.downloads.find((d) => d.track === trackId) ?? null)

  /** How far a download is, in percent; `null` when the host announced no length. */
  const percentOf = (d: DownloadProgress): number | null => (d.total && d.total > 0 ? Math.min(100, Math.round((d.received / d.total) * 100)) : null)

  function downloadText(d: DownloadProgress): string {
    const percent = percentOf(d)
    if (percent !== null) return t('clips.downloading', { name: d.name, host: d.host, percent })
    return t('clips.downloadingSize', { name: d.name, host: d.host, size: (d.received / (1 << 20)).toFixed(1) })
  }

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

  /**
   * Places a clip at the playhead: the same clip, linked, by default (#624); `copy` (Alt) makes an independent copy of a
   * clip already used. The placements it made become the arrangement's selection and take the keyboard focus.
   */
  function place(entry: PoolClip, copy: boolean) {
    const command = placeCommand(entry, playheadBar(), !copy)
    if (!command) return
    const placements = () => (trackId ? (project.view?.project.tracks[trackId]?.placements ?? {}) : {})
    const before = new Set(Object.keys(placements()))
    dispatcher.run(command)
    // A placement that lands inside another carves it: the right-hand piece is new too but starts later, so the one placed is
    // the new placement that starts first.
    const fresh = Object.entries(placements() as Record<string, { start: number } | undefined>)
      .filter(([id, p]) => !before.has(id) && p)
      .sort(([, a], [, b]) => a!.start - b!.start)[0]
    if (fresh && ui.view === 'arrangement') ui.showPlaced([fresh[0]])
  }

  /** The name a row shows: two clips of one name get a number, so that they tell apart (#624). */
  const names = $derived(poolNames(pool, t('clips.audio')))

  function toGrid(entry: PoolClip, linked: boolean) {
    if (!free) return
    const command = gridCommand(entry, free, linked)
    if (command) dispatcher.run(command)
  }

  /**
   * An audio clip's gain from its field (AU-6): a number from -96 to +6 dB, or empty for silence; anything else puts the
   * field back to the clip's gain without a command.
   */
  function setGain(entry: PoolClip, input: HTMLInputElement) {
    if (!('audio' in entry.clip.content)) return
    const current = entry.clip.content.audio.gain_db ?? null
    const text = input.value.trim()
    const value = text === '' ? null : Number(text)
    const valid = value === null || (Number.isFinite(value) && value >= -96 && value <= 6)
    if (!valid || value === current || !dispatcher.run({ type: 'clip.audio.set_gain', clip: entry.id, gain_db: value })) {
      input.value = current === null ? '' : String(Math.round(current * 10) / 10)
    }
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
      {#if download && !importing}
        <span class="progress" role="status" data-testid="audio-download-progress">
          <span class="bar" aria-hidden="true"><span style:width="{percentOf(download) ?? 0}%"></span></span>
          {downloadText(download)}
        </span>
        <button class="ghost" type="button" data-testid="audio-import-cancel" onclick={() => dispatcher.run(cancelAudioImport())}
          >{t('clips.importCancel')}</button
        >
      {:else if importing}
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
      {:else if boostNotice}
        <!-- In Boost the helper plays audio clips from the project's folder (AU-7): what it needs from the user. -->
        <span class="note" role="status" data-testid="audio-boost-folder" data-key={boostNotice.key}>{t(boostNotice.key, boostNotice.vars)}</span>
        {#if boostNotice.ask}
          <button
            class="ghost"
            type="button"
            title={t('clips.boostShowFolderHint')}
            data-testid="audio-boost-show-folder"
            onclick={() => dispatcher.run(chooseHelperFolder)}>{t('clips.boostShowFolderButton')}</button
          >
        {/if}
      {/if}
    </div>
  {/if}
  {#if !track}
    <p class="empty">{t('clips.noTrack')}</p>
  {:else if pool.length === 0}
    <p class="empty">{t('clips.empty', { track: trackTitle })}</p>
  {:else}
    <ul class="list" aria-label={t('clips.label', { track: trackTitle })}>
      {#each pool as entry (entry.id)}
        <li class="row" class:unused={entry.uses === 0} data-testid="pool-clip" data-clip={entry.id} data-uses={entry.uses}>
          <span
            class="handle"
            role="button"
            tabindex="0"
            draggable="true"
            title={t('clips.dragHint')}
            aria-label={t('clips.row', { name: names[entry.id], where: where(entry) }) + (entry.uses > 1 ? `, ${t('clips.linked')}` : '')}
            data-testid="pool-handle"
            ondragstart={(event) => onDragStart(event, entry)}
            onkeydown={(event) => onKey(event, entry)}
          >
            <span class="swatch" style:background={entry.clip.color ?? color} aria-hidden="true"></span>
            <span class="name">{names[entry.id]}</span>
            {#if 'audio' in entry.clip.content}
              {@const audio = entry.clip.content.audio}
              <!-- The part of its file the clip plays (AU-2), drawn from the file's peaks. -->
              <span class="wave" data-testid="pool-waveform" data-media={audio.media.hash}>
                <Waveform peaks={peaks.of(audio.media).peaks} from={audio.start} frames={audio.frames} color={entry.clip.color ?? color} track />
              </span>
            {/if}
          </span>
          {#if entry.uses > 1}<span class="linked" title={t('clips.linkedHint')}><LinkMark />{t('clips.linked')}</span>{/if}
          <span class="where">{where(entry)}</span>
          <span class="actions">
            {#if 'audio' in entry.clip.content}
              {@const gain = entry.clip.content.audio.gain_db}
              <!-- The clip's gain (AU-6): one command per value typed; empty is silence. -->
              <label class="gain" title={t('audio.gainHint')}>
                <span>{t('audio.gain')}</span>
                <input
                  type="number"
                  min="-96"
                  max="6"
                  step="0.1"
                  value={gain === null || gain === undefined ? '' : Math.round(gain * 10) / 10}
                  placeholder={t('audio.gainSilent')}
                  aria-label={t('audio.gainOf', { name: names[entry.id] ?? '' })}
                  data-testid="pool-gain"
                  onchange={(event) => setGain(entry, event.currentTarget)}
                />
                <span class="unit" aria-hidden="true">{t('audio.gainUnit')}</span>
              </label>
            {/if}
            <button class="ghost" type="button" title={t('clips.placeHint')} data-testid="pool-place" onclick={(event) => place(entry, event.altKey)}
              >{t('clips.place')}</button
            >
            <button
              class="ghost"
              type="button"
              disabled={entry.uses === 0}
              title={t('clips.placeCopyHint')}
              data-testid="pool-place-copy"
              onclick={() => place(entry, true)}>{t('clips.placeCopy')}</button
            >
            <button
              class="ghost"
              type="button"
              disabled={!free}
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
    align-items: center;
    gap: var(--fk-space-1);
  }
  .gain {
    display: inline-flex;
    align-items: center;
    gap: var(--fk-space-1);
    margin-right: var(--fk-space-2);
    color: var(--fk-text-muted);
    font-size: 0.85em;
  }
  .gain input {
    width: 60px;
    padding: 0 var(--fk-space-1);
    text-align: right;
    font-variant-numeric: tabular-nums;
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
