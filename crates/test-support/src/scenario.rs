//! A project session recorded as engine messages and render steps.

use commands::engine_msgs::Synced;
use commands::testing::{self, Fixture};
use commands::{Command, Context, State, device, live};
use engine::msg::ControlMsg;

use crate::render::{QUANTUM, SAMPLE_RATE};

/// One step of a scenario.
#[derive(Debug, PartialEq)]
pub enum Step {
    /// Bytes of the engine's message stream, pushed to the prepare side.
    Push(Vec<u8>),
    /// Blocks of [`QUANTUM`] frames to render.
    Render(u32),
}

/// What a session did, in the order it did it: replayed on any engine target.
#[derive(Debug, Default, PartialEq)]
pub struct Scenario {
    pub steps: Vec<Step>,
}

impl Scenario {
    /// The fixture format: per step a kind byte (0 push, 1 render), a little-endian u32
    /// (the byte count of a push, the blocks of a render), then a push's bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for step in &self.steps {
            match step {
                Step::Push(b) => {
                    out.push(0);
                    out.extend_from_slice(&(b.len() as u32).to_le_bytes());
                    out.extend_from_slice(b);
                }
                Step::Render(n) => {
                    out.push(1);
                    out.extend_from_slice(&n.to_le_bytes());
                }
            }
        }
        out
    }

    /// The scenario of [`Scenario::encode`]d bytes; `None` when they are truncated.
    pub fn decode(mut bytes: &[u8]) -> Option<Scenario> {
        let mut steps = Vec::new();
        while !bytes.is_empty() {
            let [kind, a, b, c, d, rest @ ..] = bytes else {
                return None;
            };
            let n = u32::from_le_bytes([*a, *b, *c, *d]);
            if *kind == 0 {
                let n = n as usize;
                steps.push(Step::Push(rest.get(..n)?.to_vec()));
                bytes = &rest[n..];
            } else if *kind == 1 {
                steps.push(Step::Render(n));
                bytes = rest;
            } else {
                return None;
            }
        }
        Some(Scenario { steps })
    }

    /// The seconds of audio the scenario renders.
    pub fn seconds(&self) -> f32 {
        let blocks: u32 = self
            .steps
            .iter()
            .map(|s| if let Step::Render(n) = s { *n } else { 0 })
            .sum();
        blocks as f32 * QUANTUM as f32 / SAMPLE_RATE
    }
}

/// The value of an id: its 26 Crockford base32 characters read as a 128-bit number (ids sort
/// as their values do).
fn id_value(id: &str) -> u128 {
    const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    id.bytes().fold(0u128, |v, c| {
        let digit = ALPHABET.iter().position(|a| *a == c).unwrap_or(0) as u128;
        (v << 5) | digit
    })
}

/// `ids`, or a generator past every id `state` holds when `ids` could make one of them.
///
/// The examples' context (`commands::testing::context`) always starts from the same seed. A
/// state built by a session on that context and then given to another session on a fresh one
/// would see its ids made again: the first device inserted would take the id of a device
/// already there and replace it in the project, while the engine, whose slot table already
/// holds that id, would be told nothing (#700). Every id the state holds (project, session,
/// history, library) is found in its debug form, which prints each one quoted. A quoted name
/// that happens to read as an id only moves the generator further, which is harmless. Panics
/// when the state holds the last possible id (`7ZZZZZZZZZZZZZZZZZZZZZZZZZ`): nothing comes after it.
fn ids_after(state: &State, ids: project::IdGenerator) -> project::IdGenerator {
    let text = format!("{state:?}");
    let held = text
        .split('"')
        .filter(|s| project::Id::parse(s).is_ok())
        .map(id_value)
        .max();
    let next = id_value(ids.clone().next_id().as_str());
    match held {
        Some(max) if max >= next => {
            let after = max
                .checked_add(1)
                .expect("the state holds the last id: no id comes after it");
            project::IdGenerator::new((after >> 80) as u64, after & ((1u128 << 80) - 1))
        }
        _ => ids,
    }
}

/// A project edited through commands, and the engine stream those commands produce.
///
/// Every command runs through [`commands::run`] on a real project state (so an invalid
/// command fails the test), and the engine messages of its outcome (PR-3,
/// `commands::engine_msgs::compile`) are recorded. The session starts with a `Reset` and
/// the snapshot of its project. [`Session::messages`] adds raw engine messages.
pub struct Session {
    synced: Synced,
    fixture: Option<Fixture>,
    scenario: Scenario,
}

impl Session {
    /// A project with one MIDI track playing the built-in synth (`commands::testing`'s
    /// fixture), and an engine set up for it.
    pub fn single_synth() -> Session {
        let fixture = testing::fixture();
        let mut s = Session::from_state(fixture.state.clone(), testing::context());
        s.fixture = Some(fixture);
        s
    }

