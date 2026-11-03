//! The helper's live input against a real engine: the page's project (its core, `Synced`) set
//! up on it, the page's table taken, the input's notes rendered.

use super::*;
use crate::audio::{AudioCore, Meter, audio_core};
use commands::Command;
use commands::engine_msgs::Synced;
use commands::host::LiveRouting;
use project::TrackId;

/// The helper's engine with a page's project on it, and the live input that plays on it.
struct Rig {
    core: AudioCore,
    meter: Arc<Meter>,
    engine: Arc<EngineSlot>,
    live: LiveInput,
    page: Synced,
}

impl Rig {
    /// The page's one-synth project and a second MIDI track, set up in epoch 1.
    fn new() -> Rig {
        let meter = Arc::new(Meter::default());
        let (core, sender) = audio_core(48_000.0, meter.clone()).unwrap();
        let engine = Arc::new(EngineSlot::default());
        engine.install(sender, 48_000.0);
        let mut page = Synced::single_synth(1);
        let setup = page.setup().unwrap();
        let mut rig = Rig {
            core,
            meter,
            live: LiveInput::new(engine.clone()),
            engine,
            page,
        };
        rig.submit(&setup);
        rig.run(serde_json::json!({ "type": "track.create", "kind": "midi" }));
        let first = rig.tracks()[0].clone();
        rig.select(Some(&first));
        rig
    }

    fn generation(&self) -> u64 {
        self.engine.current().unwrap().0
    }

    fn submit(&self, bytes: &[u8]) {
        let generation = self.generation();
        self.engine.submit(generation, bytes).unwrap().unwrap();
    }

    /// Runs a command in the page's project and sends its bytes to the engine.
    fn run(&mut self, command: serde_json::Value) {
        let command: Command = serde_json::from_value(command).unwrap();
        let (_, bytes) = self.page.run(&command).unwrap();
        self.submit(&bytes);
    }

    fn tracks(&self) -> Vec<TrackId> {
        let mut tracks: Vec<TrackId> = self.page.state.project.tracks.keys().cloned().collect();
        tracks.sort();
        tracks
    }

    fn select(&mut self, track: Option<&TrackId>) {
        self.run(serde_json::json!({ "type": "view.select_track", "track": track }));
    }

    fn arm(&mut self, track: &TrackId, armed: bool) {
        self.run(serde_json::json!({ "type": "track.arm", "track": track, "armed": armed }));
    }

    /// What the page sends: its table now.
    fn route(&self) -> bool {
        let routing = LiveRouting::of(&self.page.state, &self.page.ctx.engine, self.page.epoch());
        self.live
            .set_routing(self.generation(), routing.epoch, routing.tracks)
    }

    /// Renders `ms` of audio; the voices sounding at the end.
    fn voices(&mut self, ms: usize) -> u32 {
        let mut out = vec![0.0; 480 * 2];
        for _ in 0..ms / 10 {
            self.engine.flush();
            self.core.process(&mut out, 2);
        }
        self.meter.take().1
    }

    /// Messages the engine rejected so far.
    fn rejected(&self) -> u32 {
        self.engine.with(|s| s.rejected()).unwrap()
    }
}

/// The default release is 0.3 s: after a second a released note is silent.
const RELEASED_MS: usize = 1_000;

#[test]
fn decodes_as_the_page_does() {
    assert_eq!(
        decode(&[0x90, 60, 100]),
        Note::On {
            note: 60,
            velocity: 100
        }
    );
    assert_eq!(
        decode(&[0x9F, 60, 100]),
        Note::On {
            note: 60,
            velocity: 100
        },
        "any channel"
    );
    assert_eq!(decode(&[0x90, 60, 0]), Note::Off { note: 60 }, "velocity 0");
    assert_eq!(decode(&[0x80, 60, 64]), Note::Off { note: 60 });
    assert_eq!(
        decode(&[0x80, 60]),
        Note::Off { note: 60 },
        "a note off without velocity"
    );
    assert_eq!(
        decode(&[0x90, 0xFF, 0xFF]),
        Note::On {
            note: 127,
            velocity: 127
        },
        "data bytes masked"
    );
    assert_eq!(decode(&[0xB0, 123, 0]), Note::AllOff);
    assert_eq!(decode(&[0xB3, 120, 0]), Note::AllOff);
    for other in [
        &[0xB0, 1, 64][..],
        &[0x90, 60],
        &[0x90],
        &[],
        &[0xE0, 0, 64],
        &[0xC0, 5],
    ] {
        assert_eq!(decode(other), Note::Other, "{other:?}");
    }
}

#[test]
fn without_a_table_nothing_is_played() {
    let mut rig = Rig::new();
    assert!(!rig.live.receive(&[0x90, 60, 100]));
    assert!(!rig.live.receive(&[0x80, 60, 0]));
    assert_eq!(rig.voices(50), 0);
}

#[test]
fn a_note_plays_on_the_tables_tracks_and_its_release_ends_it() {
    let mut rig = Rig::new();
    assert!(rig.route());
    assert!(rig.live.receive(&[0x90, 60, 100]));
    assert_eq!(rig.voices(50), 1, "the selected track plays it");
    assert!(rig.live.receive(&[0x90, 60, 0]), "velocity 0 releases");
    assert_eq!(rig.voices(RELEASED_MS), 0);
    assert!(
        !rig.live.receive(&[0x80, 60, 0]),
        "a note not held is not played"
    );
    assert!(
        !rig.live.receive(&[0xB0, 1, 64]),
        "a control change is the page's"
    );
    assert_eq!(rig.rejected(), 0);
}

