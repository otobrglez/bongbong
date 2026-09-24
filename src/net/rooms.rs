//! Where the rooms are (docs/online-coop-prd.md §4.10, §4.13): the host
//! a client dials and the code it names a room by. There is one rooms
//! server and every room lives in it, so a link is enough to reach a
//! seat: nothing - no directory, no HTTP lookup, no path derived from
//! the code - stands between `bongbong.io/j/CK7QX` and the socket.

use std::fmt;

/// The rooms server the game ships pointing at, the default of
/// `--rooms` and `BONGBONG_ROOMS`. One deployment, one host name: there
/// is nothing behind it to pick between.
pub const DEFAULT_ROOMS_HOST: &str = "wss://rooms.bongbong.io";

/// The environment variable that overrides the host, so a local server
/// needs no code change: `BONGBONG_ROOMS=ws://127.0.0.1:4848`.
pub const ROOMS_ENV: &str = "BONGBONG_ROOMS";

/// How long a room code is: 20^5 = 3.2 million rooms, which is room
/// enough for one server. The client checks a code's shape before it
/// opens a socket; which codes name a room that exists is the room
/// server's business (`bongbong_server::code`, which re-exports this).
pub const CODE_LETTERS: usize = 5;

/// The symbols a code is drawn from: twenty with no vowels (a code never
/// spells a word by accident) and no look-alikes (no 0/O, 1/I/L, 2/Z,
/// 5/S, 6/G, 8/B, 9/g), so a code survives being read aloud or typed
/// from a photo. The room server mints from this same list
/// (`bongbong_server::code::ALPHABET`; a test below pins the two
/// together), and the lobby's on-screen code entry offers exactly these
/// keys, which is why the list lives here rather than in either screen.
pub const CODE_ALPHABET: &[u8; 20] = b"CDFGHJKMNPQRTVWXY347";

/// The site an invite points at (docs/online-coop-prd.md §4.10), which
/// plays the room in the browser at once and opens the app where it is
/// installed.
pub const SITE_BASE: &str = "https://bongbong.io";

/// Where a plain invite points: `bongbong.io/j/CK7QX`. The site is
/// static and has no page there, so `site/public/_redirects` sends it to
/// `/?join=CK7QX`, which `Invite::parse` reads the same way.
pub const JOIN_URL_BASE: &str = "https://bongbong.io/j";

