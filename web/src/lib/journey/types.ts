/**
 * One day of the Journey page (`/journey`): the track the product-owner agent made in the latest
 * development build of Faulk. Each entry is one file of `entries/`, named `<date>-<slug>.ts`; its
 * audio, MIDI and sheet live in the public repository `sprkl/faulk-journey`, folder
 * `tracks/<date>-<slug>/` (`docs/agents/journey.md`).
 */

import type { Lang } from '../i18n/lang'

/** A text in both languages of the site. */
export type Localized = Record<Lang, string>

/** Pitch classes, sharps only (the sheets spell keys this way). */
export const TONICS = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'] as const
export type Tonic = (typeof TONICS)[number]

export const MODES = ['major', 'minor', 'dorian', 'mixolydian', 'phrygian', 'lydian'] as const
export type Mode = (typeof MODES)[number]

export interface JourneyTrack {
  /** The track's name in the project, as the agent named it. */
  name: string
  /** Its colour in the project, `#rrggbb`. */
  color: string
}

export interface JourneySection {
  name: Localized
  /** Length in bars. The sections add up to the entry's `bars`. */
  bars: number
  /** Number of parts (patterns) playing in it: the arrangement's density. */
  parts: number
}

export interface JourneyEntry {
  /** Day of the run, `YYYY-MM-DD`. */
  date: string
  /** Lower-case ASCII words joined by `-`; the folder is `<date>-<slug>`. */
  slug: string
  /** The track's title, as the agent named it: the same in every language, like a work's title. */
  title: string
  /** The `develop` commit the agent built and used, short hash. */
  commit: string
  /** The style, one line. */
  genre: Localized
  bpm: number
  key: { tonic: Tonic; mode: Mode }
  /** Length of the arrangement in bars. */
  bars: number
  /** Length of the exported audio (`track.mp3`), in seconds. */
  duration: number
  /** The model the agent ran on. */
  model: string
  /** The skills the agent followed. */
  skills: string[]
  /** The brief in one or two sentences. */
  brief: Localized
  /** How far the track was built in Faulk. */
  built: Localized
  /** What was new in Faulk that day and that the track put to the test. */
  tested: Localized[]
  /** The gaps the run hit, most painful first. */
  gaps: Localized[]
  /** The run's comment in the PO journal, issue #362. */
  journal: string
  /** The project's tracks, in their order. */
  tracks: JourneyTrack[]
  /** The arrangement, section by section. */
  sections: JourneySection[]
}
