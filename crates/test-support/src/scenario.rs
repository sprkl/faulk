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
    /// the project, then the commands.
    pub fn from_state(state: State, ctx: Context) -> Session {
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
    /// single-synth helpers (`note_on`, `set_param`) no longer apply afterwards.
    pub fn open(&mut self, name: &str) -> &mut Self {
        self.fixture = None;
        self.run(Command::Project(commands::project::Command::Open {
            folder: name.to_owned(),
        }))
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
