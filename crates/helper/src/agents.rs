//! Which agents are here (#641): the helper tells the page when an agent left (`agent.left`), so
//! that the page closes the history group that agent left open (`history.begin_group` without
//! `history.end_group`), which refuses undo and redo to everyone until it is closed.
//!
//! The page knows an agent by its name only (the history's author `agent:<name>`,
//! [`crate::mcp::agent_name`]), so presence is counted by name. A name is here while:
//!
//! - one of its MCP sessions is open (Streamable HTTP, and the stdio bridge, which is a client of
//!   it). A session ends when the client closes it (`DELETE`: the stdio bridge sends one when its
//!   input ends), or after [`AGENT_IDLE`] without a request (the MCP library's idle timeout, set
//!   to that value: a bridge killed without closing its input ends this way);
//! - or it made a stateless call (an MCP revision without sessions) less than [`AGENT_IDLE`] ago:
//!   such a client never says it left, so its silence does.
//!
//! The name leaves when the last of these ends. Two sessions of one name (two windows of the same
//! client) are one author in the page's history: the name leaves with the last one. Agents of
//! different names never affect each other.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long an agent may stay silent before it counts as gone: the idle timeout of an MCP
/// session, and the lease of a stateless call. The MCP library's own default for its sessions.
pub const AGENT_IDLE: Duration = Duration::from_secs(300);

/// What keeps one name here.
#[derive(Debug, Default)]
struct Seen {
    /// Its MCP sessions open now.
    sessions: usize,
    /// Until when its last stateless call keeps it here.
    lease: Option<Instant>,
}

impl Seen {
    fn here(&self, now: Instant) -> bool {
        self.sessions > 0 || self.lease.is_some_and(|until| until > now)
    }
}

/// The agents here, by name.
#[derive(Debug, Default)]
pub struct Presence {
    by_name: HashMap<String, Seen>,
}

impl Presence {
    /// An MCP session of `name` opened.
    pub fn session_opened(&mut self, name: &str) {
        self.by_name.entry(name.to_owned()).or_default().sessions += 1;
    }

    /// An MCP session of `name` ended: whether the name left with it (no other session of that
    /// name is open, and no stateless call's lease keeps it here at `now`).
    pub fn session_closed(&mut self, name: &str, now: Instant) -> bool {
        let Some(seen) = self.by_name.get_mut(name) else {
            return false;
        };
        seen.sessions = seen.sessions.saturating_sub(1);
        self.leave_if_gone(name, now)
    }

    /// `name` made a stateless call at `now`: here until `now + idle`, at least.
    pub fn called(&mut self, name: &str, now: Instant, idle: Duration) {
        let seen = self.by_name.entry(name.to_owned()).or_default();
        let until = now + idle;
        seen.lease = Some(seen.lease.map_or(until, |l| l.max(until)));
    }

    /// The names whose last stateless call ran out by `now`, with no session open: they left.
    pub fn expire(&mut self, now: Instant) -> Vec<String> {
        let mut left: Vec<String> = self
            .by_name
            .iter()
            .filter(|(_, seen)| !seen.here(now))
            .map(|(name, _)| name.clone())
            .collect();
        for name in &left {
            self.by_name.remove(name);
        }
        left.sort();
        left
    }

    /// How many MCP sessions of `name` are open.
    pub fn sessions(&self, name: &str) -> usize {
        self.by_name.get(name).map_or(0, |seen| seen.sessions)
    }

    /// Whether `name` is here at `now`.
    pub fn is_here(&self, name: &str, now: Instant) -> bool {
        self.by_name.get(name).is_some_and(|seen| seen.here(now))
    }

    fn leave_if_gone(&mut self, name: &str, now: Instant) -> bool {
        let gone = self.by_name.get(name).is_some_and(|seen| !seen.here(now));
        if gone {
            self.by_name.remove(name);
        }
        gone
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE: Duration = Duration::from_secs(300);

    #[test]
    fn a_name_leaves_with_its_last_session() {
        let now = Instant::now();
        let mut p = Presence::default();
        p.session_opened("claude");
        p.session_opened("claude");
        assert!(!p.session_closed("claude", now), "another session is open");
        assert!(p.is_here("claude", now));
        assert!(p.session_closed("claude", now));
        assert!(!p.is_here("claude", now));
        assert!(
            !p.session_closed("claude", now),
            "a name already gone leaves once"
        );
    }

    #[test]
    fn names_are_independent() {
        let now = Instant::now();
        let mut p = Presence::default();
        p.session_opened("a");
        p.session_opened("b");
        assert!(p.session_closed("b", now));
        assert!(p.is_here("a", now));
        assert!(!p.session_closed("unknown", now));
    }

    #[test]
    fn a_stateless_call_keeps_its_name_until_its_lease_runs_out() {
        let t0 = Instant::now();
        let mut p = Presence::default();
        p.called("s", t0, IDLE);
        assert!(p.expire(t0 + IDLE / 2).is_empty());
        // A later call moves the lease on; an earlier one never shortens it.
        p.called("s", t0 + IDLE / 2, IDLE);
        p.called("s", t0, IDLE);
        assert!(p.expire(t0 + IDLE).is_empty());
        assert_eq!(p.expire(t0 + IDLE / 2 + IDLE), ["s"]);
        assert!(p.expire(t0 + 3 * IDLE).is_empty(), "left once");
    }

    #[test]
    fn a_session_that_ends_inside_a_lease_leaves_when_the_lease_does() {
        let t0 = Instant::now();
        let mut p = Presence::default();
        p.called("x", t0, IDLE);
        p.session_opened("x");
        // The lease runs out while the session is open: still here.
        assert!(p.expire(t0 + 2 * IDLE).is_empty());
        assert!(p.session_closed("x", t0 + 2 * IDLE), "nothing keeps it");

        p.called("y", t0, IDLE);
        p.session_opened("y");
        assert!(!p.session_closed("y", t0 + IDLE / 2), "the lease runs on");
        assert_eq!(p.expire(t0 + IDLE), ["y"]);
    }
}
