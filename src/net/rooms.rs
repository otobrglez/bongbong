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

/// The symbols a room's letters are drawn from: twenty with no vowels (a
/// code never spells a word by accident) and no look-alikes (no 0/O,
/// 1/I/L, 2/Z, 5/S, 6/G, 8/B, 9/g), so a code survives being read aloud
/// or typed from a photo. The room server mints from this same list
/// (`bongbong_server::code::ALPHABET`; a test below pins the two
/// together), and the lobby's on-screen code entry offers exactly these
/// keys, which is why the list lives here rather than in either screen.
pub const CODE_ALPHABET: &[u8; 20] = b"CDFGHJKMNPQRTVWXY347";

/// Where an invite points (docs/online-coop-prd.md §4.10): the join page
/// on the site, which plays the room in the browser at once and opens the
/// app where it is installed.
pub const JOIN_URL_BASE: &str = "https://bongbong.io/j";

/// The link to share for `code` - what goes on a screen, in a chat
/// message and inside the lobby's QR.
///
/// A rooms host that came from `--rooms` or `BONGBONG_ROOMS` rides along
/// as a query parameter, because the join page takes the same override
/// the game does (docs/online-coop-prd.md §4.13): without it a scan would
/// send the other device to the cluster, which knows nothing about a room
/// on a laptop.
pub fn join_url(host: &RoomsHost, code: &str) -> String {
    match host.is_override() {
        false => format!("{JOIN_URL_BASE}/{code}"),
        true => format!("{JOIN_URL_BASE}/{code}?rooms={}", host.base()),
    }
}

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

/// What a page's own URL says about the room to open
/// (docs/online-coop-prd.md §4.10).
///
/// This is [`join_url`] read backwards, which is why it lives beside it:
/// the game writes the invite - the link on the screen, the link inside
/// the QR - so the game also reads it, and one set of tests holds both
/// ends of the rule together. The web build has no command line, so the
/// page hands its location over on `window.bbInvite`
/// (`site/src/scripts/room.ts`) and `app.rs` parses it once at start, the
/// way a desktop build reads `--join` and `--rooms` once.
///
/// Two spellings name a room: the invite's own path, `/j/AK7QX`, and
/// `?join=AK7QX` for a page that serves no such route - a local Astro
/// preview, an itch.io frame. Where a page carries both, the query is
/// the one taken: a path is where the page happens to sit, a query is
/// something somebody put there. The `rooms` override rides in the
/// query either way, because that is how `join_url` puts it there.
///
/// **Nothing here refuses a URL.** A mangled link, a code somebody
/// retyped wrong, a query with no `=` in it: every one of them leaves a
/// field unset and the lobby opens on its own opening face, where a code
/// is typed by hand anyway. A browser can be pointed at any URL at all,
/// so this must never be a way to stop the game starting.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Invite {
    /// The room the link names, if it names one that could be a room.
    pub code: Option<RoomCode>,
    /// The `rooms` override the link carries, exactly as it was written;
    /// [`Invite::rooms_host`] is what turns it into a host.
    pub rooms: Option<String>,
    /// The page was served over TLS (`https`, or a `wss` link). It
    /// decides the scheme an override with none of its own gets, since a
    /// secure page can only open a secure socket.
    pub secure: bool,
}

impl Invite {
    /// Read `url` - a whole page URL, or just the path and query.
    pub fn parse(url: &str) -> Invite {
        let url = url.trim();
        let (scheme, rest) = match url.split_once("://") {
            Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
            // No scheme: a bare path, which is a local page's, so the
            // socket an override with no scheme gets is a plain one.
            None => (String::new(), url),
        };
        // The fragment is never part of an invite and may hold anything.
        let rest = rest.split('#').next().unwrap_or("");
        let (authority_and_path, query) = match rest.split_once('?') {
            Some((path, query)) => (path, query),
            None => (rest, ""),
        };
        // With a scheme there is an authority to step over; without one
        // the whole thing is the path.
        let path = match scheme.is_empty() {
            false => authority_and_path.split_once('/').map_or("", |(_, path)| path),
            true => authority_and_path,
        };
        Invite {
            code: query_value(query, "join")
                .or_else(|| code_in_path(path))
                .and_then(|text| RoomCode::parse(&text).ok()),
            rooms: query_value(query, "rooms").filter(|r| !r.trim().is_empty()),
            secure: scheme == "https" || scheme == "wss",
        }
    }

