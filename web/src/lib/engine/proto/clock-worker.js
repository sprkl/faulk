// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-nocheck: a plain-JavaScript prototype loaded by the e2e suite only (CT-3, #39), not part of the app.
// The only sub-millisecond clock an AudioWorklet can read (CT-3 prototype): an
// AudioWorkletGlobalScope has no `performance`, so this worker spins on a shared counter
// that the page calibrates against `performance.now()` and the worklet reads with
// `Atomics.load`. It pins one core for the duration of the measurement.
onmessage = (event) => {
  const ticks = new BigInt64Array(event.data)
  for (;;) Atomics.add(ticks, 0, 1n)
}
