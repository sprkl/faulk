//! The audio output supervisor (#525): the helper starts and serves without an output, and
//! opens one as soon as it can.
//!
//! A thread of its own creates the output stream, watches it and drops it, so the stream never
//! crosses threads. Without an output it tries again when the output devices change (polled)
//! or on a slow timer; when the stream fails it drops it, tells the session (`Links::lost`, the
//! server sends `audio.stream`) and looks for an output again. The engine of the open output
//! lives in the [`EngineSlot`], empty while there is none: the server answers a session with
//! `audio.no_output` and the page keeps playing itself.
//!
//! The device side is a [`Backend`], so the loop is tested without a sound card; [`Cpal`] is
//! the real one.

use crate::audio::{EngineSlot, EventSender, Meter, audio_core};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// An output just opened, with the engine that feeds it.
pub struct Opened<S> {
    /// Keeps the output playing; dropping it stops the output.
    pub stream: S,
    pub sender: EventSender,
    pub sample_rate: f32,
    /// For the journal: device, rate, channels.
    pub description: String,
}

/// The device side of the supervisor.
pub trait Backend {
    type Stream;
    /// A value that changes when the output devices change (the default output, the list).
    fn devices(&mut self) -> String;
    /// Opens the default output with an engine at its rate (`audio_core`, which points `meter`
    /// at that engine's report). The stream raises `failed` when it fails. The reason, when it
    /// cannot, is for the journal.
    fn open(
        &mut self,
        meter: &Arc<Meter>,
        failed: Arc<AtomicBool>,
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
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            watch: Duration::from_millis(100),
            devices: Duration::from_secs(2),
            retry: Duration::from_secs(30),
            min_gap: Duration::from_secs(1),
            stable: Duration::from_secs(10),
        }
    }
}

/// What the supervisor shares with the rest of the helper.
pub struct Links {
    pub engine: Arc<EngineSlot>,
    pub meter: Arc<Meter>,
    /// Called when an open output failed, once its engine is gone and before another output
    /// opens: the server tells the session (`Shared::output_lost`).
    pub lost: Box<dyn Fn() + Send>,
    /// Told `true` when an output opens and `false` when there is none (once at start, then on
    /// each change), for the tray.
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
            .spawn(move || supervise(make(), &links, &stop_rx, timing))?;
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
    loop {
        let failed = Arc::new(AtomicBool::new(false));
        let attempt = Instant::now();
        match backend.open(&links.meter, failed.clone()) {
            Ok(opened) => {
                links.engine.install(opened.sender, opened.sample_rate);
                crate::log!("audio output: {}", opened.description);
                last_reason = None;
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
                links.engine.remove();
                links.meter.reset();
                if quit {
                    return;
                }
                quick = if attempt.elapsed() < timing.stable {
                    quick + 1
                } else {
                    0
                };
                crate::log!("the audio output failed: looking for an output");
                report(false);
                (links.lost)();
                // Try again at once (the default output may have changed), but not faster than
                // `min_gap`, doubled for each quick failure in a row after the first.
                let gap = timing
                    .min_gap
                    .saturating_mul(1 << quick.saturating_sub(1).min(16))
                    .min(timing.retry.max(timing.min_gap));
                if stopped(stop, gap.saturating_sub(attempt.elapsed())) {
                    return;
                }
            }
            Err(reason) => {
                // `open` may have pointed the meter at an engine it then dropped.
                links.meter.reset();
                // The devices as they are now: a change from here on is worth a new attempt.
                let devices = backend.devices();
                if last_reason.as_deref() != Some(reason.as_str()) {
                    crate::log!("no audio output ({reason}): waiting for one");
                    last_reason = Some(reason);
                }
                report(false);
                loop {
                    if stopped(stop, timing.devices) {
                        return;
                    }
                    let waited = attempt.elapsed();
                    if waited >= timing.retry
                        || (waited >= timing.min_gap && backend.devices() != devices)
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
        meter: &Arc<Meter>,
        failed: Arc<AtomicBool>,
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
        let (mut core, sender) = audio_core(sample_rate, meter.clone())
            .map_err(|e| format!("cannot create the engine: {e:?}"))?;
        let stream = device
            .build_output_stream(
                config,
                move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
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
            sender,
            sample_rate,
            description: format!("{name} · {sample_rate} Hz, {channels} channels"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

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
        /// The audio core of the last stream opened, which the test renders as its callback.
        core: Option<crate::audio::AudioCore>,
        /// Streams alive (opened and not dropped).
        alive: u32,
    }

    struct Fake(Arc<Mutex<World>>);

    struct FakeStream(Arc<Mutex<World>>);

    impl Drop for FakeStream {
        fn drop(&mut self) {
            self.0.lock().unwrap().alive -= 1;
        }
    }

    impl Backend for Fake {
        type Stream = FakeStream;

        fn devices(&mut self) -> String {
            self.0.lock().unwrap().devices.clone()
        }

        fn open(
            &mut self,
            meter: &Arc<Meter>,
            failed: Arc<AtomicBool>,
        ) -> Result<Opened<FakeStream>, String> {
            let mut world = self.0.lock().unwrap();
            world.opens += 1;
            world.opened_at.push(Instant::now());
            if !world.available {
                return Err("no audio output device".into());
            }
            let (core, sender) = audio_core(44_100.0, meter.clone()).unwrap();
            world.last_failed = Some(failed);
            world.core = Some(core);
            world.alive += 1;
            Ok(Opened {
                stream: FakeStream(self.0.clone()),
                sender,
                sample_rate: 44_100.0,
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
        supervisor: Supervisor,
    }

    const FAST: Timing = Timing {
        watch: Duration::from_millis(5),
        devices: Duration::from_millis(5),
        retry: Duration::from_secs(3600),
        min_gap: Duration::from_millis(20),
        stable: Duration::from_secs(3600),
    };

    fn rig(world: World, timing: Timing) -> Rig {
        let world = Arc::new(Mutex::new(world));
        let engine = Arc::new(EngineSlot::default());
        let audio_error = Arc::new(AtomicBool::new(false));
        let lost = audio_error.clone();
        let reports = Arc::new(Mutex::new(Vec::new()));
        let told = reports.clone();
        let backend = world.clone();
        let meter = Arc::new(Meter::default());
        let supervisor = Supervisor::start(
            move || Fake(backend),
            Links {
                engine: engine.clone(),
                meter: meter.clone(),
                lost: Box::new(move || lost.store(true, Ordering::Relaxed)),
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
        let mut core = r.world.lock().unwrap().core.take().expect("a core");
        let mut out = vec![0.0; 4_410 * 2];
        r.meter.take();
        core.process(&mut out, 2);
        r.world.lock().unwrap().core = Some(core);
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
}
