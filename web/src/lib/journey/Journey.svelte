<script lang="ts">
  import { resolve } from '$app/paths'
  import { url as liveUrl } from 'virtual:journey-live'
  import { fill, LANGS, type Lang } from '../i18n/lang'
  import { journeyText } from '../i18n/journey'
  import { landingText } from '../i18n/landing'
  import Footer from '../landing/Footer.svelte'
  import Nav from '../landing/Nav.svelte'
  import { detectScript, inlineScript, rememberScript } from '../landing/scripts'
  import '../landing/landing.css'
  import { journeyBootScript } from './boot'
  import { ENTRIES, entriesOf, entryByFolder, folderOf, monthOf, MONTHS, neighbours, RECENT } from './entries'
  import Entry from './Entry.svelte'
  import { clock, dayName, keyName, monthName } from './format'
  import { journeyHref, type JourneyView } from './paths'
  import type { JourneyEntry } from './types'

  /**
   * Every page of the Journey: the main page (`home`: the latest day featured, the `RECENT` days
   * before it, then the months), a monthly archive, or one day's page. Each entry is shown by the
   * same `Entry` component everywhere. `detect`: the unprefixed address, which moves French
   * readers to `/fr/...`.
   */
  let { lang, view, detect = false }: { lang: Lang; view: JourneyView; detect?: boolean } = $props()

  const t = $derived(landingText(lang))
  const j = $derived(journeyText(lang))
  const href = (v: JourneyView, l: Lang = lang) => journeyHref(v, l)
  const dayHref = (e: JourneyEntry) => href({ kind: 'day', day: folderOf(e) })
  const days = (n: number) => (n === 1 ? j('index.one') : fill(j('index.count'), { count: n }))
  const scripts = $derived(
    (detect ? inlineScript(detectScript(href(view, 'fr'))) : '') +
      inlineScript(rememberScript(detect ? undefined : { href: resolve('/app'), lang })) +
      inlineScript(journeyBootScript(liveUrl), 'module'),
  )

  const [latest, ...rest] = ENTRIES
  const recent = rest.slice(0, RECENT)
  const month = $derived(view.kind === 'month' ? view.month : undefined)
  const monthEntries = $derived(month ? entriesOf(month) : [])
  const around = $derived(month ? neighbours(month) : {})
  const day = $derived(view.kind === 'day' ? entryByFolder(view.day) : undefined)
  const dayIndex = $derived(day ? ENTRIES.indexOf(day) : -1)
  const olderDay = $derived(dayIndex >= 0 ? ENTRIES[dayIndex + 1] : undefined)
  const newerDay = $derived(dayIndex > 0 ? ENTRIES[dayIndex - 1] : undefined)
  type Link = { href: string; text: string } | undefined
  /** The pager's two ends: the months around a month, or the days around a day. */
  const links = $derived.by((): { older: Link; newer: Link } => {
    const m = (to: string | undefined, key: 'month.older' | 'month.newer'): Link =>
      to ? { href: href({ kind: 'month', month: to }), text: fill(j(key), { month: monthName(to, lang) }) } : undefined
    const d = (to: JourneyEntry | undefined, key: 'day.older' | 'day.newer'): Link =>
      to ? { href: dayHref(to), text: fill(j(key), { title: to.title }) } : undefined
    return month
      ? { older: m(around.older, 'month.older'), newer: m(around.newer, 'month.newer') }
      : { older: d(olderDay, 'day.older'), newer: d(newerDay, 'day.newer') }
  })

  const title = $derived(month ? fill(j('meta.month'), { month: monthName(month, lang) }) : day ? fill(j('meta.day'), { title: day.title }) : j('meta.title'))
  const description = $derived(month ? fill(j('meta.monthDescription'), { month: monthName(month, lang) }) : day ? day.brief[lang] : j('meta.description'))
</script>

