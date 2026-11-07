//! Building blocks of the devices' signal processing, shared between devices: an LFO that
//! runs free or locked to the transport ([`lfo`]) and an envelope follower ([`envelope_follower`]).
//! Plain state and arithmetic: no allocation, no lock, no panic on any input. The Filter
//! (#686) is their first user; the tremolo, the phaser, the chorus, the echo's modulation and
//! the generic modulators take them over later.

pub mod envelope_follower;
pub mod lfo;
