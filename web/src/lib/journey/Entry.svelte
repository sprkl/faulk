<script lang="ts">
  import { resolve } from '$app/paths'
  import { fill, type Lang } from '../i18n/lang'
  import { openLinkHref } from '../open-link'
  import type { JourneyKey } from '../i18n/journey'
  import Cover from './Cover.svelte'
  import { folderOf, midiUrl, packUrl, sheetUrl } from './entries'
  import { clock, dayName, dayParts, keyName } from './format'
  import Player from './Player.svelte'
  import type { JourneyEntry } from './types'

  /**
   * One day: `featured` (the ink panel) for the latest on the main page and on the day's own page;
   * `feed` for the lists. `heading`: the title's level (`h1` on the day's page); `permalink`: the
   * day's page, linked from the title, omitted on that page itself.
   */
  let {
    entry,
    lang,
    j,
    variant,
    heading = variant === 'featured' ? 'h2' : 'h3',
    permalink,
  }: {
    entry: JourneyEntry
    lang: Lang
    j: (key: JourneyKey) => string
    variant: 'featured' | 'feed'
    heading?: 'h1' | 'h2' | 'h3'
    permalink?: string
  } = $props()

  const id = $derived(`day-${entry.date}`)
  const stage = $derived(`cover-${folderOf(entry)}`)
  const key = $derived(keyName(entry.key, lang))
  const parts = $derived(dayParts(entry.date, lang))
  const coverLabel = $derived(fill(j('entry.cover'), { tracks: entry.tracks.length, bpm: entry.bpm, key, bars: entry.bars }))
  const specs = $derived([fill(j('entry.bpm'), { bpm: entry.bpm }), key, fill(j('entry.bars'), { bars: entry.bars }), clock(entry.duration)])
  const maxParts = $derived(Math.max(...entry.sections.map((s) => s.parts)))
  /** Sub-headings sit one level under the entry title. */
  const sub = $derived(({ h1: 'h2', h2: 'h3', h3: 'h4' } as const)[heading])
  const agent = $derived(fill(j('entry.agent'), { model: entry.model, skills: entry.skills.join(', ') }))
  /** The DAW, asked to open the day's project (it asks the visitor first, #497). */
  const openHref = $derived(openLinkHref(resolve('/app'), packUrl(entry)))
</script>

