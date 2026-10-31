//! The export (`project.export_audio`, #297) rendered offline, natively and in the WASM
//! module, and the WAV file it gives read back as a [`Render`].

use std::path::PathBuf;

use commands::State;
use commands::export::{ExportJob, ExportSettings, job, offline_plan};
use engine::offline::{Offline, Phase, SampleFormat};
use wasmi::{Engine, Linker, Module, Store, TypedFunc};

use crate::signal::Render;
use crate::wasm::wasm_module;

/// Frames per step: the page's Worker steps by this much.
pub const STEP_FRAMES: usize = 8192;

/// A finished export: the file and what the engine measured while writing it.
#[derive(Clone, Debug)]
pub struct Exported {
    /// The whole WAV file.
    pub wav: Vec<u8>,
    pub settings: ExportSettings,
    pub body_frames: u64,
    pub rendered: u64,
    pub written: u64,
    pub peak: f32,
    pub clipped: u64,
    /// The output protection's peak gain reduction over the render, linear in `[0, 1]`
    /// (`0.0` = never engaged; #402).
    pub reduction: f32,
    /// Non-finite samples the output protection silenced over the render (`0` on a clean render;
    /// #402). A test asserts `0` to catch a NaN on a path the engine-level `assert_finite` cannot.
    pub silenced: u64,
    pub tail_cut: bool,
    /// Times the engine was full while the setup went in.
    pub busy: u32,
}

/// The job of a state: panics when it is refused.
pub fn job_of(state: &State, settings: ExportSettings) -> ExportJob {
    job(&state.project, &state.session, settings).expect("the export is prepared")
}

/// Exports the state through `engine::offline` (the native target).
pub fn export_native(state: &State, settings: ExportSettings) -> Exported {
    let job = job_of(state, settings);
    let mut offline = Offline::new(settings.sample_rate, offline_plan(&job.plan))
        .expect("the render is accepted");
    offline.set_stop(&job.stop);
    offline.submit(&job.setup).expect("the setup is accepted");
    let mut pcm = Vec::new();
    loop {
        let phase = offline.step(STEP_FRAMES).expect("the render goes on");
        pcm.extend_from_slice(offline.pcm());
        if phase == Phase::Done {
            break;
        }
    }
    let mut wav = offline.header();
    wav.extend_from_slice(&pcm);
    Exported {
        wav,
        settings,
        body_frames: job.plan.body_frames,
        rendered: offline.rendered(),
        written: offline.written(),
        peak: offline.peak(),
        clipped: offline.clipped(),
        reduction: offline.reduction(),
        silenced: offline.silenced(),
        tail_cut: offline.tail_cut(),
        busy: offline.busy(),
    }
}

