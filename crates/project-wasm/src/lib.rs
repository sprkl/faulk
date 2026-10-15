//! Faulk project and commands compiled to WebAssembly for the page's main thread.
//!
//! The page owns the project in both modes; in Boost mode the helper only receives engine
//! messages and project text files to write. Today this module only turns the page's
//! commands into the web engine's `ControlMsg` byte stream (MO-2,
//! `commands::engine_msgs`), so that no command is mapped to the engine in TypeScript.
//!
//! Plain C ABI, no `wasm-bindgen`. The page writes a command's JSON into the inbox
//! ([`pw_inbox`]), calls [`pw_command`], and reads the result's bytes at [`pw_out`].
//! One thread (the page's), so the buffers live in thread-locals.

use std::cell::RefCell;

use commands::engine_msgs::EngineStream;

thread_local! {
    static INBOX: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// The command JSON is not a command this build knows.
pub const ERR_BAD_COMMAND: i32 = -100;

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

/// A stream encoder for a fresh engine in `epoch` (`commands::engine_msgs::EngineStream`).
#[unsafe(no_mangle)]
pub extern "C" fn pw_stream_new(epoch: u32) -> *mut EngineStream {
    Box::into_raw(Box::new(EngineStream::new(epoch as u16)))
}

/// Frees a stream encoder.
///
/// # Safety
/// `stream` must be null or a pointer from [`pw_stream_new`], freed at most once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_stream_free(stream: *mut EngineStream) {
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

/// Encodes the setup of a fresh engine (`Reset`, the synth's track and the synth); returns
/// the length of the bytes at [`pw_out`], or a negative `MsgError` code.
///
/// # Safety
/// `stream` must be null or a live pointer from [`pw_stream_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_setup(stream: *mut EngineStream) -> i32 {
    // SAFETY: forwarded from this function's contract; one thread.
    match unsafe { stream.as_mut() } {
        Some(stream) => publish(stream.setup()),
        None => ERR_BAD_COMMAND,
    }
}

/// Encodes the command whose JSON is the first `len` bytes of the inbox; returns the
/// length of the bytes at [`pw_out`] (0: the engine does not take this command), or a
/// negative code ([`ERR_BAD_COMMAND`], a `MsgError` code).
///
/// # Safety
/// `stream` must be null or a live pointer from [`pw_stream_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pw_command(stream: *mut EngineStream, len: u32) -> i32 {
    // SAFETY: forwarded from this function's contract; one thread.
    let Some(stream) = (unsafe { stream.as_mut() }) else {
        return ERR_BAD_COMMAND;
    };
    let parsed = INBOX.with_borrow(|inbox| {
        let len = (len as usize).min(inbox.len());
        serde_json::from_slice::<commands::Command>(&inbox[..len])
    });
    match parsed {
        Ok(command) => publish(stream.command(&command)),
        Err(_) => ERR_BAD_COMMAND,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(stream: *mut EngineStream, json: &str) -> i32 {
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
        assert_eq!(command(stream, "not json"), ERR_BAD_COMMAND);
        // SAFETY: freed once.
        unsafe { pw_stream_free(stream) };
    }
}
