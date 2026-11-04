/**
 * Whether the UI offers audio import (the Clips panel's "Import audio…" button and its progress). Imported audio
 * clips are still silent until AU-4 (#113), so the production build hides the entry point; the dev server keeps
 * it for the end-to-end specs. The commands (`clip.choose_audio`, `clip.import_audio`) stay available to agents.
 * Remove with AU-4 (#113): set it to `true` (or delete it).
 */
export const AUDIO_IMPORT_UI = import.meta.env.DEV
