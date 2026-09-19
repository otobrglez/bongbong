//! The online co-op wire protocol (docs/online-coop-prd.md §4.3, §4.4,
//! §4.6): what a room server and a client say to each other, as bytes.
//! Plain synchronous code with no sockets, no threads, no async and no
//! raylib, so the server's tasks and the client's socket thread share it
//! verbatim and it compiles for every target, emscripten included. The
//! message types and the codec never touch a `Game`; `encode` and `apply`
//! are the two ends that do, through `simulation::replica`.
//!
//! - `wire`: the message types - `IntentMsg`, `Snapshot` and its families,
//!   `Welcome`, `Lobby` - and the quantisation helpers (positions in
//!   quarter pixels as `i16`, velocities as `i8`, headings as `u8`).
//! - `events`: `WireEvent`, the owned mirror of `simulation::Event` that
//!   travels inside a snapshot; the AI's trace never does.
//! - `codec`: `encode`/`decode` of a `Msg`, one kind byte then `postcard`
//!   (the lobby is JSON), so one socket carries everything.
//! - `delta`: `SnapshotDelta` against the previous snapshot, and
//!   `apply_delta` to step a baseline forward.
//! - `encode`: a live `Game` into a `Snapshot` or a `Welcome`.
//! - `apply`: a `Welcome` into a replica `Game`, a `Snapshot` into one.
//!
//! Where the PRD's protocol (§4.3) changed on contact with the real
//! types: `TankState` carries the chassis `row` (a wave tank arrives
//! mid-round and the roster names only the seats'), `ShotState` its sprite
//! `variant` (a shell's row is its shooter's chassis, a bolt's its colour),
//! `Welcome` the `enemy_count` pin (`LevelOverrides` has no such field and
//! the replica's `init` must draw the server's rolls) and the `dead_cells`
//! (a snapshot lists only tiles that differ from fresh, so a joiner cannot
//! tell a hole from an untouched tile), and a tank's `HIT` flag is the hit
//! window rather than "since the previous snapshot", which an encoder
//! holding one frame cannot know.
//!
//! Measured on the snapshot the PRD sizes (eight tanks, twenty-four
//! shells, both frogs, a fresh field; `delta::tests::sizes_of_the_prd_snapshot`
//! prints them under `--nocapture`): a full snapshot is 399 B (about 14 B
//! per tank and 10 B per shell); the delta for an interval in which every
//! hull and shell moved is 151 B, one in which two hulls were also hit,
//! four shells landed and four were fired is 220 B, and an idle interval
//! costs the 31 B header. On the default map 240 frames into a seven-tank
//! band round (`apply::tests::encoding_the_same_round_twice_gives_identical_bytes`)
//! a full snapshot is 239 B and the welcome 17.2 KB, nearly all of it the
//! map's TOML. The PRD's estimates were 300 B and 80 to 120 B; the
//! projectiles are the gap, and the PRD's next step - shots as events,
//! which `Ricochet` makes possible - is what closes it.

pub mod apply;
pub mod codec;
pub mod delta;
pub mod encode;
pub mod events;
pub mod wire;

/// The protocol's version, compared in `Welcome`: a client on another
/// number leaves instead of misreading the stream. Bump it whenever bytes
/// from the previous version would decode wrongly or not at all: a field
/// added, removed, reordered or retyped in any postcard message, a variant
/// added to or reordered in an enum on the wire (postcard encodes the
/// variant index), a change to a `codec::kind` tag or to a quantisation
/// scale. A new optional field in a `Lobby` message (JSON, `serde(default)`)
/// needs no bump.
pub const PROTOCOL_VERSION: u16 = 3;

/// The most seats a room holds (docs/online-coop-prd.md §7, decision 4):
/// the length of `Snapshot::acked`, and the same number `Input::seats`
/// carries, so the one in `lib.rs` is the source.
pub use crate::MAX_SEATS;
