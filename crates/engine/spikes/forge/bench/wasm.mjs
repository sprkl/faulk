// The spike's WASM measurement under Node (V8, the engine of Chromium's AudioWorklet).
//
//   node crates/engine/spikes/forge/bench/wasm.mjs <forge_spike.wasm> [<dir for the renders>]
//
// Prints the same CPU table as `forge bench` (percent of one core, median of 7 runs of
// 10 s at 48 kHz), then, given a folder, writes the sketches rendered in WASM as
// `forge-sketch-{a,b,c}-wasm.f32` for `forge compare`.
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { performance } from 'node:perf_hooks';

const [wasmPath, outDir] = process.argv.slice(2);
if (!wasmPath) {
  console.error('usage: node wasm.mjs <forge_spike.wasm> [<dir>]');
  process.exit(2);
}
const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), {});
const fk = instance.exports;

const SECONDS = 10;
const RUNS = 7;
const median = (xs) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];

function measure(variant, voices) {
  const times = [];
  for (let i = 0; i < RUNS; i++) {
    fk.fk_bench_prepare(variant, voices);
    const start = performance.now();
    fk.fk_bench_run(SECONDS);
    times.push(((performance.now() - start) / 1000 / SECONDS) * 100);
  }
  return median(times);
}

// One unmeasured warm-up so that the optimising tier has compiled the render loop.
fk.fk_bench_prepare(0, 16);
fk.fk_bench_run(SECONDS);

const names = ['ladder 2x', 'svf 2x', 'ladder 1x', 'svf 1x'];
console.log(`wasm (${process.version}), variant, % of a core (median of ${RUNS} runs of ${SECONDS} s at 48 kHz)`);
for (let variant = 0; variant < 4; variant++) {
  const empty = measure(variant, 0);
  const one = measure(variant, 1);
  const sixteen = measure(variant, 16);
  console.log(
    `${names[variant].padEnd(9)}: 0 voice ${empty.toFixed(3)} %, 1 voice ${one.toFixed(3)} %, 16 voices ${sixteen.toFixed(3)} %, per voice (16-0)/16 ${((sixteen - empty) / 16).toFixed(3)} %`,
  );
}

if (outDir) {
  for (const [index, name] of ['a', 'b', 'c'].entries()) {
    fk.fk_render(index);
    const ptr = fk.fk_render_ptr();
    const len = fk.fk_render_len();
    const data = new Float32Array(fk.memory.buffer, ptr, len);
    const path = join(outDir, `forge-sketch-${name}-wasm.f32`);
    writeFileSync(path, Buffer.from(data.buffer, data.byteOffset, data.byteLength));
    console.log(`wrote ${path} (${len / 2} frames)`);
  }
}
