//! The queries that measure levels (`mixer.levels`, `project.analyze_render`, #665): the
//! host stages its readings, the command names the tracks, and a reading serves one answer.

use commands::engine_msgs::snapshot;
use commands::levels::{AnalysisStage, LevelsStage, MAX_ANALYZE_SECS, MeterReading};
use commands::testing::context;
use commands::{Command, CommandError, Output, mixer, project};

use crate::arrangement_examples;

fn reading(peak: f32, rms: f32) -> MeterReading {
    MeterReading {
        peak_l: peak,
        peak_r: peak,
        rms_l: rms,
        rms_r: rms,
    }
}

fn levels(state: &mut commands::State, ctx: &mut commands::Context) -> Output {
    Command::Mixer(mixer::Command::Levels {})
        .apply(state, ctx)
        .unwrap()
        .output
}

#[test]
fn levels_name_the_tracks_through_the_engine_slots_and_serve_one_answer() {
    let f = arrangement_examples::fx();
    let mut state = f.state.clone();
    let mut ctx = context();
    ctx.engine = snapshot(&state.project, &state.session).1;
    let slot = ctx.engine.tracks.get(&f.track).unwrap() as usize;
    let mut tracks = vec![MeterReading::default(); slot + 1];
    tracks[slot] = reading(0.5, 0.25);
    ctx.levels = Some(LevelsStage {
        playing: true,
        tracks,
        master: reading(1.0, 0.5),
    });
    let before = state.clone();
    let Output::Mixer(mixer::Output::Levels {
        playing,
        tracks,
        master,
    }) = levels(&mut state, &mut ctx)
    else {
        panic!()
    };
    assert_eq!(state, before, "a query changes nothing");
    assert!(playing);
    let mine = tracks.iter().find(|t| t.track == f.track).unwrap();
    assert!((mine.peak_db.unwrap() - 20.0 * 0.5f64.log10()).abs() < 1e-9);
    assert!((mine.rms_db.unwrap() - 20.0 * 0.25f64.log10()).abs() < 1e-9);
    assert_eq!(master.peak_db, Some(0.0));
    assert_eq!(tracks.len(), state.project.tracks.len());
    // Taken: the next answer is silence, not the old reading.
    let Output::Mixer(mixer::Output::Levels {
        playing, tracks, ..
    }) = levels(&mut state, &mut ctx)
    else {
        panic!()
    };
    assert!(!playing);
    assert!(tracks.iter().all(|t| t.peak_db.is_none()));
}

fn analyze(from_bar: Option<u32>, to_bar: Option<u32>) -> Command {
    Command::Project(project::Command::AnalyzeRender { from_bar, to_bar })
}

fn stage(state: &commands::State, from_bar: Option<u32>, to_bar: Option<u32>) -> AnalysisStage {
    let plan = commands::export::analysis_plan(&state.project, from_bar, to_bar).unwrap();
    AnalysisStage {
        start_tick: plan.start_tick,
        end_tick: plan.end_tick,
        tracks: vec![reading(0.5, 0.1)],
        master: reading(2.0, 0.5),
        silenced_frames: 3,
        reduction: 0.5,
    }
}

#[test]
fn an_analysis_without_a_render_is_refused_and_a_staged_one_is_taken_once() {
    let f = arrangement_examples::fx();
    let mut state = f.state.clone();
    let mut ctx = context();
    assert!(matches!(
        analyze(None, None).apply(&mut state, &mut ctx),
        Err(CommandError::Invalid { .. })
    ));
    ctx.analysis = Some(stage(&state, None, None));
    let out = analyze(None, None).apply(&mut state, &mut ctx).unwrap();
    let Output::Project(project::Output::AnalyzeRender {
        master,
        silenced_frames,
        limiter_reduction_db,
        tracks,
        ..
    }) = out.output
    else {
        panic!()
    };
    assert_eq!(silenced_frames, 3);
    assert!(
        master.peak_db.unwrap() > 6.0,
        "the master reads above 0: {master:?}"
    );
    assert!((limiter_reduction_db - 6.0206).abs() < 1e-3);
    assert_eq!(tracks.len(), state.project.tracks.len());
    assert!(out.events.is_empty() && matches!(out.undo, commands::Undo::None));
    assert!(ctx.analysis.is_none(), "taken");
    assert!(analyze(None, None).apply(&mut state, &mut ctx).is_err());
}

#[test]
fn an_analysis_of_another_range_than_the_rendered_one_is_refused() {
    let f = arrangement_examples::fx();
    let mut state = f.state.clone();
    let mut ctx = context();
    ctx.analysis = Some(stage(&state, Some(1), Some(1)));
    let error = analyze(Some(2), Some(2))
        .apply(&mut state, &mut ctx)
        .unwrap_err();
    assert!(matches!(error, CommandError::Invalid { .. }), "{error:?}");
}

#[test]
fn the_range_is_checked_like_an_export() {
    let f = arrangement_examples::fx();
    let mut state = f.state.clone();
    for (from, to, field) in [
        (Some(3), Some(2), "to_bar"),
        (Some(1), None, "from_bar"),
        (None, Some(2), "from_bar"),
        (Some(0), Some(1), "from_bar"),
        (Some(900), Some(901), "from_bar"),
    ] {
        match analyze(from, to).apply(&mut state, &mut context()) {
            Err(CommandError::Invalid { field: got, .. }) => assert_eq!(got, field),
            other => panic!("{from:?} {to:?}: {other:?}"),
        }
    }
}

#[test]
fn a_range_longer_than_the_bound_is_a_limit_before_any_render() {
    let f = arrangement_examples::fx();
    let mut state = f.state.clone();
    let bar = state.project.time_signature().ticks_per_bar().0;
    let track = state.project.tracks.get_mut(&f.track).unwrap();
    let placement = track.placements.get_mut(&f.a).unwrap();
    placement.span = ::project::Span::Ticks {
        length: ::project::Tick(bar * 400),
        offset: ::project::Tick(0),
    };
    match commands::export::analysis_plan(&state.project, None, None) {
        Err(CommandError::Limit { max, .. }) => assert_eq!(max, MAX_ANALYZE_SECS),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        analyze(None, None).apply(&mut state, &mut context()),
        Err(CommandError::Limit { .. })
    ));
}