/// Exports the state through the `wasm32-unknown-unknown` build of `engine-wasm`
/// (`fk_export_*`) in an interpreter, stepping as the page's Worker does.
pub fn export_wasm(state: &State, settings: ExportSettings) -> Exported {
    let job = job_of(state, settings);
    let bytes = std::fs::read(wasm_module()).expect("the wasm module exists");
    let engine = Engine::default();
    let module = Module::new(&engine, &bytes[..]).expect("valid module");
    let mut store = Store::new(&engine, ());
    let instance = Linker::<()>::new(&engine)
        .instantiate_and_start(&mut store, &module)
        .expect("the module needs no imports");
    fn func<P: wasmi::WasmParams, R: wasmi::WasmResults>(
        i: &wasmi::Instance,
        s: &Store<()>,
        name: &str,
    ) -> TypedFunc<P, R> {
        i.get_typed_func::<P, R>(s, name)
            .unwrap_or_else(|e| panic!("export {name}: {e}"))
    }
    let new = func::<(i32, i32, i32, i32), i32>(&instance, &store, "fk_export_new");
    let inbox = func::<(i32, i32), i32>(&instance, &store, "fk_export_inbox");
    let submit = func::<(i32, i32), i32>(&instance, &store, "fk_export_submit");
    let set_stop = func::<(i32, i32), i32>(&instance, &store, "fk_export_set_stop");
    let step = func::<(i32, i32), i32>(&instance, &store, "fk_export_step");
    let pcm = func::<i32, i32>(&instance, &store, "fk_export_pcm");
    let pcm_len = func::<i32, i32>(&instance, &store, "fk_export_pcm_len");
    let header = func::<i32, i32>(&instance, &store, "fk_export_header");
    let header_len = func::<i32, i32>(&instance, &store, "fk_export_header_len");
    let stat = func::<(i32, i32), f64>(&instance, &store, "fk_export_stat");
    let memory = instance
        .get_memory(&store, "memory")
        .expect("exported memory");

    let bits = match settings.bit_depth.format() {
        SampleFormat::Pcm16 => 16,
        SampleFormat::Pcm24 => 24,
        SampleFormat::Float32 => 32,
    };
    let body = job.plan.body_frames;
    let ex = new
        .call(
            &mut store,
            (
                settings.sample_rate as i32,
                bits,
                body as u32 as i32,
                (body >> 32) as u32 as i32,
            ),
        )
        .unwrap();
    assert_ne!(ex, 0, "the module refused the export");
    let put = |store: &mut Store<()>, bytes: &[u8]| {
        let at = inbox.call(&mut *store, (ex, bytes.len() as i32)).unwrap();
        assert_ne!(at, 0);
        memory.write(&mut *store, at as usize, bytes).unwrap();
    };
    put(&mut store, &job.stop);
    assert_eq!(
        set_stop
            .call(&mut store, (ex, job.stop.len() as i32))
            .unwrap(),
        0
    );
    put(&mut store, &job.setup);
    assert_eq!(
        submit
            .call(&mut store, (ex, job.setup.len() as i32))
            .unwrap(),
        0,
        "the module refused the setup"
    );
    let read = |store: &Store<()>, at: i32, len: i32| {
        let mut out = vec![0u8; len as usize];
        memory.read(store, at as usize, &mut out).unwrap();
        out
    };
    let mut data = Vec::new();
    loop {
        let phase = step.call(&mut store, (ex, STEP_FRAMES as i32)).unwrap();
        assert!(phase >= 0, "the render failed with {phase}");
        let (at, len) = (
            pcm.call(&mut store, ex).unwrap(),
            pcm_len.call(&mut store, ex).unwrap(),
        );
        data.extend(read(&store, at, len));
        if phase == 2 {
            break;
        }
    }
    let at = header.call(&mut store, ex).unwrap();
    let len = header_len.call(&mut store, ex).unwrap();
    let mut wav = read(&store, at, len);
    wav.extend(data);
    let stat = |store: &mut Store<()>, which: i32| stat.call(store, (ex, which)).unwrap();
    Exported {
        wav,
        settings,
        body_frames: job.plan.body_frames,
        rendered: stat(&mut store, 0) as u64,
        written: stat(&mut store, 1) as u64,
        peak: stat(&mut store, 3) as f32,
        clipped: stat(&mut store, 4) as u64,
        reduction: stat(&mut store, 7) as f32,
        silenced: stat(&mut store, 8) as u64,
        tail_cut: stat(&mut store, 5) > 0.5,
        busy: 0,
    }
}

impl Exported {
    /// The file read back: the header checked, the samples as floats.
    pub fn render(&self) -> Render {
        let w = &self.wav;
        let u16_at = |at: usize| u16::from_le_bytes([w[at], w[at + 1]]);
        let u32_at = |at: usize| u32::from_le_bytes([w[at], w[at + 1], w[at + 2], w[at + 3]]);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        let float = self.settings.bit_depth.format() == SampleFormat::Float32;
        let header = self.settings.bit_depth.format().header_bytes();
        assert_eq!(u32_at(4) as usize, w.len() - 8, "RIFF size");
        assert_eq!(u16_at(20), if float { 3 } else { 1 }, "format tag");
        assert_eq!(u16_at(22), 2, "channels");
        assert_eq!(u32_at(24), self.settings.sample_rate, "sample rate");
        let bits = u16_at(34) as usize;
        assert_eq!(
            u32_at(28) as usize,
            self.settings.sample_rate as usize * bits / 4,
            "byte rate"
        );
        assert_eq!(u16_at(32) as usize, bits / 4, "block align");
        assert_eq!(&w[header - 8..header - 4], b"data");
        let data = u32_at(header - 4) as usize;
        assert_eq!(data, w.len() - header, "data size");
        let bytes = bits / 8;
        assert_eq!(data % (2 * bytes), 0, "whole frames");
        let (mut left, mut right) = (Vec::new(), Vec::new());
        for frame in w[header..].chunks_exact(2 * bytes) {
            let sample = |b: &[u8]| match bits {
                16 => f32::from(i16::from_le_bytes([b[0], b[1]])) / 32_768.0,
                24 => {
                    let v = i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8;
                    v as f32 / 8_388_608.0
                }
                _ => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            };
            left.push(sample(&frame[..bytes]));
            right.push(sample(&frame[bytes..]));
        }
        Render::new(self.settings.sample_rate as f32, left, right)
    }