/// The link to share for `code` - what goes on a screen, in a chat
/// message and inside the lobby's QR.
///
/// A rooms host that came from `--rooms` or `BONGBONG_ROOMS` rides along
/// as a query parameter, because the join page takes the same override
/// the game does (docs/online-coop-prd.md §4.13): without it a scan would
/// send the other device to the deployed server, which knows nothing
/// about a room on a laptop.
///
/// **The two forms differ by more than that parameter.** A plain invite
/// is the pretty path, `bongbong.io/j/CK7QX`, which
/// `site/public/_redirects` turns into `/?join=CK7QX` at the edge. That
/// redirect cannot carry a query across - its destination has one of its
/// own, and Cloudflare replaces rather than merges - so an overridden
/// host skips the path form and writes the query form itself. Both land
/// on the same page and `Invite::parse` reads either; only the prettier
/// one needs the redirect.
pub fn join_url(host: &RoomsHost, code: &str) -> String {
    match host.is_override() {
        false => format!("{JOIN_URL_BASE}/{code}"),
        true => format!("{SITE_BASE}/?join={code}&rooms={}", host.base()),
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
    /// `BONGBONG_ROOMS`, else the deployed one. Anything but the last is
    /// an override, which is what puts the host inside a join link.
    pub fn resolve(arg: Option<&str>) -> RoomsHost {
        let from_env = std::env::var(ROOMS_ENV).ok();
        match arg.map(str::to_string).or(from_env) {
            Some(base) if !base.trim().is_empty() => RoomsHost::overriding(&base),
            _ => RoomsHost { base: DEFAULT_ROOMS_HOST.into(), overridden: false },
        }
    }

    /// A server named by hand: a local `cargo run -p bongbong-server`,
    /// a staging box, the rig's own.
    pub fn overriding(base: &str) -> RoomsHost {
        RoomsHost { base: base.trim().trim_end_matches('/').to_string(), overridden: true }
    }

    /// The deployed one, which is where the game points unasked.
    pub fn deployed() -> RoomsHost {
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
}

impl RoomCode {
    /// `raw` as a code, trimmed and upper-cased.
    ///
    /// The shape is checked here - `CODE_LETTERS` alphanumerics - so a
    /// typo is caught before a socket is opened. Whether they come from
    /// `CODE_ALPHABET`, and whether they spell a room that exists, is
    /// the room server's answer: it is the one that mints them, and a
    /// player who mistypes a letter deserves the server's reason rather
    /// than a key that quietly does nothing.
    pub fn parse(raw: &str) -> Result<RoomCode, CodeError> {
        let text: String = raw.trim().to_ascii_uppercase();
        let n = text.chars().count();
        if n != CODE_LETTERS {
            return Err(CodeError::Length(n));
        }
        if let Some(bad) = text.chars().find(|c| !c.is_ascii_alphanumeric()) {
            return Err(CodeError::Character(bad));
        }
        Ok(RoomCode { text })
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
    /// Not `CODE_LETTERS` long; the count is what came in.
    Length(usize),
    /// A character a code cannot hold.
    Character(char),
}

impl fmt::Display for CodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodeError::Length(n) => {
                write!(f, "a room code is {CODE_LETTERS} letters, not {n}")
            }
            CodeError::Character(c) => write!(f, "'{c}' is not part of a room code"),
        }
    }
}

impl std::error::Error for CodeError {}

