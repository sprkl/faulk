//! TR-3: the transport commands. Live play/stop/seek, edits of tempo, signature, loop
//! and metronome, refusals that leave the state intact, and undo through the history.

mod transport_examples;

use ::project::{Tick, TickLoop};
use commands::testing::{check, check_refusal, context, fixture};
use commands::transport::Command as T;
use commands::{Command, CommandError, Output, ProjectEvent, State, Undo, history, run};
use transport_examples::{examples, looping, region, with_region};

fn tr(c: T) -> Command {
    Command::Transport(c)
}

fn metronome(enabled: Option<bool>, gain_db: Option<f64>, count_in_bars: Option<u8>) -> Command {
    tr(T::SetMetronome {
        enabled,
        gain_db,
        count_in_bars,
    })
}

fn undo() -> Command {
    Command::History(history::Command::Undo {})
}

fn refused(command: &Command, state: &State) -> CommandError {
    check_refusal(command, state, &context()).unwrap()
}

fn invalid_field(error: CommandError) -> String {
    match error {
        CommandError::Invalid { field, .. } => field,
        other => panic!("{other:?}"),
    }
}

#[test]
fn every_example_meets_the_contract_and_leaves_a_valid_project() {
    for (command, state) in examples() {
        check(&command, &state, &context()).unwrap();
        let mut s = state.clone();
        run(&command, &mut s, &mut context()).unwrap();
        s.project.validate().unwrap();
    }
}

#[test]
fn play_stop_and_seek_are_live_and_touch_nothing() {
    let f = fixture();
    for c in [
        tr(T::Play {}),
        tr(T::Stop {}),
        tr(T::Seek { tick: Tick(0) }),
        tr(T::Seek {
            tick: Tick(::project::limits::MAX_TICK),
        }),
    ] {
        let mut s = f.state.clone();
        let out = c.apply(&mut s, &mut context()).unwrap();
        assert_eq!(s, f.state);
        assert!(out.events.is_empty());
        assert_eq!(out.undo, Undo::None);
    }
    let mut s = f.state.clone();
    run(&tr(T::Play {}), &mut s, &mut context()).unwrap();
    assert_eq!(s.history, f.state.history);
}

#[test]
fn seek_refuses_a_position_outside_the_project() {
    let f = fixture();
    for tick in [-1, ::project::limits::MAX_TICK + 1, i64::MIN, i64::MAX] {
        let e = refused(&tr(T::Seek { tick: Tick(tick) }), &f.state);
        assert_eq!(invalid_field(e), "tick");
    }
}

#[test]
fn tempo_bounds_are_refused_not_clamped() {
    let f = fixture();
    for bpm in [
        19.999,
        999.001,
        0.0,
        -120.0,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ] {
        let e = refused(&tr(T::SetTempo { bpm }), &f.state);
        assert_eq!(invalid_field(e), "bpm", "{bpm}");
    }
    for bpm in [20.0, 999.0] {
        let mut s = f.state.clone();
        run(&tr(T::SetTempo { bpm }), &mut s, &mut context()).unwrap();
        assert_eq!(s.project.tempo().bpm(), bpm);
    }
}

#[test]
fn tempo_reports_the_previous_value_and_an_unchanged_one_records_nothing() {
    let f = fixture();
    let mut s = f.state.clone();
    let out = tr(T::SetTempo { bpm: 100.0 })
        .apply(&mut s, &mut context())
        .unwrap();
    assert_eq!(
        out.output,
        Output::Transport(commands::transport::Output::SetTempo { previous: 120.0 })
    );
    assert_eq!(out.events, vec![ProjectEvent::TempoChanged { bpm: 100.0 }]);
    let same = tr(T::SetTempo { bpm: 100.0 })
        .apply(&mut s, &mut context())
        .unwrap();
    assert_eq!(same.undo, Undo::None);
    assert!(same.events.is_empty());
}

#[test]
fn time_signature_refuses_invalid_values() {
    let f = fixture();
    for (n, d) in [
        (0, 4),
        (33, 4),
        (4, 0),
        (4, 3),
        (4, 33),
        (4, 64),
        (255, 255),
    ] {
        let e = refused(
            &tr(T::SetTimeSignature {
                numerator: n,
                denominator: d,
            }),
            &f.state,
        );
        assert_eq!(invalid_field(e), "time_signature", "{n}/{d}");
    }
    let mut s = f.state.clone();
    run(
        &tr(T::SetTimeSignature {
            numerator: 3,
            denominator: 8,
        }),
        &mut s,
        &mut context(),
    )
    .unwrap();
    assert_eq!(s.project.time_signature().numerator(), 3);
    assert_eq!(s.project.time_signature().denominator(), 8);
}

