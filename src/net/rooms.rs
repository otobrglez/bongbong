//! Where the rooms are (docs/online-coop-prd.md §4.10, §4.13): the host
//! a client dials and the URL it derives from a room code. A link is
//! enough to reach a seat, so no directory and no HTTP lookup stands
//! between `bongbong.io/j/AK7QX` and the socket - the code's first
//! letter names the pod that owns the room, and the pod's path falls out
//! of it.

use std::fmt;

/// The cluster's rooms host, the default of `--rooms` and
/// `BONGBONG_ROOMS`.
pub const DEFAULT_ROOMS_HOST: &str = "wss://rooms.bongbong.io";

/// The environment variable that overrides the host, so a local server
/// needs no code change: `BONGBONG_ROOMS=ws://127.0.0.1:4848`.
pub const ROOMS_ENV: &str = "BONGBONG_ROOMS";

/// Letters after the pod's, `bongbong_server::code::ROOM_LETTERS`. The
/// client checks the shape of a code and reads its pod letter; which
/// room letters exist is the room server's business.
pub const ROOM_LETTERS: usize = 4;

/// Which rooms server to talk to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomsHost {
    base: String,
    overridden: bool,
}

impl RoomsHost {
    /// The rooms host in force: `arg` (`--rooms`) if given, else
    /// `BONGBONG_ROOMS`, else the cluster's. Anything but the last is an
    /// override and changes the paths `socket_url` builds.
    pub fn resolve(arg: Option<&str>) -> RoomsHost {
        let from_env = std::env::var(ROOMS_ENV).ok();
        match arg.map(str::to_string).or(from_env) {
            Some(base) if !base.trim().is_empty() => RoomsHost::overriding(&base),
            _ => RoomsHost { base: DEFAULT_ROOMS_HOST.into(), overridden: false },
        }
    }

    /// One server, addressed directly: a local `cargo run -p
    /// bongbong-server`, a staging pod, the rig's own.
    pub fn overriding(base: &str) -> RoomsHost {
        RoomsHost { base: base.trim().trim_end_matches('/').to_string(), overridden: true }
    }

    /// The cluster's.
    pub fn cluster() -> RoomsHost {
        RoomsHost { base: DEFAULT_ROOMS_HOST.into(), overridden: false }
    }

    /// The scheme and authority, with no trailing slash.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Whether this host came from `--rooms` or `BONGBONG_ROOMS`.
    pub fn is_override(&self) -> bool {
        self.overridden
    }
}

/// A room code as a client reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomCode {
    /// Canonical text: trimmed and upper case, what travels in
    /// `Lobby::Join` and what a player is shown.
    pub text: String,
    /// The first letter, the pod that owns the room.
    pub pod: char,
}

impl RoomCode {
    /// `raw` as a code, trimmed and upper-cased.
    ///
    /// The shape is checked here - one pod letter (a capital or a digit,
    /// the operator's `--pod`) and `ROOM_LETTERS` more alphanumerics -
    /// so a typo is caught before a socket is opened. Whether those
    /// letters spell a room that exists is the pod's answer, and so is
    /// whether they come from its alphabet: a client that held its own
    /// copy of that alphabet would only drift from the server's.
    pub fn parse(raw: &str) -> Result<RoomCode, CodeError> {
        let text: String = raw.trim().to_ascii_uppercase();
        let mut chars = text.chars();
        let pod = chars.next().ok_or(CodeError::Length(text.chars().count()))?;
        let room: Vec<char> = chars.collect();
        if room.len() != ROOM_LETTERS {
            return Err(CodeError::Length(text.chars().count()));
        }
        if !pod.is_ascii_uppercase() && !pod.is_ascii_digit() {
            return Err(CodeError::Character(pod));
        }
        if let Some(&bad) = room.iter().find(|c| !c.is_ascii_alphanumeric()) {
            return Err(CodeError::Character(bad));
        }
        Ok(RoomCode { text, pod })
    }
}

impl fmt::Display for RoomCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// Why a code was refused before it ever reached a room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeError {
    /// Not a pod letter plus `ROOM_LETTERS`; the count is what came in.
    Length(usize),
    /// A character a code cannot hold.
    Character(char),
}

impl fmt::Display for CodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodeError::Length(n) => {
                write!(f, "a room code is {} letters, not {n}", ROOM_LETTERS + 1)
            }
            CodeError::Character(c) => write!(f, "'{c}' is not part of a room code"),
        }
    }
}

impl std::error::Error for CodeError {}

