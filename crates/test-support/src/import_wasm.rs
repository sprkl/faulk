//! The page's audio import (#778's parity): the steps of `web/src/lib/audio/import/pipeline.ts`
//! run on the `wasm32-unknown-unknown` build of `crates/audio-import-wasm` (the module the page
//! ships) in an interpreter, giving the files the page writes into a project's folder for a file.
//! The helper's native import is compared with it (`crates/helper/tests/import.rs`).

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use wasmi::{Caller, Engine, Extern, Linker, Memory, Module, Store, TypedFunc};

/// What the page's import writes for a file: the blob (the file itself), the PCM cache unless the
/// file is read directly, the peaks file, and the media's metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct PageImport {
    /// The SHA-256, lowercase hex.
    pub hash: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    pub direct: bool,
    /// `.faulk/cache/pcm/<sha256>.pcm`, `None` for a file read directly.
    pub pcm: Option<Vec<u8>>,
    /// `.faulk/cache/peaks/<sha256>.peaks`.
    pub peaks: Option<Vec<u8>>,
}

/// The module, built once per test run into `target/parity` (as the engine's parity module).
fn module_path() -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
            let target_dir = root.join("target/parity");
            let status = Command::new(env!("CARGO"))
                .current_dir(&root)
                .args(["build", "-p", "audio-import-wasm", "--release"])
                .args(["--target", "wasm32-unknown-unknown", "--target-dir"])
                .arg(&target_dir)
                .status()
                .expect("cargo runs");
            assert!(status.success(), "the audio-import-wasm build failed");
            target_dir.join("wasm32-unknown-unknown/release/audio_import_wasm.wasm")
        })
        .clone()
}

/// The name of a negative code of the module (`audio_import_wasm::error_code`).
fn code_name(code: i32) -> String {
    match -code {
        1 => "unsupported_format",
        2 => "unsupported_codec",
        3 => "corrupt",
        4 => "empty",
        5 => "bad_sample_rate",
        6 => "bad_channels",
        7 => "too_long",
        8 => "io",
        9 => "too_large",
        _ => "unknown",
    }
    .to_owned()
}

/// The file the module reads through `env.ai_read`.
struct Host {
    file: Vec<u8>,
}

struct Instance {
    store: Store<Host>,
    memory: Memory,
    instance: wasmi::Instance,
}

impl Instance {
    fn new(file: Vec<u8>) -> Instance {
        let bytes = std::fs::read(module_path()).expect("the module exists");
        let engine = Engine::default();
        let module = Module::new(&engine, &bytes[..]).expect("valid module");
        let mut store = Store::new(&engine, Host { file });
        let mut linker = Linker::<Host>::new(&engine);
        linker
            .func_wrap(
                "env",
                "ai_read",
                |mut caller: Caller<'_, Host>, offset: f64, len: i32, dst: i32| -> i32 {
                    let start = (offset as usize).min(caller.data().file.len());
                    let end = start
                        .saturating_add(len as u32 as usize)
                        .min(caller.data().file.len());
                    let chunk = caller.data().file[start..end].to_vec();
                    let Some(Extern::Memory(memory)) = caller.get_export("memory") else {
                        return -1;
                    };
                    if memory
                        .write(&mut caller, dst as u32 as usize, &chunk)
                        .is_err()
                    {
                        return -1;
                    }
                    chunk.len() as i32
                },
            )
            .expect("the import links");
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .expect("the module instantiates");
        let memory = instance
            .get_memory(&store, "memory")
            .expect("exported memory");
        Instance {
            store,
            memory,
            instance,
        }
    }

    fn func<P: wasmi::WasmParams, R: wasmi::WasmResults>(&self, name: &str) -> TypedFunc<P, R> {
        self.instance
            .get_typed_func::<P, R>(&self.store, name)
            .unwrap_or_else(|e| panic!("export {name}: {e}"))
    }

    fn call<P: wasmi::WasmParams, R: wasmi::WasmResults>(&mut self, name: &str, params: P) -> R {
        let f = self.func::<P, R>(name);
        f.call(&mut self.store, params)
            .unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    fn read(&self, at: i32, len: usize) -> Vec<u8> {
        let mut out = vec![0u8; len];
        self.memory
            .read(&self.store, at as u32 as usize, &mut out)
            .expect("read the module's memory");
        out
    }

    /// Puts `bytes` in the inbox; returns their length.
    fn inbox(&mut self, bytes: &[u8]) -> i32 {
        let ptr: i32 = self.call("ai_inbox", bytes.len() as i32);
        assert_ne!(ptr, 0, "the inbox");
        self.memory
            .write(&mut self.store, ptr as u32 as usize, bytes)
            .expect("write the inbox");
        bytes.len() as i32
    }
}