    /// Writes the file to `.qa/renders/<name>.wav` for listening and returns its path.
    pub fn write(&self, name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.qa/renders");
        std::fs::create_dir_all(&dir).expect("the renders folder is created");
        let path = dir.join(format!("{name}.wav"));
        std::fs::write(&path, &self.wav).expect("the WAV file is written");
        path
    }
}

/// A song of `bars` bars at 120 BPM in 4/4 (a bar is two seconds), built by commands: a bass
/// and a lead on the built-in synth, one clip each from the start of the arrangement to its
/// end, a bass note per beat and a lead arpeggio. With `echo`, the lead goes through an echo
/// (the default settings but this feedback), whose repeats ring on after the last note.
pub fn demo_song(bars: i64, echo: Option<f64>) -> State {
    use commands::{Command, Output, clip, device, track};
    use project::{Note, Tick};
    let fixture = commands::testing::fixture();
    let mut s = crate::Session::from_state(fixture.state.clone(), commands::testing::context());
    let bass = fixture.track.clone();
    let Output::Track(track::Output::Duplicate { track: lead }) =
        s.exec(Command::Track(track::Command::Duplicate {
            track: bass.clone(),
            name: Some("Lead".into()),
        }))
    else {
        panic!("a track")
    };
    let length = bars * 3840;
    let clip_on = |s: &mut crate::Session, track: &project::TrackId| {
        let Output::Clip(clip::Output::Create { clip, .. }) =
            s.exec(Command::Clip(clip::Command::Create {
                track: track.clone(),
                at: clip::ClipTarget::Arrangement { start: Tick(0) },
                length: Some(Tick(length)),
                name: None,
            }))
        else {
            panic!("a clip")
        };
        clip
    };
    let note = |start: i64, pitch: u8, length: i64, velocity: u8| Note {
        start: Tick(start),
        pitch,
        length: Tick(length),
        velocity,
        cents: 0,
        chance: Note::DEFAULT_CHANCE,
        repeat: Note::DEFAULT_REPEAT,
    };
    let bass_clip = clip_on(&mut s, &bass);
    let lead_clip = clip_on(&mut s, &lead);
    let roots = [36u8, 41, 43, 38];
    let mut bass_notes = Vec::new();
    let mut lead_notes = Vec::new();
    for bar in 0..bars {
        let root = roots[(bar % 4) as usize];
        for beat in 0..4 {
            bass_notes.push(note(bar * 3840 + beat * 960, root, 720, 100));
            for (k, interval) in [12u8, 16, 19, 24].iter().enumerate() {
                lead_notes.push(note(
                    bar * 3840 + beat * 960 + k as i64 * 240,
                    root + interval + 12,
                    200,
                    80,
                ));
            }
        }
    }
    for (clip, notes) in [(bass_clip, bass_notes), (lead_clip, lead_notes)] {
        s.run(Command::Clip(clip::Command::NotesAdd { clip, notes }));
    }
    if let Some(feedback) = echo {
        let Output::Device(device::Output::Insert { device: echo }) =
            s.exec(Command::Device(device::Command::Insert {
                track: Some(lead.clone()),
                master: false,
                kind: "faulk.echo".into(),
                index: 0,
            }))
        else {
            panic!("an echo")
        };
        s.run(Command::Device(device::Command::SetParam {
            device: echo,
            param: "feedback".into(),
            value: feedback,
        }));
    }
    s.state().clone()
}
