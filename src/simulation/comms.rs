//! The pack's vocabulary: what one enemy can tell the others, and what the
//! commander can tell one enemy.
//!
//! Split out from [`super::command`] on purpose. This file is data with no
//! logic and no dependencies beyond `Position` and the enum types the orders
//! name; `command.rs` is the thing that reads facts and decides orders.
//! Adding a capability later - "reserve this corridor", "fall back" - is a
//! variant here plus a producer there, and nothing else in the game moves.
//! That separation is the whole reason this is two files rather than one
//! (docs/enemy-command-and-control-prd.md section 4).
//!
//! Two kinds of information, with different lifetimes and different
//! addressing, which is why there are two types rather than one bus:
//!
//! - A [`Signal`] is a *fact*, broadcast and unaddressed - "player 1 was at
//!   (x, y) half a second ago". Facts are read by every producer and would
//!   have to be re-sent to every unit every frame if they were directives.
//! - An [`Order`] is a *directive*, addressed to exactly one tank for exactly
//!   one frame - "slot 3, ease off". Directives need a recipient and a
//!   priority; facts need neither.
//!
//! Everything here is keyed by `Tank::owner_slot` rather than by `Entity`.
//! Slots are stable for a whole round (never reused - see
//! `waves::take_slot`), `Ord`, and already the codebase's canonical
//! deterministic key: `EngageReport::tanks` sorts by it and `ram_enemy_pairs`
//! walks by it. A `BTreeMap` keyed that way is safe to iterate by
//! construction, so the "never iterate a `HashMap` where the body breaks
//! ties" rule cannot be violated here by accident.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::pickup::PickupKind;
use crate::tank::Dir;

/// Who a fact is about, or who an order is addressed to.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize)]
#[serde(tag = "side", rename_all = "snake_case")]
pub(crate) enum Unit {
    /// A human player, 0 or 1. The collect pass feeds the commander enemies
    /// only for now (Phase 1 of docs/enemy-command-and-control-prd.md builds the deconflictor only), so
    /// nothing builds one outside the tests; the right-of-way rules already
    /// read it.
    #[allow(dead_code)]
    Player(u8),
    /// An enemy, by `Tank::owner_slot`.
    Enemy(usize),
}

/// Where a player was last seen, and how stale that is.
#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct Sighting {
    /// Plain fields rather than a `Position`: this type is `Serialize` for
    /// the command report, and `Position` (`Vec2`) is not - the
    /// same reason `DebugSnapshot` carries x/y.
    pub x: f32,
    pub y: f32,
    /// Seconds since the sighting.
    pub age: f32,
}

/// A stretch of ground one unit has spoken for. Nothing issues these yet;
/// the deconflictor already refuses to sidestep into one, so a formation
/// producer can start reserving lanes without that code changing.
#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct Lane {
    pub by: Unit,
    pub from_x: f32,
    pub from_y: f32,
    pub to_x: f32,
    pub to_y: f32,
    /// Seconds of life left.
    pub ttl: f32,
}

/// A live pickup's stable identity. Pickups never move, so the kind plus the
/// cell it sits in is a key that survives across frames - the per-frame
/// snapshot `enemy_phase` builds carries no `Entity`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize)]
pub(crate) struct PickupKey {
    pub kind: PickupKind,
    pub col: i32,
    pub row: i32,
}

/// Who is going for a pickup, and why.
#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct Claim {
    pub by: usize,
    /// Seconds the claim has been held - the stickiness clock.
    pub held: f32,
    /// Claimed to take it off the player rather than out of need.
    pub contested: bool,
}

