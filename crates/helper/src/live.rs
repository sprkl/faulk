//! The notes of the helper's MIDI input, played on its engine at once (#598).
//!
//! The page owns the project and decides where the live input goes (`commands::live`): it
//! sends the helper its decision as a table, `live.routing { epoch, tracks }` (the engine slots
//! of the selected and armed MIDI tracks, in the engine epoch those slots belong to), after
//! each snapshot and whenever the table changes. With it, a key of the helper's input reaches
//! the engine on midir's thread, without waiting for the page's main thread: [`LiveInput`]
//! encodes a `NoteOn` per track in that epoch and remembers where the note went, so that its
//! note off reaches the same tracks whatever the table says by then. Every message still goes
//! to the page (`midi.message`), with whether the helper played it: the page runs the notes in
//! its project (keys shown, recording, its held notes) and sends engine bytes only for a note
//! the helper did not play (no table yet, a table of another epoch), so a note is never doubled
//! and never lost. Control changes play through the page (they may become automation, an edit
//! of the project).
//!
//! A table, or a held note, belongs to one engine (its generation, `EngineSlot`) and one epoch:
//! once the engine is another, or has taken a `Reset` into a newer epoch, it plays nothing (the
//! `Reset` already ended the tracks its notes were on).
//!
//! Not the audio thread: midir's thread and the server's. Lock order: the session lock, then
//! this one, then the engine's; nothing takes this lock while holding the engine's.

use crate::audio::EngineSlot;
use engine::msg::{ControlMsg, TrackSlot, limits::MAX_TRACKS};
use std::sync::{Arc, Mutex};

/// What a MIDI message means for the live input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Note {
    On {
        note: u8,
        velocity: u8,
    },
    Off {
        note: u8,
    },
    /// Control change 120 (all sound off) or 123 (all notes off).
    AllOff,
    /// Anything else (other control changes, pitch bend...): not a note.
    Other,
}

/// Decodes a message as the page's `decodeMidi` does: channels merged, a note on at velocity
/// 0 is a note off, a message cut short is nothing.
pub fn decode(message: &[u8]) -> Note {
    let byte = |i: usize| message.get(i).map(|b| b & 0x7F);
    match (message.first().map(|s| s & 0xF0), byte(1), byte(2)) {
        (Some(0x90), Some(note), Some(0)) | (Some(0x80), Some(note), _) => Note::Off { note },
        (Some(0x90), Some(note), Some(velocity)) => Note::On { note, velocity },
        (Some(0xB0), Some(120 | 123), _) => Note::AllOff,
        _ => Note::Other,
    }
}

/// The tracks of one engine and epoch.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Routing {
    generation: u64,
    epoch: u16,
    tracks: Vec<u32>,
}

#[derive(Default)]
struct State {
    routing: Option<Routing>,
    /// Where each held note went, by note number.
    held: Vec<Option<Routing>>,
}

/// The helper's live input (module doc).
pub struct LiveInput {
    engine: Arc<EngineSlot>,
    state: Mutex<State>,
}

impl LiveInput {
    pub fn new(engine: Arc<EngineSlot>) -> Self {
        LiveInput {
            engine,
            state: Mutex::new(State {
                routing: None,
                held: vec![None; 128],
            }),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        // A note off must never be skipped: a lock poisoned by a panic is taken anyway.
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Takes the page's table for the engine of `generation`. Ignored (the previous table
    /// kept) when that engine is not the open one, when `epoch` is not its epoch, or when
    /// `tracks` holds more tracks than an engine can. Returns whether it was taken.
    pub fn set_routing(&self, generation: u64, epoch: u16, tracks: Vec<u32>) -> bool {
        if tracks.len() > MAX_TRACKS as usize || self.engine.current() != Some((generation, epoch))
        {
            return false;
        }
        self.state().routing = Some(Routing {
            generation,
            epoch,
            tracks,
        });
        true
    }

    /// Plays a message of the input, when it is a note the helper can play: a note on on the
    /// table's tracks, a note off on the tracks its note went to. Returns whether it played it
    /// (the page then sends nothing for it). Control changes 120 and 123 release the notes the
    /// helper holds (the page, which plays control changes, releases every track too) and
    /// count as not played.
    pub fn receive(&self, message: &[u8]) -> bool {
        match decode(message) {
            Note::On { note, velocity } => self.note_on(note, velocity),
            Note::Off { note } => self.note_off(note),
            Note::AllOff => {
                self.release();
                false
            }
            Note::Other => false,
        }
    }

    fn note_on(&self, note: u8, velocity: u8) -> bool {
        let mut state = self.state();
        let Some(routing) = state.routing.clone() else {
            return false;
        };
        let msgs: Vec<ControlMsg> = routing
            .tracks
            .iter()
            .map(|&track| ControlMsg::NoteOn {
                track: TrackSlot(track),
                pitch: note,
                velocity,
                at: None,
            })
            .collect();
        if !self.play(&routing, &msgs) {
            return false;
        }
        // Held where it went: a key struck again while held adds the tracks it reached now.
        let held = &mut state.held[usize::from(note)];
        match held {
            Some(h) if h.generation == routing.generation && h.epoch == routing.epoch => {
                for track in routing.tracks {
                    if !h.tracks.contains(&track) {
                        h.tracks.push(track);
                    }
                }
            }
            _ => *held = Some(routing),
        }
        true
    }

    fn note_off(&self, note: u8) -> bool {
        let Some(held) = self.state().held[usize::from(note)].take() else {
            return false;
        };
        let msgs: Vec<ControlMsg> = held
            .tracks
            .iter()
            .map(|&track| ControlMsg::NoteOff {
                track: TrackSlot(track),
                pitch: note,
                at: None,
            })
            .collect();
        self.play(&held, &msgs)
    }

    /// Releases every note the helper holds, on the tracks each went to (the input went away,
    /// or it sent all notes off), and forgets them.
    pub fn release(&self) {
        let held: Vec<(u8, Routing)> = {
            let mut state = self.state();
            state
                .held
                .iter_mut()
                .enumerate()
                .filter_map(|(note, h)| Some((note as u8, h.take()?)))
                .collect()
        };
        for (note, routing) in held {
            let msgs: Vec<ControlMsg> = routing
                .tracks
                .iter()
                .map(|&track| ControlMsg::NoteOff {
                    track: TrackSlot(track),
                    pitch: note,
                    at: None,
                })
                .collect();
            self.play(&routing, &msgs);
        }
    }

    /// Forgets the table and the held notes without a message: the session ended or was
    /// replaced, and the server released every track already (`EngineSlot::release_all`).
    pub fn clear(&self) {
        let mut state = self.state();
        state.routing = None;
        state.held.iter_mut().for_each(|h| *h = None);
    }

    /// Sends `msgs`, encoded in `routing`'s epoch, to the engine of its generation, if that
    /// engine is still the open one and still in that epoch. Whether it was sent.
    fn play(&self, routing: &Routing, msgs: &[ControlMsg]) -> bool {
        let Ok(bytes) = engine::msg::encode_stream(routing.epoch, 0, msgs) else {
            return false;
        };
        self.engine
            .with_generation(routing.generation, |sender| {
                if sender.epoch() != routing.epoch {
                    return false;
                }
                sender.submit_live(&bytes);
                true
            })
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests;
