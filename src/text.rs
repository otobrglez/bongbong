//! Every string a player reads (docs/localization-prd.md): the language
//! catalogues, the one place they are looked up, the fold that brings a
//! letter the font lacks onto one it has, and the width of a string in
//! raylib's default font measured with no raylib in reach.
//!
//! **The catalogue.** One Fluent file per language under `lang/`,
//! embedded the way `SHIPPED_MAPS` embeds maps (`SHIPPED_LANGS`).
//! `text()` hands out the current [`Catalogue`], a shared snapshot cloned
//! under a lock that is released at once - `tuning()`'s discipline - and
//! a lookup that the language has not got falls through to English per
//! message, so a half-translated file ships a mixed screen rather than an
//! English one. Every message is named by a [`Key`] constant in [`keys`],
//! so a typo is a compile error; `text_tests` holds `keys::ALL` and
//! `lang/en.ftl` together in both directions.
//!
//! **The fold.** raylib's default font is Basic Latin and the Latin-1
//! Supplement - U+0020..U+007E and U+00A0..U+00FF - and nothing else, so
//! č, š and ž draw as `?`. [`fold`] maps every codepoint the font has no
//! glyph for onto the Latin-1 letter it is built on (Č to C, Ő to Ö
//! because the font has Ö) from one hand-written table, and every
//! message is folded as it is resolved, so what a model or a test sees
//! is what the screen shows. User text - a nickname, a map name - goes
//! through the same function where it enters a model. A codepoint the
//! table does not know draws as `?`, which is what raylib would draw
//! anyway; `text_tests::every_message_is_drawable` makes that a red test
//! rather than a `?` on a phone.
//!
//! **The width.** The default font is proportional and its 224 glyph
//! widths are a constant table in raylib's `rtext.c`, so [`width`] gives
//! the pixels `MeasureText` would for any folded string, headless; the
//! `render`-only test in `render::text` pins it against raylib glyph by
//! glyph. This is what replaces the `len() * CHAR_W` guesses.
//!
//! **The language** is the platform's (section 4.5): `choose` takes an
//! explicit request (`--lang`, `?lang=`) first, then the platform's list
//! (`sys-locale` on a desktop, the page's `navigator.languages`, SDL on
//! iOS, the system properties on Android), then English. There is no
//! picker and no saved preference. Nothing in `simulation/` or
//! `net/wire.rs` calls into this module - a test greps for it.

use std::borrow::Cow;
use std::sync::{Arc, RwLock};

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use unic_langid::LanguageIdentifier;

/// The languages the game ships, by tag, English first: the fallback for
/// every other one, and the only file every message must be in.
pub const SHIPPED_LANGS: &[(&str, &str)] = &[("en", include_str!("../lang/en.ftl")), ("sl", include_str!("../lang/sl.ftl"))];

/// The source language and the fallback.
pub const DEFAULT_LANG: &str = "en";

/// A message's id in the catalogue. Only the constants in [`keys`] exist,
/// so a lookup names a message that is in `lang/en.ftl` or does not
/// compile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key(pub &'static str);

macro_rules! keys {
    ($($name:ident = $id:literal;)*) => {
        /// The message ids, one constant each. `ALL` is the list a test
        /// holds against `lang/en.ftl`.
        pub mod keys {
            use super::Key;
            $(pub const $name: Key = Key($id);)*
            pub const ALL: &[Key] = &[$($name,)*];
        }
    };
}

keys! {
    HUD_SPEED = "hud-speed";
    HUD_SHIELD = "hud-shield";
    HUD_FROG = "hud-frog";
    BUTTON_BUILD = "button-build";
    BUTTON_PLAY = "button-play";
    BUTTON_LEAVE = "button-leave";
    BUTTON_ONLINE = "button-online";
    PLAYERS_TITLE = "players-title";
    PLAYERS_KEYS = "players-keys";
    PLAYERS_ONE = "players-one";
    PLAYERS_TWO = "players-two";
    LEAVE_TITLE = "leave-title";
    LEAVE_SUB = "leave-sub";
    LEAVE_CONFIRM = "leave-confirm";
    LEAVE_STAY = "leave-stay";
    ROUND_WON = "round-won";
    ROUND_LOST = "round-lost";
    ROUND_RESTARTING = "round-restarting";
    ROUND_BACK_TO_LOBBY = "round-back-to-lobby";
    PAUSED = "paused";
    MISSION_PROTECT = "mission-protect";
    MISSION_HUNT = "mission-hunt";
    MISSION_DESTROY = "mission-destroy";
    MISSION_PROTECT_BANNER = "mission-protect-banner";
    MISSION_HUNT_BANNER = "mission-hunt-banner";
    MISSION_DESTROY_BANNER = "mission-destroy-banner";
    WAVE_BANNER = "wave-banner";
    WAVE_FINAL = "wave-final";
    LEVEL_NUMBER = "level-number";
    RESULT_TIME = "result-time";
    RESULT_WRECKS = "result-wrecks";
    RESULT_ALL_CLEAR = "result-all-clear";
    RESULT_AGAIN = "result-again";
    RESULT_NEXT = "result-next";
    RESULT_FIRST = "result-first";
    RESULT_LEVELS = "result-levels";
    RESULT_NEXT_IN = "result-next-in";
    RESULT_AGAIN_IN = "result-again-in";
    LEVELS_TITLE = "levels-title";
    LEVELS_SUB = "levels-sub";
    LEVELS_BACK = "levels-back";
    BAR_LEVEL = "bar-level";
    SEAT_LABEL = "seat-label";
    TOUCH_STEER = "touch-steer";
    TOUCH_FIRE = "touch-fire";
    LOBBY_TITLE_START = "lobby-title-start";
    LOBBY_TITLE_CODE = "lobby-title-code";
    LOBBY_TITLE_WAITING = "lobby-title-waiting";
    LOBBY_TITLE_CLOSED = "lobby-title-closed";
    LOBBY_TITLE_HOST = "lobby-title-host";
    LOBBY_TITLE_GUEST = "lobby-title-guest";
    LOBBY_SUB_START = "lobby-sub-start";
    LOBBY_SUB_CODE = "lobby-sub-code";
    LOBBY_SUB_WAITING = "lobby-sub-waiting";
    LOBBY_SUB_CLOSED = "lobby-sub-closed";
    LOBBY_SUB_HOST = "lobby-sub-host";
    LOBBY_SUB_GUEST = "lobby-sub-guest";
    LOBBY_OUTCOME_WON = "lobby-outcome-won";
    LOBBY_OUTCOME_LOST = "lobby-outcome-lost";
    LOBBY_OUTCOME_OVER = "lobby-outcome-over";
    LOBBY_REMATCH_HOST = "lobby-rematch-host";
    LOBBY_REMATCH_GUEST = "lobby-rematch-guest";
    LOBBY_MAP = "lobby-map";
    LOBBY_MISSION = "lobby-mission";
    SEAT_AWAY = "seat-away";
    SEAT_HOST = "seat-host";
    SEAT_READY = "seat-ready";
    SEAT_WAITING = "seat-waiting";
    SEAT_EMPTY = "seat-empty";
    LOBBY_MORE = "lobby-more";
    LOBBY_HOST = "lobby-host";
    LOBBY_JOIN = "lobby-join";
    LOBBY_BACK = "lobby-back";
    LOBBY_CLOSE = "lobby-close";
    LOBBY_DELETE = "lobby-delete";
    LOBBY_CONFIRM = "lobby-confirm";
    LOBBY_READY = "lobby-ready";
    LOBBY_IM_READY = "lobby-im-ready";
    LOBBY_START = "lobby-start";
    LOBBY_REMATCH = "lobby-rematch";
    LOBBY_LEAVE = "lobby-leave";
    LOBBY_KICK = "lobby-kick";
    CODE_ERROR_LENGTH = "code-error-length";
    CODE_ERROR_CHARACTER = "code-error-character";
    STATUS_CONNECTING = "status-connecting";
    STATUS_GREETING = "status-greeting";
    STATUS_LOBBY = "status-lobby";
    STATUS_PING = "status-ping";
    STATUS_BUFFER = "status-buffer";
    STATUS_WAITING = "status-waiting";
    STATUS_OFFLINE = "status-offline";
    NOTE_TUNING_REFUSED = "note-tuning-refused";
    NOTE_WELCOME_REFUSED = "note-welcome-refused";
    NOTE_NOT_CLEARED = "note-not-cleared";
    REFUSAL_ALREADY_IN_ROOM = "refusal-already-in-room";
    REFUSAL_NOT_IN_ROOM = "refusal-not-in-room";
    REFUSAL_NOT_YOURS = "refusal-not-yours";
    REFUSAL_BAD_MESSAGE = "refusal-bad-message";
    REFUSAL_BAD_MAP = "refusal-bad-map";
    REFUSAL_BAD_CODE = "refusal-bad-code";
    REFUSAL_NO_SUCH_ROOM = "refusal-no-such-room";
    REFUSAL_ROOM_GONE = "refusal-room-gone";
    REFUSAL_SERVER_DRAINING = "refusal-server-draining";
    REFUSAL_SERVER_FULL = "refusal-server-full";
    REFUSAL_ROOM_FULL = "refusal-room-full";
    REFUSAL_ALREADY_STARTED = "refusal-already-started";
    REFUSAL_KICKED = "refusal-kicked";
    REFUSAL_RECONNECTED = "refusal-reconnected";
    REFUSAL_GRACE_OVER = "refusal-grace-over";
    REFUSAL_LEFT_ROOM = "refusal-left-room";
    REFUSAL_ONLY_HOST_STARTS = "refusal-only-host-starts";
    REFUSAL_ONLY_HOST_KICKS = "refusal-only-host-kicks";
    REFUSAL_KICK_SELF = "refusal-kick-self";
    REFUSAL_NO_SUCH_SEAT = "refusal-no-such-seat";
    REFUSAL_IN_PROGRESS = "refusal-in-progress";
    REFUSAL_NOT_READY = "refusal-not-ready";
    REFUSAL_SERVER_RESTARTING = "refusal-server-restarting";
    REFUSAL_ROOM_CLOSED = "refusal-room-closed";
    EDITOR_BUILD = "editor-build";
    EDITOR_UNDO = "editor-undo";
    EDITOR_REDO = "editor-redo";
    EDITOR_FILE = "editor-file";
    EDITOR_MAP = "editor-map";
    EDITOR_FIT = "editor-fit";
    EDITOR_PLAY_HERE = "editor-play-here";
    EDITOR_CHECK = "editor-check";
    CHECK_TITLE = "check-title";
    CHECK_HINT = "check-hint";
    CHECK_NONE = "check-none";
    CHECK_FIX = "check-fix";
    CHECK_CLEARED = "check-cleared";
    CHECK_NOT_CLEARED = "check-not-cleared";
    CHECK_PAR = "check-par";
    CHECK_CLEARED_HINT = "check-cleared-hint";
    CHECK_NOT_CLEARED_HINT = "check-not-cleared-hint";
    EDITOR_TOOL = "editor-tool";
    CATEGORY_WALL = "category-wall";
    CATEGORY_PROP = "category-prop";
    CATEGORY_GROUND = "category-ground";
    CATEGORY_ACTOR = "category-actor";
    CATEGORY_PICKUP = "category-pickup";
    FILE_LOAD = "file-load";
    FILE_SAVE = "file-save";
    FILE_SAVE_AS = "file-save-as";
    FILE_CLEAR = "file-clear";
    SETTINGS_TANKS = "settings-tanks";
    SETTINGS_TANK = "settings-tank";
    SETTINGS_TANK2 = "settings-tank2";
    SETTINGS_MISSION = "settings-mission";
    SETTINGS_SPAWN = "settings-spawn";
    SETTINGS_WAVES = "settings-waves";
    SETTINGS_SIZE = "settings-size";
    SETTINGS_GROWTH = "settings-growth";
    SETTINGS_TIER_START = "settings-tier-start";
    SETTINGS_TIER_END = "settings-tier-end";
    SETTINGS_THEME = "settings-theme";
    SETTINGS_WEATHER = "settings-weather";
    SETTINGS_WIDTH = "settings-width";
    SETTINGS_HEIGHT = "settings-height";
    SETTINGS_ANCHOR = "settings-anchor";
    SETTINGS_RESET = "settings-reset";
    SETTINGS_AUTO = "settings-auto";
    SETTINGS_CLI = "settings-cli";
    EDITOR_SAVE_AS = "editor-save-as";
    EDITOR_SAVE_HINT = "editor-save-hint";
    EDITOR_NO_MAPS = "editor-no-maps";
    EDITOR_SHIPPED = "editor-shipped";
    EDITOR_PAGE = "editor-page";
    EDITOR_UNTITLED = "editor-untitled";
    EDITOR_SAVED = "editor-saved";
    EDITOR_LOADED = "editor-loaded";
    EDITOR_SAVING_UNAVAILABLE = "editor-saving-unavailable";
    EDITOR_NO_NAME = "editor-no-name";
    EDITOR_BAD_NAME = "editor-bad-name";
}

