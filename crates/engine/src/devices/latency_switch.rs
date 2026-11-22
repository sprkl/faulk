//! The switch of a device's latency while it sounds (#159, shared with the Compressor by
//! #157): a change of a parameter that moves the latency (a look-ahead) must not be heard as
//! a jump of the delayed signal, nor let samples measured with the old setting through.
//!
//! The change waits until no other one came for [`SETTLE_SECONDS`] (a drag of the control or
//! an automation lane sends many: one switch for the whole gesture, the old latency
//! meanwhile), then the output fades out over [`FADE_SECONDS`], the device takes the new
//! latency, the output stays silent for the new latency (the samples still in the line were
//! measured with the old setting, so they are never heard), then fades back in over
//! [`FADE_SECONDS`]. A change made before any frame since the last reset (a device being
//! inserted with its state) applies at once. A change during the fade out only moves the
//! target; one during the silence takes the newer latency and starts the silence again; one
//! during the fade in waits again (the fade in goes on meanwhile). A value that brings the
//! latency back to the one in use cancels the wait. The fades only lower the output.
//!
//! The device keeps the latency itself: [`LatencySwitch`] only says when to take the wanted
//! one and what gain the output gets. No allocation, constant time.

/// How long a change of latency waits for the next one before it fades out.
pub const SETTLE_SECONDS: f32 = 0.05;
/// Fade out, then in, around a change of latency.
pub const FADE_SECONDS: f32 = 0.005;

/// What the output is doing around a change of latency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Steady,
    /// A change of latency waits for these many more frames without a new one, fading back
    /// in meanwhile if it was.
    Settle(u32),
    /// Fading out towards the switch.
    Out,
    /// Silent for these many more frames after the switch.
    Mute(u32),
    /// Fading back in.
    In,
}

/// What the device does on a change of the wanted latency ([`LatencySwitch::ask`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Take {
    /// Nothing now: the switch comes later (or is cancelled).
    Later,
    /// Take the wanted latency now; nothing was heard since the reset, so no silence.
    Now,
    /// Take the wanted latency now (it changed during the silence), then call
    /// [`LatencySwitch::silence`] with the new latency.
    NowSilent,
}

#[derive(Clone, Copy, Debug)]
pub struct LatencySwitch {
    phase: Phase,
    fade: f32,
    fade_step: f32,
    settle_frames: u32,
}

impl LatencySwitch {
    /// A switch at rest, its times at `sample_rate` (finite, positive: the device's own).
    pub fn new(sample_rate: f32) -> Self {
        Self {
            phase: Phase::Steady,
            fade: 1.0,
            fade_step: 1.0 / (FADE_SECONDS * sample_rate),
            settle_frames: (SETTLE_SECONDS * sample_rate) as u32,
        }
    }

    /// Back at rest, the output fully up.
    pub fn reset(&mut self) {
        self.phase = Phase::Steady;
        self.fade = 1.0;
    }

    /// The wanted latency changed. `fresh`: no frame since the last reset; `differs`: the
    /// wanted latency is not the one in use.
    pub fn ask(&mut self, fresh: bool, differs: bool) -> Take {
        if fresh {
            self.reset();
            return Take::Now;
        }
        match self.phase {
            // A drag sends many values: the switch waits until they stop.
            Phase::Steady | Phase::In | Phase::Settle(_) => {
                self.phase = if differs {
                    Phase::Settle(self.settle_frames)
                } else if self.phase == Phase::Steady {
                    Phase::Steady
                } else {
                    Phase::In
                };
                Take::Later
            }
            // Taken back during the fade out: it fades back in, nothing switches.
            Phase::Out => {
                if !differs {
                    self.phase = Phase::In;
                }
                Take::Later
            }
            Phase::Mute(_) => {
                if differs {
                    Take::NowSilent
                } else {
                    Take::Later
                }
            }
        }
    }

    /// True at the start of a frame when the fade out is over: the device takes the wanted
    /// latency, then calls [`Self::silence`].
    #[inline]
    pub fn due(&self) -> bool {
        self.phase == Phase::Out && self.fade <= 0.0
    }

    /// Silent for the latency just taken (nothing older than it may be heard), then the fade
    /// in; with no latency nothing is in flight: straight to the fade in.
    pub fn silence(&mut self, latency: u32) {
        self.phase = if latency > 0 {
            Phase::Mute(latency)
        } else {
            Phase::In
        };
    }

    /// The gain of this frame's output (0 to 1, exactly 1 at rest), one frame on. `differs`:
    /// the wanted latency is not the one in use (read when the wait ends).
    #[inline]
    pub fn gain(&mut self, differs: bool) -> f32 {
        match self.phase {
            Phase::Steady => 1.0,
            Phase::Settle(left) => {
                self.fade = (self.fade + self.fade_step).min(1.0);
                self.phase = if left > 1 {
                    Phase::Settle(left - 1)
                } else if differs {
                    Phase::Out
                } else {
                    Phase::In
                };
                self.fade
            }
            Phase::Out => {
                self.fade = (self.fade - self.fade_step).max(0.0);
                self.fade
            }
            Phase::Mute(left) => {
                self.phase = if left <= 1 {
                    Phase::In
                } else {
                    Phase::Mute(left - 1)
                };
                0.0
            }
            Phase::In => {
                self.fade = (self.fade + self.fade_step).min(1.0);
                if self.fade >= 1.0 {
                    self.phase = Phase::Steady;
                }
                self.fade
            }
        }
    }
}
