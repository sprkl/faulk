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
- `project.faulkpack`: the run's Faulk project as the agent left it, exported from the app (Project menu > Export as .faulkpack…, the file renamed), or built afterwards from the saved project folder without its `.faulk/` with `project::file::export_pack` (say so in `built`). Check that it opens: `/app?open=<its raw URL>` asks first, then the project opens and plays (#497).

`<date>` is the run's day (`YYYY-MM-DD`). `<slug>` is the title in lower-case ASCII words joined by `-` (`Néon sur la rocade` gives `neon-sur-la-rocade`). Push to `main`. The page reads the files from `raw.githubusercontent.com` (`https://raw.githubusercontent.com/sprkl/faulk-journey/main/tracks/<date>-<slug>/track.mp3`, the `CDN` constant of `entries.ts`), which sends open CORS and the right MIME type, so the visualiser can analyse the audio. jsDelivr was used first and refused files once the repository passed 50 MB ("Package size exceeded the configured limit of 50 MB", #945); raw served the 66 MB repository without refusing a file (measured on 2026-11-20), but it is not a CDN with a contract (no SLA, undocumented limits; it answered `cache-control: max-age=300`, a 5-minute cache), so if it ever throttles, change that one constant to another host (GitHub Pages or a Blob store, each needing a setting of the account). After the push, check the four URLs with `node scripts/journey/check-urls.mjs <date>-<slug> --dir <the day's folder in the audio repository>`: each must answer 200 with the right `Content-Type`, `access-control-allow-origin: *` and the committed file's SHA-256 (`--commit <sha>` compares with the file at a commit instead). A file not served yet is a cache of up to 5 minutes: run it again. Do not open the pull request before it passes. The content is licensed CC BY 4.0 (`LICENSE` in that repository; the page says so once).

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

## Correcting a published day

When the founder asks to replace a day's track (a wrong note, a bad export), keep the date, the slug and the folder, so the day's URL stays. In the audio repository, replace only the files that changed (`track.mp3`, `project.faulkpack`; `sheet.md` and `track.mid` stay when the sheet was right), then run `check-urls.mjs` with `--dir` on the corrected folder until it passes: raw caches for 5 minutes, so the old file can still be heard for that long (the check compares the checksum, so it fails until the new file is served). In the entry, add ` (v2)` to `title` (then v3…; the slug does not change) and one sentence in `built`, in both languages, saying what the new version changed and why; leave `journal` on the run's comment and post the correction as a reply in #362. Check every melodic clip of the corrected project against the sheet before publishing.

## 3. Check and ship

From `web/`: `pnpm exec vitest run src/lib/journey` validates every entry (fields, file name, bars, links; the build throws on an invalid one too), then `pnpm exec playwright test journey` (with `FAULK_E2E_PROJECTS=dev`). Open the pull request against `develop` as usual; the newest entry becomes the featured one and the previous one moves to the feed.

The pages follow from the entry files, with nothing to add: the main page shows the latest day, the 10 days before it (`RECENT` in `entries.ts`) and the months; each month has its archive (`/journey/2026-11`), each day its own page (`/journey/2026-11-02-neon-sur-la-rocade`), in English and French (`/en/...`, `/fr/...`). The routes' `entries()` read the same list, and `routes.test.ts` checks that every day and month is prerendered. Every entry is drawn by the one `Entry.svelte` wherever it appears, with its actions row: "Open in Faulk" (`/app?open=` with the day's `project.faulkpack` on the journey host, `packUrl` in `entries.ts`; an old `/app?open=` link to the jsDelivr `@main` URL of a day's pack is moved to the raw one by `currentLink` in `open-link.ts`), the sheet, the MIDI file and the journal. The button points at the pack by the folder's name, so a day without `project.faulkpack` shows a button that cannot download: add the pack before the entry. The players fetch no audio before a play (`preload="none"`).