/// The one-word title of a mission (the bar, the lobby's stepper).
pub fn mission_title(mission: crate::level::Mission) -> Key {
    use crate::level::Mission;
    match mission {
        Mission::Protect => keys::MISSION_PROTECT,
        Mission::Hunt => keys::MISSION_HUNT,
        Mission::Destroy => keys::MISSION_DESTROY,
    }
}

/// The banner a mission's round opens with.
pub fn mission_banner(mission: crate::level::Mission) -> Key {
    use crate::level::Mission;
    match mission {
        Mission::Protect => keys::MISSION_PROTECT_BANNER,
        Mission::Hunt => keys::MISSION_HUNT_BANNER,
        Mission::Destroy => keys::MISSION_DESTROY_BANNER,
    }
}

/// What the room refused, in the language on screen. The server sent a
/// code (`net::wire::Refusal`); this is its words, the server's own
/// detail carried through where it has one.
pub fn refusal(refusal: &crate::net::wire::Refusal) -> String {
    use crate::net::wire::Refusal as R;
    let t = text();
    match refusal {
        R::AlreadyInRoom => t.get(keys::REFUSAL_ALREADY_IN_ROOM),
        R::NotInRoom => t.get(keys::REFUSAL_NOT_IN_ROOM),
        R::NotYours => t.get(keys::REFUSAL_NOT_YOURS),
        R::BadMessage { detail } => t.fmt(keys::REFUSAL_BAD_MESSAGE, &[("detail", detail.as_str().into())]),
        R::BadMap { detail } => t.fmt(keys::REFUSAL_BAD_MAP, &[("detail", detail.as_str().into())]),
        R::BadCode { detail } => t.fmt(keys::REFUSAL_BAD_CODE, &[("detail", detail.as_str().into())]),
        R::NoSuchRoom { code } => t.fmt(keys::REFUSAL_NO_SUCH_ROOM, &[("code", code.as_str().into())]),
        R::RoomGone { code } => t.fmt(keys::REFUSAL_ROOM_GONE, &[("code", code.as_str().into())]),
        R::ServerDraining => t.get(keys::REFUSAL_SERVER_DRAINING),
        R::ServerFull { rooms } => t.fmt(keys::REFUSAL_SERVER_FULL, &[("rooms", (*rooms).into())]),
        R::RoomFull { seats } => t.fmt(keys::REFUSAL_ROOM_FULL, &[("seats", (*seats).into())]),
        R::AlreadyStarted => t.get(keys::REFUSAL_ALREADY_STARTED),
        R::Kicked => t.get(keys::REFUSAL_KICKED),
        R::Reconnected => t.get(keys::REFUSAL_RECONNECTED),
        R::GraceOver => t.get(keys::REFUSAL_GRACE_OVER),
        R::LeftRoom => t.get(keys::REFUSAL_LEFT_ROOM),
        R::OnlyHostStarts => t.get(keys::REFUSAL_ONLY_HOST_STARTS),
        R::OnlyHostKicks => t.get(keys::REFUSAL_ONLY_HOST_KICKS),
        R::KickSelf => t.get(keys::REFUSAL_KICK_SELF),
        R::NoSuchSeat => t.get(keys::REFUSAL_NO_SUCH_SEAT),
        R::InProgress => t.get(keys::REFUSAL_IN_PROGRESS),
        R::NotReady { nick } => t.fmt(keys::REFUSAL_NOT_READY, &[("nick", fold(nick).into_owned().into())]),
        R::ServerRestarting => t.get(keys::REFUSAL_SERVER_RESTARTING),
        R::RoomClosed => t.get(keys::REFUSAL_ROOM_CLOSED),
    }
}

/// One argument to a message with a placeholder: `("n", 3.into())`.
pub type Arg<'a> = (&'a str, FluentValue<'a>);

/// The messages of one language, layered over English.
pub struct Catalogue {
    tag: &'static str,
    /// The chosen language's bundle first, English's after it (just the
    /// one when English is the choice). A lookup takes the first bundle
    /// that has the message.
    bundles: Vec<FluentBundle<FluentResource>>,
}