fn lo(n: u64) -> i32 {
    n as u32 as i32
}

fn hi(n: u64) -> i32 {
    (n >> 32) as u32 as i32
}

/// The decode of a job: the PCM bytes (interleaved `f32`, little-endian), the metadata and the
/// peaks; or the module's error code.
#[allow(clippy::type_complexity)]
fn decode(
    m: &mut Instance,
    len: u64,
    hint: &str,
) -> Result<(Vec<u8>, u32, u16, u64, Option<Vec<u8>>), String> {
    let hint_len = m.inbox(hint.as_bytes());
    let status: i32 = m.call("ai_open", (lo(len), hi(len), hint_len));
    if status != 0 {
        return Err(code_name(status));
    }
    let mut pcm = Vec::new();
    loop {
        let next: i32 = m.call("ai_next", ());
        match next {
            1 => {
                let ptr: i32 = m.call("ai_block", ());
                let n: i32 = m.call("ai_block_len", ());
                pcm.extend_from_slice(&m.read(ptr, n as usize * 4));
            }
            0 => break,
            error => return Err(code_name(error)),
        }
    }
    let finished: i32 = m.call("ai_finish", ());
    if finished != 0 {
        return Err(code_name(finished));
    }
    let rate: i32 = m.call("ai_rate", ());
    let channels: i32 = m.call("ai_channels", ());
    let frames: f64 = m.call("ai_frames", ());
    let peaks_len: i32 = m.call("ai_peaks_len", ());
    let peaks = if peaks_len > 0 {
        let ptr: i32 = m.call("ai_peaks", ());
        Some(m.read(ptr, peaks_len as usize))
    } else {
        None
    };
    Ok((pcm, rate as u32, channels as u16, frames as u64, peaks))
}

/// Imports `file` as the page does (`runImport`: hash, a decode with the cache unless the head
/// looks read directly, the check, a second decode into a cache when that check fails), `hint`
/// being the extension `extensionOf` takes from the file's name (empty for none). The error is the
/// module's code (`too_large`, `corrupt`...).
pub fn page_import(file: &[u8], hint: &str) -> Result<PageImport, String> {
    let mut m = Instance::new(file.to_vec());
    let len = file.len() as u64;
    let () = m.call("ai_hash_begin", ());
    for chunk in file.chunks(1 << 20) {
        let n = m.inbox(chunk);
        let pushed: i32 = m.call("ai_hash_push", n);
        assert_eq!(pushed, 0, "ai_hash_push");
    }
    let ptr: i32 = m.call("ai_hash_end", ());
    let hash: String = m.read(ptr, 32).iter().map(|b| format!("{b:02x}")).collect();
    let head = &file[..file.len().min(engine::audio_source::layout::HEAD_BYTES)];
    let head_len = m.inbox(head);
    let maybe_direct: i32 = m.call("ai_layout_candidate", (head_len, lo(len), hi(len)));
    let (samples, rate, channels, frames, peaks) = decode(&mut m, len, hint)?;
    let head_len = m.inbox(head);
    let direct = maybe_direct == 1 && {
        let layout: i32 = m.call(
            "ai_layout",
            (
                head_len,
                lo(len),
                hi(len),
                rate as i32,
                channels as i32,
                frames as f64,
            ),
        );
        layout == 1
    };
    let pcm = if direct {
        None
    } else {
        // A file that looked direct but is not was decoded again into a cache by a new instance:
        // the same samples.
        let ptr: i32 = m.call(
            "ai_cache_header",
            (rate as i32, channels as i32, frames as f64),
        );
        assert_ne!(ptr, 0, "ai_cache_header");
        let mut bytes = m.read(ptr, 32);
        bytes.extend_from_slice(&samples);
        Some(bytes)
    };
    Ok(PageImport {
        hash,
        sample_rate: rate,
        channels,
        frames,
        direct,
        pcm,
        peaks,
    })
}
