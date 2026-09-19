//! The online co-op wire protocol (docs/online-coop-prd.md §4.3, §4.4,
//! §4.6): what a room server and a client say to each other, as bytes.
//! Plain synchronous code with no sockets, no threads, no async and no
//! raylib, so the server's tasks and the client's socket thread share it
//! verbatim and it compiles for every target, emscripten included. Nothing
//! here reads or writes a `Game`: filling a `Snapshot` from a round and
//! writing one into a replica are the simulation's business.
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
//!
//! Measured on the snapshot the PRD sizes (eight tanks, twenty-four
//! shells, both frogs, a fresh field; `delta::tests::sizes_of_the_prd_snapshot`
//! prints them under `--nocapture`): a full snapshot is 367 B (about 13 B
//! per tank and 9 B per shell); the delta for an interval in which every
//! hull and shell moved is 151 B, one in which two hulls were also hit,
//! four shells landed and four were fired is 214 B, and an idle interval
//! costs the 31 B header. The PRD's estimates were 300 B and 80 to 120 B;
//! the projectiles are the gap, and the PRD's next step - shots as events
//! once `Ricochet` exists - is what closes it.

pub mod codec;
pub mod delta;
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
pub const PROTOCOL_VERSION: u16 = 1;

/// The most seats a room holds (docs/online-coop-prd.md §7, decision 4):
/// the length of `Snapshot::acked`, and the same number `Input::seats`
/// carries, so the one in `lib.rs` is the source.
pub use crate::MAX_SEATS;