<svelte:head>
  <title>{title}</title>
  <meta name="description" content={description} />
  <link rel="canonical" href={href(view)} />
  {#each LANGS as l (l)}
    <link rel="alternate" hreflang={l} href={href(view, l)} />
  {/each}
  <link rel="alternate" hreflang="x-default" href={journeyHref(view)} />
  <!-- Generated from tested code (landing/scripts.ts, journey/boot.ts); the players load lazily. -->
  <!-- eslint-disable-next-line svelte/no-at-html-tags -->
  {@html scripts}
</svelte:head>

{#snippet months()}
  <section class="lp-wrap archive" id="archive" aria-labelledby="archive-title">
    <h2 class="lp-display archive-title" id="archive-title">{j('archive.title')}</h2>
    <ol>
      {#each MONTHS as m (m)}
        <li>
          <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- journeyHref resolves -->
          <a href={href({ kind: 'month', month: m })} aria-current={m === month ? 'page' : undefined}>
            <span class="name">{monthName(m, lang)}</span>
            <span class="facts">{days(entriesOf(m).length)}</span>
          </a>
        </li>
      {/each}
    </ol>
  </section>
{/snippet}

{#snippet pager(label: string, older: Link, newer: Link)}
  <nav class="lp-wrap pager" aria-label={label}>
    {#if older}
      <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- journeyHref resolves -->
      <a class="older" rel="prev" href={older.href}>{older.text}</a>
    {/if}
    {#if newer}
      <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- journeyHref resolves -->
      <a class="newer" rel="next" href={newer.href}>{newer.text}</a>
    {/if}
  </nav>
{/snippet}

<div class="landing journey">
  <a class="skip" href="#main">{t('skip')}</a>
  <Nav {t} {lang} page="journey" langHref={(l) => href(view, l)} />
  <main id="main" tabindex="-1">
    {#if view.kind === 'home'}
      <header class="lp-wrap intro">
        <h1 class="lp-display lp-enter">{j('hero.title')}</h1>
        <div class="row">
          <div class="copy lp-enter" style="--lp-delay: 120ms">
            <p class="lead">{j('hero.lead')}</p>
            <p class="note">{j('hero.note')}</p>
          </div>
          <nav class="index lp-enter" style="--lp-delay: 220ms" aria-labelledby="index-title">
            <h2 class="index-title" id="index-title">
              <span>{j('index.label')}</span><span class="count">{days(ENTRIES.length)}</span>
            </h2>
            <ol>
              {#each ENTRIES.slice(0, 4) as entry (entry.date)}
                <li>
                  <a href="#day-{entry.date}">
                    <time datetime={entry.date}>{dayName(entry.date, lang)}</time>
                    <span class="name" lang="fr">{entry.title}</span>
                    <span class="facts">{fill(j('entry.bpm'), { bpm: entry.bpm })} · {keyName(entry.key, lang)} · {clock(entry.duration)}</span>
                  </a>
                </li>
              {/each}
            </ol>
            <a class="more" href="#archive">{j('index.archive')}</a>
          </nav>
        </div>
      </header>

      <section class="lp-wrap latest" aria-label={j('latest.label')}>
        <Entry entry={latest} {lang} {j} variant="featured" permalink={dayHref(latest)} />
      </section>

      {#if recent.length}
        <section class="lp-wrap feed" aria-labelledby="earlier-title">
          <h2 class="lp-display lp-h2" id="earlier-title">{j('earlier.title')}</h2>
          <ol class="days">
            {#each recent as entry (entry.date)}
              <li><Entry {entry} {lang} {j} variant="feed" permalink={dayHref(entry)} /></li>
            {/each}
          </ol>
        </section>
      {/if}
      {@render months()}
    {:else if month}
      <header class="lp-wrap intro">
        <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- journeyHref resolves -->
        <a class="back" href={href({ kind: 'home' })}>{j('back.home')}</a>
        <h1 class="lp-display">{monthName(month, lang).toUpperCase()}</h1>
        <p class="note">{days(monthEntries.length)}</p>
      </header>
      <section class="lp-wrap feed" aria-labelledby="month-title">
        <h2 class="visually-hidden" id="month-title">{monthName(month, lang)}</h2>
        <ol class="days">
          {#each monthEntries as entry (entry.date)}
            <li><Entry {entry} {lang} {j} variant="feed" permalink={dayHref(entry)} /></li>
          {/each}
        </ol>
      </section>
      {@render pager(j('month.nav'), links.older, links.newer)}
      {@render months()}
    {:else if day}
      <div class="lp-wrap crumbs">
        <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- journeyHref resolves -->
        <a class="back" href={href({ kind: 'home' })}>{j('back.home')}</a>
        <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- journeyHref resolves -->
        <a class="back" href={href({ kind: 'month', month: monthOf(day) })}>{fill(j('day.month'), { month: monthName(monthOf(day), lang) })}</a>
      </div>
      <section class="lp-wrap latest">
        <Entry entry={day} {lang} {j} variant="featured" heading="h1" />
      </section>
      {@render pager(j('day.nav'), links.older, links.newer)}
    {/if}
    <p class="lp-wrap licence">
      {j('licence.lead')}
      <a href="https://creativecommons.org/licenses/by/4.0/" rel="license external">{j('licence.name')}</a>. {j('licence.credit')}
    </p>
  </main>
  <Footer {t} {lang} page="journey" langHref={(l) => href(view, l)} />
</div>

<style>
  .intro {
    display: flex;
    flex-direction: column;
    gap: clamp(24px, 3vw, 40px);
    padding-top: clamp(24px, 4vw, 56px);
    padding-bottom: clamp(40px, 5vw, 72px);
  }
  h1 {
    font-size: clamp(38px, 6.4vw, 96px);
    line-height: 0.86;
    letter-spacing: -0.035em;
    text-wrap: balance;
  }
  .row {
    display: grid;
    grid-template-columns: minmax(0, 7fr) minmax(0, 5fr);
    gap: clamp(24px, 5vw, 96px);
    align-items: start;
  }
  .copy {
    display: flex;
    flex-direction: column;
    gap: 16px;
    max-width: 640px;
  }
  .lead {
    margin: 0;
    font-size: clamp(19px, 1.8vw, 24px);
    font-weight: 300;
    line-height: 1.35;
    text-wrap: pretty;
  }
  .note {
    margin: 0;
    font-size: 15px;
    color: var(--fk-text-muted);
  }
  /* The index: the latest days, as a short track list. */
  .index-title {
    display: flex;
    justify-content: space-between;
    margin: 0;
    padding-bottom: 10px;
    border-bottom: 1px solid var(--fk-text);
    font-family: var(--fk-font-mono);
    font-size: 13px;
    font-weight: 400;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
  .count {
    color: var(--fk-text-muted);
  }
  .index ol {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .index li {
    border-bottom: 1px solid var(--fk-line);
  }
  .index a:not(.more) {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 2px 16px;
    padding: 12px 0;
    text-decoration: none;
  }
  .index time {
    grid-column: 1 / -1;
    font-family: var(--fk-font-mono);
    font-size: 12px;
    letter-spacing: 0.04em;
    color: var(--fk-text-muted);
  }
  .index .name {
    font-size: 19px;
    font-weight: 600;
  }
  .index .facts {
    align-self: center;
    font-family: var(--fk-font-mono);
    font-size: 12px;
    font-feature-settings: 'tnum' 1;
    color: var(--fk-text-soft);
  }
  .index a:hover .name {
    color: var(--fk-accent);
  }
  a:focus-visible {
    outline: 2px solid var(--fk-accent);
    outline-offset: 2px;
  }
  .more,
  .back {
    display: inline-block;
    font-size: 15px;
    font-weight: 500;
    text-underline-offset: 4px;
  }
  .more {
    margin-top: 14px;
  }
  .crumbs {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 24px;
    padding-block: clamp(16px, 2vw, 32px);
  }
  .latest {
    padding-bottom: clamp(48px, 6vw, 96px);
  }
  .feed {
    display: flex;
    flex-direction: column;
    gap: clamp(32px, 4vw, 56px);
    padding-bottom: clamp(48px, 6vw, 96px);
  }
  .days {
    display: flex;
    flex-direction: column;
    gap: clamp(56px, 6vw, 96px);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  /* Older and newer: a plain two-ended row. */
  .pager {
    display: flex;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 12px 24px;
    padding-bottom: clamp(48px, 6vw, 96px);
  }
  .pager a {
    font-size: 17px;
    font-weight: 500;
    text-underline-offset: 4px;
  }
  .pager .newer {
    margin-left: auto;
    text-align: right;
  }
  /* The months: a grid of rows on rules. */
  .archive {
    display: flex;
    flex-direction: column;
    gap: 24px;
    padding-bottom: clamp(48px, 6vw, 96px);
  }
  .archive-title {
    font-size: clamp(32px, 4vw, 56px);
    line-height: 0.9;
    letter-spacing: -0.03em;
  }
  .archive ol {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(240px, 100%), 1fr));
    gap: 0 32px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .archive li {
    border-top: 1px solid var(--fk-text);
  }
  .archive a {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 16px;
    padding: 14px 0 18px;
    text-decoration: none;
  }
  .archive .name {
    font-size: 19px;
    font-weight: 600;
  }
  .archive .name::first-letter {
    text-transform: uppercase;
  }
  .archive .facts {
    font-family: var(--fk-font-mono);
    font-size: 12px;
    color: var(--fk-text-soft);
  }
  .archive a[aria-current='page'] .name {
    text-decoration: underline;
    text-decoration-thickness: 2px;
    text-underline-offset: 5px;
  }
  .licence {
    margin: 0 auto;
    padding-block: 8px 48px;
    font-size: 14px;
    color: var(--fk-text-soft);
  }
  .licence a {
    font-weight: 600;
    text-underline-offset: 3px;
  }
  .journey :global(::selection) {
    background: var(--fk-accent);
    color: #fff;
  }
  @media (max-width: 860px) {
    .row {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
