//! The audio output supervisor (#525): the helper starts and serves without an output, and
//! opens one as soon as it can.
//!
//! A thread of its own creates the output stream, watches it and drops it, so the stream never
//! crosses threads. Without an output it tries again when the output devices change (polled)
//! or on a slow timer. The engine of the open output lives in the [`EngineSlot`], empty while
//! there is none: the server answers a session with `audio.no_output` and the page keeps
//! playing itself.
//!
//! When the stream fails, the supervisor drops it and gets the engine's audio core back from
//! it ([`Held`]); the engine's prepare side stays in the slot. For [`Timing::grace`] it looks
//! for an output again with that engine (#577). An output that opens in time plays the same
//! engine: at another rate, every device is built again for it first
//! (`EngineSlot::set_sample_rate`, `engine::host::Prepare::set_sample_rate`, #236), and the
//! playhead, the project and the session are kept; the server sends the session a new
//! `hello` with the output's rate and latency (`Links::reopened`). Once the grace is over
//! with no output, the engine is dropped and the session told (`Links::lost`, the server sends
//! `audio.stream`), as when the core does not come back or the rate change is refused; the
//! next output gets a new engine.
//!
//! The device side is a [`Backend`], so the loop is tested without a sound card; [`Cpal`] is
//! the real one.

use crate::audio::{AudioCore, EngineSlot, EventSender, Meter, audio_core};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// An output just opened, playing the core [`Backend::open`] was given.
pub struct Opened<S> {
    /// Keeps the output playing; dropping it stops the output and drops the [`Held`] core.
    pub stream: S,
    pub sample_rate: f32,
    /// For the journal: device, rate, channels.
    pub description: String,
}

/// The audio core a stream's callback renders, lent by the supervisor: dropped with the
/// stream, it gives the core back (#577), so that the next output can play the same engine.
pub struct Held {
    core: Option<AudioCore>,
    back: mpsc::SyncSender<AudioCore>,
}

impl Held {
    fn new(core: AudioCore) -> (Held, mpsc::Receiver<AudioCore>) {
        let (back, returned) = mpsc::sync_channel(1);
        (
            Held {
                core: Some(core),
                back,
            },
            returned,
        )
    }

    /// The core, for the audio callback. Allocation-free; `None` only while it is dropped.
    pub fn core(&mut self) -> Option<&mut AudioCore> {
        self.core.as_mut()
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        // At the stream's teardown, never inside a callback. The channel holds one core and
        // nothing else is sent on it: the send cannot block. A supervisor that stopped waiting
        // dropped the receiver, and the core is dropped here.
        if let Some(core) = self.core.take() {
            let _ = self.back.try_send(core);
        }
    }
}

/// What [`Backend::open`] calls once it knows the output's rate: the core to render at that
/// rate, or why there is none (for the journal).
pub type CoreFor<'a> = &'a mut dyn FnMut(f32) -> Result<Held, String>;

/// The device side of the supervisor.
pub trait Backend {
    type Stream;
    /// A value that changes when the output devices change (the default output, the list).
    fn devices(&mut self) -> String;
    /// Opens the default output: reads its rate, takes the core to render from `core_for`
    /// at that rate, and builds the stream around it. The stream raises `failed` when it
    /// fails. The reason, when it cannot, is for the journal; a core it took is dropped then
    /// (and so given back).
    fn open(
        &mut self,
        failed: Arc<AtomicBool>,
        core_for: CoreFor<'_>,
    ) -> Result<Opened<Self::Stream>, String>;
}

/// How often the supervisor looks, and how long it waits.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// While an output plays: how often the stream's failure and Quit are looked at.
    pub watch: Duration,
    /// Without an output: how often the devices are listed.
    pub devices: Duration,
    /// Without an output: a new attempt this long after the last one, devices changed or not.
    pub retry: Duration,
    /// Shortest time between two attempts, so a device that fails at once does not spin.
    pub min_gap: Duration,
    /// A stream that failed sooner than this after it opened is a quick failure: each one in a
    /// row doubles the gap before the next attempt (up to `retry`), so a device that keeps
    /// dropping does not fill the journal and flip the tray every second.
    pub stable: Duration,
    /// After a stream failed: how long its engine is kept for the next output (#577), the
    /// session waiting. Zero drops it at once.
    pub grace: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            watch: Duration::from_millis(100),
            devices: Duration::from_secs(2),
            retry: Duration::from_secs(30),
            min_gap: Duration::from_secs(1),
            stable: Duration::from_secs(10),
            grace: Duration::from_secs(2),
        }
    }
}

/// What the supervisor shares with the rest of the helper.
pub struct Links {
    pub engine: Arc<EngineSlot>,
    pub meter: Arc<Meter>,
    /// Called when an open output failed and its engine is gone (once the slot is empty) and
    /// before another output opens: the server tells the session (`Shared::output_lost`).
    pub lost: Box<dyn Fn() + Send>,
    /// Called when an output opened with the engine of the one that failed (#577), once it
    /// called back (or after a moment): the server sends the session a new `hello` at its rate
    /// (`Shared::output_reopened`).
    pub reopened: Box<dyn Fn() + Send>,
    /// Told `true` when an output opens and `false` when there is none (once at start, then on
    /// each change), for the tray. Not told while a failed output's engine waits for the next.
    pub report: Box<dyn Fn(bool) + Send>,
}

