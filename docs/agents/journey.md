---
type: Agent Configuration
title: "Journey: adding a day"
description: "How the lead or the product-owner agent publishes a run's track on the public Journey page: one folder in sprkl/faulk-journey, one entry file in this repository."
status: draft
---

# Journey: adding a day

The Journey page (`/journey`, `/en/journey`, `/fr/journey`) shows, day by day, the track the product-owner agent made in the latest `develop` (#495). Adding a day is two steps and nothing else: one folder in the audio repository, one entry file here. No list, route or component changes.

## 1. The audio repository

The public repository `sprkl/faulk-journey` holds the audio, so this repository never receives an audio file. Add one folder, `tracks/<date>-<slug>/`, with:

- `track.mp3`: the WAV exported from Faulk, encoded to MP3 with no other processing (`ffmpeg -i "<export>.wav" -codec:a libmp3lame -q:a 2 track.mp3`);
- `track.mid`: the MIDI render of the sheet (`sheet_tool.py render`);
- `sheet.md`: the production sheet;
- `project.faulkpack`: the run's Faulk project as the agent left it, exported from the app (Project menu > Export as .faulkpack…, the file renamed). Check that it opens: `/app?open=<its jsDelivr URL>` asks first, then the project opens and plays (#497).

`<date>` is the run's day (`YYYY-MM-DD`). `<slug>` is the title in lower-case ASCII words joined by `-` (`Néon sur la rocade` gives `neon-sur-la-rocade`). Push to `main`. The page reads the files through jsDelivr (`https://cdn.jsdelivr.net/gh/sprkl/faulk-journey@main/tracks/<date>-<slug>/track.mp3`), which sends open CORS and the right MIME type, so the visualiser can analyse the audio. The CDN caches `@main` for a while: check that the new URL answers before opening the pull request. The content is licensed CC BY 4.0 (`LICENSE` in that repository; the page says so once).

## 2. The entry file

Copy the latest file of `web/src/lib/journey/entries/` to `<date>-<slug>.ts` (the name must be the folder's) and fill it in from the sheet, the run's report and its comment in the journal #362. The fields are typed and documented in `web/src/lib/journey/types.ts`:

- `date`, `slug`, `title` (as the agent named it, never translated), `commit` (the `develop` hash the run built);
- `genre`, `bpm`, `key` (`tonic`, `mode`), `bars`, `duration` (the MP3's length in seconds);
- `model` and `skills`, and `style` (optional): the name of the style profile the run applied when its brief asked for one (`.claude/skills/music-producer/references/styles/`), written as the profile is named (`morzak`), one line of at most 40 characters, the same in both languages; leave it out when the run applied none. The entry shows it discreetly in its last line, next to the model and the commit ("Style profile: morzak", "Profil de style : morzak");
- `brief`, `built` (how far it got in Faulk), `tested` (the day's new features the track put to the test), `gaps` (most painful first), each in English and French;
- `journal`: the URL of the run's comment in #362;
- `tracks`: names and colours from the saved project (`track.json` of each track);
- `sections`: name (EN and FR), bars and number of parts, from the arrangement block of `sheet_tool.py check`. They must add up to `bars`.

Write the texts short and factual: what the agent tried, how far it got, what was missing. No hype, no claim the run does not support, never the name of another DAW.

The cover is generated from the date, the slug and the musical data (`art.ts`): nothing to draw or store, and fixing a typo in the texts does not change it.

## 3. Check and ship

From `web/`: `pnpm exec vitest run src/lib/journey` validates every entry (fields, file name, bars, links; the build throws on an invalid one too), then `pnpm exec playwright test journey` (with `FAULK_E2E_PROJECTS=dev`). Open the pull request against `develop` as usual; the newest entry becomes the featured one and the previous one moves to the feed.

The pages follow from the entry files, with nothing to add: the main page shows the latest day, the 10 days before it (`RECENT` in `entries.ts`) and the months; each month has its archive (`/journey/2026-11`), each day its own page (`/journey/2026-11-02-neon-sur-la-rocade`), in English and French (`/en/...`, `/fr/...`). The routes' `entries()` read the same list, and `routes.test.ts` checks that every day and month is prerendered. Every entry is drawn by the one `Entry.svelte` wherever it appears, with its actions row: "Open in Faulk" (`/app?open=` with the day's `project.faulkpack` on jsDelivr, `packUrl` in `entries.ts`), the sheet, the MIDI file and the journal. The button points at the pack by the folder's name, so a day without `project.faulkpack` shows a button that cannot download: add the pack before the entry. The players fetch no audio before a play (`preload="none"`).