#[test]
fn a_note_held_while_the_selection_or_the_arming_changes_ends_on_the_tracks_it_went_to() {
    let mut rig = Rig::new();
    let [first, second] = <[TrackId; 2]>::try_from(rig.tracks()).unwrap();
    rig.select(Some(&first));
    rig.arm(&second, true);
    assert!(rig.route());
    assert!(rig.live.receive(&[0x90, 60, 100]));
    assert_eq!(rig.voices(50), 2, "the selected and the armed track");
    // The selection goes, the arming too: the next table names no track.
    rig.select(None);
    rig.arm(&second, false);
    assert!(rig.route());
    // A new key is played on no track (as the page would: nothing to double).
    assert!(rig.live.receive(&[0x90, 62, 100]));
    assert_eq!(rig.voices(50), 2, "a new key reaches no track");
    assert!(rig.live.receive(&[0x80, 62, 0]));
    assert!(rig.live.receive(&[0x80, 60, 0]));
    assert_eq!(
        rig.voices(RELEASED_MS),
        0,
        "released on both tracks, nothing stuck"
    );
}

#[test]
fn a_key_struck_again_while_held_adds_the_tracks_it_reached_now() {
    let mut rig = Rig::new();
    let [first, second] = <[TrackId; 2]>::try_from(rig.tracks()).unwrap();
    rig.select(Some(&first));
    assert!(rig.route());
    assert!(rig.live.receive(&[0x90, 60, 100]));
    rig.select(Some(&second));
    assert!(rig.route());
    assert!(rig.live.receive(&[0x90, 60, 100]));
    assert_eq!(rig.voices(50), 2);
    // One note off ends it on both.
    assert!(rig.live.receive(&[0x80, 60, 0]));
    assert_eq!(rig.voices(RELEASED_MS), 0);
}

#[test]
fn a_table_of_another_epoch_or_engine_or_too_long_is_ignored() {
    let mut rig = Rig::new();
    let generation = rig.generation();
    let epoch = rig.page.epoch();
    assert!(
        !rig.live
            .set_routing(generation, epoch.wrapping_add(1), vec![0]),
        "a newer epoch"
    );
    assert!(
        !rig.live
            .set_routing(generation, epoch.wrapping_sub(1), vec![0]),
        "an older one"
    );
    assert!(
        !rig.live.set_routing(generation + 1, epoch, vec![0]),
        "another engine"
    );
    assert!(
        !rig.live
            .set_routing(generation, epoch, vec![0; MAX_TRACKS as usize + 1]),
        "longer than an engine can hold"
    );
    assert!(!rig.live.receive(&[0x90, 60, 100]), "no table was taken");
    // An ignored table keeps the previous one.
    assert!(rig.route());
    assert!(
        !rig.live
            .set_routing(generation, epoch.wrapping_add(1), vec![])
    );
    assert!(rig.live.receive(&[0x90, 60, 100]));
    assert_eq!(rig.voices(50), 1);
}

#[test]
fn an_epoch_change_while_a_note_is_held_drops_it_without_a_message_and_the_old_table_plays_nothing()
{
    let mut rig = Rig::new();
    assert!(rig.route());
    assert!(rig.live.receive(&[0x90, 60, 100]));
    assert_eq!(rig.voices(50), 1);
    // The page starts the engine over (a resync, a project opened): its `Reset` ends the note.
    let snapshot = rig.page.resync().unwrap();
    rig.submit(&snapshot);
    assert!(
        !rig.live.receive(&[0x80, 60, 0]),
        "the note went with the old epoch"
    );
    assert!(
        !rig.live.receive(&[0x90, 62, 100]),
        "the old table plays nothing"
    );
    assert_eq!(rig.voices(RELEASED_MS), 0);
    assert_eq!(rig.rejected(), 0, "nothing was sent in the old epoch");
    // The page's new table.
    assert!(rig.route());
    assert!(rig.live.receive(&[0x90, 62, 100]));
    assert_eq!(rig.voices(50), 1);
}

#[test]
fn a_new_engine_ends_the_old_ones_table_and_held_notes() {
    let mut rig = Rig::new();
    assert!(rig.route());
    assert!(rig.live.receive(&[0x90, 60, 100]));
    // Another output opened with a fresh engine (#525): a new generation.
    let (core, sender) = audio_core(48_000.0, rig.meter.clone()).unwrap();
    rig.engine.install(sender, 48_000.0);
    rig.core = core;
    assert!(!rig.live.receive(&[0x80, 60, 0]));
    assert!(!rig.live.receive(&[0x90, 60, 100]));
    assert_eq!(rig.engine.with(|s| s.waiting()), Some(0));
    assert_eq!(rig.voices(50), 0);
}

#[test]
fn all_notes_off_and_release_end_the_helpers_notes_and_clear_forgets_them() {
    let mut rig = Rig::new();
    assert!(rig.route());
    assert!(rig.live.receive(&[0x90, 60, 100]));
    assert!(rig.live.receive(&[0x90, 64, 100]));
    assert_eq!(rig.voices(50), 2);
    assert!(
        !rig.live.receive(&[0xB0, 123, 0]),
        "the page still plays the control change"
    );
    assert_eq!(
        rig.voices(RELEASED_MS),
        0,
        "the helper released its notes itself"
    );
    assert!(!rig.live.receive(&[0x80, 60, 0]), "forgotten");

    assert!(rig.live.receive(&[0x90, 60, 100]));
    rig.live.release();
    assert_eq!(rig.voices(RELEASED_MS), 0);

    assert!(rig.live.receive(&[0x90, 60, 100]));
    rig.live.clear();
    assert!(
        !rig.live.receive(&[0x80, 60, 0]),
        "clear forgets the held notes"
    );
    assert!(!rig.live.receive(&[0x90, 60, 100]), "and the table");
}
