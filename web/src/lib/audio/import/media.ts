// The audio files a project's clips play (AU-1), as the store checks their PCM caches.
import type { Project } from '../../generated/protocol'
import type { MediaRef } from './pipeline'

/** Every audio file the project's clips play, once each. */
export function audioMediaOf(project: Project | undefined): MediaRef[] {
  if (!project) return []
  const found = new Map<string, MediaRef>()
  for (const track of Object.values(project.tracks)) {
    if (!track) continue
    for (const clip of Object.values(track.clips)) {
      if (!clip) continue
      if (!('audio' in clip.content)) continue
      const { hash, sample_rate, channels, frames } = clip.content.audio.media
      if (!found.has(hash)) found.set(hash, { hash, sample_rate, channels, frames })
    }
  }
  return [...found.values()]
}
