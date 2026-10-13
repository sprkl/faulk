//! Security of the local bridge: which handshakes are accepted, and pairing.
//!
//! Any web page open in the browser can try to reach `127.0.0.1`. The helper accepts
//! a WebSocket handshake only when its `Host` names the helper (against DNS
//! rebinding) and its `Origin` is in the allow-list. The probe and pairing routes need
//! no token; the session route needs the pairing token.

use protocol::paths;
use std::time::{Duration, Instant};

/// Pairing codes stay valid this long.
pub const CODE_LIFETIME: Duration = Duration::from_secs(120);
/// Wrong codes accepted before the code is replaced.
pub const CODE_ATTEMPTS: u32 = 5;

/// The route a handshake was accepted for.
#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    Probe,
    Pair,
    Session,
}

/// Why a handshake was refused. The reason is logged, never sent to the client.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    BadHost,
    BadOrigin,
    UnknownPath,
    BadToken,
}

/// What the handshake check needs from the HTTP request.
pub struct Handshake<'a> {
    pub path: &'a str,
    pub query: Option<&'a str>,
    pub host: Option<&'a str>,
    pub origin: Option<&'a str>,
}

pub struct Policy {
    pub port: u16,
    pub allowed_origins: Vec<String>,
    pub token: String,
}

impl Policy {
    pub fn check(&self, h: &Handshake<'_>) -> Result<Route, Refusal> {
        let host_ok = h.host.is_some_and(|host| {
            host == format!("127.0.0.1:{}", self.port) || host == format!("localhost:{}", self.port)
        });
        if !host_ok {
            return Err(Refusal::BadHost);
        }
        let origin_ok = h
            .origin
            .is_some_and(|o| self.allowed_origins.iter().any(|a| a == o));
        if !origin_ok {
            return Err(Refusal::BadOrigin);
        }
        match h.path {
            paths::PROBE => Ok(Route::Probe),
            paths::PAIR => Ok(Route::Pair),
            paths::SESSION => {
                let token = h
                    .query
                    .unwrap_or_default()
                    .split('&')
                    .find_map(|kv| kv.strip_prefix("token="));
                match token {
                    Some(t) if constant_time_eq(t.as_bytes(), self.token.as_bytes()) => {
                        Ok(Route::Session)
                    }
                    _ => Err(Refusal::BadToken),
                }
            }
            _ => Err(Refusal::UnknownPath),
        }
    }
}

/// Compares without an early exit, so timing does not reveal the matching prefix.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Random bytes as lowercase hex.
pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("the OS random source is available");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// An 8-digit pairing code (about 26.6 bits, with 5 attempts per code).
fn random_code() -> String {
    let mut buf = [0u8; 8];
    getrandom::fill(&mut buf).expect("the OS random source is available");
    format!("{:08}", u64::from_le_bytes(buf) % 100_000_000)
}

/// The one-time pairing code currently shown to the user.
pub struct Pairing {
    code: String,
    issued: Instant,
    attempts_left: u32,
}

/// Result of a pairing attempt.
#[derive(Debug, PartialEq, Eq)]
pub enum PairResult {
    Accepted,
    /// Wrong or expired code; `new_code` is set when the code was replaced.
    Refused {
        attempts_left: u32,
        new_code: bool,
    },
}

impl Pairing {
    pub fn new(now: Instant) -> Self {
        Self {
            code: random_code(),
            issued: now,
            attempts_left: CODE_ATTEMPTS,
        }
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    /// Checks a code typed in the page. A correct code is used once: a new one replaces
    /// it. An expired code, or the last wrong attempt, also replaces it.
    pub fn attempt(&mut self, code: &str, now: Instant) -> PairResult {
        let expired = now.duration_since(self.issued) > CODE_LIFETIME;
        if !expired && constant_time_eq(code.trim().as_bytes(), self.code.as_bytes()) {
            *self = Self::new(now);
            return PairResult::Accepted;
        }
        self.attempts_left = self.attempts_left.saturating_sub(1);
        if expired || self.attempts_left == 0 {
            *self = Self::new(now);
            return PairResult::Refused {
                attempts_left: 0,
                new_code: true,
            };
        }
        PairResult::Refused {
            attempts_left: self.attempts_left,
            new_code: false,
        }
    }
}

/// Loads the session token from the helper's config directory, or creates it.
/// The token survives restarts so a paired page stays paired.
pub fn load_or_create_token(dir: &std::path::Path) -> std::io::Result<String> {
    let path = dir.join("session-token");
    if let Ok(token) = std::fs::read_to_string(&path) {
        let token = token.trim().to_owned();
        if token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(token);
        }
    }
    std::fs::create_dir_all(dir)?;
    let token = random_hex(16);
    std::fs::write(&path, &token)?;
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> Policy {
        Policy {
            port: 47800,
            allowed_origins: vec!["http://localhost:5173".into()],
            token: "0123456789abcdef0123456789abcdef".into(),
        }
    }

