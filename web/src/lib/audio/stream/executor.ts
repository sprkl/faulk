// The disk side of the Web engine (AU-3, docs/architecture/03-stockage-local.md § 5.11): runs in the
// engine's prepare Worker, next to the prepare instance that plans the reads
// (`engine::audio_source::Disk`). Every tick it asks the planner for its read jobs (`fk_disk_poll`),
// reads each byte range of `blobs/<hash>` or `.faulk/cache/pcm/<hash>.pcm` in the project's folder
// with `File.slice()`, and hands the bytes back (`fk_disk_done`) or says why it could not
// (`fk_disk_failed`). It never touches the audio thread: the planner writes the rings and sends the
// segments. The reads are asynchronous, so the Worker's 5 ms flush is never held up by the disk.
//
// `File.slice` on the `File` of a handle, for both a real folder and the browser's storage (OPFS): a
// `SyncAccessHandle` would take an exclusive lock on the file, which the storage Worker needs while it
// makes a PCM cache or imports a file.

/** The exports the executor calls (crates/engine-wasm, prepare instance). */
export interface DiskExports {
  fk_disk_poll(channel: number, nowMs: number): number
  fk_disk_jobs(channel: number): number
  fk_disk_inbox(channel: number, len: number): number
  fk_disk_done(channel: number, id: number, len: number, fileLen: number): number
  fk_disk_failed(channel: number, id: number, code: number): void
}

/** `engine::audio_source::JOB_WIRE_BYTES`. */
export const JOB_BYTES = 56
/** `engine::audio_source::Failure` codes. */
export const FAIL_NOT_FOUND = 1
export const FAIL_UNREADABLE = 2
export const FAIL_CHANGED = 3

/** A read job: a byte range of a media's blob (`file` 0) or PCM cache (`file` 1). */
export interface Job {
  id: number
  file: 0 | 1
  media: string
  offset: number
  len: number
}

/** The part of a directory handle the executor uses (a real `FileSystemDirectoryHandle`, or a test's). */
export interface Dir {
  getDirectoryHandle(name: string): Promise<Dir>
  getFileHandle(name: string): Promise<{ getFile(): Promise<File> }>
}

const hex = (bytes: Uint8Array) => Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')

/** Decodes the job table `fk_disk_poll` wrote. */
export function readJobs(buffer: ArrayBufferLike, at: number, count: number): Job[] {
  const view = new DataView(buffer, at, count * JOB_BYTES)
  const bytes = new Uint8Array(buffer, at, count * JOB_BYTES)
  const jobs: Job[] = []
  for (let i = 0; i < count; i++) {
    const o = i * JOB_BYTES
    jobs.push({
      id: view.getUint32(o, true),
      file: view.getUint8(o + 4) === 1 ? 1 : 0,
      offset: Number(view.getBigUint64(o + 8, true)),
      len: view.getUint32(o + 16, true),
      media: hex(bytes.slice(o + 24, o + 56)),
    })
  }
  return jobs
}

/** The path parts of a job's file in the project folder. */
export function pathOf(job: Pick<Job, 'file' | 'media'>): string[] {
  return job.file === 0 ? ['blobs', job.media] : ['.faulk', 'cache', 'pcm', `${job.media}.pcm`]
}

const nameOf = (error: unknown) => (error instanceof DOMException || error instanceof Error ? error.name : '')

/**
 * Runs the planner's jobs against the project's folder. `folder(null)` (no project place yet, or the
 * helper plays): every job fails as not found, so the planner keeps its sources offline.
 */
export class DiskExecutor {
  private dir: Dir | null = null
  /** The `File` of each path, read once (a `File` is a snapshot: a file written since throws `NotReadableError`). */
  private readonly files = new Map<string, File>()
  /** Bumped by each `folder`: a read of the previous folder is answered as not found. */
  private generation = 0
  /** Reads in flight, for tests to wait on. */
  readonly inFlight = new Set<Promise<void>>()
  /** Bytes read and time spent reading, for the measurements (AU-3). */
  readonly stats = { reads: 0, bytes: 0, ms: 0 }

  constructor(
    private readonly exports: DiskExports,
    private readonly memory: WebAssembly.Memory,
    private readonly channel: number,
    private readonly now: () => number = () => performance.now(),
  ) {}

  /** The project's folder (a real folder or a directory of the browser's storage), or none. */
  folder(dir: Dir | null) {
    this.dir = dir
    this.files.clear()
    this.generation++
  }

  /** One poll of the planner, and its reads started. */
  tick() {
    const count = this.exports.fk_disk_poll(this.channel, this.now())
    if (count === 0) return
    const jobs = readJobs(this.memory.buffer, this.exports.fk_disk_jobs(this.channel), count)
    for (const job of jobs) {
      const read = this.run(job).finally(() => this.inFlight.delete(read))
      this.inFlight.add(read)
    }
  }

  private async run(job: Job) {
    const generation = this.generation
    const started = this.now()
    let code = 0
    let bytes: Uint8Array | null = null
    let size = 0
    try {
      const file = await this.file(job)
      size = file.size
      bytes = new Uint8Array(await file.slice(job.offset, job.offset + job.len).arrayBuffer())
    } catch (error) {
      const name = nameOf(error)
      code = name === 'NotFoundError' || name === 'TypeMismatchError' ? FAIL_NOT_FOUND : name === 'NotReadableError' ? FAIL_CHANGED : FAIL_UNREADABLE
      // A `File` that changed under it, or a file that is gone: read it again next time.
      this.files.delete(pathOf(job).join('/'))
    }
    if (generation !== this.generation) code = FAIL_NOT_FOUND
    if (code !== 0 || !bytes) {
      this.exports.fk_disk_failed(this.channel, job.id, code || FAIL_UNREADABLE)
      return
    }
    this.stats.reads++
    this.stats.bytes += bytes.length
    this.stats.ms += this.now() - started
    const at = this.exports.fk_disk_inbox(this.channel, bytes.length)
    if (!at) {
      this.exports.fk_disk_failed(this.channel, job.id, FAIL_UNREADABLE)
      return
    }
    // A fresh view: the memory may have grown since the last one.
    new Uint8Array(this.memory.buffer, at, bytes.length).set(bytes)
    this.exports.fk_disk_done(this.channel, job.id, bytes.length, size)
  }

  private async file(job: Job): Promise<File> {
    const parts = pathOf(job)
    const key = parts.join('/')
    const known = this.files.get(key)
    if (known) return known
    if (!this.dir) throw new DOMException('no folder', 'NotFoundError')
    let dir = this.dir
    for (const part of parts.slice(0, -1)) dir = await dir.getDirectoryHandle(part)
    const file = await (await dir.getFileHandle(parts[parts.length - 1])).getFile()
    this.files.set(key, file)
    return file
  }
}
