<script lang="ts">
  /**
   * The history panel (PR-7): every entry of the undo history, newest first, with what was done,
   * who did it (you, or an agent) and when; the entries undone come first, dimmed, since a redo
   * brings them back. Undo and redo sit at its head. It reads the entries from the project core
   * when the history changes, and only while it is shown.
   */
  import { messages } from '../../lib/i18n/app'
  import { COMMAND_LABEL_PREFIX } from '../../lib/i18n/history'
  import type { HistoryEntryView } from '../../lib/generated/protocol'
  import { redo, undo } from '../../lib/track-commands'
  import { useStores } from '../../lib/stores'

  const { project, dispatcher, ui } = useStores()
  const t = ui.t

  /** Rows drawn at first, and added by each click on "older entries": drawing every row of a long history made each edit cost many frames (#432). */
  const PAGE = 200
  let shown = $state(PAGE)

  const all = $derived.by(() => {
    void project.historyRev
    const { undo: done, redo: undone } = dispatcher.history()
    // The last of the redo list is the next one redone (the oldest undone): newest first reads it backwards, the undone entries on top.
    return [...undone.map((entry) => ({ entry, undone: true })), ...done.toReversed().map((entry) => ({ entry, undone: false }))]
  })
  const entries = $derived(all.slice(0, shown))
  const hidden = $derived(all.length - entries.length)

  /** The label of an entry: its first command's, translated; a command without one shows its name in words. */
  function label(name: string): string {
    const key = COMMAND_LABEL_PREFIX + name
    if (key in messages.en) return t(key as Parameters<typeof t>[0])
    return name.replace(/[._]/g, ' ')
  }

  function author(entry: HistoryEntryView): string {
    const agent = entry.author.startsWith('agent:') ? entry.author.slice('agent:'.length) : null
    return agent === null ? t('history.you') : t('history.agent', { name: agent })
  }

  const clock = $derived(new Intl.DateTimeFormat(ui.lang, { hour: '2-digit', minute: '2-digit', second: '2-digit' }))
  const time = (at: number) => (at > 0 ? clock.format(at) : '')
</script>

<div class="history" data-testid="history">
  <div class="head">
    <button class="ghost" type="button" disabled={!project.canUndo} onclick={() => dispatcher.run(undo())} data-testid="history-undo"
      >{t('history.undo')}</button
    >
    <button class="ghost" type="button" disabled={!project.canRedo} onclick={() => dispatcher.run(redo())} data-testid="history-redo"
      >{t('history.redo')}</button
    >
  </div>
  {#if entries.length === 0}
    <p class="empty" data-testid="history-empty">{t('history.empty')}</p>
  {:else}
    <ol class="list" aria-label={t('history.label')}>
      {#each entries as { entry, undone } (entry.id)}
        <li class="row" class:undone data-testid="history-entry" data-name={entry.name} data-author={entry.author} data-undone={undone}>
          <span class="what">{label(entry.name)}</span>
          {#if undone}<span class="tag">{t('history.undone')}</span>{/if}
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
    grid-template-columns: minmax(0, 1fr) auto auto auto;
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