/// The WebSocket to dial, for hosting and for joining alike.
///
/// There is one rooms server and every room is in its memory, so the
/// whole rule is its own `/ws` - the same socket for a create and for a
/// join, whether the host is the deployed one or a laptop named by
/// `--rooms`. The code travels inside `Lobby::Join`; it picks no path,
/// because there is nowhere else a room could be.
///
/// Spreading rooms over several servers is deferred distribution work
/// (docs/online-coop-prd.md §4.8): it is what would put something
/// derived from the code back into this URL.
pub fn socket_url(host: &RoomsHost) -> String {
    format!("{}/ws", host.base())
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
/// Two spellings name a room: the invite's own path, `/j/CK7QX`, and
/// `?join=CK7QX` for a page that serves no such route - a local Astro
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
                // `site/public/_redirects` splats the path into the
                // query, so `/j/CK7QX/` arrives as `join=CK7QX/`. The
                // path form drops empty segments and never sees this;
                // trim it here so a link somebody's client tidied with a
                // trailing slash still joins.
                .map(|text| text.trim_end_matches('/').to_string())
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
    /// the deployed server.
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

    /// One server, so one socket: hosting and joining dial the same
    /// `/ws`, and no part of a code appears in the URL.
    #[test]
    fn every_room_is_reached_at_the_one_servers_own_ws() {
        let deployed = RoomsHost::deployed();
        assert_eq!(deployed.base(), "wss://rooms.bongbong.io");
        assert!(!deployed.is_override());
        assert_eq!(socket_url(&deployed), "wss://rooms.bongbong.io/ws");

        let local = RoomsHost::overriding("ws://127.0.0.1:4848/");
        assert!(local.is_override());
        assert_eq!(local.base(), "ws://127.0.0.1:4848", "a trailing slash is not part of the base");
        assert_eq!(socket_url(&local), "ws://127.0.0.1:4848/ws");
    }

    #[test]
    fn resolve_prefers_the_argument_then_the_environment_then_the_deployed_host() {
        // The environment is process-wide, so this test owns it; no
        // other test in this module reads it.
        let saved = std::env::var(ROOMS_ENV).ok();
        // SAFETY: single-threaded within this test, restored below.
        unsafe { std::env::remove_var(ROOMS_ENV) };
        assert_eq!(RoomsHost::resolve(None), RoomsHost::deployed());
        assert_eq!(RoomsHost::resolve(Some("ws://host:1/")), RoomsHost::overriding("ws://host:1"));
        unsafe { std::env::set_var(ROOMS_ENV, "ws://127.0.0.1:4848") };
        assert_eq!(RoomsHost::resolve(None), RoomsHost::overriding("ws://127.0.0.1:4848"));
        assert_eq!(
            RoomsHost::resolve(Some("ws://elsewhere:2")),
            RoomsHost::overriding("ws://elsewhere:2"),
            "the flag outranks the environment"
        );
        unsafe { std::env::set_var(ROOMS_ENV, "   ") };
        assert_eq!(RoomsHost::resolve(None), RoomsHost::deployed(), "an empty override is no override");
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
        // The room server mints from this very list, and to this very
        // length, rather than from copies of them - which is what keeps a
        // minted code typable on the lobby's grid and a five-box entry
        // exactly one code wide.
        let server = include_str!("../../server/src/code.rs");
        for line in [
            "pub use bongbong::net::rooms::CODE_ALPHABET as ALPHABET;",
            "pub use bongbong::net::rooms::CODE_LETTERS as LETTERS;",
        ] {
            assert!(server.contains(line), "the room server should mint from the client's own {line:?}");
        }
    }

    #[test]
    fn a_join_link_carries_a_local_override_so_a_scan_reaches_the_same_server() {
        assert_eq!(join_url(&RoomsHost::deployed(), "CK7QX"), "https://bongbong.io/j/CK7QX");
        // An override writes the query form rather than the path form,
        // because the `/j/*` redirect would drop the `rooms` parameter.
        assert_eq!(
            join_url(&RoomsHost::overriding("ws://127.0.0.1:4848"), "CK7QX"),
            "https://bongbong.io/?join=CK7QX&rooms=ws://127.0.0.1:4848"
        );
        // Both fit the codes the lobby draws (`qr::MAX_BYTES` is 106).
        assert!(join_url(&RoomsHost::overriding("ws://127.0.0.1:4848"), "CK7QX").len() <= crate::qr::MAX_BYTES);
    }

    /// The site is static, so `bongbong.io/j/CK7QX` only reaches the game
    /// because `site/public/_redirects` sends it to the query form. That
    /// file is read off disk here rather than trusted, the way the room
    /// server's alphabet is: if the rule is dropped or its shape changes,
    /// every shared invite and every QR 404s, and nothing else in the
    /// tree would notice.
    #[test]
    fn the_invite_path_is_redirected_to_the_query_form_the_game_reads() {
        let redirects = include_str!("../../site/public/_redirects");
        let rule = redirects
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with("/j/"))
            .expect("a rule for the invite path");
        let mut parts = rule.split_whitespace();
        assert_eq!(parts.next(), Some("/j/*"), "the invite path, splatted");
        assert_eq!(parts.next(), Some("/?join=:splat"), "the query form `Invite::parse` reads");
        // A redirect, not a rewrite: the code moves into the query, so
        // the browser has to be told where it ended up.
        assert_eq!(parts.next(), Some("302"));
        // And the path the rule matches is the one `join_url` writes.
        let link = join_url(&RoomsHost::deployed(), "CK7QX");
        assert!(link.starts_with("https://bongbong.io/j/"), "{link} is not the path the rule catches");
    }

    /// The two shapes a link names a room in: the invite's own path and
    /// the query a page with no such route carries.
    #[test]
    fn an_invite_link_is_read_back_the_way_it_was_written() {
        let invite = Invite::parse("https://bongbong.io/j/CK7QX");
        assert_eq!(invite.code.as_ref().map(|c| c.text.as_str()), Some("CK7QX"));
        assert_eq!(invite.rooms, None);
        assert!(invite.secure, "an https page can only open a secure socket");
        // The page a local preview serves: no /j/ route, the code in the
        // query, and a lower-case code from somebody's address bar.
        let typed = Invite::parse("http://localhost:4321/?join=ck7qx");
        assert_eq!(typed.code.map(|c| c.text), Some("CK7QX".into()));
        assert!(!typed.secure);
        // A path and query with no origin in front of them.
        assert_eq!(Invite::parse("/j/CK7QX").code.map(|c| c.text), Some("CK7QX".into()));
        // The site may serve the route under a prefix, and a trailing
        // slash is not a segment.
        assert_eq!(Invite::parse("https://bongbong.io/play/j/CK7QX/").code.map(|c| c.text), Some("CK7QX".into()));
        // A fragment is not part of the link.
        assert_eq!(Invite::parse("https://bongbong.io/j/CK7QX#top").code.map(|c| c.text), Some("CK7QX".into()));
        // The query is the explicit one where a page carries both.
        assert_eq!(Invite::parse("https://bongbong.io/j/CK7QX?join=DM4WT").code.map(|c| c.text), Some("DM4WT".into()));
        // What the `/j/*` redirect makes of a trailing slash: the splat
        // takes it along, so `?join=CK7QX/` has to read as the code.
        assert_eq!(Invite::parse("https://bongbong.io/?join=CK7QX/").code.map(|c| c.text), Some("CK7QX".into()));
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
            "https://bongbong.io/j/CK7Q",
            "https://bongbong.io/j/CK7QXX",
            "https://bongbong.io/j/CK-QX",
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
        let link = join_url(&host, "CK7QX");
        let invite = Invite::parse(&link);
        assert_eq!(invite.code.as_ref().map(|c| c.text.as_str()), Some("CK7QX"));
        assert_eq!(invite.rooms_host(), Some(host), "the link carries the whole override");

        // Percent-encoded, which is what `encodeURIComponent` writes.
        let encoded = Invite::parse("https://bongbong.io/j/CK7QX?rooms=ws%3A%2F%2F127.0.0.1%3A4848");
        assert_eq!(encoded.rooms.as_deref(), Some("ws://127.0.0.1:4848"));
        assert_eq!(encoded.rooms_host(), Some(RoomsHost::overriding("ws://127.0.0.1:4848")));
        // Beside other parameters, in either order.
        let among = Invite::parse("https://bongbong.io/j/CK7QX?v=3&rooms=ws://h:1&x=y");
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
        assert_eq!(bare("/j/CK7QX?rooms=127.0.0.1:4848"), Some("ws://127.0.0.1:4848".into()));
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
    fn a_code_is_trimmed_and_read_in_capitals() {
        let code = RoomCode::parse("  ck7qx ").unwrap();
        assert_eq!(code.text, "CK7QX");
        assert_eq!(code.to_string(), "CK7QX");
    }

    #[test]
    fn a_malformed_code_is_refused_before_a_socket_is_opened() {
        assert_eq!(RoomCode::parse(""), Err(CodeError::Length(0)));
        assert_eq!(RoomCode::parse("CK7Q"), Err(CodeError::Length(4)));
        assert_eq!(RoomCode::parse("CK7QXX"), Err(CodeError::Length(6)));
        assert_eq!(RoomCode::parse("-K7QX"), Err(CodeError::Character('-')));
        assert_eq!(RoomCode::parse("CK7Q-"), Err(CodeError::Character('-')));
        assert!(RoomCode::parse("CK7Q ").is_err(), "an inner gap is not trimmed away");
        assert!(CodeError::Length(3).to_string().contains("5 letters"));
        assert!(CodeError::Character('-').to_string().contains('-'));
        assert!(RoomCode::parse("CK7QO").is_ok(), "the room server decides which letters it mints, not the client");
    }
}