    /// A session on `state`: the engine receives a `Reset` (epoch 1) and the snapshot of
    /// the project, then the commands. The ids the commands create come after every id the
    /// state holds ([`ids_after`]).
    pub fn from_state(state: State, mut ctx: Context) -> Session {
        ctx.ids = ids_after(&state, ctx.ids);
        let mut synced = Synced::new(state, ctx, 1);
        let setup = synced.setup().expect("the setup encodes");
        Session {
            synced,
            fixture: None,
            scenario: Scenario {
                steps: vec![Step::Push(setup)],
            },
        }
    }

    /// The project state the commands ran on.
    pub fn state(&self) -> &State {
        &self.synced.state
    }

    /// The command context: its slot table is the engine's view of the project.
    pub fn ctx(&self) -> &Context {
        &self.synced.ctx
    }

    fn fixture(&self) -> &Fixture {
        self.fixture
            .as_ref()
            .expect("a single_synth session: the fixture's track and synth")
    }

    /// Applies a command to the project and sends its engine messages. Panics when the
    /// command is refused.
    pub fn run(&mut self, command: Command) -> &mut Self {
        self.exec(command);
        self
    }

    /// [`Session::run`], returning the command's output (the ids it created).
    pub fn exec(&mut self, command: Command) -> commands::Output {
        let (outcome, bytes) = match self.synced.run(&command) {
            Ok(done) => done,
            Err(e) => panic!("command {command:?} refused: {e:?}"),
        };
        if !bytes.is_empty() {
            self.scenario.steps.push(Step::Push(bytes));
        }
        outcome.output
    }

    /// Stages an imported audio file's metadata (AU-1), as the page does after it imported
    /// the file and before it runs `clip.import_audio`.
    pub fn stage_media(&mut self, media: project::AudioMedia) -> &mut Self {
        self.synced.ctx.media.insert(media.hash, media);
        self
    }

    /// Stages a project folder for `project.open`, as the page does before it runs the
    /// command (`commands::contract::Folder`).
    pub fn stage(&mut self, folder: commands::contract::Folder) -> &mut Self {
        self.synced.ctx.folder = Some(folder);
        self
    }

    /// Runs `project.open` on the staged folder `name`: the project, its session and its
    /// history are replaced and the engine starts over from a `Reset` in the next epoch and
    /// the snapshot of the opened project. Panics when the command is refused. The
    /// single-synth helpers (`note_on`, `set_param`) no longer apply afterwards. As in
    /// [`Session::from_state`], the ids made afterwards come after every id the opened
    /// state holds ([`ids_after`]).
    pub fn open(&mut self, name: &str) -> &mut Self {
        self.fixture = None;
        self.run(Command::Project(commands::project::Command::Open {
            folder: name.to_owned(),
        }));
        let ids = self.synced.ctx.ids.clone();
        self.synced.ctx.ids = ids_after(&self.synced.state, ids);
        self
    }

    /// Sends a `Reset` in a new epoch and the snapshot of the project as it is now.
    pub fn resync(&mut self) -> &mut Self {
        let bytes = self.synced.resync().expect("the snapshot encodes");
        self.scenario.steps.push(Step::Push(bytes));
        self
    }

    /// Sends raw engine messages (what no command maps).
    pub fn messages(&mut self, msgs: &[ControlMsg]) -> &mut Self {
        let bytes = self.synced.encode(msgs).expect("the messages encode");
        self.scenario.steps.push(Step::Push(bytes));
        self
    }

    pub fn note_on(&mut self, note: u8, velocity: u8) -> &mut Self {
        let track = self.fixture().track.clone();
        self.run(Command::Live(live::Command::NoteOn {
            track,
            note,
            velocity,
        }))
    }

    pub fn note_off(&mut self, note: u8) -> &mut Self {
        let track = self.fixture().track.clone();
        self.run(Command::Live(live::Command::NoteOff { track, note }))
    }

    pub fn all_notes_off(&mut self) -> &mut Self {
        self.run(Command::Live(live::Command::AllNotesOff {}))
    }

    /// `device.set_param` on the synth.
    pub fn set_param(&mut self, param: &str, value: f64) -> &mut Self {
        let device = self.fixture().device.clone();
        self.run(Command::Device(device::Command::SetParam {
            device,
            param: param.into(),
            value,
        }))
    }

    /// Renders `blocks` blocks of [`QUANTUM`] frames.
    pub fn render_blocks(&mut self, blocks: u32) -> &mut Self {
        self.scenario.steps.push(Step::Render(blocks));
        self
    }

