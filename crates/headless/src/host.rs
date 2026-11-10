//! The headless host: one project folder, owned by this process, and the page's project core
//! (`project_wasm::Core`) running every command, as the page runs them.
//!
//! The host stages what the core needs, as the page does: the folder for the opening (as
//! `project.open` of the staged folder, run once here), the measures of an offline render for
//! `project.analyze_render`. It writes what the core computes: the save frame into the folder,
//! the export's file into `exports/`. Who runs a command is the core's author
//! (`agent:<name>`): the history stamps it and groups an agent's consecutive edits into one
//! turn (`commands::history`).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use commands::host::RunReport;
use commands::levels::{AnalysisStage, MeterReading};
use project::file::{FileSystem, StdFs};
use project_wasm::{Core, KIND_EMPTY, Mode};
use serde_json::{Map, Value, json};

use crate::folder::{self, Media, STAGED_NAME};
use crate::policy::{self, Route};
use crate::render::{self, RenderError, Rendered};

/// Why the host could not open, create or save its folder.
#[derive(Debug)]
pub enum HostError {
    /// The folder could not be read or written, or is not where a project can be made.
    Folder(String),
    /// The project core refused the folder (`project.open`'s error, as JSON).
    Refused(Value),
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostError::Folder(message) => f.write_str(message),
            HostError::Refused(error) => write!(f, "the project was refused: {error}"),
        }
    }
}

/// What a command did.
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    /// It applied: the command's output, and what the host wrote for it (an export: the file,
    /// its peak, what was silenced, the media missing from the folder).
    Applied {
        output: Value,
        export: Option<Exported>,
    },
    /// The project refused it (the command's error): nothing changed.
    Refused { error: Value },
    /// It is not a command this build knows, or its author is not valid: nothing changed.
    Invalid { message: String },
    /// The headless host does not run it ([`policy::UNSUPPORTED`]): nothing changed.
    Unsupported { message: String },
    /// The host failed to do its part (render, write the export): nothing changed in the project.
    Failed { message: String },
    /// The command applied in the project, but the folder could not be written: the project
    /// holds the change and the next save writes it again. Not to be retried.
    Unsaved { output: Value, message: String },
}

/// The error code of [`Answer::Unsaved`].
pub const UNSAVED: &str = "host.unsaved";

impl Answer {
    /// The answer as one JSON object with its `status`.
    pub fn to_json(&self) -> Value {
        match self {
            Answer::Applied { output, export } => {
                let mut body = json!({ "status": "applied", "output": output });
                if let Some(export) = export {
                    body["export"] = export.to_json();
                }
                body
            }
            Answer::Refused { error } => json!({ "status": "refused", "error": error }),
            Answer::Invalid { message } => json!({ "status": "invalid", "message": message }),
            Answer::Unsupported { message } => {
                json!({ "status": "unsupported", "code": policy::UNSUPPORTED, "message": message })
            }
            Answer::Failed { message } => json!({ "status": "failed", "message": message }),
            Answer::Unsaved { output, message } => json!({
                "status": "unsaved",
                "code": UNSAVED,
                "output": output,
                "message": message
            }),
        }
    }

    pub fn applied(&self) -> bool {
        matches!(self, Answer::Applied { .. })
    }
}

/// What an export wrote.
#[derive(Clone, Debug, PartialEq)]
pub struct Exported {
    /// The file: relative to the folder for one in `exports/`, else as given.
    pub file: String,
    pub duration_secs: f64,
    /// The output's peak, in dBFS (`None`: silence).
    pub peak_db: Option<f64>,
    pub silenced_frames: u64,
    /// The media (hex) whose clips are silent in the file: their audio is not in the folder.
    pub missing: Vec<String>,
}

impl Exported {
    pub fn to_json(&self) -> Value {
        json!({
            "file": self.file,
            "duration_secs": self.duration_secs,
            "peak_db": self.peak_db,
            "silenced_frames": self.silenced_frames,
            "missing_media": self.missing,
        })
    }
}

/// Why an export wrote nothing.
#[derive(Debug)]
pub enum ExportFailure {
    /// The project cannot be exported as asked (the command's error, as JSON).
    Refused(Value),
    Failed(String),
}

/// The headless host of one project folder.
pub struct Host {
    core: Core,
    fs: StdFs,
    root: PathBuf,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// The randomness the ids are made of, as the page seeds its core (80 bits are kept).
fn seed() -> Result<u128, HostError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| HostError::Folder(format!("no random source: {e}")))?;
    Ok(u128::from_le_bytes(bytes))
}

fn new_core() -> Result<Core, HostError> {
    Core::new(KIND_EMPTY, 0, now_ms(), seed()?)
        .ok_or_else(|| HostError::Folder("the project core refused to start".into()))
}

