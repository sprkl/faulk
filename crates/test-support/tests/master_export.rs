//! #364: the export (`project.export_audio`) goes through the master's chain; #378: and plays
//! the master's lanes.

use commands::export::ExportSettings;
use commands::{Command, automation, device};
use test_support::export::{demo_song, export_native};

fn song(state: &commands::State) -> test_support::Render {
    export_native(state, ExportSettings::default()).render()
}

#[test]
fn the_export_goes_through_the_masters_chain_and_a_bypassed_chain_changes_nothing() {
    let plain = demo_song(2, None);
    let with_chain = |bypass: bool| {
        let mut state = plain.clone();
        let mut ctx = commands::testing::context();
        let out = commands::run(
            &Command::Device(device::Command::Insert {
                track: None,
                master: true,
                kind: "faulk.compressor".into(),
                index: 0,
            }),
            &mut state,
            &mut ctx,
        )
        .unwrap();
        let commands::Output::Device(device::Output::Insert { device: fx }) = out.output else {
            panic!()
        };
        for (param, value) in [("threshold", -40.0), ("ratio", 20.0)] {
            commands::run(
                &Command::Device(device::Command::SetParam {
                    device: fx.clone(),
                    param: param.into(),
                    value,
                }),
                &mut state,
                &mut ctx,
            )
            .unwrap();
        }
        if bypass {
            commands::run(
                &Command::Device(device::Command::SetBypass {
                    device: fx,
                    bypass: true,
                }),
                &mut state,
                &mut ctx,
            )
            .unwrap();
        }
        song(&state)
    };
    let dry = song(&plain);
    let treated = with_chain(false);
    let bypassed = with_chain(true);
    treated.assert_clean();
    assert_eq!(dry.left, bypassed.left, "a bypassed chain is bit-exact");
    assert!(
        treated.rms() < 0.8 * dry.rms(),
        "the chain did not act on the export: {} vs {}",
        treated.rms(),
        dry.rms()
    );
}

#[test]
fn the_export_plays_the_masters_lanes_from_the_first_sample() {
    let plain = demo_song(2, None);
    let mut state = plain.clone();
    let mut ctx = commands::testing::context();
    let out = commands::run(
        &Command::Automation(automation::Command::LaneCreate {
            track: None,
            master: true,
            target: project::AutomationTarget::Mixer {
                param: project::MixerParam::Gain,
            },
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    let commands::Output::Automation(automation::Output::LaneCreate { lane }) = out.output else {
        panic!()
    };
    // A single point holds its value from the start: -20 dB for the whole song.
    commands::run(
        &Command::Automation(automation::Command::PointsAdd {
            lane,
            points: vec![automation::Point {
                tick: project::Tick(0),
                value: -20.0,
                shape: project::Shape::Step,
            }],
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    let dry = song(&plain);
    let lowered = song(&state);
    lowered.assert_clean();
    let ratio = f64::from(lowered.rms() / dry.rms());
    assert!(
        (ratio - 0.1).abs() < 0.002,
        "the export did not play the lane: x{ratio}"
    );
}