impl Catalogue {
    /// The catalogue for `tag`, one of `SHIPPED_LANGS` - anything else is
    /// English. A file that does not parse is a bug in the repo, so it
    /// panics with the parser's complaint rather than shipping half a
    /// language silently; `text_tests` parses every shipped file.
    pub fn new(tag: &str) -> Catalogue {
        let tag = shipped(tag).unwrap_or(DEFAULT_LANG);
        let mut bundles = vec![bundle(tag)];
        if tag != DEFAULT_LANG {
            bundles.push(bundle(DEFAULT_LANG));
        }
        Catalogue { tag, bundles }
    }

    /// The language this catalogue speaks, as its shipped tag.
    pub fn tag(&self) -> &'static str {
        self.tag
    }

    /// A message with no placeholders, folded for the screen.
    pub fn get(&self, key: Key) -> String {
        self.fmt(key, &[])
    }

    /// A message with its placeholders filled, folded for the screen. A
    /// key English has not got - which the tests forbid - comes back as
    /// the key itself, visible rather than fatal.
    pub fn fmt(&self, key: Key, args: &[Arg]) -> String {
        self.message(key.0, args).unwrap_or_else(|| key.0.to_string())
    }

    /// A message by a dynamic id - `tool-brick`, `tank-scout`, the data
    /// families whose ids are built from a data name - or `None` when no
    /// shipped language has it.
    pub fn message(&self, id: &str, args: &[Arg]) -> Option<String> {
        let mut fluent_args = FluentArgs::new();
        for (name, value) in args {
            fluent_args.set(*name, value.clone());
        }
        let fluent_args = (!args.is_empty()).then_some(&fluent_args);
        for bundle in &self.bundles {
            let Some(message) = bundle.get_message(id) else { continue };
            let Some(pattern) = message.value() else { continue };
            let mut errors = Vec::new();
            let text = bundle.format_pattern(pattern, fluent_args, &mut errors);
            return Some(fold(&text).into_owned());
        }
        None
    }

    /// A data name as a word: `family-name` if the catalogue has it, else
    /// the name itself (a chassis called `scout` reads `scout`).
    pub fn named(&self, family: &str, name: &str) -> String {
        self.message(&format!("{family}-{name}"), &[]).unwrap_or_else(|| fold(name).into_owned())
    }
}

/// Parse `tag`'s file into a bundle with isolation marks off: the marks
/// (U+2068/U+2069) Fluent wraps placeables in by default have no glyph in
/// the font and no bidi to isolate here.
fn bundle(tag: &str) -> FluentBundle<FluentResource> {
    let (_, source) = SHIPPED_LANGS.iter().find(|(t, _)| *t == tag).expect("a shipped language");
    let resource = match FluentResource::try_new(source.to_string()) {
        Ok(resource) => resource,
        Err((_, errors)) => panic!("lang/{tag}.ftl does not parse: {errors:?}"),
    };
    let langid: LanguageIdentifier = tag.parse().expect("a shipped tag is a language identifier");
    let mut bundle = FluentBundle::new_concurrent(vec![langid]);
    bundle.set_use_isolating(false);
    if let Err(errors) = bundle.add_resource(resource) {
        panic!("lang/{tag}.ftl has duplicate messages: {errors:?}");
    }
    bundle
}

static CURRENT: RwLock<Option<Arc<Catalogue>>> = RwLock::new(None);

/// The catalogue in force: a shared snapshot, cloned under a lock that is
/// released at once, so holding it across another call is safe. English
/// until `set_language` says otherwise. Bind it once per frame or per
/// draw call rather than per string.
pub fn text() -> Arc<Catalogue> {
    if let Some(current) = CURRENT.read().expect("text poisoned").as_ref() {
        return Arc::clone(current);
    }
    let catalogue = Arc::new(Catalogue::new(DEFAULT_LANG));
    let mut slot = CURRENT.write().expect("text poisoned");
    Arc::clone(slot.get_or_insert(catalogue))
}

/// Put `tag`'s catalogue in force - `choose`'s answer at startup, the
/// dev server's `lang` tool later, both at a frame boundary, since the
/// models re-gather their strings every frame and nothing holds one
/// across frames. Answers the tag actually chosen, English for one not
/// shipped. Never called by a test: tests build their own `Catalogue`.
pub fn set_language(tag: &str) -> &'static str {
    let catalogue = Arc::new(Catalogue::new(tag));
    let chosen = catalogue.tag();
    *CURRENT.write().expect("text poisoned") = Some(catalogue);
    chosen
}

/// The tag of the language in force.
pub fn language() -> &'static str {
    text().tag()
}

/// The shipped tag `tag` names: itself, case and `_`/`-` aside
/// (`sl-SI`, `sl_SI.UTF-8` and `SL` all name `sl`), or the shipped
/// language with its language subtag (`en-GB` names `en`, `pt-PT` would
/// name a shipped `pt-BR`). `None` for a language the game has not got.
pub fn shipped(tag: &str) -> Option<&'static str> {
    let tag = tag.trim().split(['.', '@']).next().unwrap_or("").replace('_', "-").to_ascii_lowercase();
    if tag.is_empty() {
        return None;
    }
    if let Some((shipped, _)) = SHIPPED_LANGS.iter().find(|(t, _)| t.eq_ignore_ascii_case(&tag)) {
        return Some(shipped);
    }
    let language = tag.split('-').next().unwrap_or("");
    SHIPPED_LANGS.iter().map(|(t, _)| *t).find(|t| t.split('-').next().unwrap_or("").eq_ignore_ascii_case(language))
}

/// The shipped language nearest a platform's list of preferred
/// languages, most preferred first: the first one that names a shipped
/// language, else English.
pub fn negotiate<S: AsRef<str>>(requested: &[S]) -> &'static str {
    requested.iter().find_map(|tag| shipped(tag.as_ref())).unwrap_or(DEFAULT_LANG)
}

/// The language to start in: an explicit request (`--lang`, `?lang=`)
/// when it names a shipped language, else the platform's list, else
/// English. An explicit request for a language the game has not got is
/// ignored rather than refused - the platform's answer is still better
/// than English for someone who asked for Slovak and has a Slovene
/// phone.
pub fn choose<S: AsRef<str>>(explicit: Option<&str>, platform: &[S]) -> &'static str {
    explicit.and_then(shipped).unwrap_or_else(|| negotiate(platform))
}

/// The `lang` query parameter of a page URL, as written - the web
/// build's `--lang`. `None` when the URL carries none.
pub fn lang_from_url(url: &str) -> Option<String> {
    let query = url.split('#').next()?.split_once('?')?.1;
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| *name == "lang")
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The languages the platform this build runs on prefers, most preferred
/// first, as the platform spells them. The desktop's from `sys-locale`;
/// the web's, iOS's and Android's are read by `app.rs` and the platform
/// entries and handed to `choose`, since they need the page, SDL or the
/// system properties.
#[cfg(not(any(target_os = "emscripten", target_os = "ios", target_os = "android")))]
pub fn platform_languages() -> Vec<String> {
    sys_locale::get_locales().collect()
}

/// Whether raylib's default font has a glyph for `c`: Basic Latin and
/// the Latin-1 Supplement, less the controls between them.
pub const fn drawable(c: char) -> bool {
    matches!(c, '\u{20}'..='\u{7e}' | '\u{a1}'..='\u{ff}')
}

