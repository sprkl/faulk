/**
 * The hook for an audio file dropped on the empty area of the track list (#190). The audio import has landed
 * (AU-1: `AudioImport.importFiles` into an existing audio track, one history entry per file), but creating the track
 * and importing the files as one undo group needs a history group held open over asynchronous work, and the track
 * list does not offer audio tracks yet (`OFFER_AUDIO_TRACK`). So the drop is recognised, nothing is changed, and the
 * person is told so. The files come from the browser's own `DataTransfer`.
 */

import type { MessageKey } from '../i18n'

/** What a drop of audio files does today: nothing, and it says so. Returns the message to show. */
export function dropAudioFiles(files: readonly File[]): MessageKey {
  void files
  return 'browser.audioSoon'
}