    /// The rooms server this link points at, if it points at one of its
    /// own - `RoomsHost::resolve`'s `--rooms` and `BONGBONG_ROOMS`, in
    /// the one form a browser has.
    ///
    /// A socket is `ws://` or `wss://`, so the two spellings a person
    /// actually writes are completed: `http`/`https` become the socket
    /// scheme they pair with, and a host with no scheme at all takes the
    /// page's own - a plain socket from a plain page, a secure one from a
    /// secure page. An override with a scheme no socket speaks
    /// (`ftp://...`) is no override at all, and the link falls back to
    /// the cluster.
    pub fn rooms_host(&self) -> Option<RoomsHost> {
        let raw = self.rooms.as_deref()?.trim();
        if raw.is_empty() {
            return None;
        }
        let base = match raw.split_once("://") {
            Some((scheme, rest)) => match scheme.to_ascii_lowercase().as_str() {
                "ws" => format!("ws://{rest}"),
                "wss" => format!("wss://{rest}"),
                "http" => format!("ws://{rest}"),
                "https" => format!("wss://{rest}"),
                _ => return None,
            },
            None => {
                let scheme = if self.secure { "wss" } else { "ws" };
                format!("{scheme}://{}", raw.trim_start_matches('/'))
            }
        };
        Some(RoomsHost::overriding(&base))
    }
}

/// `name`'s value in a query string, percent-decoded.
///
/// Whichever spelling came out of the share sheet: a link is typed,
/// forwarded and re-encoded on its way to a phone, so `ws://host` and
/// `ws%3A%2F%2Fhost` are the same override. A pair with no `=` and a
/// name that is not there both read as absent.
fn query_value(query: &str, name: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| percent_decode(value))
        .filter(|value| !value.is_empty())
}