    /// Renders `seconds` of audio, rounded up to whole blocks.
    pub fn render_secs(&mut self, seconds: f32) -> &mut Self {
        let blocks = (seconds * SAMPLE_RATE / QUANTUM as f32).ceil() as u32;
        self.render_blocks(blocks)
    }

    /// The recorded scenario.
    pub fn scenario(&self) -> &Scenario {
        &self.scenario
    }

    pub fn into_scenario(self) -> Scenario {
        self.scenario
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_track() -> Command {
        serde_json::from_value(serde_json::json!({"type": "track.create", "kind": "midi"}))
            .expect("a track.create")
    }

    fn device_ids(state: &State) -> Vec<project::DeviceId> {
        state
            .project
            .tracks
            .values()
            .flat_map(|t| t.devices.keys().cloned())
            .collect()
    }

    #[test]
    fn an_id_reads_as_the_number_it_sorts_as() {
        let mut ids = project::IdGenerator::new(1_700_000_000_000, (1u128 << 80) - 2);
        let a = ids.next_id();
        let b = ids.next_id();
        let c = project::IdGenerator::new(1_700_000_000_001, 0).next_id();
        assert_eq!(
            id_value(a.as_str()),
            (1_700_000_000_000u128 << 80) | ((1 << 80) - 2)
        );
        assert!(a < b && b < c);
        assert!(id_value(a.as_str()) < id_value(b.as_str()));
        assert!(id_value(b.as_str()) < id_value(c.as_str()));
        assert_eq!(id_value("00000000000000000000000000"), 0);
    }

    /// A state built on the examples' context, given to a session on a fresh one: the
    /// session's ids come after the state's, so a new track and its synth take new ids
    /// instead of replacing the ones there.
    #[test]
    fn a_session_on_a_state_from_the_same_seed_makes_new_ids() {
        let mut built = Session::single_synth();
        built.run(new_track());
        let state = built.state().clone();
        let held = device_ids(&state);
        // The trap: a fresh context makes the ids of the state again.
        let mut fresh = testing::context().ids;
        let again: Vec<String> = (0..8).map(|_| fresh.next_id().to_string()).collect();
        assert!(
            held.iter().any(|d| again.contains(&d.to_string())),
            "the test needs a state whose ids the examples' seed makes"
        );
        let mut s = Session::from_state(state.clone(), testing::context());
        let commands::Output::Track(commands::track::Output::Create { track, device }) =
            s.exec(new_track())
        else {
            panic!("a track")
        };
        assert!(!state.project.tracks.contains_key(&track));
        assert!(!held.contains(&device));
        let after = device_ids(s.state());
        assert_eq!(after.len(), held.len() + 1, "{after:?}");
        assert!(held.iter().all(|d| after.contains(d)));
    }

    /// The same trap through `project.open`: a folder saved from a state built on the
    /// examples' context, opened in a session, then a track created: new ids again.
    #[test]
    fn a_project_opened_from_the_same_seed_makes_new_ids() {
        use project::file::{MemFs, SaveOptions, SaveState, save};
        let mut built = Session::single_synth();
        built.run(new_track());
        let state = built.state().clone();
        let held = device_ids(&state);
        let mut fs = MemFs::new();
        save(
            &mut fs,
            &state.project,
            &mut SaveState::new(),
            None,
            &SaveOptions::default(),
        )
        .expect("the folder saves");
        let mut s = Session::single_synth();
        s.stage(commands::contract::Folder::new("built", fs))
            .open("built");
        assert_eq!(device_ids(s.state()).len(), held.len());
        let commands::Output::Track(commands::track::Output::Create { device, .. }) =
            s.exec(new_track())
        else {
            panic!("a track")
        };
        assert!(!held.contains(&device));
        let after = device_ids(s.state());
        assert_eq!(after.len(), held.len() + 1, "{after:?}");
        assert!(held.iter().all(|d| after.contains(d)));
    }

    /// A state holding the last possible id (here a name that reads as one) leaves no id
    /// to make after it: the session refuses to start rather than wrap around.
    #[test]
    #[should_panic(expected = "the state holds the last id")]
    fn a_state_holding_the_last_id_is_refused() {
        let mut state = testing::fixture().state;
        state.project.name = "7ZZZZZZZZZZZZZZZZZZZZZZZZZ".into();
        let _ = Session::from_state(state, testing::context());
    }

    /// A state whose ids all come before the context's leaves the context as it is.
    #[test]
    fn a_session_on_an_older_state_keeps_its_context() {
        let ctx = testing::context();
        let s = Session::from_state(testing::fixture().state, ctx.clone());
        assert_eq!(s.ctx().ids, ctx.ids);
    }
}