#[test]
fn loop_region_is_validated_and_never_clears_under_an_active_loop() {
    let f = fixture();
    for (start, end) in [
        (10, 10),
        (10, 5),
        (-1, 5),
        (0, ::project::limits::MAX_TICK + 1),
        (i64::MIN, i64::MAX),
    ] {
        let e = refused(
            &tr(T::SetLoop {
                loop_region: Some(region(start, end)),
            }),
            &f.state,
        );
        assert_eq!(invalid_field(e), "loop_region", "{start}..{end}");
    }
    // Clearing the region while the loop is on would break the project: refused.
    let e = refused(&tr(T::SetLoop { loop_region: None }), &looping());
    assert_eq!(invalid_field(e), "loop_region");
    // A region replaced while the loop is on is fine.
    let mut s = looping();
    run(
        &tr(T::SetLoop {
            loop_region: Some(region(960, 1920)),
        }),
        &mut s,
        &mut context(),
    )
    .unwrap();
    s.project.validate().unwrap();
    assert!(s.project.transport.loop_enabled);
}

#[test]
fn toggle_loop_needs_a_region_and_is_its_own_inverse() {
    let f = fixture();
    let e = refused(&tr(T::ToggleLoop {}), &f.state);
    assert_eq!(invalid_field(e), "loop_region");
    let mut s = with_region();
    let out = tr(T::ToggleLoop {}).apply(&mut s, &mut context()).unwrap();
    assert!(s.project.transport.loop_enabled);
    assert_eq!(
        out.events,
        vec![ProjectEvent::LoopChanged {
            region: Some(TickLoop {
                start: Tick(0),
                end: Tick(3840)
            }),
            enabled: true
        }]
    );
    let out = tr(T::ToggleLoop {}).apply(&mut s, &mut context()).unwrap();
    assert!(!s.project.transport.loop_enabled);
    assert_eq!(
        out.output,
        Output::Transport(commands::transport::Output::ToggleLoop { enabled: false })
    );
}

#[test]
fn metronome_validates_every_field_and_needs_one() {
    let f = fixture();
    let cases = [
        (metronome(None, None, None), "enabled"),
        (metronome(None, Some(6.1), None), "gain_db"),
        (metronome(None, Some(-96.1), None), "gain_db"),
        (metronome(None, Some(f64::NAN), None), "gain_db"),
        (metronome(None, Some(f64::NEG_INFINITY), None), "gain_db"),
        (metronome(None, None, Some(17)), "count_in_bars"),
    ];
    for (c, field) in cases {
        assert_eq!(invalid_field(refused(&c, &f.state)), field);
    }
}

#[test]
fn metronome_changes_only_what_is_given_and_undoes_only_what_changed() {
    let f = fixture();
    let mut s = f.state.clone();
    let before = s.project.transport.metronome;
    let out = metronome(Some(true), None, Some(4))
        .apply(&mut s, &mut context())
        .unwrap();
    let m = s.project.transport.metronome;
    assert!(m.enabled);
    assert_eq!(m.count_in_bars, 4);
    assert_eq!(m.gain_db, before.gain_db);
    let Undo::Inverse(inv) = out.undo else {
        panic!("an inverse")
    };
    assert_eq!(
        *inv,
        metronome(Some(false), None, Some(before.count_in_bars))
    );
    // Nothing changes: nothing recorded.
    let same = metronome(Some(true), Some(m.gain_db), Some(4))
        .apply(&mut s, &mut context())
        .unwrap();
    assert_eq!(same.undo, Undo::None);
    assert!(same.events.is_empty());
}

#[test]
fn metronome_gain_outside_the_range_in_the_document_is_not_edited_by_command() {
    let mut s = fixture().state;
    s.project.transport.metronome.gain_db = f64::NEG_INFINITY;
    let e = refused(&metronome(None, Some(-6.0), None), &s);
    assert_eq!(invalid_field(e), "gain_db");
    // The other fields stay editable.
    run(&metronome(Some(true), None, None), &mut s, &mut context()).unwrap();
}

#[test]
fn edits_are_recorded_and_one_undo_restores_each() {
    let mut s = with_region();
    let start = s.clone();
    let edits = [
        tr(T::SetTempo { bpm: 77.0 }),
        tr(T::SetTimeSignature {
            numerator: 5,
            denominator: 4,
        }),
        tr(T::ToggleLoop {}),
        tr(T::SetLoop {
            loop_region: Some(region(960, 4800)),
        }),
        metronome(Some(true), Some(-3.0), Some(3)),
    ];
    for c in &edits {
        let before = s.clone();
        run(c, &mut s, &mut context()).unwrap();
        assert_eq!(s.history.undo.len(), before.history.undo.len() + 1, "{c:?}");
        let mut undone = s.clone();
        run(&undo(), &mut undone, &mut context()).unwrap();
        assert_eq!(undone.project, before.project, "{c:?}");
    }
    for _ in &edits {
        run(&undo(), &mut s, &mut context()).unwrap();
    }
    assert_eq!(s.project, start.project);
}