/// Letters the font lacks, each with the Latin-1 letter it is drawn as:
/// Latin Extended-A whole, the Extended-B and Additional letters the
/// European alphabets use, and the typographic punctuation a translator
/// or a nickname is likely to carry. The replacement is the base letter,
/// or the nearest letter the font *has* (ő is ö, not o). A language's
/// letters missing here are a red `text_tests::every_message_is_drawable`,
/// which is how the table grows.
const FOLDS: &[(char, &str)] = &[
    // Latin Extended-A, U+0100..U+017F.
    ('Ā', "A"), ('ā', "a"), ('Ă', "A"), ('ă', "a"), ('Ą', "A"), ('ą', "a"),
    ('Ć', "C"), ('ć', "c"), ('Ĉ', "C"), ('ĉ', "c"), ('Ċ', "C"), ('ċ', "c"), ('Č', "C"), ('č', "c"),
    ('Ď', "D"), ('ď', "d"), ('Đ', "D"), ('đ', "d"),
    ('Ē', "E"), ('ē', "e"), ('Ĕ', "E"), ('ĕ', "e"), ('Ė', "E"), ('ė', "e"), ('Ę', "E"), ('ę', "e"), ('Ě', "E"), ('ě', "e"),
    ('Ĝ', "G"), ('ĝ', "g"), ('Ğ', "G"), ('ğ', "g"), ('Ġ', "G"), ('ġ', "g"), ('Ģ', "G"), ('ģ', "g"),
    ('Ĥ', "H"), ('ĥ', "h"), ('Ħ', "H"), ('ħ', "h"),
    ('Ĩ', "I"), ('ĩ', "i"), ('Ī', "I"), ('ī', "i"), ('Ĭ', "I"), ('ĭ', "i"), ('Į', "I"), ('į', "i"), ('İ', "I"), ('ı', "i"),
    ('Ĳ', "IJ"), ('ĳ', "ij"), ('Ĵ', "J"), ('ĵ', "j"), ('Ķ', "K"), ('ķ', "k"), ('ĸ', "k"),
    ('Ĺ', "L"), ('ĺ', "l"), ('Ļ', "L"), ('ļ', "l"), ('Ľ', "L"), ('ľ', "l"), ('Ŀ', "L"), ('ŀ', "l"), ('Ł', "L"), ('ł', "l"),
    ('Ń', "N"), ('ń', "n"), ('Ņ', "N"), ('ņ', "n"), ('Ň', "N"), ('ň', "n"), ('ŉ', "n"), ('Ŋ', "N"), ('ŋ', "n"),
    ('Ō', "O"), ('ō', "o"), ('Ŏ', "O"), ('ŏ', "o"), ('Ő', "Ö"), ('ő', "ö"), ('Œ', "OE"), ('œ', "oe"),
    ('Ŕ', "R"), ('ŕ', "r"), ('Ŗ', "R"), ('ŗ', "r"), ('Ř', "R"), ('ř', "r"),
    ('Ś', "S"), ('ś', "s"), ('Ŝ', "S"), ('ŝ', "s"), ('Ş', "S"), ('ş', "s"), ('Š', "S"), ('š', "s"),
    ('Ţ', "T"), ('ţ', "t"), ('Ť', "T"), ('ť', "t"), ('Ŧ', "T"), ('ŧ', "t"),
    ('Ũ', "U"), ('ũ', "u"), ('Ū', "U"), ('ū', "u"), ('Ŭ', "U"), ('ŭ', "u"), ('Ů', "U"), ('ů', "u"), ('Ű', "Ü"), ('ű', "ü"), ('Ų', "U"), ('ų', "u"),
    ('Ŵ', "W"), ('ŵ', "w"), ('Ŷ', "Y"), ('ŷ', "y"), ('Ÿ', "Y"),
    ('Ź', "Z"), ('ź', "z"), ('Ż', "Z"), ('ż', "z"), ('Ž', "Z"), ('ž', "z"), ('ſ', "s"),
    // Extended-B and Additional: Romanian's comma-below letters, the
    // caron vowels, Vietnamese's horned and barred letters.
    ('Ș', "S"), ('ș', "s"), ('Ț', "T"), ('ț', "t"),
    ('Ǎ', "A"), ('ǎ', "a"), ('Ǐ', "I"), ('ǐ', "i"), ('Ǒ', "O"), ('ǒ', "o"), ('Ǔ', "U"), ('ǔ', "u"),
    ('Ơ', "O"), ('ơ', "o"), ('Ư', "U"), ('ư', "u"), ('Ǵ', "G"), ('ǵ', "g"), ('Ǹ', "N"), ('ǹ', "n"),
    // Typographic punctuation onto the ASCII it stands for.
    ('\u{a0}', " "), ('\u{2018}', "'"), ('\u{2019}', "'"), ('\u{201a}', "'"), ('\u{201c}', "\""), ('\u{201d}', "\""),
    ('\u{201e}', "\""), ('\u{2010}', "-"), ('\u{2011}', "-"), ('\u{2012}', "-"), ('\u{2013}', "-"), ('\u{2014}', "-"),
    ('\u{2026}', "..."), ('\u{2022}', "*"), ('\u{2039}', "<"), ('\u{203a}', ">"),
];

/// What the fold turns `c` into, or `None` for a codepoint the table has
/// no row for.
pub fn fold_char(c: char) -> Option<&'static str> {
    FOLDS.iter().find(|(from, _)| *from == c).map(|(_, to)| *to)
}

/// `text` as the font can draw it: every glyph the font has passes,
/// every letter in the table becomes the letter it is built on, and
/// anything else - a Cyrillic or CJK codepoint, a control - becomes `?`,
/// which is what raylib draws for a glyph it lacks. A string the font
/// already draws comes back borrowed.
pub fn fold(text: &str) -> Cow<'_, str> {
    if text.chars().all(drawable) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if drawable(c) {
            out.push(c);
        } else if let Some(to) = fold_char(c) {
            out.push_str(to);
        } else {
            out.push('?');
        }
    }
    Cow::Owned(out)
}

/// The default font's glyph widths at its native 10 px, one per
/// codepoint from U+0020 to U+00FF: raylib's `charsWidth[224]` in
/// `rtext.c`, copied whole (the 32 between U+007F and U+009F are the
/// one-pixel placeholders the font keeps for the controls).
const GLYPH_WIDTHS: [u8; 224] = [
    3, 1, 4, 6, 5, 7, 6, 2, 3, 3, 5, 5, 2, 4, 1, 7, 5, 2, 5, 5, 5, 5, 5, 5, 5, 5, 1, 1, 3, 4, 3, 6, //
    7, 6, 6, 6, 6, 6, 6, 6, 6, 3, 5, 6, 5, 7, 6, 6, 6, 6, 6, 6, 7, 6, 7, 7, 6, 6, 6, 2, 7, 2, 3, 5, //
    2, 5, 5, 5, 5, 5, 4, 5, 5, 1, 2, 5, 2, 5, 5, 5, 5, 5, 5, 5, 4, 5, 5, 5, 5, 5, 5, 3, 1, 3, 4, 4, //
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, //
    1, 1, 5, 5, 5, 7, 1, 5, 3, 7, 3, 5, 4, 1, 7, 4, 3, 5, 3, 3, 2, 5, 6, 1, 2, 2, 3, 5, 6, 6, 6, 6, //
    6, 6, 6, 6, 6, 6, 7, 6, 6, 6, 6, 6, 3, 3, 3, 3, 7, 6, 6, 6, 6, 6, 6, 5, 6, 6, 6, 6, 6, 6, 4, 6, //
    5, 5, 5, 5, 5, 5, 9, 5, 5, 5, 5, 5, 2, 2, 3, 3, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 3, 5, //
];

/// The default font's native glyph height, which `DrawText` scales
/// `size` against.
const BASE_SIZE: i32 = 10;

/// The glyph `c` is drawn with, at the native size: its own width, or
/// `?`'s for a codepoint the font has not got, as raylib falls back.
fn glyph_width(c: char) -> u8 {
    let index = (c as u32).wrapping_sub(0x20);
    match GLYPH_WIDTHS.get(index as usize) {
        Some(w) => *w,
        None => GLYPH_WIDTHS[('?' as usize) - 0x20],
    }
}

/// The pixels `DrawText`/`MeasureText` give `text` at `size` in the
/// default font: every glyph's width scaled by `size / 10`, plus the
/// spacing `DrawText` puts between glyphs (`size / 10`, whole) between
/// each pair. Sizes under 10 are drawn at 10, as raylib draws them.
/// `text` is measured as given - fold it first if it may carry a letter
/// the font lacks, or the `?` is what is measured.
pub fn width(text: &str, size: i32) -> i32 {
    let size = size.max(BASE_SIZE);
    let scale = size as f32 / BASE_SIZE as f32;
    let spacing = size / BASE_SIZE;
    let mut glyphs = 0i32;
    let mut units = 0.0f32;
    for c in text.chars() {
        glyphs += 1;
        units += glyph_width(c) as f32;
    }
    if glyphs == 0 {
        return 0;
    }
    (units * scale + ((glyphs - 1) * spacing) as f32) as i32
}

/// `text` cut down to what `width` allows at `size`, the last letter
/// that fits replaced by `~`, the builder's own truncation. A string that
/// fits comes back whole.
pub fn fit(text: &str, max_px: i32, size: i32) -> Cow<'_, str> {
    if width(text, size) <= max_px {
        return Cow::Borrowed(text);
    }
    let chars: Vec<char> = text.chars().collect();
    for keep in (0..chars.len()).rev() {
        let mut cut: String = chars[..keep].iter().collect();
        cut.push('~');
        if width(&cut, size) <= max_px {
            return Cow::Owned(cut);
        }
    }
    Cow::Owned("~".to_string())
}

/// The largest size, at most `size`, at which `text` is no wider than
/// `max_px` - a banner on a narrow window - and `BASE_SIZE` when even that
/// is too wide, the font's own size being the least it draws at.
pub fn fit_size(text: &str, size: i32, max_px: i32) -> i32 {
    (BASE_SIZE..=size.max(BASE_SIZE)).rev().find(|&s| width(text, s) <= max_px).unwrap_or(BASE_SIZE)
}