/// A fact the pack knows this frame.
///
/// Deliberately an enum even though [`Blackboard`] stores most facts in typed
/// fields: a signal is something a *unit* reports, and the enum is what a
/// future "who told us this" audit trail would carry. The blackboard is the
/// digested form.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "signal", rename_all = "snake_case")]
#[allow(dead_code)] // no unit reports one yet - the Phase 2 producers do (see the PRD)
pub(crate) enum Signal {
    /// A unit can see a player right now.
    Spotted { by: Unit, player: u8, x: f32, y: f32 },
    /// A unit was hit from somewhere.
    TookFire { by: Unit, x: f32, y: f32 },
    /// A unit is in hull contact with another.
    Contact { by: Unit, with: Unit },
}

/// What the commander tells one tank to do.
///
/// Split by *when it takes effect*, which is not cosmetic. The reflex orders
/// are applied to an intent the behaviour tree has already produced, this
/// frame, and need no cooperation from `ai.rs` at all. The goal orders are
/// read by the *next* frame's `Ai::think` through the channel `engage_target`
/// already uses. Goals are slow on purpose: the tree sits behind
/// `ai_dir_hold_seconds` and the engagement ring's stickiness, so a goal that
/// could change every frame is exactly the flip-flop that hysteresis exists
/// to prevent.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "order", rename_all = "snake_case")]
pub(crate) enum Order {
    // --- reflex: applied to this frame's intent ---
    /// Ease off the throttle, 0 (no change) to 1 (stop). Becomes
    /// `Intent::slow`.
    Slow { by: f32 },
    /// Stop driving this frame; keep the facing, so the tank still aims and
    /// shoots.
    Hold,
    /// Drive this cardinal instead of the one the tree picked.
    #[allow(dead_code)] // applied by `Commander::apply`, issued by no producer yet
    Nudge { dir: Dir },
    /// Contact damage against `victim` is authorised - and the deconflictor
    /// leaves this pair alone rather than preventing the very ram that was
    /// ordered.
    #[allow(dead_code)] // applied by `Commander::apply`, issued by no producer yet
    Ram { victim: Unit },
    // --- goal: read by the next frame's `think` ---
    /// Walk at this point instead of the engagement-ring slot.
    #[allow(dead_code)] // applied by `Commander::apply`, issued by no producer yet
    Goto { x: f32, y: f32 },
    /// Go and collect this.
    #[allow(dead_code)] // applied by `Commander::apply`, issued by no producer yet
    Fetch { what: PickupKey },
    /// Concentrate on this player.
    #[allow(dead_code)] // applied by `Commander::apply`, issued by no producer yet
    Focus { player: u8 },
    /// Do not fire this frame.
    #[allow(dead_code)] // applied by `Commander::apply`, issued by no producer yet
    HoldFire,
}

/// What the pack knows, and what it has spoken for.
///
/// Facts live in typed fields rather than a `Vec<Signal>` because a fact is
/// always queried *by kind* - nothing ever scans for "any fact" - so a struct
/// is both cheaper and impossible to mis-match. A new kind of fact is a new
/// field here plus its writer.
#[derive(Clone, Default, Debug, Serialize)]
pub(crate) struct Blackboard {
    /// Last known position of each player.
    pub spotted: [Option<Sighting>; 2],
    /// What fraction of the live pack is engaged on each player. The input a
    /// focus-fire or pincer producer reads; nothing writes it yet.
    pub threat: [f32; 2],
    /// Mean health of the live enemies, scaled down while outnumbered. The
    /// input a retreat producer reads; nothing writes it yet.
    pub morale: f32,
    /// Ground spoken for this frame, sorted by holder. Nothing writes it yet.
    pub lanes: Vec<Lane>,
    /// One claimant per pickup.
    pub claims: BTreeMap<PickupKey, Claim>,
}

impl Blackboard {
    /// Drop the per-frame facts, keeping the ones with their own lifetime
    /// (claims, and lanes until their `ttl` runs out).
    pub fn begin_frame(&mut self, dt: f32) {
        self.lanes.retain_mut(|lane| {
            lane.ttl -= dt;
            lane.ttl > 0.0
        });
        for claim in self.claims.values_mut() {
            claim.held += dt;
        }
    }
}