/// The WebSocket to dial: `code` to join that room, `None` to create one.
///
/// The rule, whole:
///
/// - On the cluster (the default host) the pods are a StatefulSet behind
///   one Ingress, so the path has to name the pod that owns the room.
///   Joining `AK7QX` is `wss://rooms.bongbong.io/r/rooms-a/ws`: the
///   code's pod letter, lower-cased because that is how the pod is
///   named (`rooms-0`, `rooms-a`), in the per-pod path. Creating a room
///   goes to `/rooms`, which spreads across every pod, and the pod that
///   answers mints a code beginning with its own letter - so the code a
///   host shares already says where its room lives.
/// - With an override (`--rooms`, `BONGBONG_ROOMS`) there is exactly one
///   server and no Ingress in front of it, so both ends of the rule
///   collapse onto that server's own `/ws`. The code is still parsed,
///   because a join carries it and the server refuses a code minted by
///   another pod, but its letter no longer picks the path.
pub fn socket_url(host: &RoomsHost, code: Option<&RoomCode>) -> String {
    let base = host.base();
    match (host.is_override(), code) {
        (true, _) => format!("{base}/ws"),
        (false, None) => format!("{base}/rooms"),
        (false, Some(code)) => {
            format!("{base}/r/rooms-{}/ws", code.pod.to_ascii_lowercase())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cluster_puts_the_pod_in_the_path_and_creates_across_all_of_them() {
        let host = RoomsHost::cluster();
        assert_eq!(host.base(), "wss://rooms.bongbong.io");
        assert!(!host.is_override());
        assert_eq!(socket_url(&host, None), "wss://rooms.bongbong.io/rooms");
        let code = RoomCode::parse("AK7QX").unwrap();
        assert_eq!(socket_url(&host, Some(&code)), "wss://rooms.bongbong.io/r/rooms-a/ws");
        let numbered = RoomCode::parse("0K7QX").unwrap();
        assert_eq!(
            socket_url(&host, Some(&numbered)),
            "wss://rooms.bongbong.io/r/rooms-0/ws",
            "a digit pod letter is the StatefulSet's ordinal as it stands"
        );
    }

    #[test]
    fn an_override_points_at_that_one_server_for_both_create_and_join() {
        let host = RoomsHost::overriding("ws://127.0.0.1:4848/");
        assert!(host.is_override());
        assert_eq!(host.base(), "ws://127.0.0.1:4848", "a trailing slash is not part of the base");
        assert_eq!(socket_url(&host, None), "ws://127.0.0.1:4848/ws");
        let code = RoomCode::parse("AK7QX").unwrap();
        assert_eq!(socket_url(&host, Some(&code)), "ws://127.0.0.1:4848/ws", "the pod letter picks no path here");
        let other_pod = RoomCode::parse("BK7QX").unwrap();
        assert_eq!(socket_url(&host, Some(&other_pod)), socket_url(&host, Some(&code)));
    }

    #[test]
    fn resolve_prefers_the_argument_then_the_environment_then_the_cluster() {
        // The environment is process-wide, so this test owns it; no
        // other test in this module reads it.
        let saved = std::env::var(ROOMS_ENV).ok();
        // SAFETY: single-threaded within this test, restored below.
        unsafe { std::env::remove_var(ROOMS_ENV) };
        assert_eq!(RoomsHost::resolve(None), RoomsHost::cluster());
        assert_eq!(RoomsHost::resolve(Some("ws://host:1/")), RoomsHost::overriding("ws://host:1"));
        unsafe { std::env::set_var(ROOMS_ENV, "ws://127.0.0.1:4848") };
        assert_eq!(RoomsHost::resolve(None), RoomsHost::overriding("ws://127.0.0.1:4848"));
        assert_eq!(
            RoomsHost::resolve(Some("ws://elsewhere:2")),
            RoomsHost::overriding("ws://elsewhere:2"),
            "the flag outranks the environment"
        );
        unsafe { std::env::set_var(ROOMS_ENV, "   ") };
        assert_eq!(RoomsHost::resolve(None), RoomsHost::cluster(), "an empty override is no override");
        match saved {
            Some(v) => unsafe { std::env::set_var(ROOMS_ENV, v) },
            None => unsafe { std::env::remove_var(ROOMS_ENV) },
        }
    }

    #[test]
    fn a_code_is_canonicalised_and_its_pod_read_off_the_front() {
        let code = RoomCode::parse("  ak7qx ").unwrap();
        assert_eq!(code.text, "AK7QX");
        assert_eq!(code.pod, 'A');
        assert_eq!(code.to_string(), "AK7QX");
    }

    #[test]
    fn a_malformed_code_is_refused_before_a_socket_is_opened() {
        assert_eq!(RoomCode::parse(""), Err(CodeError::Length(0)));
        assert_eq!(RoomCode::parse("AK7Q"), Err(CodeError::Length(4)));
        assert_eq!(RoomCode::parse("AK7QXX"), Err(CodeError::Length(6)));
        assert_eq!(RoomCode::parse("-K7QX"), Err(CodeError::Character('-')));
        assert_eq!(RoomCode::parse("AK7Q-"), Err(CodeError::Character('-')));
        assert!(RoomCode::parse("AK7Q ").is_err(), "an inner gap is not trimmed away");
        assert!(CodeError::Length(3).to_string().contains("5 letters"));
        assert!(CodeError::Character('-').to_string().contains('-'));
        assert!(RoomCode::parse("AK7QO").is_ok(), "the pod decides which letters it mints, not the client");
    }
}
