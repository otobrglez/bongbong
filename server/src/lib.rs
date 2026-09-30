//! The room server (docs/online-coop-prd.md §4.7): rooms that own an
//! authoritative `bongbong::simulation::Game` each and the WebSocket
//! connections that feed them intents and receive snapshots, plus the
//! HTTP surface an operator watches. `main.rs` is the command line over
//! `Server`; the integration test in `tests/` binds one on an ephemeral
//! port and plays a round through it.
//!
//! - `code`: room codes, five letters from the alphabet.
//! - `mailbox`: a seat's intents between the socket and the tick, the
//!   game crate's `net::mailbox`: an ordered jitter buffer for a
//!   server-driven seat, one intent a tick; a play point on the room's
//!   clock for a client that owns its hull, one client tick a room tick
//!   with every intent at or before it taken at once and its trigger
//!   merged press for press, the margin kept by a once-a-second
//!   controller, its pose dead-reckoned on a tick that has not arrived,
//!   and the reach the pose is believed to (`pose_reach_ticks`) and the
//!   intent a shot's press came on (`press_tick`, which `Fired` is
//!   stamped with) read off each tick's read.
//! - `room`: the room task - the lifecycle, the tick, the snapshots, the
//!   lobby.
//! - `conn`: one WebSocket connection - decode, route, write.
//! - `hub`: every room this server holds, and the drain.
//! - `metrics`: the Prometheus text on `/metrics`.
//! - `http`: the axum routers and `Server` - the public listener and the
//!   metrics listener beside it.
//! - `devserver` (feature `dev-tools`): the loopback JSON socket
//!   `bbmcp rooms` drives - never on the axum router.

pub mod code;
pub mod conn;
#[cfg(feature = "dev-tools")]
pub mod devserver;
pub mod http;
pub mod hub;
/// The seats' mailboxes are the game crate's (`net::mailbox`), so the rig
/// and this server hold a seat's intents by one rule.
pub use bongbong::net::mailbox;
pub mod metrics;
pub mod room;

pub use http::{Config, Server};
