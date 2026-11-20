//! The round's last enemies are called to the fight
//! (docs/levels.md "A round that cannot stall"): once no enemy is still to
//! come and `straggler_call_alive` or fewer are left, a lull of
//! `straggler_call_after_seconds` - no live enemy with a seat or the
//! players' bong in its sight, none freshly hit - calls every one of them
//! to the nearest live seat (`FieldMind::called`, `field::call_target`).
//! That wakes a band tank nothing ever woke on a field map, takes a tank
//! off its home leash, and on an arena heads the last tanks at a seat
//! their shared alert has lost. The call ends as a wave's does: within
//! sight range of a seat, or on a hit; another lull calls again.
//!
//! Only where wrecks win the round - a Protect or Destroy round with
//! enemies in its plan, never a training script - and a guard keeping its
//! live bong is never called off it. No RNG; a round that never falls
//! quiet at its end replays as it did.

use hecs::Entity;

use crate::ai::{Ai, Role};
use crate::level::{Mission, SpawnPlan};
use crate::tank::Tank;
use crate::tuning::tuning;
use crate::Position;

use super::{with_bong, with_tank, Event, Frame, Game};

/// The straggler rule's memory for one round.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Stragglers {
    /// Seconds the round's last enemies have gone out of touch with the
    /// fight (`Game::straggler_phase`).
    lull: f32,
}

impl Game {
    /// Seconds the round's last enemies have been out of touch with the
    /// fight; 0 while the rule is not counting.
    pub fn straggler_lull(&self) -> f32 {
        self.stragglers.lull
    }

    /// Whether the round is won by wrecking its enemies and has some: a
    /// Protect or Destroy round, not a sandbox (`band_enemy_count` 0) and
    /// not a training script.
    fn won_by_wrecks(&self) -> bool {
        let sandbox = matches!(self.spawn_plan, SpawnPlan::Band { .. }) && self.band_enemy_count == 0;
        self.mission != Mission::Hunt && self.training.is_none() && !sandbox
    }

    /// The straggler rule, run before the enemies think - see the module
    /// doc. Enemies are walked in owner-slot order.
    pub(super) fn straggler_phase(&mut self, f: &mut Frame) {
        let (alive_max, after) = {
            let t = tuning();
            (t.straggler_call_alive, t.straggler_call_after_seconds)
        };
        if alive_max == 0 || !self.won_by_wrecks() || !self.spawn_plan_finished() {
            self.stragglers.lull = 0.0;
            return;
        }
        // The live seats on the field, each with how far it is seen
        // (`sight_on`), and the players' bong.
        let seats: Vec<(Position, f32)> = self
            .seats_on_field()
            .into_iter()
            .flatten()
            .filter_map(|e| with_tank(&self.world, e, |t| (!t.is_wreck()).then_some(t.position)))
            .map(|p| (p, self.sight_on(p)))
            .collect();
        let live: Vec<Position> = seats.iter().map(|&(p, _)| p).collect();
        let bong = self.bong_position();
        let view_range = self.enemy_sight();
        let guarding = self.enemy_bong.is_some_and(|e| with_bong(&self.world, e, |b| !b.is_dead()));
        let field_map = self.field_map;

        let mut last: Vec<(usize, Entity)> =
            self.world.query::<(Entity, &Tank)>().with::<&Ai>().iter().filter(|(_, t)| !t.is_wreck()).map(|(e, t)| (t.owner_slot(), e)).collect();
        if last.is_empty() || last.len() > alive_max || live.is_empty() {
            self.stragglers.lull = 0.0;
            return;
        }
        last.sort_by_key(|&(slot, _)| slot);

        let mut contact = false;
        for &(_, entity) in &last {
            let position = with_tank(&self.world, entity, |t| t.position);
            let mut ai = self.world.get::<&mut Ai>(entity).expect("queried with an Ai");
            let hit = ai.is_hit_alerted();
            let near = live.iter().any(|&s| s.distance_to(position) <= view_range);
            contact |= hit
                || seats.iter().any(|&(s, sight)| s.distance_to(position) <= sight)
                || bong.is_some_and(|b| b.distance_to(position) <= view_range);
            // An arena ends the call here; a field map's alert pass does
            // (`Game::field_alerts`).
            if !field_map && ai.field.called && (hit || near) {
                ai.field.called = false;
            }
        }
        if contact {
            self.stragglers.lull = 0.0;
            return;
        }
        self.stragglers.lull += f.dt;
        if self.stragglers.lull < after {
            return;
        }
        self.stragglers.lull = 0.0;
        let mut count = 0;
        for &(_, entity) in &last {
            let mut ai = self.world.get::<&mut Ai>(entity).expect("queried with an Ai");
            if ai.field.called || (ai.role == Role::Guard && guarding) {
                continue;
            }
            ai.field.called = true;
            count += 1;
        }
        if count > 0 {
            f.events.push(Event::StragglersCalled { count });
        }
    }
}