{#snippet name()}
  {#if permalink}
    <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- resolved by the caller (`journeyHref`) -->
    <a class="permalink" href={permalink}>{entry.title}</a>
  {:else}
    {entry.title}
  {/if}
{/snippet}

{#snippet details()}
  <div class="details">
    <div class="story">
      <section>
        <svelte:element this={sub} class="label">{j('entry.brief')}</svelte:element>
        <p class="brief">{entry.brief[lang]}</p>
      </section>
      <section>
        <svelte:element this={sub} class="label">{j('entry.built')}</svelte:element>
        <p>{entry.built[lang]}</p>
      </section>
    </div>
    <div class="side">
      <section>
        <svelte:element this={sub} class="label" id="{id}-form">{j('entry.form')}</svelte:element>
        <ol class="form" aria-labelledby="{id}-form" data-fk-form={stage} data-bpm={entry.bpm}>
          {#each entry.sections as s, i (i)}
            <li style="flex-grow: {s.bars}; --h: {(s.parts / maxParts).toFixed(3)}" data-bars={s.bars}>
              <span class="bar" aria-hidden="true"></span>
              <span class="name">{s.name[lang]}</span>
              <span class="count">{fill(j('entry.bars'), { bars: s.bars })}</span>
            </li>
          {/each}
        </ol>
      </section>
      <section>
        <svelte:element this={sub} class="label">{j('entry.tested')}</svelte:element>
        <ul class="list">
          {#each entry.tested as item, i (i)}
            <li>{item[lang]}</li>
          {/each}
        </ul>
      </section>
      <section>
        <svelte:element this={sub} class="label">{j('entry.gaps')}</svelte:element>
        <ul class="list gaps">
          {#each entry.gaps as item, i (i)}
            <li>{item[lang]}</li>
          {/each}
        </ul>
      </section>
      <!-- The entry's actions, on every page that shows it: the project first, then the files. -->
      <ul class="links" data-fk-actions>
        <li>
          <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- `resolve('/app')` plus the link to open -->
          <a class="lp-pill lp-pill-accent open" href={openHref} data-sveltekit-reload aria-describedby="{id}-title" data-testid="open-in-faulk"
            >{j('entry.open')}</a
          >
        </li>
        <li><a href={sheetUrl(entry)} rel="external">{j('entry.sheet')}</a></li>
        <li><a href={midiUrl(entry)} rel="external">{j('entry.midi')}</a></li>
        <li><a href={entry.journal} rel="external">{j('entry.journal')}</a></li>
      </ul>
      <p class="agent">
        {agent}{#if entry.style}
          · <span data-testid="entry-style">{fill(j('entry.style'), { style: entry.style })}</span>{/if} ·
        <span class="commit">{fill(j('entry.commit'), { commit: entry.commit })}</span>
      </p>
    </div>
  </div>
{/snippet}

{#if variant === 'featured'}
  <article class="featured" {id} aria-labelledby="{id}-title">
    <div class="panel">
      <Cover {entry} id={stage} label={coverLabel} />
      <div class="head">
        <svelte:element this={heading} class="lp-display title" id="{id}-title" lang="fr">{@render name()}</svelte:element>
        <p class="genre">{entry.genre[lang]} · <time datetime={entry.date}>{dayName(entry.date, lang)}</time></p>
        <ul class="specs" aria-label={j('entry.specs')}>
          {#each specs as spec, i (i)}<li>{spec}</li>{/each}
        </ul>
        <Player {entry} {j} {stage} />
      </div>
    </div>
    {@render details()}
  </article>
{:else}
  <article class="day" {id} aria-labelledby="{id}-title">
    <div class="rail" aria-hidden="true">
      <span class="lp-display num">{parts.day}</span>
      <span class="month">{parts.month}</span>
    </div>
    <div class="media">
      <Cover {entry} id={stage} label={coverLabel} />
      <Player {entry} {j} {stage} size="small" />
    </div>
    <div class="body">
      <svelte:element this={heading} class="lp-display title" id="{id}-title" lang="fr">{@render name()}</svelte:element>
      <p class="genre">{entry.genre[lang]} · <time datetime={entry.date}>{dayName(entry.date, lang)}</time></p>
      <ul class="specs" aria-label={j('entry.specs')}>
        {#each specs as spec, i (i)}<li>{spec}</li>{/each}
      </ul>
      {@render details()}
    </div>
  </article>
{/if}

<style>
  /* Featured: an ink panel like the landing page's stage, the cover on the left. */
  .featured {
    display: flex;
    flex-direction: column;
    gap: clamp(32px, 4vw, 56px);
  }
  .panel {
    display: grid;
    grid-template-columns: minmax(0, 5fr) minmax(0, 6fr);
    gap: clamp(24px, 4vw, 64px);
    align-items: end;
    padding: clamp(16px, 2.4vw, 32px);
    border-radius: 28px;
    background: var(--fk-ink);
    color: var(--fk-ink-text);
  }
  .panel .head {
    /* Ultraviolet on ink (DS-0.1): the accent where it sits on the dark panel. */
    --player-played: #9c92ff;
    --player-ground: var(--fk-ink);
    display: flex;
    flex-direction: column;
    gap: 20px;
    padding: 0 clamp(0px, 1.6vw, 24px) clamp(8px, 1.6vw, 24px) 0;
  }
  .featured .title {
    font-size: clamp(40px, 6.4vw, 104px);
    line-height: 0.86;
    letter-spacing: -0.035em;
    text-transform: uppercase;
    text-wrap: balance;
    hyphens: manual;
    overflow-wrap: break-word;
  }
  .permalink {
    text-decoration: none;
  }
  .permalink:hover {
    text-decoration: underline;
    text-decoration-thickness: 0.06em;
    text-underline-offset: 0.08em;
  }
  .panel .permalink:hover {
    color: inherit;
  }
  .panel .genre {
    color: var(--fk-ink-muted-2);
  }
  .panel .specs li {
    border-color: rgb(236 233 230 / 0.35);
  }

  /* Feed: a date rail, the cover and its player, then the text. */
  .day {
    display: grid;
    grid-template-columns: 140px minmax(220px, 300px) minmax(0, 1fr);
    gap: clamp(20px, 3vw, 48px);
    padding-top: 24px;
    border-top: 1px solid var(--fk-text);
  }
  .rail {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .num {
    font-size: clamp(56px, 6vw, 96px);
    line-height: 0.82;
    letter-spacing: -0.04em;
  }
  .month {
    font-family: var(--fk-font-mono);
    font-size: 13px;
    letter-spacing: 0.04em;
  }
  .media {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
  }
  .day .title {
    font-size: clamp(32px, 3.6vw, 56px);
    line-height: 0.9;
    letter-spacing: -0.03em;
    text-transform: uppercase;
    text-wrap: balance;
  }
  .day .details {
    margin-top: 20px;
    grid-template-columns: minmax(0, 1fr);
    gap: 28px;
  }

  /* Shared. */
  .genre {
    margin: 0;
    font-size: 17px;
  }
  .genre time {
    white-space: nowrap;
  }
  .specs {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin: 0;
    padding: 0;
    list-style: none;
    font-family: var(--fk-font-mono);
    font-size: 13px;
    font-feature-settings: 'tnum' 1;
  }
  .specs li {
    padding: 5px 12px;
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-pill);
  }
  .details {
    display: grid;
    grid-template-columns: minmax(0, 7fr) minmax(0, 5fr);
    gap: clamp(24px, 4vw, 72px);
  }
  .story,
  .side {
    display: flex;
    flex-direction: column;
    gap: 28px;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .label {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--fk-text-muted);
  }
  p {
    margin: 0;
    max-width: 66ch;
    font-size: 17px;
    font-weight: 400;
    line-height: 1.5;
    color: var(--fk-text-soft);
    text-wrap: pretty;
  }
  .brief {
    font-size: clamp(19px, 1.7vw, 23px);
    font-weight: 300;
    line-height: 1.38;
    color: var(--fk-text);
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 66ch;
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: 15px;
    line-height: 1.45;
  }
  .list li {
    position: relative;
    padding-left: 18px;
  }
  .list li::before {
    content: '';
    position: absolute;
    left: 0;
    top: 0.62em;
    width: 8px;
    height: 2px;
    background: var(--fk-text);
  }
  .gaps li::before {
    background: var(--fk-accent);
  }
  /* The form: one block per section, as wide as its bars, as tall as its density. */
  .form {
    display: flex;
    gap: 3px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .form li {
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    flex-basis: 0;
    min-width: 0;
    gap: 6px;
  }
  .bar {
    display: block;
    height: calc(32px * var(--h));
    min-height: 6px;
    border-radius: 2px;
    background: var(--fk-text);
  }
  .form li:nth-child(even) .bar {
    background: var(--fk-text-soft);
  }
  /*
   * The section playing (set by live/index.ts: aria-current, and --p, its progress). Its bar swaps
   * ink for the accent's tint and an accent fill grows across it; the name takes the accent.
   * Without the script, or at rest, none of this applies.
   */
  .bar {
    position: relative;
    overflow: hidden;
    transition:
      background-color 280ms,
      min-height 280ms cubic-bezier(0.2, 0.7, 0, 1);
  }
  .bar::after {
    content: '';
    position: absolute;
    inset: 0;
    background: var(--fk-accent);
    transform: scaleX(0);
    transform-origin: left;
  }
  .form li:global([aria-current]) .bar {
    min-height: 12px;
    background: var(--fk-accent-tint);
  }
  .form li:global([aria-current]) .bar::after {
    transform: scaleX(var(--p, 1));
  }
  .form li:global([aria-current]) .name {
    color: var(--fk-accent-strong);
  }
  .name {
    transition: color 280ms;
  }
  @media (prefers-reduced-motion: reduce) {
    .bar,
    .name {
      transition: none;
    }
    .form li:global([aria-current]) .bar {
      background: var(--fk-accent);
    }
  }
  .name,
  .count {
    overflow: hidden;
    font-size: 11px;
    line-height: 1.2;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .name {
    font-weight: 600;
  }
  .count {
    font-family: var(--fk-font-mono);
    color: var(--fk-text-muted);
  }
  .links {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 20px;
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: 15px;
    font-weight: 500;
  }
  .links a:not(.open) {
    text-decoration: underline;
    text-decoration-thickness: 1px;
    text-underline-offset: 4px;
  }
  /* The entry's one call to action: smaller than the landing page's hero pill, so a list of days stays calm. */
  .links .open {
    height: 44px;
    padding: 0 22px;
    font-size: 15px;
  }
  .agent {
    font-family: var(--fk-font-mono);
    font-size: 12px;
    letter-spacing: 0.02em;
    color: var(--fk-text-muted);
  }
  .commit {
    white-space: nowrap;
  }

  /* On wide screens the cover and its player stay in view while the text scrolls by. */
  @media (min-width: 1101px) {
    .media {
      position: sticky;
      top: 24px;
      align-self: start;
    }
  }
  @media (max-width: 1100px) {
    .day {
      grid-template-columns: 120px minmax(0, 1fr);
    }
    .day .body {
      grid-column: 1 / -1;
    }
  }
  @media (max-width: 860px) {
    .panel {
      grid-template-columns: minmax(0, 1fr);
    }
    .details {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  @media (max-width: 560px) {
    .day {
      grid-template-columns: minmax(0, 1fr);
      gap: 16px;
    }
    .rail {
      flex-direction: row;
      align-items: baseline;
      gap: 12px;
    }
    .num {
      font-size: 56px;
    }
  }
</style>