#[cfg(test)]
mod text_tests {
    use super::*;
    use crate::hud::{
        BANNER_MIN_SIZE, BANNER_SIZE, BANNER_SUB_SIZE, DIALOG_BUTTON_W, DIALOG_W, HUD_GAUGE_LABEL_MAX_PX, HUD_LABEL_SIZE,
        HUD_TEXT_SIZE, INFO_TITLE_W, LEVEL_BUTTON_W, LEVEL_BUTTON_WORD_GAP, LEVEL_NUMBER_SIZE, LEVEL_TITLE_SIZE,
        MODE_BUTTON_W, ONLINE_BUTTON_W, RESULT_BUTTON_W, RESULT_LEVELS_W, RESULT_LINE_SIZE, RESULT_STATS_GAP,
        RESULT_TEXT_PX, RESULT_TITLE_SIZE, UI_SMALL_TEXT, WAVE_BANNER_SIZE,
    };
    use crate::level::Mission;
    use crate::level_select::{SELECT_BACK_W, SELECT_MARGIN, SELECT_W, TILE_TITLE_SIZE};
    use crate::lobby::{
        LOBBY_BUTTON_W, LOBBY_KICK_W, LOBBY_MARGIN, LOBBY_SEAT_CHASSIS_X, LOBBY_SEAT_STATE_X, LOBBY_WIDE_W, LOBBY_W,
    };

