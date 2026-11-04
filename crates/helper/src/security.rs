//! Security of the local bridge: which handshakes are accepted, and pairing.
//!
//! Any web page open in the browser can try to reach `127.0.0.1`. The helper accepts
//! a WebSocket handshake only when its `Host` names the helper (against DNS
//! rebinding) and its `Origin` is in the allow-list. The probe and pairing routes need
//! no token; the session route and the control link (#608) need the pairing token.
//!
//! The MCP server (`/mcp`, AG-1) takes plain HTTP requests from local agents: their `Host`
//! names the helper, they carry no `Origin` (no web page drives it) and they hold the MCP
//! token, which only the user creates (`faulk-helper mcp-token`): until then every MCP request
//! is refused.

use protocol::paths;
use std::time::{Duration, Instant};

/// Pairing codes stay valid this long. The limit of attempts per code, not the
/// lifetime, is what stops guessing; the lifetime only bounds a code left on screen.
pub const CODE_LIFETIME: Duration = Duration::from_secs(600);
/// Wrong codes accepted before the code is replaced.
pub const CODE_ATTEMPTS: u32 = 5;

/// The route a handshake was accepted for.
#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    Probe,
    Pair,
    Session,
    /// The page's control link (#608): the agents' relay only.
    Control,
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
    /// The MCP server's bearer token (AG-1), apart from the page's: the page never learns it,
    /// and an agent's configuration never holds the page's.
    pub mcp_token: McpToken,
}

/// Where the MCP token comes from. The user creates it (`faulk-helper mcp-token`, a
/// confirmation of theirs); the helper never does, and refuses every MCP request while the file
/// is missing. The file is read at each request: a token created while the helper runs is taken
/// at once, and deleting the file closes the MCP server at once (the user's revocation).
pub enum McpToken {
    /// A token given as is (tests).
    Fixed(String),
    /// The token file in the helper's config directory.
    File(std::path::PathBuf),
}

impl McpToken {
    pub fn file(dir: std::path::PathBuf) -> Self {
        Self::File(dir)
    }

    /// The token, or `None` while the user has not created it (or removed it).
    pub fn get(&self) -> Option<String> {
        match self {
            Self::Fixed(token) => Some(token.clone()),
            Self::File(dir) => read_secret(dir, MCP_TOKEN_FILE),
        }
    }
}

/// What the MCP check needs from an HTTP request to `/mcp`.
pub struct McpRequest<'a> {
    pub host: Option<&'a str>,
    /// Whether the request carries an `Origin` header, whatever its value.
    pub has_origin: bool,
    pub authorization: Option<&'a str>,
}

/// Why an MCP request was refused before the MCP server saw it. Logged, never sent.
#[derive(Debug, PartialEq, Eq)]
pub enum McpRefusal {
    /// `Host` does not name the helper (DNS rebinding): 403.
    BadHost,
    /// The request comes from a web page (it has an `Origin`): 403. MCP clients are not
    /// browsers, and no page, Faulk's included, drives the MCP server.
    Origin,
    /// No `Authorization: Bearer <token>`, or the wrong token: 401.
    BadToken,
    /// The user has not created the MCP token yet: 401.
    NoToken,
}

impl Policy {
    fn host_ok(&self, host: Option<&str>) -> bool {
        host.is_some_and(|host| {
            host == format!("127.0.0.1:{}", self.port) || host == format!("localhost:{}", self.port)
        })
    }

    /// Checks a request to the MCP server: its `Host`, no `Origin`, and the MCP token.
    pub fn check_mcp(&self, r: &McpRequest<'_>) -> Result<(), McpRefusal> {
        if !self.host_ok(r.host) {
            return Err(McpRefusal::BadHost);
        }
        if r.has_origin {
            return Err(McpRefusal::Origin);
        }
        let token = r
            .authorization
            .and_then(|a| a.strip_prefix("Bearer "))
            .map(str::trim);
        // An empty token would let `Bearer ` in: no token at all.
        let Some(expected) = self.mcp_token.get().filter(|t| !t.is_empty()) else {
            return Err(McpRefusal::NoToken);
        };
        match token {
            Some(t) if constant_time_eq(t.as_bytes(), expected.as_bytes()) => Ok(()),
            _ => Err(McpRefusal::BadToken),
        }
    }

    pub fn check(&self, h: &Handshake<'_>) -> Result<Route, Refusal> {
        if !self.host_ok(h.host) {
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
            paths::SESSION => self.with_token(h, Route::Session),
            paths::CONTROL => self.with_token(h, Route::Control),
            _ => Err(Refusal::UnknownPath),
        }
    }

