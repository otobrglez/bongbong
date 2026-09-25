//! The room server (docs/online-coop-prd.md §4.7): rooms that own an
//! authoritative `bongbong::simulation::Game` each and the WebSocket
//! connections that feed them intents and receive snapshots, plus the
//! HTTP surface an operator watches. `main.rs` is the command line over
//! `Server`; the integration test in `tests/` binds one on an ephemeral
//! port and plays a round through it.
//!
//! - `code`: room codes, five letters from the alphabet.
//! - `mailbox`: a seat's newest intent, sampled by the tick.
//! - `room`: the room task - the lifecycle, the tick, the snapshots, the
//!   lobby.
//! - `conn`: one WebSocket connection - decode, route, write.
//! - `hub`: every room this server holds, and the drain.
//! - `metrics`: the Prometheus text on `/metrics`.
//! - `http`: the axum router and `Server`.
//! - `devserver` (feature `dev-tools`): the loopback JSON socket
//!   `bbmcp rooms` drives - never on the axum router.

pub mod code;
pub mod conn;
#[cfg(feature = "dev-tools")]
pub mod devserver;
pub mod http;
pub mod hub;
/// The seats' jitter buffers are the game crate's (`net::mailbox`), so the
/// rig and this server hold a seat's intents by one rule.
pub use bongbong::net::mailbox;
pub mod metrics;
pub mod room;

pub use http::{Config, Server};