    /// The ids of every message in a shipped file: a message starts a
    /// line with its id and `=`; comments, blank lines and the
    /// continuation lines of a selector do not.
    fn message_ids(source: &str) -> Vec<String> {
        source
            .lines()
            .filter_map(|line| {
                let (id, _) = line.split_once('=')?;
                let id = id.trim();
                let starts = id.chars().next().is_some_and(|c| c.is_ascii_lowercase());
                (starts && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')).then(|| id.to_string())
            })
            .collect()
    }

    fn english() -> Vec<String> {
        message_ids(SHIPPED_LANGS[0].1)
    }

    /// The families whose ids are built from a data name at runtime,
    /// each with the names the code can build.
    fn dynamic_ids() -> Vec<String> {
        let mut ids = Vec::new();
        for tool in crate::editor::TOOLS {
            ids.push(format!("tool-{}", tool.name()));
        }
        for name in crate::tuning::TANK_NAMES {
            ids.push(format!("tank-{name}"));
        }
        for theme in [crate::map::Theme::Grass, crate::map::Theme::Desert] {
            ids.push(format!("theme-{}", theme.name()));
        }
        for weather in crate::map::Weather::ALL {
            ids.push(format!("weather-{}", weather.name()));
        }
        for spawn in [crate::level::SpawnKind::Band, crate::level::SpawnKind::Waves] {
            ids.push(format!("spawn-{}", spawn.name()));
        }
        for tier in crate::level::Tier::ALL {
            ids.push(format!("tier-{}", tier.name()));
        }
        for kind in crate::maplint::LintKind::ALL {
            ids.push(format!("lint-{}", kind.tag()));
        }
        ids.push("status-label-room".to_string());
        ids
    }

    /// Every key constant names a message in English, every English
    /// message is a key constant or a member of a dynamic family, and no
    /// two constants share an id.
    #[test]
    fn the_keys_and_the_english_file_name_the_same_messages() {
        let english = english();
        for key in keys::ALL {
            assert!(english.contains(&key.0.to_string()), "{} has no message in lang/en.ftl", key.0);
        }
        let dynamic = dynamic_ids();
        for id in &english {
            let known = keys::ALL.iter().any(|k| k.0 == id) || dynamic.contains(id) || id.starts_with("tool-short-");
            assert!(known, "lang/en.ftl's {id} has no key constant and is in no dynamic family");
        }
        for id in &dynamic {
            assert!(english.contains(id), "lang/en.ftl lacks the data name {id}");
        }
        let mut ids: Vec<&str> = keys::ALL.iter().map(|k| k.0).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), keys::ALL.len(), "two key constants share an id");
    }

    /// Every shipped language parses, is a language identifier, and has
    /// no message English has not got (a stray key would never be shown).
    /// A message missing from a language is allowed - it falls back - and
    /// printed, so a partial file is visible in the test output.
    #[test]
    fn every_shipped_language_parses_and_is_a_subset_of_english() {
        let english = english();
        let levels = crate::levels::Levels::shipped();
        for (tag, source) in SHIPPED_LANGS {
            let _: LanguageIdentifier = tag.parse().unwrap_or_else(|e| panic!("{tag}: {e:?}"));
            let ids = message_ids(source);
            for id in &ids {
                // A language may give a tool a short spelling English
                // never needed, since its full name fits the bar.
                let short_tool = id.strip_prefix("tool-short-").is_some_and(|name| crate::editor::Tool::parse(name).is_some());
                // A level's English title is levels.toml's, so only the
                // other languages carry `level-<map>`.
                let level_title = id.strip_prefix("level-").is_some_and(|map| levels.position(map).is_some());
                assert!(english.contains(id) || short_tool || level_title, "lang/{tag}.ftl has {id}, which English has not");
            }
            let missing: Vec<&String> = english.iter().filter(|id| !ids.contains(id)).collect();
            if !missing.is_empty() {
                println!("lang/{tag}.ftl falls back to English for: {missing:?}");
            }
            let catalogue = Catalogue::new(tag);
            assert_eq!(catalogue.tag(), *tag);
        }
    }

    /// Every message of every language resolves to glyphs the font has,
    /// with the fold applied - so a language whose letters are missing
    /// from the fold table is a red test naming the letter.
    #[test]
    fn every_message_is_drawable() {
        let args: Vec<Arg> = vec![
            ("n", 3.into()),
            ("seconds", 5.into()),
            ("host", "wss://rooms.bongbong.io".into()),
            ("room", "ROOM".into()),
            ("code", "AK7QX".into()),
            ("seat", 2.into()),
            ("ms", 100.into()),
            ("rtt", " - PING 40 MS".into()),
            ("reason", "closed".into()),
            ("detail", "x".into()),
            ("nick", "ana".into()),
            ("expected", 5.into()),
            ("got", 4.into()),
            ("char", "!".into()),
            ("name", "x".into()),
            ("from", 1.into()),
            ("to", 8.into()),
            ("rooms", 64.into()),
            ("seats", 8.into()),
            ("count", 14.into()),
            ("total", 14.into()),
            ("time", "2:34".into()),
        ];
        for (tag, source) in SHIPPED_LANGS {
            let catalogue = Catalogue::new(tag);
            for id in message_ids(source) {
                let text = catalogue.message(&id, &args).unwrap_or_else(|| panic!("{tag}: {id} does not resolve"));
                assert!(!text.contains('?') || id.contains("title") || id.contains("error") || id.contains("bad-name"), "{tag}: {id} folds to {text:?} - a letter is missing from the fold table");
                assert!(text.chars().all(drawable), "{tag}: {id} resolves to an undrawable {text:?}");
                assert!(!text.contains('{'), "{tag}: {id} has an unfilled placeholder: {text:?}");
            }
        }
    }

    /// Every box a message is drawn in, with its size and the room it
    /// has in pixels, and the widest arguments it can carry. Every
    /// language is measured against every one of them.
    fn budgets() -> Vec<(Key, i32, i32, Vec<Arg<'static>>)> {
        let button = |k: Key, w: f32| (k, HUD_TEXT_SIZE, w as i32 - 8, vec![]);
        let dialog_button = |k: Key| (k, HUD_TEXT_SIZE, DIALOG_BUTTON_W as i32 - 16, vec![]);
        let lobby_button = |k: Key| (k, HUD_TEXT_SIZE, LOBBY_BUTTON_W as i32 - 12, vec![]);
        let wide = |k: Key| (k, HUD_TEXT_SIZE, LOBBY_WIDE_W as i32 - 12, vec![]);
        let content = (LOBBY_W - 2.0 * LOBBY_MARGIN) as i32;
        let area = crate::hud::UiFrame::plain((crate::hud::UI_MIN_W, crate::hud::UI_MIN_H)).area;
        let seat_state_px = (crate::lobby::seats_rect(area).width - LOBBY_KICK_W - LOBBY_SEAT_STATE_X) as i32 - 4;
        let mut out = vec![
            // A gauge's label over its bar, in the corners' small size.
            (keys::HUD_SPEED, UI_SMALL_TEXT, HUD_GAUGE_LABEL_MAX_PX, vec![]),
            (keys::HUD_SHIELD, UI_SMALL_TEXT, HUD_GAUGE_LABEL_MAX_PX, vec![]),
            (keys::HUD_FROG, UI_SMALL_TEXT, HUD_GAUGE_LABEL_MAX_PX, vec![]),
            button(keys::BUTTON_BUILD, MODE_BUTTON_W),
            button(keys::BUTTON_PLAY, MODE_BUTTON_W),
            button(keys::BUTTON_LEAVE, MODE_BUTTON_W),
            button(keys::BUTTON_ONLINE, ONLINE_BUTTON_W),
            (keys::PLAYERS_TITLE, 28, DIALOG_W as i32 - 32, vec![]),
            (keys::PLAYERS_KEYS, 16, DIALOG_W as i32 - 32, vec![]),
            (keys::LEAVE_TITLE, 28, DIALOG_W as i32 - 32, vec![]),
            (keys::LEAVE_SUB, 16, DIALOG_W as i32 - 32, vec![]),
            dialog_button(keys::PLAYERS_ONE),
            dialog_button(keys::PLAYERS_TWO),
            dialog_button(keys::LEAVE_CONFIRM),
            dialog_button(keys::LEAVE_STAY),
            // The right cluster's title shares its slot with " 12/12" in
            // a wave round.
            (keys::MISSION_PROTECT, HUD_TEXT_SIZE, INFO_TITLE_W as i32 - width(" 12/12", HUD_TEXT_SIZE), vec![]),
            (keys::MISSION_HUNT, HUD_TEXT_SIZE, INFO_TITLE_W as i32 - width(" 12/12", HUD_TEXT_SIZE), vec![]),
            (keys::MISSION_DESTROY, HUD_TEXT_SIZE, INFO_TITLE_W as i32 - width(" 12/12", HUD_TEXT_SIZE), vec![]),
            (keys::LOBBY_TITLE_START, 22, content, vec![]),
            (keys::LOBBY_TITLE_CODE, 22, content, vec![]),
            (keys::LOBBY_TITLE_WAITING, 22, content, vec![]),
            (keys::LOBBY_TITLE_CLOSED, 22, content, vec![]),
            (keys::LOBBY_TITLE_HOST, 22, content, vec![]),
            (keys::LOBBY_TITLE_GUEST, 22, content, vec![]),
            (keys::LOBBY_SUB_START, UI_SMALL_TEXT, content, vec![]),
            (keys::LOBBY_SUB_CODE, UI_SMALL_TEXT, content, vec![]),
            (keys::LOBBY_SUB_CLOSED, UI_SMALL_TEXT, content, vec![]),
            (keys::LOBBY_SUB_HOST, UI_SMALL_TEXT, content, vec![]),
            (keys::LOBBY_SUB_GUEST, UI_SMALL_TEXT, content, vec![]),
            // A room refused before it was dialled: the closed face's line.
            (keys::NOTE_NOT_CLEARED, UI_SMALL_TEXT, content, vec![]),
            (keys::LOBBY_MAP, HUD_TEXT_SIZE, 152, vec![]),
            (keys::LOBBY_MISSION, HUD_TEXT_SIZE, 152, vec![]),
            // The touch hint, centred on each half of the smallest window
            // the chrome is laid out in.
            (keys::TOUCH_STEER, crate::touch::HINT_TEXT_PT, (crate::hud::UI_MIN_W / 2.0) as i32 - 16, vec![]),
            (keys::TOUCH_FIRE, crate::touch::HINT_TEXT_PT, (crate::hud::UI_MIN_W / 2.0) as i32 - 16, vec![]),
            // A seat's state runs from its column to the kick button at
            // the row's right end.
            (keys::SEAT_AWAY, UI_SMALL_TEXT, seat_state_px, vec![]),
            (keys::SEAT_HOST, UI_SMALL_TEXT, seat_state_px, vec![]),
            (keys::SEAT_READY, UI_SMALL_TEXT, seat_state_px, vec![]),
            (keys::SEAT_WAITING, UI_SMALL_TEXT, seat_state_px, vec![]),
            (keys::SEAT_EMPTY, UI_SMALL_TEXT, 200, vec![]),
            wide(keys::LOBBY_HOST),
            wide(keys::LOBBY_JOIN),
            lobby_button(keys::LOBBY_BACK),
            lobby_button(keys::LOBBY_CLOSE),
            lobby_button(keys::LOBBY_DELETE),
            lobby_button(keys::LOBBY_CONFIRM),
            lobby_button(keys::LOBBY_READY),
            lobby_button(keys::LOBBY_IM_READY),
            lobby_button(keys::LOBBY_START),
            lobby_button(keys::LOBBY_REMATCH),
            lobby_button(keys::LOBBY_LEAVE),
            (keys::LOBBY_KICK, HUD_TEXT_SIZE, LOBBY_KICK_W as i32 - 8, vec![]),
            // The build bar (`editor::Bar`): BUILD in its slot, FILE and
            // MAP beside their carets; the small buttons' labels are
            // measured below, in a mouse's bar and a touch screen's.
            (keys::EDITOR_BUILD, HUD_TEXT_SIZE, 64, vec![]),
            (keys::EDITOR_FILE, HUD_TEXT_SIZE, 42, vec![]),
            (keys::EDITOR_MAP, HUD_TEXT_SIZE, 42, vec![]),
            // The CHECK panel (`editor::chrome::LINT_PANEL_W`): its title,
            // 16 pt, left of the three counts; the line under it in the
            // small size across the panel; a clean map's line, 16 pt; FIX
            // in its 80 pt button.
            (keys::CHECK_TITLE, 16, 240, vec![]),
            (keys::CHECK_HINT, UI_SMALL_TEXT, crate::editor::LINT_HINT_W as i32, vec![]),
            (keys::CHECK_NONE, 16, crate::editor::LINT_HINT_W as i32, vec![]),
            (keys::CHECK_FIX, 16, 72, vec![]),
            // The clear check's row: its title from beside the flag, the
            // par at the row's right end, the line under them, all inside
            // the room from the flag's words to the right inset.
            (keys::CHECK_CLEARED, 16, 200, vec![]),
            (keys::CHECK_NOT_CLEARED, 16, crate::editor::LINT_CLEAR_W as i32, vec![]),
            (keys::CHECK_PAR, 16, 160, vec![("time", "59:59".into())]),
            (keys::CHECK_CLEARED_HINT, UI_SMALL_TEXT, crate::editor::LINT_CLEAR_W as i32, vec![]),
            (keys::CHECK_NOT_CLEARED_HINT, UI_SMALL_TEXT, crate::editor::LINT_CLEAR_W as i32, vec![]),
            // A pager's span, in the small size between its `<` and `>`
            // in the narrowest paged panel, a column of the MAP panel.
            (
                keys::EDITOR_PAGE,
                UI_SMALL_TEXT,
                crate::EDITOR_SETTINGS_W as i32 - 2 * (16 + width(">", HUD_TEXT_SIZE) + 8),
                vec![("from", 99.into()), ("to", 99.into()), ("n", 99.into())],
            ),
            // The Save prompt's line under the name.
            (keys::EDITOR_SAVE_HINT, UI_SMALL_TEXT, crate::editor::chrome::SAVE_PROMPT.0 as i32 - 24, vec![]),
            (keys::FILE_LOAD, HUD_TEXT_SIZE, 168, vec![]),
            (keys::FILE_SAVE, HUD_TEXT_SIZE, 168, vec![]),
            (keys::FILE_SAVE_AS, HUD_TEXT_SIZE, 168, vec![]),
            (keys::FILE_CLEAR, HUD_TEXT_SIZE, 168, vec![]),
            // A settings row's label runs from its 4 px inset to the `<`
            // button at 124, and the TANK rows' stops at their chassis
            // icon, 32 px wide and one inset clear of the button
            // (`editor/render.rs`'s `settings_icon_rect`).
            (keys::SETTINGS_TANKS, 16, 116, vec![]),
            (keys::SETTINGS_TANK, 16, 80, vec![]),
            (keys::SETTINGS_TANK2, 16, 80, vec![]),
            (keys::SETTINGS_MISSION, 16, 116, vec![]),
            (keys::SETTINGS_SPAWN, 16, 116, vec![]),
            (keys::SETTINGS_WAVES, 16, 116, vec![]),
            (keys::SETTINGS_SIZE, 16, 116, vec![]),
            (keys::SETTINGS_GROWTH, 16, 116, vec![]),
            (keys::SETTINGS_TIER_START, 16, 116, vec![]),
            (keys::SETTINGS_TIER_END, 16, 116, vec![]),
            (keys::SETTINGS_THEME, 16, 116, vec![]),
            (keys::SETTINGS_WEATHER, 16, 116, vec![]),
            (keys::SETTINGS_WIDTH, 16, 116, vec![]),
            (keys::SETTINGS_HEIGHT, 16, 116, vec![]),
            (keys::SETTINGS_ANCHOR, 16, 116, vec![]),
            (keys::SETTINGS_RESET, 16, 116, vec![]),
            // The level's lines and the end screen: across the smallest
            // area the chrome is laid out in less a margin, the end
            // screen's two numbers side by side with a gap, and the
            // countdown free play and a room's round end on, two digits.
            (keys::LEVEL_NUMBER, LEVEL_NUMBER_SIZE, RESULT_TEXT_PX, vec![("n", 14.into()), ("count", 14.into())]),
            (keys::ROUND_RESTARTING, BANNER_SUB_SIZE, RESULT_TEXT_PX, vec![("seconds", 30.into())]),
            (keys::ROUND_BACK_TO_LOBBY, BANNER_SUB_SIZE, RESULT_TEXT_PX, vec![("seconds", 30.into())]),
            (keys::ROUND_WON, RESULT_TITLE_SIZE, RESULT_TEXT_PX, vec![]),
            (keys::ROUND_LOST, RESULT_TITLE_SIZE, RESULT_TEXT_PX, vec![]),
            (keys::RESULT_ALL_CLEAR, RESULT_LINE_SIZE, RESULT_TEXT_PX, vec![("count", 14.into())]),
            (keys::RESULT_TIME, RESULT_LINE_SIZE, (RESULT_TEXT_PX - RESULT_STATS_GAP) / 2, vec![("time", "59:59".into())]),
            (keys::RESULT_WRECKS, RESULT_LINE_SIZE, (RESULT_TEXT_PX - RESULT_STATS_GAP) / 2, vec![("n", 99.into()), ("total", 99.into())]),
            (keys::RESULT_AGAIN, HUD_TEXT_SIZE, RESULT_BUTTON_W as i32 - 16, vec![]),
            (keys::RESULT_NEXT, HUD_TEXT_SIZE, RESULT_BUTTON_W as i32 - 16, vec![]),
            (keys::RESULT_FIRST, HUD_TEXT_SIZE, RESULT_BUTTON_W as i32 - 16, vec![]),
            (keys::RESULT_LEVELS, HUD_TEXT_SIZE, RESULT_LEVELS_W as i32 - 16, vec![]),
            // The counting labels hold two digits: `restart_delay` goes
            // up to 30.
            (keys::RESULT_NEXT_IN, HUD_TEXT_SIZE, RESULT_BUTTON_W as i32 - 16, vec![("seconds", 30.into())]),
            (keys::RESULT_AGAIN_IN, HUD_TEXT_SIZE, RESULT_BUTTON_W as i32 - 16, vec![("seconds", 30.into())]),
            // The level select: the lobby's title and line sizes across
            // its content, BACK in its button; the corners' level button
            // holds its word, in the small size, and a two-digit number.
            (keys::LEVELS_TITLE, 22, (SELECT_W - 2.0 * SELECT_MARGIN) as i32, vec![]),
            (keys::LEVELS_SUB, UI_SMALL_TEXT, (SELECT_W - 2.0 * SELECT_MARGIN) as i32, vec![]),
            (keys::LEVELS_BACK, HUD_TEXT_SIZE, SELECT_BACK_W as i32 - 12, vec![]),
            (
                keys::BAR_LEVEL,
                UI_SMALL_TEXT,
                LEVEL_BUTTON_W as i32 - 12 - LEVEL_BUTTON_WORD_GAP - width("88", HUD_TEXT_SIZE),
                vec![],
            ),
        ];
        // The build bar's small buttons - UNDO, REDO, FIT, CHECK, PLAY
        // HERE - inside their drawn boxes (an outline is drawn outside
        // its box), at the size each bar draws them
        // (`editor::chrome::small_text`): a mouse's at 11 pt in the boxes
        // a desktop's bar has always had, a touch screen's at 12 in its
        // wider ones; and the five categories' names beside their row of
        // the palette a narrow bar folds them into.
        for touch in [false, true] {
            let ui = crate::hud::UiFrame::new((1600.0, 900.0), 1.0, 1.0, crate::hud::Insets::default(), touch);
            let bar = crate::editor::Bar::of(&ui);
            let size = crate::editor::chrome::small_text(touch);
            let room = |r: crate::math::Rectangle| (r.width - crate::editor::chrome::SMALL_BOX_INSET) as i32;
            for (key, r) in [
                (keys::EDITOR_UNDO, bar.undo),
                (keys::EDITOR_REDO, bar.redo),
                (keys::EDITOR_FIT, bar.fit),
                (keys::EDITOR_CHECK, bar.check),
                (keys::EDITOR_PLAY_HERE, bar.here),
            ] {
                out.push((key, size, room(r), vec![]));
            }
        }
        for category in crate::editor::Category::ALL {
            out.push((category.label_key(), UI_SMALL_TEXT, crate::editor::chrome::PALETTE_LABEL_W as i32 - 12, vec![]));
        }
        out
    }

    /// Every language fits every budget. A language file that does not
    /// does not merge, the way a probe fixture over its ceiling does not.
    #[test]
    fn every_language_fits_every_budget() {
        let mut over = Vec::new();
        for (tag, _) in SHIPPED_LANGS {
            let catalogue = Catalogue::new(tag);
            for (key, size, max_px, args) in budgets() {
                let text = catalogue.fmt(key, &args);
                let w = width(&text, size);
                if w > max_px {
                    over.push(format!("{tag}: {} = {text:?} is {w} px at {size} px, over its {max_px} px budget", key.0));
                }
            }
            // The banners: set at their size where the line has the room
            // and shrunk to fit where it has not, but never under
            // `BANNER_MIN_SIZE` in the smallest area the chrome is laid
            // out in.
            let mut banners: Vec<(String, i32)> = [Mission::Protect, Mission::Hunt, Mission::Destroy]
                .into_iter()
                .map(|m| (catalogue.get(mission_banner(m)), BANNER_SIZE))
                .collect();
            banners.push((catalogue.get(keys::PAUSED), BANNER_SIZE));
            banners.push((catalogue.get(keys::WAVE_FINAL), WAVE_BANNER_SIZE));
            banners.push((catalogue.fmt(keys::WAVE_BANNER, &[("n", 99.into())]), WAVE_BANNER_SIZE));
            for (banner, size) in banners {
                let fitted = fit_size(&banner, size, RESULT_TEXT_PX);
                if fitted < BANNER_MIN_SIZE.min(size) {
                    over.push(format!("{tag}: banner {banner:?} only fits at {fitted} px, under {BANNER_MIN_SIZE}"));
                }
            }
            // A seat's chassis runs from its column to the state's.
            for kind in crate::tank::TankKind::ALL {
                let name = catalogue.named("tank", kind.name());
                if width(&name, UI_SMALL_TEXT) > (LOBBY_SEAT_STATE_X - LOBBY_SEAT_CHASSIS_X) as i32 - 4 {
                    over.push(format!("{tag}: chassis {name:?} runs into the seat's state"));
                }
            }
            // Every level's title under the mission banner, and on its
            // tile in the level select, whole.
            let tile_px = crate::level_select::tile_text_px();
            for level in crate::levels::Levels::shipped().iter() {
                let title = catalogue.message(&format!("level-{}", level.map), &[]).unwrap_or_else(|| fold(&level.title).into_owned());
                if width(&title, LEVEL_TITLE_SIZE) > RESULT_TEXT_PX {
                    over.push(format!("{tag}: level {} = {title:?} overflows the smallest area", level.map));
                }
                let lines = crate::level_select::wrap(&title, tile_px, TILE_TITLE_SIZE);
                if lines.iter().any(|line| line.ends_with('~')) {
                    over.push(format!("{tag}: level {} = {title:?} is cut on its tile: {lines:?}", level.map));
                }
            }
            // The tool names in their dropdown rows, and the short ones
            // in the bar's line.
            for tool in crate::editor::TOOLS {
                let long = catalogue.named("tool", tool.name());
                if width(&long, HUD_TEXT_SIZE) > 200 - 48 - 8 {
                    over.push(format!("{tag}: tool {} = {long:?} overflows its row", tool.name()));
                }
                // The short name is the status line's and the cursor
                // readout's, kept to a word or two.
                let short = catalogue.message(&format!("tool-short-{}", tool.name()), &[]).unwrap_or(long);
                if width(&short, HUD_LABEL_SIZE) > 48 {
                    over.push(format!("{tag}: short tool name {short:?} is longer than a word or two"));
                }
            }
            // The CHECK panel's findings, 16 px from beside a row's mark to
            // its FIX button.
            for kind in crate::maplint::LintKind::ALL {
                let words = catalogue.named("lint", kind.tag());
                if width(&words, 16) > crate::editor::LINT_FINDING_W as i32 {
                    over.push(format!("{tag}: finding {} = {words:?} runs into its FIX button", kind.tag()));
                }
            }
            // A settings row's value runs from 180 pt into the row to its
            // `>` button at 288 (`editor/render.rs`'s `SETTINGS_VALUE_X`,
            // `settings_inc_rect`), with the `(cli)` mark after it in the
            // small size when a flag outranks the map - the weather's can.
            for weather in crate::map::Weather::ALL {
                let name = catalogue.named("weather", weather.name());
                let mark = catalogue.get(keys::SETTINGS_CLI);
                if width(&name, HUD_TEXT_SIZE) + 4 + width(&mark, UI_SMALL_TEXT) > 288 - 180 - 4 {
                    over.push(format!("{tag}: weather {} = {name:?} overflows its settings row", weather.name()));
                }
            }
        }
        assert!(over.is_empty(), "over budget:\n{}", over.join("\n"));
    }

    /// The fold: the font's own glyphs pass untouched and borrowed, the
    /// table's letters become their base, the rest become `?`.
    #[test]
    fn the_fold_keeps_what_the_font_has_and_bases_the_rest() {
        assert!(matches!(fold("HOST A ROOM"), Cow::Borrowed(_)));
        assert!(matches!(fold("café über niño"), Cow::Borrowed(_)), "Latin-1 letters are the font's");
        assert_eq!(fold("ČAKAM ŠE ŽABO"), "CAKAM SE ZABO");
        assert_eq!(fold("Začni čez pol ure"), "Zacni cez pol ure");
        assert_eq!(fold("Łódź, Kraków, Gdańsk"), "Lódz, Kraków, Gdansk", "ó is Latin-1 and stays");
        assert_eq!(fold("Győr Ștefan ĳ"), "Györ Stefan ij");
        assert_eq!(fold("Привет 日本"), "?????? ??");
        assert_eq!(fold("it’s “quoted” – done…"), "it's \"quoted\" - done...");
        assert_eq!(fold("a\u{a0}b"), "a b");
        assert_eq!(fold("tab\there"), "tab?here", "a control has no glyph");
        for (c, to) in FOLDS {
            assert!(!drawable(*c), "{c} is in the fold table but the font draws it");
            assert!(to.chars().all(drawable), "{c} folds to {to:?}, which the font cannot draw");
        }
    }

    /// The width rule is raylib's: glyph widths scaled by `size / 10`,
    /// the whole-pixel spacing between each pair, sizes under 10 drawn
    /// at 10, an unknown glyph measured as `?`.
    #[test]
    fn width_follows_the_default_fonts_table() {
        // At the native size a string is the sum of its widths plus one
        // pixel between each pair.
        assert_eq!(width("A", 10), 6);
        assert_eq!(width("AB", 10), 6 + 6 + 1);
        assert_eq!(width("", 10), 0);
        assert_eq!(width("i", 8), 1, "sizes under ten are drawn at ten");
        // Scaled: `DrawText`'s `MeasureText` truncates the float.
        assert_eq!(width("BUILD", 18), ((6 + 6 + 3 + 5 + 6) as f32 * 1.8 + 4.0) as i32);
        assert_eq!(width("HOST A ROOM", 18), ((6 + 6 + 6 + 7 + 3 + 6 + 3 + 6 + 6 + 6 + 7) as f32 * 1.8 + 10.0) as i32);
        assert_eq!(width("日", 10), width("?", 10), "a glyph the font lacks is drawn as ?");
        assert_eq!(width("ü", 10), 5, "Latin-1 has its own widths");
        // The guesses this replaces were in the right region.
        let leave = width("LEAVE", 18);
        assert!((50..=60).contains(&leave), "{leave}");
    }

    /// `fit` keeps what fits and marks the cut.
    #[test]
    fn fit_truncates_with_a_tilde() {
        assert_eq!(fit("BUILD", 200, 18), "BUILD");
        let cut = fit("A VERY LONG MAP NAME INDEED", 60, 18);
        assert!(cut.ends_with('~') && width(&cut, 18) <= 60, "{cut}");
        assert_eq!(fit("WIDE", 1, 18), "~");
    }

    /// `fit_size` keeps the size where the text fits and otherwise takes
    /// the largest that does, down to the font's own.
    #[test]
    fn fit_size_shrinks_only_what_does_not_fit() {
        assert_eq!(fit_size("YOU WIN", 72, 1000), 72);
        let banner = "PROTECT THE FROG!";
        let fitted = fit_size(banner, 72, 672);
        assert!(fitted < 72 && width(banner, fitted) <= 672, "{fitted}");
        assert!(width(banner, fitted + 1) > 672, "the largest size that fits");
        assert_eq!(fit_size(banner, 72, 1), BASE_SIZE, "nothing narrower than the font's own size");
        assert_eq!(fit_size("", 72, 0), 72);
    }

    /// Tags: exact, case and separator aside, then by language subtag,
    /// and nothing for a language the game has not got.
    #[test]
    fn shipped_tags_negotiate_by_tag_then_by_language() {
        assert_eq!(shipped("en"), Some("en"));
        assert_eq!(shipped("EN-us"), Some("en"));
        assert_eq!(shipped("sl-SI"), Some("sl"));
        assert_eq!(shipped("sl_SI.UTF-8"), Some("sl"));
        assert_eq!(shipped("de-DE"), None);
        assert_eq!(shipped(""), None);
        assert_eq!(shipped("C"), None);
        assert_eq!(negotiate(&["de-AT", "sl-SI", "en"]), "sl");
        assert_eq!(negotiate(&["de-AT"]), "en");
        assert_eq!(negotiate::<&str>(&[]), "en");
        assert_eq!(choose(Some("sl"), &["en-GB"]), "sl", "an explicit request wins");
        assert_eq!(choose(Some("xx"), &["sl-SI"]), "sl", "an unshipped request falls to the platform");
        assert_eq!(choose(None, &["fr", "sl"]), "sl");
        assert_eq!(lang_from_url("https://bongbong.io/?join=AK7QX&lang=sl#x"), Some("sl".into()));
        assert_eq!(lang_from_url("https://bongbong.io/j/AK7QX"), None);
        assert_eq!(lang_from_url("https://bongbong.io/?lang="), None);
    }

    /// A lookup falls through to English per message, plurals select
    /// per language, and placeholders fill without isolation marks.
    #[test]
    fn lookups_fall_back_per_message_and_fill_placeholders() {
        let en = Catalogue::new("en");
        assert_eq!(en.get(keys::LOBBY_HOST), "HOST A ROOM");
        assert_eq!(en.fmt(keys::LOBBY_MORE, &[("n", 4.into())]), "+4 MORE");
        assert_eq!(en.fmt(keys::WAVE_BANNER, &[("n", 2.into())]), "WAVE 2");
        assert_eq!(en.fmt(keys::STATUS_BUFFER, &[("room", "ROOM".into()), ("code", "AK7QX".into()), ("seat", 2.into()), ("rtt", "".into()), ("ms", 100.into())]), "ROOM AK7QX - SEAT 2 - BUFFER 100 MS");
        assert_eq!(en.fmt(keys::STATUS_PING, &[("ms", 40.into())]), "PING 40 MS");
        assert_eq!(en.named("tank", "scout"), "scout");
        assert_eq!(en.named("tank", "unknown"), "unknown", "a name the catalogue lacks is itself");
        assert_eq!(en.message("no-such-message", &[]), None);
        let sl = Catalogue::new("sl");
        assert_eq!(sl.tag(), "sl");
        assert_ne!(sl.get(keys::LOBBY_HOST), en.get(keys::LOBBY_HOST), "Slovenian is translated");
        assert_eq!(sl.get(keys::LOBBY_HOST), fold(&sl.get(keys::LOBBY_HOST)), "resolved text is already folded");
        assert_eq!(Catalogue::new("xx").tag(), "en", "an unshipped tag is English");
    }

    /// Nothing under `simulation/` or in the wire format names this
    /// module: a string is presentation, and a replica or a room server
    /// must never depend on a language.
    #[test]
    fn the_simulation_and_the_wire_never_read_the_catalogue() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        let mut stack = vec![root.join("simulation"), root.join("net")];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("readable") {
                let path = entry.expect("entry").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                // The window's side of the wire is presentation and does
                // look strings up; the protocol and the replica do not.
                if path.ends_with("round.rs") {
                    continue;
                }
                let source = std::fs::read_to_string(&path).expect("readable");
                if source.contains("text::text()") || source.contains("crate::text::") || source.contains("use crate::text") {
                    offenders.push(path);
                }
            }
        }
        assert!(offenders.is_empty(), "these files read the catalogue: {offenders:?}");
    }
}
