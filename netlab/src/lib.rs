//! netlab: how far networked play is from local play, per metric, on a
//! given link (docs/online-coop-prd.md §4.16, "Measurement first";
//! netlab/README.md).
//!
//! A run puts the real room server in this process behind an impairment
//! proxy that models TCP (`link`, `proxy`), reads the WebSocket stream
//! going through it back into protocol messages (`wstap`), plays two
//! scripted seats through the window's own `OnlineRound` over the real
//! `NativeTransport` (`client`, `script`), plays the same scripts through a
//! local two-seat round (`twin`), and measures both the same way
//! (`sample`, `metrics`) into a report with a verdict (`report`). `suite`
//! sweeps link profiles and scenarios into one table.

pub mod client;
pub mod link;
pub mod metrics;
pub mod proxy;
pub mod report;
pub mod run;
pub mod sample;
pub mod script;
pub mod suite;
pub mod twin;
pub mod wstap;
