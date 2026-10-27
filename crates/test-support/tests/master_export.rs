//! #364: the export (`project.export_audio`) goes through the master's chain.

use commands::export::ExportSettings;
use commands::{Command, device};
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
