//! One example of every branch of the `transport.*` commands, with the state it runs
//! from. `tests/it/contract.rs` adds [`examples`] to the catalog of examples (a command
//! without one fails there); `tests/it/transport.rs` also checks each against `validate`.

#![allow(dead_code)]

use ::project::{LoopRegion, Quantize, Tick, TickLoop};
use commands::testing::fixture;
use commands::transport::Command as T;
use commands::{Command, State};

/// The fixture with a loop region `[0, 3840)`, the loop off.
pub fn with_region() -> State {
    let mut s = fixture().state;
    s.project.transport.loop_region = LoopRegion::new(Tick(0), Tick(3840));
    s
}

/// The fixture with a loop region `[0, 3840)` and the loop on.
pub fn looping() -> State {
    let mut s = with_region();
    s.project.transport.loop_enabled = true;
    s
}

/// The fixture with a silent click (`gain_db` null in the document).
pub fn silent_click() -> State {
    let mut s = fixture().state;
    s.project.transport.metronome.gain_db = f64::NEG_INFINITY;
    s
}

pub fn region(start: i64, end: i64) -> TickLoop {
    TickLoop {
        start: Tick(start),
        end: Tick(end),
    }
}

pub fn examples() -> Vec<(Command, State)> {
    let base = fixture().state;
    let tr = |c: T| Command::Transport(c);
    vec![
        (tr(T::Play {}), base.clone()),
        (tr(T::Stop {}), base.clone()),
        (tr(T::Seek { tick: Tick(1920) }), base.clone()),
        (tr(T::Pause { tick: Tick(1920) }), base.clone()),
        (
            tr(T::SetTempo {
                bpm: 90.5,
                placements: None,
            }),
            base.clone(),
        ),
        (
            tr(T::SetTempo {
                bpm: 20.0,
                placements: None,
            }),
            base.clone(),
        ),
        (
            tr(T::SetTempo {
                bpm: 999.0,
                placements: None,
            }),
            base.clone(),
        ),
        (
            tr(T::SetTimeSignature {
                numerator: 7,
                denominator: 8,
            }),
            base.clone(),
        ),
        (
            tr(T::SetLaunchQuantize {
                quantize: Quantize::Bars8,
            }),
            base.clone(),
        ),
        (
            tr(T::SetLaunchQuantize {
                quantize: Quantize::None,
            }),
            base.clone(),
        ),
        (
            tr(T::SetLoop {
                loop_region: Some(region(960, 3840)),
                enabled: None,
            }),
            base.clone(),
        ),
        (
            tr(T::SetLoop {
                loop_region: Some(region(0, 1)),
                enabled: None,
            }),
            with_region(),
        ),
        (
            tr(T::SetLoop {
                loop_region: None,
                enabled: None,
            }),
            with_region(),
        ),
        (tr(T::ToggleLoop {}), with_region()),
        (tr(T::ToggleLoop {}), looping()),
        (
            tr(T::SetMetronome {
                enabled: Some(true),
                gain_db: Some(Some(-9.0)),
                count_in_bars: Some(2),
            }),
            base.clone(),
        ),
        (
            tr(T::SetMetronome {
                enabled: None,
                gain_db: Some(None),
                count_in_bars: None,
            }),
            base.clone(),
        ),
        (
            tr(T::SetMetronome {
                enabled: None,
                gain_db: Some(Some(-3.0)),
                count_in_bars: None,
            }),
            silent_click(),
        ),
        (
            tr(T::SetMetronome {
                enabled: None,
                gain_db: None,
                count_in_bars: Some(0),
            }),
            base.clone(),
        ),
        (
            tr(T::SetMetronome {
                enabled: Some(false),
                gain_db: Some(Some(6.0)),
                count_in_bars: None,
            }),
            base,
        ),
    ]
}
