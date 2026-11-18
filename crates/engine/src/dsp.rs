//! Building blocks of the devices' signal processing, shared between devices: an LFO that
//! runs free or locked to the transport ([`lfo`]), an envelope follower ([`envelope_follower`])
//! and the movement that combines them into a modulation in octaves with its eleven parameters
//! ([`movement`]). Plain state and arithmetic: no allocation, no lock, no panic on any input.
//! The Filter (#686) and Ripple (#688) use the movement; the tremolo, the chorus, the echo's
//! modulation and the generic modulators take them over later.

pub mod envelope_follower;
pub mod lfo;
pub mod movement;