    /// `route` when the handshake's `?token=` is the pairing token.
    fn with_token(&self, h: &Handshake<'_>, route: Route) -> Result<Route, Refusal> {
        let token = h
            .query
            .unwrap_or_default()
            .split('&')
            .find_map(|kv| kv.strip_prefix("token="));
        match token {
            Some(t) if constant_time_eq(t.as_bytes(), self.token.as_bytes()) => Ok(route),
            _ => Err(Refusal::BadToken),
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

    /// Replaces the code once it has expired, so the code on screen is always one that
    /// works. Returns true when it did.
    pub fn renew_if_expired(&mut self, now: Instant) -> bool {
        if now.duration_since(self.issued) > CODE_LIFETIME {
            *self = Self::new(now);
            true
        } else {
            false
        }
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
    load_or_create_secret(dir, SESSION_TOKEN_FILE)
}

/// The file of the page's session token in the helper's config directory.
pub const SESSION_TOKEN_FILE: &str = "session-token";
/// The file of the MCP server's token (AG-1) in the helper's config directory: what an agent's
/// configuration names, and what `faulk-helper mcp` (stdio) reads.
pub const MCP_TOKEN_FILE: &str = "mcp-token";

/// Loads the MCP server's token from the helper's config directory, or creates it: only on the
/// user's request (`faulk-helper mcp-token`). It survives restarts, so an agent configured once
/// stays configured.
pub fn load_or_create_mcp_token(dir: &std::path::Path) -> std::io::Result<String> {
    load_or_create_secret(dir, MCP_TOKEN_FILE)
}

/// Writes a secret readable by its owner only (on Unix; the user's own config folder on
/// Windows already is).
fn write_private(path: &std::path::Path, secret: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    std::io::Write::write_all(&mut options.open(path)?, secret.as_bytes())
}

/// Reads a token file without creating it: `None` when it is missing or not a token.
pub fn read_secret(dir: &std::path::Path, file: &str) -> Option<String> {
    let token = std::fs::read_to_string(dir.join(file)).ok()?;
    let token = token.trim().to_owned();
    (token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit())).then_some(token)
}

fn load_or_create_secret(dir: &std::path::Path, file: &str) -> std::io::Result<String> {
    if let Some(token) = read_secret(dir, file) {
        return Ok(token);
    }
    let path = dir.join(file);
    std::fs::create_dir_all(dir)?;
    let token = random_hex(16);
    write_private(&path, &token)?;
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
            mcp_token: McpToken::Fixed(MCP.into()),
        }
    }

    const MCP: &str = "fedcba9876543210fedcba9876543210";

    fn mcp<'a>(host: &'a str, has_origin: bool, authorization: Option<&'a str>) -> McpRequest<'a> {
        McpRequest {
            host: Some(host),
            has_origin,
            authorization,
        }
    }

    #[test]
    fn mcp_accepts_its_bearer_token_on_the_helpers_host_without_origin() {
        let p = policy();
        let bearer = format!("Bearer {MCP}");
        assert_eq!(p.check_mcp(&mcp(HOST, false, Some(&bearer))), Ok(()));
        assert_eq!(
            p.check_mcp(&mcp("localhost:47800", false, Some(&bearer))),
            Ok(())
        );
    }

    #[test]
    fn mcp_refuses_another_host_any_origin_and_any_other_token() {
        let p = policy();
        let bearer = format!("Bearer {MCP}");
        let ok = Some(bearer.as_str());
        for host in ["evil.example:47800", "127.0.0.1:80", "127.0.0.1", ""] {
            assert_eq!(
                p.check_mcp(&mcp(host, false, ok)),
                Err(McpRefusal::BadHost),
                "{host}"
            );
        }
        assert_eq!(
            p.check_mcp(&McpRequest {
                host: None,
                has_origin: false,
                authorization: ok
            }),
            Err(McpRefusal::BadHost)
        );
        // Any page, the allowed ones included: no web page drives the MCP server.
        assert_eq!(p.check_mcp(&mcp(HOST, true, ok)), Err(McpRefusal::Origin));
        let session = "Bearer 0123456789abcdef0123456789abcdef";
        for auth in [
            None,
            Some(""),
            Some("Bearer "),
            Some(MCP),
            Some("Basic fedcba9876543210fedcba9876543210"),
            Some("bearer x"),
            Some("Bearer fedcba9876543210fedcba987654321"),
            // The page's session token opens no MCP door.
            Some(session),
        ] {
            assert_eq!(
                p.check_mcp(&mcp(HOST, false, auth)),
                Err(McpRefusal::BadToken),
                "{auth:?}"
            );
        }
    }