impl Host {
    /// Makes a new, empty project in `root` (made if absent; refused when it holds anything)
    /// and writes it whole.
    pub fn create(root: &Path, name: Option<&str>) -> Result<Host, HostError> {
        let folder = |e: std::io::Error| HostError::Folder(format!("{}: {e}", root.display()));
        if root.exists() {
            if std::fs::read_dir(root).map_err(folder)?.next().is_some() {
                return Err(HostError::Folder(format!(
                    "{}: the folder is not empty: a project is made in an empty folder",
                    root.display()
                )));
            }
        } else {
            std::fs::create_dir_all(root).map_err(folder)?;
        }
        let mut core = new_core()?;
        if let Some(name) = name
            && !core.name_new(name)
        {
            return Err(HostError::Folder(format!("{name:?} is not a project name")));
        }
        let mut host = Host {
            core,
            fs: StdFs::new(root),
            root: root.to_path_buf(),
        };
        host.save_mode(Mode::Everything)?;
        Ok(host)
    }

    /// Opens the project of `root`, as the page opens a folder: the core reads it (migrating
    /// an older format in memory, written at the first save) and restores its session.
    pub fn open(root: &Path) -> Result<Host, HostError> {
        let frame = folder::folder_frame(root).map_err(HostError::Folder)?;
        let mut core = new_core()?;
        core.stage(&frame)
            .map_err(|_| HostError::Folder("the folder could not be staged".into()))?;
        core.set_clock(now_ms());
        let open = json!({ "type": "project.open", "folder": STAGED_NAME });
        let (report, _) = core.run(open.to_string().as_bytes());
        match report {
            RunReport::Applied { .. } => {}
            RunReport::Refused { error } => {
                return Err(HostError::Refused(
                    serde_json::to_value(error).unwrap_or(Value::Null),
                ));
            }
            RunReport::Invalid { message } => return Err(HostError::Folder(message)),
        }
        Ok(Host {
            core,
            fs: StdFs::new(root),
            root: root.to_path_buf(),
        })
    }

    /// The folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The project, its session and the engine slots, as the page sees them.
    pub fn view(&self) -> commands::host::ProjectView {
        self.core.view()
    }

    /// The undo history, as the page's history panel shows it (authors included).
    pub fn history(&self) -> commands::host::HistoryView {
        self.core.history()
    }

    /// Whether the project has changes the folder does not hold.
    pub fn dirty(&self) -> bool {
        self.core.dirty()
    }

    /// Runs `command` (the JSON of a `Command`, with its `type`) as `author` (`agent:<name>`).
    pub fn call(&mut self, author: &str, command: &Value) -> Answer {
        let Some(name) = command.get("type").and_then(Value::as_str) else {
            return Answer::Invalid {
                message: "a command is a JSON object with its `type`".into(),
            };
        };
        let Some(route) = policy::route_of(name) else {
            return Answer::Invalid {
                message: format!("no command is named {name:?}"),
            };
        };
        if let Route::Refused(message) = route {
            return Answer::Unsupported { message };
        }
        if self.core.set_author(author.as_bytes()).is_err() {
            return Answer::Invalid {
                message: format!("{author:?} is not an author (`agent:<name>`)"),
            };
        }
        self.core.set_clock(now_ms());
        let bytes = command.to_string().into_bytes();
        match route {
            Route::Core => self.run(&bytes),
            Route::Save => match self.run(&bytes) {
                Answer::Applied { output, .. } => match self.save() {
                    Ok(_) => Answer::Applied {
                        output,
                        export: None,
                    },
                    Err(e) => Answer::Failed {
                        message: e.to_string(),
                    },
                },
                other => other,
            },
            Route::Export => match self.run(&bytes) {
                Answer::Applied { output, .. } => match self.export(&fields(command, None), None) {
                    Ok(exported) => Answer::Applied {
                        output,
                        export: Some(exported),
                    },
                    Err(ExportFailure::Refused(error)) => Answer::Refused { error },
                    Err(ExportFailure::Failed(message)) => Answer::Failed { message },
                },
                other => other,
            },
            Route::Analyze => {
                let mut request = fields(command, Some(&["from_bar", "to_bar"]));
                request.insert("analysis".into(), Value::Bool(true));
                match self.analyze(&request) {
                    Ok(()) => self.run(&bytes),
                    Err(answer) => answer,
                }
            }
            Route::Refused(_) => unreachable!("answered above"),
        }
    }

    fn run(&mut self, bytes: &[u8]) -> Answer {
        let (report, _engine) = self.core.run(bytes);
        match report {
            RunReport::Applied { output, .. } => Answer::Applied {
                output: serde_json::to_value(output).unwrap_or(Value::Null),
                export: None,
            },
            RunReport::Refused { error } => Answer::Refused {
                error: serde_json::to_value(error).unwrap_or(Value::Null),
            },
            RunReport::Invalid { message } => Answer::Invalid { message },
        }
    }

