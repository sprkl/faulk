//! The session state on disk (`.faulk/session.json`): solo, arming, monitoring, the chosen
//! MIDI input. Machine-local and never versioned, so unlike the project files it is
//! lenient: absent or unreadable, the default session is used and opening goes on.

use super::canon;
use super::error::FileError;
use super::fs::{FileSystem, FsError};
use super::unit::SESSION_FILE;
use crate::model::Project;
use crate::session::SessionState;

/// What reading the session found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionLoad {
    /// There was no session file.
    Missing,
    /// The session was read.
    Loaded,
    /// The file was there and unreadable (another build's, a hand edit): the default
    /// session is used.
    Reset,
}

const MAX_SESSION_BYTES: usize = 1 << 20;

/// The session of the folder for `project`: what it names of tracks the project no longer
/// has is forgotten.
pub fn load_session(fs: &dyn FileSystem, project: &Project) -> (SessionState, SessionLoad) {
    let (mut session, how) = match fs.read(SESSION_FILE) {
        Err(FsError::NotFound) => (SessionState::default(), SessionLoad::Missing),
        Err(_) => (SessionState::default(), SessionLoad::Reset),
        Ok(bytes) if bytes.len() > MAX_SESSION_BYTES => {
            (SessionState::default(), SessionLoad::Reset)
        }
        Ok(bytes) => match serde_json::from_slice::<SessionState>(&bytes) {
            Ok(s) => (s, SessionLoad::Loaded),
            Err(_) => (SessionState::default(), SessionLoad::Reset),
        },
    };
    session.forget_missing_tracks(project);
    (session, how)
}

/// Writes the session when it differs from what is there; `true` when it wrote.
pub fn save_session(fs: &mut dyn FileSystem, session: &SessionState) -> Result<bool, FileError> {
    let bytes = canon::json(session).map_err(|message| FileError::Unwritable {
        path: SESSION_FILE.to_owned(),
        message,
    })?;
    if fs.read(SESSION_FILE).is_ok_and(|old| old == bytes) {
        return Ok(false);
    }
    fs.write(SESSION_FILE, &bytes)
        .map_err(|e| FileError::io(SESSION_FILE, e))?;
    Ok(true)
}
