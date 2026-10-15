//! Faulk project and commands compiled to WebAssembly for the page's main thread.
//!
//! The page owns the project in both modes; in Boost mode the helper only receives engine
//! messages and project text files to write. Today this module runs the page's commands
//! on the one-synth project (`commands::engine_msgs::Synced`) and returns the web engine's
//! `ControlMsg` byte stream they produce (PR-3), so that no command is mapped to the engine
//! in TypeScript. PR-4 replaces this interim ABI.
//!
//! Plain C ABI, no `wasm-bindgen`. The page writes a command's JSON into the inbox
//! ([`pw_inbox`]), calls [`pw_command`], and reads the result's bytes at [`pw_out`].
//! One thread (the page's), so the buffers live in thread-locals.

use std::cell::RefCell;

use commands::engine_msgs::{SyncError, Synced};

thread_local! {
    static INBOX: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// The command JSON is not a command this build knows.
pub const ERR_BAD_COMMAND: i32 = -100;
/// The command was refused by the project (unknown track or device, invalid value):
/// nothing changed.
pub const ERR_REFUSED: i32 = -101;

/// The address of an inbox of at least `len` bytes (it may move between calls); null when
/// the memory is refused.
#[unsafe(no_mangle)]
pub extern "C" fn pw_inbox(len: u32) -> *mut u8 {
    INBOX.with_borrow_mut(|inbox| {
        let len = len as usize;
        if len > inbox.len() {
            if inbox.try_reserve_exact(len - inbox.len()).is_err() {
                return core::ptr::null_mut();
            }
            inbox.resize(len, 0);
        }
        inbox.as_mut_ptr()
    })
}

/// The address of the last result's bytes (valid until the next call).
#[unsafe(no_mangle)]
pub extern "C" fn pw_out() -> *const u8 {
    OUT.with_borrow(|out| out.as_ptr())
}

/// The one-synth project and the stream of a fresh engine in `epoch`
/// (`commands::engine_msgs::Synced`).
#[unsafe(no_mangle)]
pub extern "C" fn pw_stream_new(epoch: u32) -> *mut Synced {
    Box::into_raw(Box::new(Synced::single_synth(epoch as u16)))
}

/// Frees a stream encoder.
///
/// # Safety
/// `stream` must be null or a pointer from [`pw_stream_new`], freed at most once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_stream_free(stream: *mut Synced) {
    if !stream.is_null() {
        // SAFETY: the caller guarantees a pointer from `pw_stream_new`, freed once.
        drop(unsafe { Box::from_raw(stream) });
    }
}

fn publish(result: Result<Vec<u8>, engine::msg::MsgError>) -> i32 {
    match result {
        Ok(bytes) => {
            let len = bytes.len() as i32;
            OUT.with_borrow_mut(|out| *out = bytes);
            len
        }
        Err(error) => error.code(),
    }
}

/// Encodes the setup of a fresh engine (`Reset` and the snapshot of the project); returns
/// the length of the bytes at [`pw_out`], or a negative `MsgError` code.
///
/// # Safety
/// `stream` must be null or a live pointer from [`pw_stream_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_setup(stream: *mut Synced) -> i32 {
    // SAFETY: forwarded from this function's contract; one thread.
    match unsafe { stream.as_mut() } {
        Some(stream) => publish(stream.setup()),
        None => ERR_BAD_COMMAND,
    }
}

/// Runs the command whose JSON is the first `len` bytes of the inbox on the project and
/// encodes its engine messages; returns the length of the bytes at [`pw_out`] (0: nothing
/// for the engine), or a negative code ([`ERR_BAD_COMMAND`], [`ERR_REFUSED`], a `MsgError`
/// code).
///
/// # Safety
/// `stream` must be null or a live pointer from [`pw_stream_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_command(stream: *mut Synced, len: u32) -> i32 {
    // SAFETY: forwarded from this function's contract; one thread.
    let Some(stream) = (unsafe { stream.as_mut() }) else {
        return ERR_BAD_COMMAND;
    };
    let parsed = INBOX.with_borrow(|inbox| {
        let len = (len as usize).min(inbox.len());
        serde_json::from_slice::<commands::Command>(&inbox[..len])
    });
    match parsed {
        Ok(command) => match stream.run(&command) {
            Ok((_, bytes)) => publish(Ok(bytes)),
            Err(SyncError::Refused(_)) => ERR_REFUSED,
            Err(SyncError::Stream(error)) => publish(Err(error)),
        },
        Err(_) => ERR_BAD_COMMAND,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(stream: *mut Synced, json: &str) -> i32 {
        let ptr = pw_inbox(json.len() as u32);
        // SAFETY: the inbox holds at least `json.len()` bytes.
        unsafe { core::ptr::copy_nonoverlapping(json.as_ptr(), ptr, json.len()) };
        // SAFETY: a live stream.
        unsafe { pw_command(stream, json.len() as u32) }
    }

    #[test]
    fn commands_become_the_engine_stream() {
        let stream = pw_stream_new(1);
        // SAFETY: a live stream.
        let setup = unsafe { pw_setup(stream) };
        assert!(setup > 0);
        let note = command(
            stream,
            r#"{"type":"live.note_on","track":"00000000000000000000000001","note":60,"velocity":100}"#,
        );
        assert!(note > 0);
        let midi = command(stream, r#"{"type":"midi.list_inputs"}"#);
        assert_eq!(midi, 0, "not an engine command");
        assert_eq!(command(stream, r#"{"type":"nope"}"#), ERR_BAD_COMMAND);
        let unknown_track = command(
            stream,
            r#"{"type":"live.note_on","track":"00000000000000000000000009","note":60,"velocity":100}"#,
        );
        assert_eq!(unknown_track, ERR_REFUSED);
        let play = command(stream, r#"{"type":"transport.play"}"#);
        assert!(play > 0, "transport commands reach the engine");
        assert_eq!(command(stream, "not json"), ERR_BAD_COMMAND);
        // SAFETY: freed once.
        unsafe { pw_stream_free(stream) };
    }
}