/// The running supervisor. [`Supervisor::stop`] (or dropping it) ends its thread, which drops
/// the stream: release the voices first.
pub struct Supervisor {
    stop: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Supervisor {
    /// Starts the thread; `make` builds the backend on it.
    pub fn start<B, F>(make: F, links: Links, timing: Timing) -> std::io::Result<Self>
    where
        B: Backend,
        F: FnOnce() -> B + Send + 'static,
    {
        let (stop_tx, stop_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("faulk-audio-output".into())
            .spawn(move || {
                supervise(make(), &links, &stop_rx, timing);
                // Quit: the stream is gone, and with it the core; then the prepare side.
                links.engine.remove();
                links.meter.reset();
            })?;
        Ok(Self {
            stop: Some(stop_tx),
            thread: Some(thread),
        })
    }

    /// Ends the thread and waits for it: the output, if any, is closed when this returns.
    pub fn stop(mut self) {
        self.end();
    }

    fn end(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.end();
    }
}

/// Waits up to `limit`; `true` when Quit was asked meanwhile.
fn stopped(stop: &mpsc::Receiver<()>, limit: Duration) -> bool {
    !matches!(stop.recv_timeout(limit), Err(RecvTimeoutError::Timeout))
}

/// How long the supervisor waits for the core of a dropped stream to come back.
const CORE_BACK: Duration = Duration::from_secs(1);

/// How long a reopened output is given to call back before the session's new `hello` (the
/// server's wait before a session's first one, 25 × 10 ms).
const FIRST_CALLBACK: Duration = Duration::from_millis(250);

/// The engine of an output that failed, kept until `until` for the next output (#577). Its
/// prepare side stays in the slot.
struct Kept {
    core: AudioCore,
    until: Instant,
}

/// The engine is gone: the slot is emptied and the session told (#525's path).
fn drop_engine(links: &Links) {
    links.engine.remove();
    links.meter.reset();
    (links.lost)();
}

/// The kept engine's grace is over with no output for it: dropped, the tray and the session
/// told.
fn expire(kept: &mut Option<Kept>, links: &Links, report: &mut impl FnMut(bool)) {
    crate::log!("no output for the engine in time: it is dropped");
    *kept = None;
    report(false);
    drop_engine(links);
}

fn supervise<B: Backend>(mut backend: B, links: &Links, stop: &mpsc::Receiver<()>, timing: Timing) {
    let mut reported = None;
    let mut report = |open: bool| {
        if reported != Some(open) {
            reported = Some(open);
            (links.report)(open);
        }
    };
    let mut last_reason: Option<String> = None;
    // Quick stream failures in a row (`Timing::stable`).
    let mut quick: u32 = 0;
    let mut kept: Option<Kept> = None;
    loop {
        // An engine is never lent past its grace.
        if kept.as_ref().is_some_and(|k| Instant::now() >= k.until) {
            expire(&mut kept, links, &mut report);
        }
        let failed = Arc::new(AtomicBool::new(false));
        let attempt = Instant::now();
        let until = kept.as_ref().map(|k| k.until);
        let mut reuse = kept.take();
        // Set by `core_for`: the sender of a new engine, to install once the stream opened;
        // whether the kept engine was lent; where the lent core comes back.
        let mut built: Option<EventSender> = None;
        let mut lent = false;
        let mut back: Option<mpsc::Receiver<AudioCore>> = None;
        let opened = backend.open(failed.clone(), &mut |rate| {
            let core = match reuse.take() {
                Some(k) => match links.engine.set_sample_rate(rate) {
                    Ok(()) => {
                        lent = true;
                        Some(k.core)
                    }
                    Err(error) => {
                        crate::log!(
                            "the engine could not change to {rate} Hz ({error:?}): a new one"
                        );
                        drop(k.core);
                        drop_engine(links);
                        None
                    }
                },
                None => None,
            };
            let core = match core {
                Some(core) => core,
                None => {
                    let (core, sender) = audio_core(rate, links.meter.clone())
                        .map_err(|e| format!("cannot create the engine: {e:?}"))?;
                    built = Some(sender);
                    core
                }
            };
            let (held, returned) = Held::new(core);
            back = Some(returned);
            Ok(held)
        });
        match opened {
            Ok(opened) => {
                last_reason = None;
                if let Some(sender) = built.take() {
                    links.engine.install(sender, opened.sample_rate);
                    crate::log!("audio output: {}", opened.description);
                } else {
                    crate::log!("audio output: {} (the same engine)", opened.description);
                    // The session's new hello carries the output's buffer and latency.
                    let waited = Instant::now();
                    while links.meter.callback_frames().is_none()
                        && waited.elapsed() < FIRST_CALLBACK
                    {
                        if stopped(stop, Duration::from_millis(5)) {
                            return;
                        }
                    }
                    (links.reopened)();
                }
                report(true);
                let quit = loop {
                    if stopped(stop, timing.watch) {
                        break true;
                    }
                    if failed.load(Ordering::Relaxed) {
                        break false;
                    }
                };
                // The stream first: its callback holds the audio core, then the prepare side.
                drop(opened.stream);
                if quit {
                    return;
                }
                let core = back.and_then(|returned| returned.recv_timeout(CORE_BACK).ok());
                quick = if attempt.elapsed() < timing.stable {
                    quick + 1
                } else {
                    0
                };
                match core {
                    Some(core) if !timing.grace.is_zero() => {
                        crate::log!(
                            "the audio output failed: looking for an output for the engine"
                        );
                        links.meter.forget_output();
                        kept = Some(Kept {
                            core,
                            until: Instant::now() + timing.grace,
                        });
                    }
                    _ => {
                        crate::log!("the audio output failed: looking for an output");
                        report(false);
                        drop_engine(links);
                    }
                }
                // Try again at once (the default output may have changed), but not faster than
                // `min_gap`, doubled for each quick failure in a row after the first. A kept
                // engine whose grace ends meanwhile is dropped then, not after the wait.
                let gap = timing
                    .min_gap
                    .saturating_mul(1 << quick.saturating_sub(1).min(16))
                    .min(timing.retry.max(timing.min_gap));
                let next = attempt + gap;
                if let Some(until) = kept.as_ref().map(|k| k.until).filter(|u| *u < next) {
                    if stopped(stop, until.saturating_duration_since(Instant::now())) {
                        return;
                    }
                    expire(&mut kept, links, &mut report);
                }
                if stopped(stop, next.saturating_duration_since(Instant::now())) {
                    return;
                }
            }
            Err(reason) => {
                // A lent core the backend dropped came back: still kept, for the time left.
                if lent {
                    let core = back.and_then(|returned| returned.try_recv().ok());
                    match (core, until) {
                        (Some(core), Some(until)) => kept = Some(Kept { core, until }),
                        _ => drop_engine(links),
                    }
                } else {
                    kept = reuse.take();
                }
                if kept.as_ref().is_some_and(|k| Instant::now() >= k.until) {
                    expire(&mut kept, links, &mut report);
                }
                if kept.is_some() {
                    // The meter still shows the kept engine's report.
                    links.meter.forget_output();
                } else {
                    // `open` may have pointed the meter at an engine it then dropped.
                    links.meter.reset();
                    report(false);
                }
                // The devices as they are now: a change from here on is worth a new attempt.
                let devices = backend.devices();
                if last_reason.as_deref() != Some(reason.as_str()) {
                    crate::log!("no audio output ({reason}): waiting for one");
                    last_reason = Some(reason);
                }
                // While an engine waits, an attempt every `min_gap` until its grace ends.
                let waiting = kept.as_ref().map(|k| k.until);
                let step = match waiting {
                    Some(_) => timing.devices.min(timing.min_gap),
                    None => timing.devices,
                };
                loop {
                    if stopped(stop, step) {
                        return;
                    }
                    let waited = attempt.elapsed();
                    if waited >= timing.retry
                        || (waited >= timing.min_gap
                            && (waiting.is_some() || backend.devices() != devices))
                        || waiting.is_some_and(|until| Instant::now() >= until)
                    {
                        break;
                    }
                }
            }
        }
    }
}

/// The real backend: the system's default output through cpal.
pub struct Cpal {
    host: cpal::Host,
    /// A debug build's tests ask for no output (`FAULK_HELPER_TEST_NO_AUDIO`).
    no_audio: bool,
}

impl Cpal {
    pub fn new(no_audio: bool) -> Self {
        Self {
            host: cpal::default_host(),
            no_audio,
        }
    }
}

fn device_name(device: &cpal::Device) -> Option<String> {
    use cpal::traits::DeviceTrait;
    device.description().ok().map(|d| d.name().to_owned())
}

impl Backend for Cpal {
    type Stream = cpal::Stream;

    fn devices(&mut self) -> String {
        use cpal::traits::HostTrait;
        if self.no_audio {
            return String::new();
        }
        let default = self
            .host
            .default_output_device()
            .and_then(|d| device_name(&d));
        // The count, not the names: no property store is read for every device every poll.
        let count = self
            .host
            .output_devices()
            .map(|devices| devices.count())
            .unwrap_or(0);
        format!("{default:?} {count}")
    }

    fn open(
        &mut self,
        failed: Arc<AtomicBool>,
        core_for: CoreFor<'_>,
    ) -> Result<Opened<cpal::Stream>, String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        if self.no_audio {
            return Err("no audio output device".into());
        }
        let device = self
            .host
            .default_output_device()
            .ok_or("no audio output device")?;
        let supported = device
            .default_output_config()
            .map_err(|e| format!("cannot read the output configuration: {e}"))?;
        if supported.sample_format() != cpal::SampleFormat::F32 {
            return Err(format!(
                "the default output uses {:?} samples; only f32 is supported for now",
                supported.sample_format()
            ));
        }
        let config: cpal::StreamConfig = supported.config();
        let sample_rate = config.sample_rate as f32;
        let channels = config.channels as usize;
        let mut held = core_for(sample_rate)?;
        let stream = device
            .build_output_stream(
                config,
                move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                    let Some(core) = held.core() else {
                        data.fill(0.0);
                        return;
                    };
                    let at = info.timestamp();
                    core.meter()
                        .set_output_delay(at.playback.saturating_duration_since(at.callback));
                    #[cfg(debug_assertions)]
                    assert_no_alloc::assert_no_alloc(|| core.process(data, channels));
                    #[cfg(not(debug_assertions))]
                    core.process(data, channels);
                },
                // May run on the stream's thread: only raise the flag; the supervisor looks.
                move |_err| failed.store(true, Ordering::Relaxed),
                None,
            )
            .map_err(|e| format!("cannot open the audio output: {e}"))?;
        stream
            .play()
            .map_err(|e| format!("cannot start the audio output: {e}"))?;
        let name = device_name(&device).unwrap_or_else(|| "default output".into());
        Ok(Opened {
            stream,
            sample_rate,
            description: format!("{name} · {sample_rate} Hz, {channels} channels"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicU32;

    /// What the fake devices do, changed by the test while the supervisor runs.
    #[derive(Default)]
    struct World {
        /// The device signature `devices` returns.
        devices: String,
        /// `open` succeeds while this is set.
        available: bool,
        /// Every `open` call, in order.
        opens: u32,
        /// When each `open` was called.
        opened_at: Vec<Instant>,
        /// The `failed` flag of the last stream opened, to make it fail.
        last_failed: Option<Arc<AtomicBool>>,
        /// The audio core of the last stream opened, which the test renders as its callback;
        /// dropped with the stream, as cpal drops the callback.
        core: Option<Held>,
        /// Streams alive (opened and not dropped).
        alive: u32,
        /// The output's rate; 0 is 44.1 kHz.
        rate: f32,
        /// `open` takes the core, then fails (a stream that cannot be built).
        fail_after_core: bool,
    }

    struct Fake(Arc<Mutex<World>>);

    struct FakeStream(Arc<Mutex<World>>);

    impl Drop for FakeStream {
        fn drop(&mut self) {
            let held = {
                let mut world = self.0.lock().unwrap();
                world.alive -= 1;
                world.core.take()
            };
            drop(held);
        }
    }

    impl Backend for Fake {
        type Stream = FakeStream;

        fn devices(&mut self) -> String {
            self.0.lock().unwrap().devices.clone()
        }

        fn open(
            &mut self,
            failed: Arc<AtomicBool>,
            core_for: CoreFor<'_>,
        ) -> Result<Opened<FakeStream>, String> {
            let (rate, fail_after_core) = {
                let mut world = self.0.lock().unwrap();
                world.opens += 1;
                world.opened_at.push(Instant::now());
                if !world.available {
                    return Err("no audio output device".into());
                }
                let rate = if world.rate == 0.0 {
                    44_100.0
                } else {
                    world.rate
                };
                (rate, world.fail_after_core)
            };
            let held = core_for(rate)?;
            if fail_after_core {
                drop(held);
                return Err("the stream could not be built".into());
            }
            let mut world = self.0.lock().unwrap();
            world.last_failed = Some(failed);
            world.core = Some(held);
            world.alive += 1;
            Ok(Opened {
                stream: FakeStream(self.0.clone()),
                sample_rate: rate,
                description: "fake".into(),
            })
        }
    }

    struct Rig {
        world: Arc<Mutex<World>>,
        engine: Arc<EngineSlot>,
        audio_error: Arc<AtomicBool>,
        reports: Arc<Mutex<Vec<bool>>>,
        meter: Arc<Meter>,
        /// `Links::lost` and `Links::reopened` calls.
        lost: Arc<AtomicU32>,
        /// For each `Links::lost` call, the `open` calls made before it.
        lost_after: Arc<Mutex<Vec<u32>>>,
        reopened: Arc<AtomicU32>,
        supervisor: Supervisor,
    }

    /// #525's timing: no grace, a failed output's engine is dropped at once.
    const FAST: Timing = Timing {
        watch: Duration::from_millis(5),
        devices: Duration::from_millis(5),
        retry: Duration::from_secs(3600),
        min_gap: Duration::from_millis(20),
        stable: Duration::from_secs(3600),
        grace: Duration::ZERO,
    };

    /// With a grace far longer than a test (#577).
    const GRACE: Timing = Timing {
        grace: Duration::from_secs(3600),
        ..FAST
    };

    fn rig(world: World, timing: Timing) -> Rig {
        let world = Arc::new(Mutex::new(world));
        let engine = Arc::new(EngineSlot::default());
        let audio_error = Arc::new(AtomicBool::new(false));
        let error = audio_error.clone();
        let lost = Arc::new(AtomicU32::new(0));
        let lost_count = lost.clone();
        let lost_after = Arc::new(Mutex::new(Vec::new()));
        let opens_at_lost = lost_after.clone();
        let seen = world.clone();
        let reopened = Arc::new(AtomicU32::new(0));
        let reopened_count = reopened.clone();
        let reports = Arc::new(Mutex::new(Vec::new()));
        let told = reports.clone();
        let backend = world.clone();
        let meter = Arc::new(Meter::default());
        let supervisor = Supervisor::start(
            move || Fake(backend),
            Links {
                engine: engine.clone(),
                meter: meter.clone(),
                lost: Box::new(move || {
                    error.store(true, Ordering::Relaxed);
                    lost_count.fetch_add(1, Ordering::Relaxed);
                    let opens = seen.lock().unwrap().opens;
                    opens_at_lost.lock().unwrap().push(opens);
                }),
                reopened: Box::new(move || {
                    reopened_count.fetch_add(1, Ordering::Relaxed);
                }),
                report: Box::new(move |open| told.lock().unwrap().push(open)),
            },
            timing,
        )
        .unwrap();
        Rig {
            world,
            engine,
            audio_error,
            reports,
            meter,
            lost,
            lost_after,
            reopened,
            supervisor,
        }
    }

    /// Plays a note through the slot, as a page or a MIDI key would, renders 100 ms with the
    /// last core opened (the stream's callback) and returns the peak.
    fn play_through_the_slot(r: &Rig) -> f32 {
        r.engine
            .send(&commands::Command::Live(commands::live::Command::NoteOn {
                track: project::TrackId::parse(protocol::single_synth::TRACK).unwrap(),
                note: 69,
                velocity: 127,
            }));
        r.engine.flush();
        let mut held = r.world.lock().unwrap().core.take().expect("a core");
        let mut out = vec![0.0; 4_410 * 2];
        r.meter.take();
        held.core().unwrap().process(&mut out, 2);
        r.world.lock().unwrap().core = Some(held);
        r.meter.take().0
    }

    /// Waits up to 10 s for `condition`.
    fn eventually(what: &str, condition: impl Fn() -> bool) {
        let start = Instant::now();
        while !condition() {
            assert!(start.elapsed() < Duration::from_secs(10), "{what}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn opens_at_once_when_an_output_is_there() {
        let r = rig(
            World {
                available: true,
                ..World::default()
            },
            FAST,
        );
        eventually("installed", || r.engine.is_open());
        assert_eq!(r.engine.sample_rate(), Some(44_100.0));
        assert!(play_through_the_slot(&r) > 0.05, "the output plays");
        assert_eq!(*r.reports.lock().unwrap(), [true]);
        r.supervisor.stop();
        assert!(!r.engine.is_open(), "Quit empties the slot");
        assert_eq!(r.world.lock().unwrap().alive, 0, "Quit drops the stream");
        assert!(!r.audio_error.load(Ordering::Relaxed), "Quit is no failure");
    }

    #[test]
    fn without_an_output_it_waits_then_opens_when_the_devices_change() {
        let r = rig(World::default(), FAST);
        eventually("reported none", || *r.reports.lock().unwrap() == [false]);
        // Unchanged devices, slow timer far away: no new attempt however long it waits.
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(r.world.lock().unwrap().opens, 1);
        assert!(!r.engine.is_open());
        {
            let mut world = r.world.lock().unwrap();
            world.available = true;
            world.devices = "speakers".into();
        }
        eventually("installed", || r.engine.is_open());
        assert_eq!(r.world.lock().unwrap().opens, 2);
        assert_eq!(*r.reports.lock().unwrap(), [false, true]);
        r.supervisor.stop();
    }

    #[test]
    fn the_slow_timer_tries_again_with_the_same_devices() {
        let r = rig(
            World::default(),
            Timing {
                retry: Duration::from_millis(60),
                ..FAST
            },
        );
        eventually("first attempt", || r.world.lock().unwrap().opens >= 1);
        // The device was there all along but would not open; now it does.
        r.world.lock().unwrap().available = true;
        eventually("installed by the timer", || r.engine.is_open());
        r.supervisor.stop();
    }

    #[test]
    fn a_failed_stream_empties_the_slot_tells_the_session_and_opens_again() {
        let r = rig(
            World {
                available: true,
                ..World::default()
            },
            FAST,
        );
        eventually("installed", || r.engine.is_open());
        let failed = r.world.lock().unwrap().last_failed.clone().unwrap();
        // The output vanishes for good: the stream fails and nothing opens again.
        r.world.lock().unwrap().available = false;
        failed.store(true, Ordering::Relaxed);
        eventually("emptied", || !r.engine.is_open());
        eventually("session told", || r.audio_error.load(Ordering::Relaxed));
        eventually("reported", || *r.reports.lock().unwrap() == [true, false]);
        assert_eq!(
            r.world.lock().unwrap().alive,
            0,
            "the failed stream is dropped"
        );
        // It tried again at once, found nothing and waits for the devices.
        eventually("tried again", || r.world.lock().unwrap().opens == 2);
        {
            let mut world = r.world.lock().unwrap();
            world.available = true;
            world.devices = "headphones".into();
        }
        eventually("open again", || r.engine.is_open());
        assert_eq!(*r.reports.lock().unwrap(), [true, false, true]);
        // The new output sounds: a note sent through the slot reaches the core it plays.
        assert!(
            play_through_the_slot(&r) > 0.05,
            "the reopened output plays"
        );
        r.supervisor.stop();
    }

    /// Opens a stream that fails `after` its open, `count` times, and returns the times between
    /// one attempt and the next.
    fn gaps_of_failing_streams(timing: Timing, after: Duration, count: usize) -> Vec<Duration> {
        let r = rig(
            World {
                available: true,
                ..World::default()
            },
            timing,
        );
        let start = Instant::now();
        loop {
            let (opens, last, failed) = {
                let world = r.world.lock().unwrap();
                (
                    world.opens as usize,
                    world.opened_at.last().copied(),
                    world.last_failed.clone(),
                )
            };
            if opens > count {
                break;
            }
            if let (Some(last), Some(failed)) = (last, failed)
                && last.elapsed() >= after
            {
                failed.store(true, Ordering::Relaxed);
            }
            assert!(start.elapsed() < Duration::from_secs(20), "too slow");
            std::thread::sleep(Duration::from_millis(1));
        }
        let at = r.world.lock().unwrap().opened_at.clone();
        r.supervisor.stop();
        at.windows(2).map(|w| w[1] - w[0]).take(count).collect()
    }

    #[test]
    fn a_device_that_keeps_failing_at_once_is_retried_less_and_less_often() {
        let min_gap = Duration::from_millis(100);
        let gaps = gaps_of_failing_streams(
            Timing {
                min_gap,
                retry: Duration::from_millis(1000),
                ..FAST
            },
            Duration::ZERO,
            4,
        );
        // 100 ms, then 200, 400, 800 (each under the 1 s cap).
        for (gap, factor) in gaps.iter().zip([1u32, 2, 4, 8]) {
            assert!(*gap >= min_gap * factor, "{gaps:?}");
            assert!(
                *gap < min_gap * factor + Duration::from_millis(150),
                "{gaps:?}"
            );
        }
    }

    #[test]
    fn a_stream_that_played_a_while_is_reopened_after_the_shortest_gap() {
        let min_gap = Duration::from_millis(100);
        let gaps = gaps_of_failing_streams(
            Timing {
                min_gap,
                stable: Duration::from_millis(40),
                ..FAST
            },
            Duration::from_millis(60),
            4,
        );
        // Each stream lasted longer than `stable`: no doubling.
        for gap in &gaps {
            assert!(*gap < min_gap + Duration::from_millis(80), "{gaps:?}");
        }
    }

    #[test]
    fn stop_ends_a_supervisor_that_waits_for_an_output() {
        let r = rig(World::default(), FAST);
        eventually("reported none", || *r.reports.lock().unwrap() == [false]);
        let start = Instant::now();
        r.supervisor.stop();
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    // ------------------------------------------------ the engine kept through a reopen (#577)

    fn synth_track() -> project::TrackId {
        project::TrackId::parse(protocol::single_synth::TRACK).unwrap()
    }

    fn note(r: &Rig, on: bool) {
        let command = if on {
            commands::live::Command::NoteOn {
                track: synth_track(),
                note: 69,
                velocity: 127,
            }
        } else {
            commands::live::Command::NoteOff {
                track: synth_track(),
                note: 69,
            }
        };
        r.engine.send(&commands::Command::Live(command));
    }

    fn play(r: &Rig) {
        r.engine.send(&commands::Command::Transport(
            commands::transport::Command::Play {},
        ));
    }

    /// Renders `secs` with the core of the open output, as its callback would, in callbacks
    /// of 256 frames each followed by the control thread's flush; returns the left channel.
    fn render(r: &Rig, secs: f32) -> Vec<f32> {
        let rate = r.engine.sample_rate().expect("an open output");
        let mut held = r.world.lock().unwrap().core.take().expect("a core");
        let mut todo = (secs * rate) as usize;
        let mut left = Vec::with_capacity(todo);
        let mut out = vec![0.0; 256 * 2];
        while todo > 0 {
            let frames = todo.min(256);
            r.engine.flush();
            held.core().unwrap().process(&mut out[..frames * 2], 2);
            left.extend(out[..frames * 2].iter().step_by(2));
            todo -= frames;
        }
        r.world.lock().unwrap().core = Some(held);
        left
    }

    /// The playhead of the open output's engine, in ticks.
    fn playhead(r: &Rig) -> f64 {
        let mut world = r.world.lock().unwrap();
        let held = world.core.as_mut().expect("a core");
        held.core().unwrap().engine().clock().position()
    }

    /// Rising zero crossings per second at `rate`.
    fn pitch(x: &[f32], rate: f32) -> f32 {
        let crossings: Vec<usize> = x
            .windows(2)
            .enumerate()
            .filter(|(_, w)| w[0] < 0.0 && w[1] >= 0.0)
            .map(|(i, _)| i)
            .collect();
        assert!(crossings.len() > 10, "it sounds");
        let (first, last) = (crossings[0], *crossings.last().unwrap());
        (crossings.len() - 1) as f32 * rate / (last - first) as f32
    }

    /// A new note's pitch: on, 300 ms, off, measured after its first 50 ms.
    fn pitch_of_a_note(r: &Rig) -> f32 {
        let rate = r.engine.sample_rate().unwrap();
        note(r, true);
        let x = render(r, 0.3);
        note(r, false);
        render(r, 0.05);
        pitch(&x[(0.05 * rate) as usize..], rate)
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0f32, |m, s| m.max(s.abs()))
    }

    fn fail_stream(r: &Rig) {
        let failed = r.world.lock().unwrap().last_failed.take().unwrap();
        failed.store(true, Ordering::Relaxed);
    }

    #[test]
    fn an_output_reopened_at_another_rate_keeps_the_playhead_and_the_pitch() {
        let r = rig(
            World {
                available: true,
                ..World::default()
            },
            GRACE,
        );
        eventually("installed", || r.engine.is_open());
        play(&r);
        render(&r, 0.2);
        let before = pitch_of_a_note(&r);
        // One second at 44.1 kHz: how far the playhead goes.
        let p = playhead(&r);
        render(&r, 1.0);
        let at44 = playhead(&r) - p;
        let last = playhead(&r);

        // The device goes; another opens at 48 kHz.
        r.world.lock().unwrap().rate = 48_000.0;
        fail_stream(&r);
        eventually("reopened", || r.reopened.load(Ordering::Relaxed) == 1);
        assert_eq!(r.engine.sample_rate(), Some(48_000.0));
        assert_eq!(r.lost.load(Ordering::Relaxed), 0, "the session is not told");
        assert_eq!(*r.reports.lock().unwrap(), [true], "the tray does not flip");
        assert_eq!(r.world.lock().unwrap().opens, 2);
        // The same engine: the playhead goes on from where it stopped.
        assert_eq!(playhead(&r), last);
        // Let the rebuilt devices land, then a second at 48 kHz.
        render(&r, 0.1);
        let p = playhead(&r);
        render(&r, 1.0);
        let at48 = playhead(&r) - p;
        assert!(p > last, "it played on: {p} after {last}");
        assert!(
            (at48 - at44).abs() < 1e-3 * at44,
            "a second is {at48} ticks at 48 kHz, {at44} at 44.1"
        );
        // A note keeps its pitch: a synth left at 44.1 kHz would play it 48/44.1 higher.
        let after = pitch_of_a_note(&r);
        assert!(
            (after - before).abs() < 0.005 * before,
            "{before} Hz at 44.1 kHz, {after} Hz at 48 kHz"
        );
        assert!((before - 440.0).abs() < 0.01 * 440.0, "A4: {before} Hz");
        r.supervisor.stop();
        assert!(!r.engine.is_open());
    }

    #[test]
    fn an_output_reopened_at_the_same_rate_plays_on_untouched() {
        let r = rig(
            World {
                available: true,
                ..World::default()
            },
            GRACE,
        );
        eventually("installed", || r.engine.is_open());
        play(&r);
        note(&r, true);
        assert!(peak(&render(&r, 0.2)) > 0.05);
        let last = playhead(&r);
        fail_stream(&r);
        eventually("reopened", || r.reopened.load(Ordering::Relaxed) == 1);
        assert_eq!(r.engine.sample_rate(), Some(44_100.0));
        assert_eq!(r.lost.load(Ordering::Relaxed), 0);
        assert_eq!(playhead(&r), last);
        // Nothing was built again: the held note still sounds, with no new note-on.
        let x = render(&r, 0.2);
        assert!(peak(&x) > 0.05, "the held note goes on");
        assert!((pitch(&x, 44_100.0) - 440.0).abs() < 0.01 * 440.0);
        assert!(playhead(&r) > last);
        r.supervisor.stop();
    }

    #[test]
    fn with_no_output_before_the_grace_ends_the_engine_is_dropped_and_the_session_told() {
        let r = rig(
            World {
                available: true,
                ..World::default()
            },
            Timing {
                grace: Duration::from_millis(150),
                ..FAST
            },
        );
        eventually("installed", || r.engine.is_open());
        play(&r);
        render(&r, 0.5);
        r.world.lock().unwrap().available = false;
        let failed_at = Instant::now();
        fail_stream(&r);
        // During the grace: the engine waits in the slot, nobody is told.
        std::thread::sleep(Duration::from_millis(50));
        assert!(r.engine.is_open(), "kept during the grace");
        assert_eq!(r.lost.load(Ordering::Relaxed), 0);
        assert!(r.world.lock().unwrap().opens >= 2, "it looks for an output");
        eventually("told", || r.lost.load(Ordering::Relaxed) == 1);
        assert!(failed_at.elapsed() >= Duration::from_millis(150));
        assert!(!r.engine.is_open(), "the engine is dropped");
        assert_eq!(*r.reports.lock().unwrap(), [true, false]);
        // The next output gets a new engine: the playhead from zero, no reopened hello.
        {
            let mut world = r.world.lock().unwrap();
            world.available = true;
            world.devices = "speakers".into();
        }
        eventually("open again", || r.engine.is_open());
        assert_eq!(playhead(&r), 0.0);
        assert_eq!(r.reopened.load(Ordering::Relaxed), 0);
        assert_eq!(r.lost.load(Ordering::Relaxed), 1);
        r.supervisor.stop();
    }

    #[test]
    fn the_grace_ends_on_time_even_when_quick_failures_lengthen_the_wait() {
        let r = rig(
            World {
                available: true,
                ..World::default()
            },
            Timing {
                min_gap: Duration::from_millis(380),
                grace: Duration::from_millis(400),
                ..FAST
            },
        );
        eventually("installed", || r.engine.is_open());
        play(&r);
        render(&r, 0.2);
        // First quick failure: the next attempt is at most 380 ms after the first open, within
        // the grace.
        fail_stream(&r);
        eventually("reopened", || r.reopened.load(Ordering::Relaxed) == 1);
        assert_eq!(r.lost.load(Ordering::Relaxed), 0);
        let last = playhead(&r);
        // Second quick failure (the reopened stream fails within 250 ms of its open, once the
        // hello was sent): the next attempt is 760 ms after that open, past the 400 ms grace. The
        // engine is dropped and the session told when the grace ends, before that attempt.
        let failed_at = Instant::now();
        fail_stream(&r);
        eventually("told", || r.lost.load(Ordering::Relaxed) == 1);
        let told = failed_at.elapsed();
        assert!(
            told >= Duration::from_millis(400) && told < Duration::from_millis(480),
            "told after {told:?}"
        );
        assert_eq!(*r.lost_after.lock().unwrap(), [2], "before the third open");
        assert!(!r.engine.is_open());
        assert_eq!(*r.reports.lock().unwrap(), [true, false]);
        // The next output gets a new engine.
        eventually("open again", || r.engine.is_open());
        assert_eq!(r.world.lock().unwrap().opens, 3);
        assert_eq!(r.reopened.load(Ordering::Relaxed), 1);
        assert!(last > 0.0);
        assert_eq!(playhead(&r), 0.0, "a new engine");
        r.supervisor.stop();
    }

    #[test]
    fn a_stream_that_cannot_be_built_gives_the_core_back_for_the_next_attempt() {
        let r = rig(
            World {
                available: true,
                ..World::default()
            },
            GRACE,
        );
        eventually("installed", || r.engine.is_open());
        play(&r);
        render(&r, 0.3);
        let last = playhead(&r);
        {
            let mut world = r.world.lock().unwrap();
            world.rate = 48_000.0;
            world.fail_after_core = true;
        }
        fail_stream(&r);
        eventually("tried at 48 kHz", || r.world.lock().unwrap().opens >= 3);
        // The core went to the failed attempt and came back: still kept, at the rate it was
        // built for last.
        assert!(r.engine.is_open());
        assert_eq!(r.engine.sample_rate(), Some(48_000.0));
        assert_eq!(r.lost.load(Ordering::Relaxed), 0);
        {
            let mut world = r.world.lock().unwrap();
            world.rate = 44_100.0;
            world.fail_after_core = false;
        }
        eventually("reopened", || r.reopened.load(Ordering::Relaxed) == 1);
        assert_eq!(r.engine.sample_rate(), Some(44_100.0));
        assert_eq!(playhead(&r), last, "the same engine");
        render(&r, 0.1);
        assert!((pitch_of_a_note(&r) - 440.0).abs() < 0.01 * 440.0);
        assert_eq!(r.lost.load(Ordering::Relaxed), 0);
        r.supervisor.stop();
    }

    #[test]
    fn a_rate_change_the_engine_refuses_falls_back_to_a_new_engine() {
        let r = rig(
            World {
                available: true,
                ..World::default()
            },
            GRACE,
        );
        eventually("installed", || r.engine.is_open());
        play(&r);
        render(&r, 0.3);
        r.world.lock().unwrap().available = false;
        fail_stream(&r);
        eventually("looking", || r.world.lock().unwrap().opens >= 2);
        // Nothing drains the kept engine during the grace: a flood of parameter changes fills
        // its queues, and the rate change finds no room (`EngineBusy`).
        let device = project::DeviceId::parse(protocol::single_synth::DEVICE).unwrap();
        for i in 0..10_000 {
            r.engine.send(&commands::Command::Device(
                commands::device::Command::SetParam {
                    device: device.clone(),
                    param: "gain".into(),
                    value: if i % 2 == 0 { 0.5 } else { 0.6 },
                },
            ));
        }
        {
            let mut world = r.world.lock().unwrap();
            world.rate = 48_000.0;
            world.available = true;
        }
        eventually("open again", || r.engine.sample_rate() == Some(48_000.0));
        assert_eq!(r.lost.load(Ordering::Relaxed), 1, "the session is told");
        assert_eq!(r.reopened.load(Ordering::Relaxed), 0);
        assert_eq!(playhead(&r), 0.0, "a new engine");
        render(&r, 0.1);
        assert!((pitch_of_a_note(&r) - 440.0).abs() < 0.01 * 440.0);
        r.supervisor.stop();
    }

    #[test]
    fn quit_during_the_grace_drops_the_engine_without_telling_a_failure() {
        let r = rig(
            World {
                available: true,
                ..World::default()
            },
            GRACE,
        );
        eventually("installed", || r.engine.is_open());
        note(&r, true);
        render(&r, 0.1);
        r.world.lock().unwrap().available = false;
        fail_stream(&r);
        eventually("looking", || r.world.lock().unwrap().opens >= 2);
        assert!(r.engine.is_open());
        // Quit waits for the voices to fall silent: a kept engine reports none.
        assert_eq!(r.meter.take().1, 0);
        let start = Instant::now();
        r.supervisor.stop();
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(!r.engine.is_open());
        assert_eq!(r.lost.load(Ordering::Relaxed), 0);
    }
}