    fn hs<'a>(
        path: &'a str,
        query: Option<&'a str>,
        host: &'a str,
        origin: Option<&'a str>,
    ) -> Handshake<'a> {
        Handshake {
            path,
            query,
            host: Some(host),
            origin,
        }
    }

    const ORIGIN: Option<&str> = Some("http://localhost:5173");
    const HOST: &str = "127.0.0.1:47800";

    #[test]
    fn accepts_probe_pair_and_authenticated_session() {
        let p = policy();
        assert_eq!(p.check(&hs("/probe", None, HOST, ORIGIN)), Ok(Route::Probe));
        assert_eq!(
            p.check(&hs("/pair", None, "localhost:47800", ORIGIN)),
            Ok(Route::Pair)
        );
        let q = Some("token=0123456789abcdef0123456789abcdef");
        assert_eq!(
            p.check(&hs("/session", q, HOST, ORIGIN)),
            Ok(Route::Session)
        );
    }

    #[test]
    fn refuses_bad_host_origin_path_or_token() {
        let p = policy();
        assert_eq!(
            p.check(&hs("/probe", None, "evil.example:47800", ORIGIN)),
            Err(Refusal::BadHost)
        );
        assert_eq!(
            p.check(&hs("/probe", None, "127.0.0.1:80", ORIGIN)),
            Err(Refusal::BadHost)
        );
        assert_eq!(
            p.check(&Handshake {
                path: "/probe",
                query: None,
                host: None,
                origin: ORIGIN
            }),
            Err(Refusal::BadHost)
        );
        assert_eq!(
            p.check(&hs("/probe", None, HOST, None)),
            Err(Refusal::BadOrigin)
        );
        assert_eq!(
            p.check(&hs("/probe", None, HOST, Some("https://evil.example"))),
            Err(Refusal::BadOrigin)
        );
        assert_eq!(
            p.check(&hs("/probe", None, HOST, Some("null"))),
            Err(Refusal::BadOrigin)
        );
        assert_eq!(
            p.check(&hs("/admin", None, HOST, ORIGIN)),
            Err(Refusal::UnknownPath)
        );
        assert_eq!(
            p.check(&hs("/session", None, HOST, ORIGIN)),
            Err(Refusal::BadToken)
        );
        assert_eq!(
            p.check(&hs("/session", Some("token=wrong"), HOST, ORIGIN)),
            Err(Refusal::BadToken)
        );
        assert_eq!(
            p.check(&hs("/session", Some("token="), HOST, ORIGIN)),
            Err(Refusal::BadToken)
        );
    }

    #[test]
    fn pairing_code_is_one_time() {
        let now = Instant::now();
        let mut pairing = Pairing::new(now);
        let code = pairing.code().to_owned();
        assert_eq!(code.len(), 8);
        assert_eq!(pairing.attempt(&code, now), PairResult::Accepted);
        assert_ne!(pairing.code(), code, "a used code is replaced");
        assert!(matches!(
            pairing.attempt(&code, now),
            PairResult::Refused { .. }
        ));
    }

    #[test]
    fn wrong_codes_are_limited_then_the_code_changes() {
        let now = Instant::now();
        let mut pairing = Pairing::new(now);
        let code = pairing.code().to_owned();
        for left in (1..CODE_ATTEMPTS).rev() {
            assert_eq!(
                pairing.attempt("not-it", now),
                PairResult::Refused {
                    attempts_left: left,
                    new_code: false
                }
            );
        }
        assert_eq!(
            pairing.attempt("not-it", now),
            PairResult::Refused {
                attempts_left: 0,
                new_code: true
            }
        );
        assert!(
            matches!(pairing.attempt(&code, now), PairResult::Refused { .. }),
            "the old code is dead"
        );
    }

    #[test]
    fn expired_code_is_refused_and_replaced() {
        let start = Instant::now();
        let mut pairing = Pairing::new(start);
        let code = pairing.code().to_owned();
        let later = start + CODE_LIFETIME + Duration::from_secs(1);
        assert_eq!(
            pairing.attempt(&code, later),
            PairResult::Refused {
                attempts_left: 0,
                new_code: true
            }
        );
    }

    #[test]
    fn token_persists_and_corrupt_token_is_replaced() {
        let dir = std::env::temp_dir().join(format!("faulk-token-test-{}", random_hex(4)));
        let first = load_or_create_token(&dir).unwrap();
        assert_eq!(first.len(), 32);
        assert_eq!(load_or_create_token(&dir).unwrap(), first);
        std::fs::write(dir.join("session-token"), "short").unwrap();
        let replaced = load_or_create_token(&dir).unwrap();
        assert_ne!(replaced, first);
        assert_eq!(replaced.len(), 32);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
