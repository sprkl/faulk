<script lang="ts">
  /**
   * The history panel (PR-7): every entry of the undo history, newest first, with what was done,
   * who did it (you, or an agent) and when; the entries undone come first, dimmed, since a redo
   * brings them back. Each label names the objects (#473: "Mute Lead"). The settings changed outside
   * the undo history (the loop switch, the metronome) sit among the entries in the order they
   * happened, marked as not undoable. An agent's turn (AG-3: its consecutive edits, one entry that one undo undoes)
   * says how many edits it holds. Undo and redo sit at its head. It reads the entries from the project core
   * when the history changes, and only while it is shown.
   */
  import type { ActivityView, HistoryEntryView } from '../../lib/generated/protocol'
  import { label } from '../../lib/history-label'
  import { runHistory } from '../../lib/history-said'
  import { useStores } from '../../lib/stores'

  const { project, dispatcher, ui } = useStores()
  const t = ui.t

  /** Rows drawn at first, and added by each click on "older entries": drawing every row of a long history made each edit cost many frames (#432). */
  const PAGE = 200
  let shown = $state(PAGE)

  type Row = { key: string; entry: HistoryEntryView | ActivityView; undone: boolean; setting: boolean }

  const all = $derived.by((): Row[] => {
    void project.historyRev
    const { undo: done, redo: undone, activity } = dispatcher.history()
    // The last of the redo list is the next one redone (the oldest undone): newest first reads it backwards, the undone entries on top.
    // Below them, the entries done and the settings, newest first: both take their ids from one counter.
    const past = [
      ...done.map((entry) => ({ key: `e${entry.id}`, entry, undone: false, setting: false })),
      ...activity.map((entry) => ({ key: `a${entry.id}`, entry, undone: false, setting: true })),
    ].sort((a, b) => b.entry.id - a.entry.id)
    return [...undone.map((entry) => ({ key: `e${entry.id}`, entry, undone: true, setting: false })), ...past]
  })
  const entries = $derived(all.slice(0, shown))
  const hidden = $derived(all.length - entries.length)

  function author(entry: HistoryEntryView | ActivityView): string {
    const agent = entry.author.startsWith('agent:') ? entry.author.slice('agent:'.length) : null
    return agent === null ? t('history.you') : t('history.agent', { name: agent })
  }

  /** An agent's entry with a group is one of its turns; one of several edits says how many. */
  function turnEdits(entry: HistoryEntryView | ActivityView): number | null {
    if (!('edits' in entry) || entry.group == null || !entry.author.startsWith('agent:')) return null
    return entry.edits > 1 ? entry.edits : null
  }

  const clock = $derived(new Intl.DateTimeFormat(ui.lang, { hour: '2-digit', minute: '2-digit', second: '2-digit' }))
  const time = (at: number) => (at > 0 ? clock.format(at) : '')
</script>

<div class="history" data-testid="history">
  <div class="head">
    <button class="ghost" type="button" disabled={!project.canUndo} onclick={() => runHistory(dispatcher, ui, 'undo')} data-testid="history-undo"
      >{t('history.undo')}</button
    >
    <button class="ghost" type="button" disabled={!project.canRedo} onclick={() => runHistory(dispatcher, ui, 'redo')} data-testid="history-redo"
      >{t('history.redo')}</button
    >
  </div>
  {#if entries.length === 0}
    <p class="empty" data-testid="history-empty">{t('history.empty')}</p>
  {:else}
    <ol class="list" aria-label={t('history.label')}>
      {#each entries as { key, entry, undone, setting } (key)}
        {@const turn = turnEdits(entry)}
        <li
          class="row"
          class:undone
          class:setting
          data-testid={setting ? 'history-activity' : 'history-entry'}
          data-name={entry.name}
          data-author={entry.author}
          data-undone={undone}
          data-edits={turn ?? undefined}
        >
          <span class="what">{label(entry, t)}</span>
          {#if undone}<span class="tag">{t('history.undone')}</span>{:else if setting}<span class="tag">{t('history.notUndoable')}</span>{/if}
          {#if turn !== null}<span class="tag turn" data-testid="history-turn">{t('history.turn', { count: turn })}</span>{/if}
          <span class="who" class:agent={entry.author !== 'user'}>{author(entry)}</span>
          <time class="when" datetime={entry.at > 0 ? new Date(entry.at).toISOString() : undefined}>{time(entry.at)}</time>
        </li>
      {/each}
      {#if hidden > 0}
        <li class="more">
          <button class="ghost" type="button" onclick={() => (shown += PAGE)} data-testid="history-older">{t('history.older', { count: hidden })}</button>
        </li>
      {/if}
    </ol>
  {/if}
</div>

<style>
  .history {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .head {
    display: flex;
    gap: var(--fk-space-2);
    padding: var(--fk-space-2) var(--fk-space-3);
    border-bottom: 1px solid var(--fk-line);
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
    grid-template-columns: minmax(0, 1fr) auto auto auto auto;
    align-items: baseline;
    gap: var(--fk-space-3);
    padding: var(--fk-space-1) var(--fk-space-3);
    border-bottom: 1px solid var(--fk-line);
  }
  .more {
    padding: var(--fk-space-2) var(--fk-space-3);
  }
  .row.undone {
    opacity: 0.55;
  }
  .what {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .setting .tag,
  .tag.turn {
    color: var(--fk-text-muted);
  }
  .tag,
  .who,
  .when {
    font-size: 0.85em;
  }
  .who.agent {
    font-weight: 600;
  }
  .when {
    font-variant-numeric: tabular-nums;
  }
  .empty {
    margin: 0;
    padding: var(--fk-space-4);
  }
</style>
