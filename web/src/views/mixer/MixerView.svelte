<script lang="ts">
  /**
   * The mixer: one strip per track in the order of the track list, and the master's pinned at the
   * right. The meters come from the engine's report (30 Hz): a track's strip reads the meter of
   * its slot (`ViewSlots`), only when the report is of the slot table's epoch; anything else is
   * silence. Every control is a command (`MixerStrip`).
   */
  import { untrack } from 'svelte'
  import type { FeedbackReport } from '../../lib/engine/feedback'
  import { useStores } from '../../lib/stores'
  import { MeterBank, SILENT, type MeterSnap } from './ballistics'
  import MixerStrip from './MixerStrip.svelte'

  const { project, engine, ui } = useStores()
  const t = ui.t

  const MASTER = 'master'
  const bank = new MeterBank()
  let snaps = $state.raw<Map<string, MeterSnap>>(new Map())

  /** Reports stop when the engine stops or the helper goes: after this long without one, the meters fall to silence. */
  const STALE_MS = 500
  let lastReportAt = 0
  /** The last report fed: one that comes back (the engine restarted, no new report yet) is old news, not a fresh signal. */
  let seen: FeedbackReport | null = null

  /** Feeds the bank: from a report, or silence (`null`). Reads nothing reactive. */
  function feed(report: FeedbackReport | null, now: number) {
    const slots = project.view?.slots
    const ids = project.tracks.map((entry) => entry.id)
    const silence = { peakL: 0, peakR: 0 }
    snaps = bank.update(
      [...ids, MASTER],
      (id) => {
        if (!report) return silence
        if (id === MASTER) return report.master
        const slot = slots?.tracks[id]
        // A slot beyond the report (a track added since, an engine a little behind) is silent, not an error.
        return slot === undefined || report.epoch !== slots?.epoch ? silence : (report.tracks[slot] ?? silence)
      },
      now / 1000,
    )
  }

  // One pass per new report. A project change in between must not feed the same peaks twice, so only the report is tracked.
  // The clock is the page's, taken when the report arrives: a report carries none.
  $effect(() => {
    const current = engine.started ? engine.feedback : null
    const now = performance.now()
    untrack(() => {
      const report = current !== null && current === seen ? null : current
      if (current) seen = current
      if (report) lastReportAt = now
      feed(report, now)
    })
  })

  $effect(() => {
    const timer = setInterval(() => {
      const now = performance.now()
      if (now - lastReportAt > STALE_MS) feed(null, now)
    }, 100)
    return () => clearInterval(timer)
  })

  function reset(id: string) {
    // A copy that replaces the raw state, never mutated in place.
    // eslint-disable-next-line svelte/prefer-svelte-reactivity
    const next = new Map(snaps)
    next.set(id, bank.reset(id))
    snaps = next
  }
</script>

<div class="mixer" data-testid="mixer">
  {#if project.tracks.length === 0}
    <p class="empty" data-testid="mixer-empty">{t('mixer.empty')}</p>
  {/if}
  <ul class="strips" aria-label={t('mixer.strips')}>
    {#each project.tracks as entry (entry.id)}
      <MixerStrip
        {entry}
        snap={snaps.get(entry.id) ?? SILENT}
        selected={project.selected?.id === entry.id}
        soloed={project.soloed.includes(entry.id)}
        armed={project.armed.includes(entry.id)}
        onreset={() => reset(entry.id)}
      />
    {/each}
  </ul>
  <ul class="master" aria-label={t('mixer.master')}>
    <MixerStrip snap={snaps.get(MASTER) ?? SILENT} onreset={() => reset(MASTER)} />
  </ul>
</div>

<style>
  .mixer {
    display: flex;
    align-items: flex-start;
    gap: var(--fk-space-3);
    height: 100%;
    min-height: 0;
    padding: var(--fk-space-3);
    overflow: auto;
  }
  .strips {
    flex: 1;
    min-width: 0;
    display: flex;
    gap: var(--fk-space-2);
    margin: 0;
    padding: 0;
    overflow-x: auto;
  }
  .master {
    flex: none;
    margin: 0;
    padding: 0;
  }
  .empty {
    position: absolute;
    margin: 0;
    color: var(--fk-text-muted);
  }
</style>