    /// Renders the analysis `request` and stages its measures for `project.analyze_render`.
    fn analyze(&mut self, request: &Map<String, Value>) -> Result<(), Answer> {
        let rendered = self.render(request).map_err(|e| match e {
            ExportFailure::Refused(error) => Answer::Refused { error },
            ExportFailure::Failed(message) => Answer::Failed { message },
        })?;
        let Some(measure) = rendered.measure else {
            return Err(Answer::Failed {
                message: "the render measured nothing".into(),
            });
        };
        let reading = |l: &engine::host::Level| MeterReading {
            peak_l: l.peak_l,
            peak_r: l.peak_r,
            rms_l: l.rms_l,
            rms_r: l.rms_r,
        };
        let stage = AnalysisStage {
            start_tick: rendered.plan.start_tick,
            end_tick: rendered.plan.end_tick,
            slots: rendered.tracks,
            tracks: measure.tracks.iter().map(reading).collect(),
            master: reading(&measure.master),
            silenced_frames: measure.silenced,
            reduction: f64::from(measure.reduction),
        };
        let json = serde_json::to_vec(&Some(stage)).unwrap_or_default();
        self.core.stage_analysis(&json).map_err(|_| Answer::Failed {
            message: "the measures could not be staged".into(),
        })
    }

    fn render(&self, request: &Map<String, Value>) -> Result<Rendered, ExportFailure> {
        let media = Media::new(&self.root).map_err(ExportFailure::Failed)?;
        let request = serde_json::to_vec(request).unwrap_or_default();
        render::render(&self.core, &request, &media).map_err(|e| match e {
            RenderError::Refused(error) => {
                ExportFailure::Refused(serde_json::to_value(error).unwrap_or(Value::Null))
            }
            RenderError::Failed(message) => ExportFailure::Failed(message),
        })
    }

    /// Renders the export `request` (the fields of `project.export_audio`) and writes the WAV
    /// file to `out`, or to `exports/<project name>.wav` in the folder (an existing file is
    /// kept: the new one takes a numbered name).
    pub fn export(
        &mut self,
        request: &Map<String, Value>,
        out: Option<&Path>,
    ) -> Result<Exported, ExportFailure> {
        let rendered = self.render(request)?;
        let file = match out {
            Some(path) => {
                // Whole or not at all: beside the target, then renamed over it, so that a failed
                // write never leaves a truncated file where an earlier one was.
                let mut part = path.as_os_str().to_owned();
                part.push(".part");
                let part = PathBuf::from(part);
                let failed = |e: std::io::Error| {
                    let _ = std::fs::remove_file(&part);
                    ExportFailure::Failed(format!("{}: {e}", path.display()))
                };
                std::fs::write(&part, &rendered.wav).map_err(failed)?;
                std::fs::rename(&part, path).map_err(failed)?;
                path.display().to_string()
            }
            None => {
                let path = self.export_path();
                self.fs
                    .write(&path, &rendered.wav)
                    .map_err(|e| ExportFailure::Failed(format!("{path}: {e:?}")))?;
                path
            }
        };
        let peak = f64::from(rendered.peak);
        Ok(Exported {
            file,
            duration_secs: rendered.plan.duration_secs,
            peak_db: (peak > 1e-6).then(|| 20.0 * peak.log10()),
            silenced_frames: rendered.silenced,
            missing: rendered.missing,
        })
    }

    /// `exports/<name>.wav` for the project's name, kept to what every file system takes; a
    /// name already taken gets ` 2`, ` 3`... as the helper's export does.
    fn export_path(&self) -> String {
        let name = &self.core.view().project.name;
        let mut stem: String = name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') {
                    c
                } else {
                    '_'
                }
            })
            .take(100)
            .collect();
        stem = stem.trim().to_owned();
        if stem.is_empty() {
            stem = "export".to_owned();
        }
        let mut n = 1;
        loop {
            let path = if n == 1 {
                format!("exports/{stem}.wav")
            } else {
                format!("exports/{stem} {n}.wav")
            };
            if !self.root.join(&path).exists() {
                return path;
            }
            n += 1;
        }
    }

    /// Writes into the folder what changed since the last save. Whether anything was written.
    pub fn save(&mut self) -> Result<bool, HostError> {
        self.save_mode(Mode::Changed)
    }

    fn save_mode(&mut self, mode: Mode) -> Result<bool, HostError> {
        let frame = self.core.save(mode);
        let entries = match folder::parse_save_frame(&frame) {
            Ok(entries) => entries,
            Err(message) => {
                self.core.saved(false);
                return Err(HostError::Folder(format!("the save failed: {message}")));
            }
        };
        match folder::apply(&mut self.fs, &entries) {
            Ok(()) => {
                self.core.saved(true);
                Ok(!entries.is_empty())
            }
            Err(message) => {
                self.core.saved(false);
                Err(HostError::Folder(format!("the save failed: {message}")))
            }
        }
    }

    /// The agent `author` left (its MCP session ended): its open history group is closed and
    /// its turn ends, as the page does when the helper says so.
    pub fn agent_left(&mut self, author: &str) {
        let _ = self.core.agent_left(author.as_bytes());
    }
}

/// The fields of `command` without its `type`, or only those of `keep`.
fn fields(command: &Value, keep: Option<&[&str]>) -> Map<String, Value> {
    command
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| *key != "type" && keep.is_none_or(|keep| keep.contains(&key.as_str())))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