/// `%XX` back to the byte it stands for; anything else is itself. A
/// truncated or non-hex escape is left as written rather than dropped,
/// so a broken link still shows what it said.
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let decoded = match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                hex.and_then(|hex| u8::from_str_radix(hex, 16).ok())
            }
            _ => None,
        };
        match decoded {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The segment after the last `j` in `path`: the invite link's own
/// shape, `bongbong.io/j/AK7QX`, wherever the site happens to serve it
/// from.
fn code_in_path(path: &str) -> Option<String> {
    let mut segments = path.split('/').filter(|s| !s.is_empty()).peekable();
    let mut found = None;
    while let Some(segment) = segments.next() {
        if segment.eq_ignore_ascii_case("j")
            && let Some(next) = segments.peek()
        {
            found = Some(percent_decode(next));
        }
    }
    found
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

    /// The alphabet is one list: the room server mints its codes from it
    /// and the lobby's key grid offers it. The server's copy is read off
    /// disk rather than trusted, so the two cannot drift apart quietly.
    #[test]
    fn the_code_alphabet_is_the_one_the_room_server_mints_from() {
        assert_eq!(CODE_ALPHABET.len(), 20);
        let mut sorted = CODE_ALPHABET.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 20, "every symbol is distinct");
        assert!(CODE_ALPHABET.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()));
        for vowel in b"AEIOU" {
            assert!(!CODE_ALPHABET.contains(vowel), "a code could spell a word");
        }
        // One of each look-alike pair is dropped: 0/O, 1/I/L, 2/Z, 5/S,
        // 6/G, 8/B, 9/g - so only G survives its pair, and every digit
        // but 3, 4 and 7 is gone.
        for look_alike in b"BLSZ0125689" {
            assert!(!CODE_ALPHABET.contains(look_alike), "{} is read wrong", *look_alike as char);
        }
        // The room server mints from this very list rather than a copy of
        // it, which is what keeps a minted code typable on the lobby's grid.
        let server = include_str!("../../server/src/code.rs");
        assert!(
            server.contains("pub use bongbong::net::rooms::CODE_ALPHABET as ALPHABET;"),
            "the room server should mint from CODE_ALPHABET, not its own list"
        );
    }

    #[test]
    fn a_join_link_carries_a_local_override_so_a_scan_reaches_the_same_server() {
        assert_eq!(join_url(&RoomsHost::cluster(), "AK7QX"), "https://bongbong.io/j/AK7QX");
        assert_eq!(
            join_url(&RoomsHost::overriding("ws://127.0.0.1:4848"), "AK7QX"),
            "https://bongbong.io/j/AK7QX?rooms=ws://127.0.0.1:4848"
        );
        // Both fit the codes the lobby draws (`qr::MAX_BYTES` is 106).
        assert!(join_url(&RoomsHost::overriding("ws://127.0.0.1:4848"), "AK7QX").len() <= crate::qr::MAX_BYTES);
    }

    /// The two shapes a link names a room in: the invite's own path and
    /// the query a page with no such route carries.
    #[test]
    fn an_invite_link_is_read_back_the_way_it_was_written() {
        let invite = Invite::parse("https://bongbong.io/j/AK7QX");
        assert_eq!(invite.code.as_ref().map(|c| c.text.as_str()), Some("AK7QX"));
        assert_eq!(invite.code.as_ref().map(|c| c.pod), Some('A'));
        assert_eq!(invite.rooms, None);
        assert!(invite.secure, "an https page can only open a secure socket");
        // The page a local preview serves: no /j/ route, the code in the
        // query, and a lower-case code from somebody's address bar.
        let typed = Invite::parse("http://localhost:4321/?join=ak7qx");
        assert_eq!(typed.code.map(|c| c.text), Some("AK7QX".into()));
        assert!(!typed.secure);
        // A path and query with no origin in front of them.
        assert_eq!(Invite::parse("/j/AK7QX").code.map(|c| c.text), Some("AK7QX".into()));
        // The site may serve the route under a prefix, and a trailing
        // slash is not a segment.
        assert_eq!(Invite::parse("https://bongbong.io/play/j/AK7QX/").code.map(|c| c.text), Some("AK7QX".into()));
        // A fragment is not part of the link.
        assert_eq!(Invite::parse("https://bongbong.io/j/AK7QX#top").code.map(|c| c.text), Some("AK7QX".into()));
        // The query is the explicit one where a page carries both.
        assert_eq!(Invite::parse("https://bongbong.io/j/AK7QX?join=CK7QX").code.map(|c| c.text), Some("CK7QX".into()));
    }

    /// Every way a link can say nothing, or nothing usable. None of them
    /// is an error: the lobby opens where a code is typed by hand.
    #[test]
    fn a_link_with_no_room_in_it_is_simply_a_page() {
        for url in [
            "https://bongbong.io/",
            "https://bongbong.io",
            "http://localhost:4321/index.html",
            // A code that is not one: too short, too long, and a
            // character a code cannot hold.
            "https://bongbong.io/j/AK7Q",
            "https://bongbong.io/j/AK7QXX",
            "https://bongbong.io/j/AK-QX",
            "https://bongbong.io/?join=",
            // `j` with nothing after it.
            "https://bongbong.io/j",
            "https://bongbong.io/j/",
            // Not a URL at all.
            "",
            "   ",
            "://///?&&=",
            "%%%",
        ] {
            let invite = Invite::parse(url);
            assert_eq!(invite.code, None, "{url} named a room");
            assert_eq!(invite.rooms_host(), None, "{url} named a rooms server");
        }
    }

    /// The override `join_url` appends for a local server, back out of
    /// the link and into a host the client can dial - however the share
    /// sheet, the QR reader or the address bar spelled it on the way.
    #[test]
    fn the_rooms_override_survives_the_round_trip_to_a_link_and_back() {
        let host = RoomsHost::overriding("ws://127.0.0.1:4848");
        let link = join_url(&host, "AK7QX");
        let invite = Invite::parse(&link);
        assert_eq!(invite.code.as_ref().map(|c| c.text.as_str()), Some("AK7QX"));
        assert_eq!(invite.rooms_host(), Some(host), "the link carries the whole override");

        // Percent-encoded, which is what `encodeURIComponent` writes.
        let encoded = Invite::parse("https://bongbong.io/j/AK7QX?rooms=ws%3A%2F%2F127.0.0.1%3A4848");
        assert_eq!(encoded.rooms.as_deref(), Some("ws://127.0.0.1:4848"));
        assert_eq!(encoded.rooms_host(), Some(RoomsHost::overriding("ws://127.0.0.1:4848")));
        // Beside other parameters, in either order.
        let among = Invite::parse("https://bongbong.io/j/AK7QX?v=3&rooms=ws://h:1&x=y");
        assert_eq!(among.rooms_host(), Some(RoomsHost::overriding("ws://h:1")));
    }

    /// A socket is `ws` or `wss`, so an override written any other way
    /// is completed - by its own scheme where it has a usable one, and
    /// by the page's where it has none. A page served over TLS can open
    /// no plain socket at all, which is why the page's own scheme is the
    /// one that decides.
    #[test]
    fn an_override_with_no_socket_scheme_takes_the_pages_own() {
        let bare = |url: &str| Invite::parse(url).rooms_host().map(|h| h.base().to_string());
        assert_eq!(bare("http://localhost:4321/?rooms=127.0.0.1:4848"), Some("ws://127.0.0.1:4848".into()));
        assert_eq!(bare("https://bongbong.io/?rooms=rooms.example.com"), Some("wss://rooms.example.com".into()));
        assert_eq!(bare("https://bongbong.io/?rooms=//rooms.example.com"), Some("wss://rooms.example.com".into()));
        // A page URL with no scheme of its own is a local one.
        assert_eq!(bare("/j/AK7QX?rooms=127.0.0.1:4848"), Some("ws://127.0.0.1:4848".into()));
        // The two schemes people paste, each mapped onto its socket.
        assert_eq!(bare("https://bongbong.io/?rooms=http://127.0.0.1:4848"), Some("ws://127.0.0.1:4848".into()));
        assert_eq!(bare("http://localhost:4321/?rooms=https://rooms.example.com"), Some("wss://rooms.example.com".into()));
        // An explicit socket scheme is kept as written, mixed content
        // and all: the browser's own refusal names the URL, which says
        // more than a silent rewrite would.
        assert_eq!(bare("https://bongbong.io/?rooms=ws://127.0.0.1:4848"), Some("ws://127.0.0.1:4848".into()));
        // A trailing slash is not part of a base, as `RoomsHost` has it.
        assert_eq!(bare("http://localhost:4321/?rooms=ws://127.0.0.1:4848/"), Some("ws://127.0.0.1:4848".into()));
        // Nothing the game could dial.
        assert_eq!(bare("https://bongbong.io/?rooms=ftp://x"), None);
        assert_eq!(bare("https://bongbong.io/?rooms=%20"), None);
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
