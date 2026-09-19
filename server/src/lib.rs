//! The room server (docs/online-coop-prd.md §4.7): rooms that own an
//! authoritative `bongbong::simulation::Game` each and the WebSocket
//! connections that feed them intents and receive snapshots, plus the
//! HTTP surface an operator watches. `main.rs` is the command line over
//! `Server`; the integration test in `tests/` binds one on an ephemeral
//! port and plays a round through it.
//!
//! - `code`: room codes, one pod letter plus four from the alphabet.
//! - `mailbox`: a seat's newest intent, sampled by the tick.
//! - `room`: the room task - the lifecycle, the tick, the snapshots, the
//!   lobby.
//! - `conn`: one WebSocket connection - decode, route, write.
//! - `hub`: the pod's rooms, the drain.
//! - `metrics`: the Prometheus text on `/metrics`.
//! - `http`: the axum router and `Server`.

pub mod code;
pub mod conn;
pub mod http;
pub mod hub;
pub mod mailbox;
pub mod metrics;
pub mod room;

pub use http::{Config, Server};