    #[test]
    fn the_mcp_token_is_its_own_file() {
        let dir = std::env::temp_dir().join(format!("faulk-mcp-token-test-{}", random_hex(4)));
        assert_eq!(read_secret(&dir, MCP_TOKEN_FILE), None);
        let session = load_or_create_token(&dir).unwrap();
        let mcp = load_or_create_mcp_token(&dir).unwrap();
        assert_ne!(session, mcp);
        assert_eq!(read_secret(&dir, MCP_TOKEN_FILE), Some(mcp.clone()));
        assert_eq!(load_or_create_mcp_token(&dir).unwrap(), mcp);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn mcp_is_closed_until_the_user_creates_the_token_then_opens_with_it() {
        let dir = std::env::temp_dir().join(format!("faulk-mcp-file-test-{}", random_hex(4)));
        let p = Policy {
            mcp_token: McpToken::file(dir.clone()),
            ..policy()
        };
        let any = format!("Bearer {MCP}");
        assert_eq!(
            p.check_mcp(&mcp(HOST, false, Some(&any))),
            Err(McpRefusal::NoToken)
        );
        assert!(
            !dir.join(MCP_TOKEN_FILE).exists(),
            "the helper never creates it"
        );
        // The user creates it while the helper runs (`faulk-helper mcp-token`).
        let token = load_or_create_mcp_token(&dir).unwrap();
        assert_eq!(
            p.check_mcp(&mcp(HOST, false, Some(&any))),
            Err(McpRefusal::BadToken)
        );
        let bearer = format!("Bearer {token}");
        assert_eq!(p.check_mcp(&mcp(HOST, false, Some(&bearer))), Ok(()));
        // Deleting the file revokes it at once.
        std::fs::remove_file(dir.join(MCP_TOKEN_FILE)).unwrap();
        assert_eq!(
            p.check_mcp(&mcp(HOST, false, Some(&bearer))),
            Err(McpRefusal::NoToken)
        );
        let token = load_or_create_mcp_token(&dir).unwrap();
        let renewed = format!("Bearer {token}");
        assert_eq!(p.check_mcp(&mcp(HOST, false, Some(&renewed))), Ok(()));
        assert_eq!(
            p.check_mcp(&mcp(HOST, false, Some(&bearer))),
            Err(McpRefusal::BadToken),
            "a new token replaces the old one"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(MCP_TOKEN_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "readable by its owner only");
        }
        std::fs::remove_dir_all(&dir).unwrap();
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
    fn accepts_probe_pair_and_authenticated_session_and_control_link() {
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
        assert_eq!(
            p.check(&hs("/control", q, HOST, ORIGIN)),
            Ok(Route::Control)
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
        // The control link (#608) asks for the same token, Host and Origin as the session.
        let q = Some("token=0123456789abcdef0123456789abcdef");
        for (what, handshake, refusal) in [
            (
                "no token",
                hs("/control", None, HOST, ORIGIN),
                Refusal::BadToken,
            ),
            (
                "wrong token",
                hs("/control", Some("token=wrong"), HOST, ORIGIN),
                Refusal::BadToken,
            ),
            (
                "no origin",
                hs("/control", q, HOST, None),
                Refusal::BadOrigin,
            ),
            (
                "foreign origin",
                hs("/control", q, HOST, Some("https://evil.example")),
                Refusal::BadOrigin,
            ),
            (
                "rebinding",
                hs("/control", q, "evil.example:47800", ORIGIN),
                Refusal::BadHost,
            ),
        ] {
            assert_eq!(p.check(&handshake), Err(refusal), "{what}");
        }
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

    #[test]
    fn an_expired_code_is_renewed_and_a_fresh_one_kept() {
        let start = Instant::now();
        let mut pairing = Pairing::new(start);
        let first = pairing.code().to_owned();
        assert!(!pairing.renew_if_expired(start + CODE_LIFETIME));
        assert_eq!(pairing.code(), first);
        let later = start + CODE_LIFETIME + Duration::from_secs(1);
        assert!(pairing.renew_if_expired(later));
        let renewed = pairing.code().to_owned();
        assert!(
            !pairing.renew_if_expired(later),
            "the new code starts its own lifetime"
        );
        assert_eq!(pairing.attempt(&renewed, later), PairResult::Accepted);
    }
}
