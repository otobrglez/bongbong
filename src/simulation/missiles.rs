//! The seeker missiles' world-facing half (`missile.rs` is the flight):
//! locking a missile at the top of its climb onto the nearest opposing
//! tank, keeping its aim on that tank while it chases, and bursting it in
//! a small blast where it comes down. No RNG outside the blast's damage
//! rolls, which are drawn only for what is inside the radius - a round
//! with no missiles in the air draws exactly what it did before.

use crate::tuning::tuning;
use hecs::Entity;
use crate::math::Vec2;

use crate::ai::Ai;
use crate::blast::{BlastFx, BlastKind, BlastShape, Lean, Scorch};
use crate::frog::{Frog, Side};
use crate::missile::Missile;
use crate::obstacle::{face_toward, Obstacle};
use crate::shell::Owner;
use crate::shockwave::Shockwave;
use crate::tank::Tank;
use crate::Position;

use super::combat::{explosion_hit, BlastParams};
use super::props::DamageCause;
use super::{Event, Frame, Game, SHOCK_FROG};

/// How hard one missile's burst shakes the screen, relative to a tank
/// dying (see `SHOCK_KILL`): a volley is four of these in quick succession,
/// so each is well short of a barrel.
pub(super) const SHOCK_MISSILE: f32 = 0.3;

impl BlastParams {
    /// One seeker missile's burst.
    pub fn missile() -> Self {
        BlastParams {
            radius: tuning().missile_blast_radius,
            damage: (tuning().missile_blast_damage_min, tuning().missile_blast_damage_max),
            knockback: tuning().missile_blast_knockback_speed,
        }
    }
}

impl Game {
    /// Every missile in the air, between frames: a missile done with its
    /// climb locks onto the nearest live tank on the other side within
    /// `missile_seek_range` of it (ties broken on slot), or - with none -
    /// keeps the launcher's aim point - either way offset by its tube
    /// (`Missile::impact_offset`) so a salvo lands spread out; a missile
    /// following a tank takes the tank's position plus that offset as its
    /// aim, and one whose tank has died dives on where it last aimed. No
    /// RNG.
    pub(super) fn guide_missiles(&mut self, f: &mut Frame) {
        // Everything a missile can lock: live tanks with a body (a wave tank
        // still rolling in has neither a body nor an `Ai`).
        let mut targets: Vec<(Entity, usize, Owner, Position)> = Vec::new();
        for player in self.players().into_iter().flatten() {
            super::with_tank(&self.world, player, |t| {
                if !t.is_wreck() {
                    targets.push((player, t.owner_slot(), t.owner(), t.position));
                }
            });
        }
        for (entity, tank) in self.world.query::<(Entity, &Tank)>().with::<&Ai>().iter() {
            if !tank.is_wreck() {
                targets.push((entity, tank.owner_slot(), tank.owner(), tank.position));
            }
        }
        targets.sort_by_key(|&(_, slot, _, _)| slot);
        let range = tuning().missile_seek_range;
        for missile in self.world.query::<&mut Missile>().iter() {
            if missile.wants_lock() {
                let nearest = targets
                    .iter()
                    .filter(|(_, _, owner, _)| !owner.same_side(missile.owner))
                    .map(|&(e, slot, _, pos)| (e, slot, pos, pos.distance_to(missile.position)))
                    .filter(|&(_, _, _, d)| d <= range)
                    // `targets` is in slot order and `min_by` keeps the
                    // first of equals, so a tie goes to the lower slot.
                    .min_by(|a, b| a.3.total_cmp(&b.3));
                missile.locked = true;
                let target = nearest.map(|(e, slot, pos, _)| {
                    missile.target = Some(e);
                    missile.aim = pos;
                    slot
                });
                // Each tube comes down a little beside the others.
                missile.aim_offset = missile.impact_offset(missile.aim);
                let spread = missile.aim + missile.aim_offset;
                missile.aim = Position::new(spread.x.clamp(0.0, f.width), spread.y.clamp(0.0, f.height));
                f.events.push(Event::MissileLocked { slot: missile.owner.slot(), target, x: missile.aim.x, y: missile.aim.y });
            } else if missile.tracking() {
                let target = missile.target.expect("a tracking missile has a target");
                match targets.iter().find(|&&(e, ..)| e == target) {
                    Some(&(_, _, _, pos)) => missile.aim = pos + missile.aim_offset,
                    // Wrecked or gone: come down where it last was.
                    None => missile.target = None,
                }
            }
        }
    }

    /// Burst every missile that reached the ground this frame
    /// (`Missile::arrived`) and remove it. `live` is false on the end
    /// screen, where the blast plays out without touching anything.
    pub(super) fn resolve_missiles(&mut self, f: &mut Frame, live: bool) {
        let landed: Vec<(Entity, Position, Owner, Vec2)> = self
            .world
            .query::<(Entity, &Missile)>()
            .iter()
            .filter(|(_, m)| m.arrived)
            .map(|(e, m)| (e, m.position, m.owner, m.dir))
            .collect();
        for (entity, center, owner, dir) in landed {
            self.world.despawn(entity).ok();
            self.missile_blast(f, center, owner, dir, live);
        }
    }

    /// One missile's burst at `center`, fired by `owner`, travelling `dir`
    /// as it came down. When `live`: the side opposing `owner` takes
    /// linear-falloff damage and every live tank in range is shoved (the
    /// wreck blast's rule - `explosion_hit` - so one draw per tank in
    /// range, players first, then enemies); frogs of the opposing side take
    /// damage; tiles crack and barrels go off (`damage_obstacle`, as any
    /// blast). Then the show: a small fireball leaning downrange, a ripple,
    /// a scorch and flattened grass.
    fn missile_blast(&mut self, f: &mut Frame, center: Position, owner: Owner, dir: Vec2, live: bool) {
        let params = BlastParams::missile();
        f.events.push(Event::MissileBlast { slot: owner.slot(), x: center.x, y: center.y });
        if live {
            for player in self.players().into_iter().flatten() {
                let mut q = self.world.query_one::<&mut Tank>(player);
                let tank = q.get().expect("player entity always has a Tank");
                let hurts = !owner.same_side(tank.owner());
                explosion_hit(tank, center, hurts, &mut self.physics, &mut f.rng, &mut f.kills, &params);
            }
            for tank in self.world.query::<&mut Tank>().with::<&Ai>().iter() {
                let hurts = !owner.same_side(tank.owner());
                explosion_hit(tank, center, hurts, &mut self.physics, &mut f.rng, &mut f.kills, &params);
            }
            let own_side = if owner.is_player() { Side::Player } else { Side::Enemy };
            let mut dead_frogs = Vec::new();
            for frog in self.world.query::<&mut Frog>().iter() {
                if frog.is_dead() || frog.side == own_side {
                    continue;
                }
                let dist = frog.position.distance_to(center);
                if dist > params.radius {
                    continue;
                }
                frog.damage(params.roll_damage(&mut f.rng) * (1.0 - dist / params.radius));
                if frog.is_dead() {
                    dead_frogs.push(frog.position);
                }
            }
            for pos in dead_frogs {
                f.shocks.push(Shockwave::scaled(pos, SHOCK_FROG));
            }
            // Collect first: `damage_obstacle` needs the world free.
            let hits: Vec<(Entity, f32)> = self
                .world
                .query::<(Entity, &mut Obstacle)>()
                .iter()
                .filter(|(_, o)| !o.destroyed)
                .filter_map(|(e, o)| {
                    let dist = o.position.distance_to(center);
                    if dist > params.radius {
                        return None;
                    }
                    if o.material.is_wall() {
                        o.scorched |= face_toward(o.position, center);
                    }
                    Some((e, 1.0 - dist / params.radius))
                })
                .collect();
            for (entity, falloff) in hits {
                let amount = params.roll_damage(&mut f.rng) * falloff;
                self.damage_obstacle(f, entity, amount, DamageCause::Blast { falloff, from: center });
            }
        }

        let lean = Lean { x: dir.x, y: dir.y };
        let mut fx = BlastFx::shaped(center, BlastKind::Oil, BlastShape::Shot { dir: lean });
        fx.scale *= tuning().missile_blast_fx_scale;
        f.blast_fx.push(fx);
        f.shocks.push(Shockwave::scaled(center, SHOCK_MISSILE));
        f.impact_flashes.push(Shockwave::new(center));
        if self.water.depth_at(center) == crate::ground::Depth::Dry {
            f.scorches.push(Scorch::with(center, tuning().missile_blast_fx_scale, None));
        }
        crate::grass::flatten(&mut self.grass, center, params.radius * tuning().blast_grass_flatten);
    }

    /// The missiles in the air, for the presentation: (a stable per-missile
    /// key, where its exhaust is, how high it is 0..=1).
    pub fn missiles(&self) -> Vec<(u32, Position, f32)> {
        self.world.query::<(Entity, &Missile)>().iter().map(|(e, m)| (e.id(), m.tail(), m.lift())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::Mission;
    use crate::map::MapFile;
    use crate::simulation::{with_tank, with_tank_mut, Input};
    use crate::tank::ActiveWeapon;

    const W: f32 = 1280.0;
    const H: f32 = 720.0;

    /// An open sandbox, player 1 at cell (3, 6), plus `extra` cells.
    fn sandbox(extra: &str) -> Game {
        let mut game = Game::default();
        game.seed_override = Some(7);
        game.level_overrides.mission = Some(Mission::Destroy);
        let map = format!("version = 1\ntanks = 0\ncells.\"3,6\" = {{ kind = \"start\" }}\n{extra}");
        game.map = MapFile::from_toml_str(&map).expect("test map parses");
        game.init(W, H);
        game
    }

    /// A parked enemy at `pos`: it cannot drive (no speed) or shoot (no
    /// shells), so where it stands is where the missiles have to go.
    fn parked_enemy(game: &mut Game, pos: Position) -> Entity {
        let slot = game.debug_spawn_enemy(pos, None, None).expect("spawns");
        let entity = game.tank_entity_by_slot(slot).expect("exists");
        with_tank_mut(&game.world, entity, |t| {
            t.speed_scale = 0.0;
            t.shells_ammo = 0;
            t.weapon_queue.clear();
        });
        entity
    }

    fn player(game: &Game) -> Entity {
        game.player().expect("player")
    }

    /// Hand player 1 `count` missiles, facing right.
    fn arm(game: &mut Game, count: i32) {
        with_tank_mut(&game.world, player(game), |t| {
            t.enqueue_weapon(ActiveWeapon::Missiles);
            t.missile_ammo += count;
            t.rotation = 90.0;
        });
    }

    fn fire(game: &mut Game) -> Vec<Event> {
        let mut input = Input::default();
        input.seats[0].fire = true;
        game.update(input, crate::PHYSICS_FIXED_DT, W, H);
        game.events().to_vec()
    }

    fn idle(game: &mut Game, frames: usize) -> Vec<Event> {
        let mut seen = Vec::new();
        for _ in 0..frames {
            game.update(Input::default(), crate::PHYSICS_FIXED_DT, W, H);
            seen.extend(game.events().iter().cloned());
        }
        seen
    }

    fn blasts(events: &[Event]) -> Vec<Position> {
        events
            .iter()
            .filter_map(|e| match *e {
                Event::MissileBlast { x, y, .. } => Some(Position::new(x, y)),
                _ => None,
            })
            .collect()
    }

    fn missiles_in_air(game: &Game) -> usize {
        game.world.query::<&Missile>().iter().count()
    }

    #[test]
    fn a_trigger_pull_fires_two_salvos_of_four() {
        let mut game = sandbox("");
        arm(&mut game, 24);
        let events = fire(&mut game);
        assert!(events.iter().any(|e| matches!(e, Event::Fired { weapon: "missiles", .. })));
        assert_eq!(missiles_in_air(&game), 1, "the first leaves at once");
        // Watch the pod frame by frame: it empties through the first
        // salvo, holds empty through the gap, reloads and empties again.
        let tubes = |game: &Game| with_tank(&game.world, player(game), |t| t.missile_tubes_empty);
        let mut counts = Vec::new();
        let mut tube_readings = Vec::new();
        for _ in 0..40 {
            idle(&mut game, 1);
            counts.push(missiles_in_air(&game));
            tube_readings.push(tubes(&game));
        }
        let first_four = counts.iter().position(|&n| n == 4).expect("the first salvo completes");
        assert!(counts[first_four..].iter().take(8).all(|&n| n == 4), "a gap before the second salvo: {counts:?}");
        assert_eq!(*counts.last().unwrap(), 8, "then four more: {counts:?}");
        let reload = tube_readings.windows(2).position(|w| w[0] == 4 && w[1] < 4).expect("the pod reloads between salvos");
        assert_eq!(tube_readings[reload + 1..].iter().max(), Some(&4), "and empties again: {tube_readings:?}");
        assert_eq!(with_tank(&game.world, player(&game), |t| t.missile_ammo), 16);
        idle(&mut game, 240);
        assert_eq!(tubes(&game), 0, "reloaded for the next pull");
    }

    #[test]
    fn missiles_lock_onto_the_enemy_and_hurt_it() {
        let mut game = sandbox("");
        let enemy = parked_enemy(&mut game, Position::new(15.0 * 32.0 + 16.0, 6.0 * 32.0 + 16.0));
        let slot = with_tank(&game.world, enemy, |t| t.owner_slot());
        arm(&mut game, 4);
        let mut events = fire(&mut game);
        events.extend(idle(&mut game, 400));
        let locks: Vec<&Event> = events.iter().filter(|e| matches!(e, Event::MissileLocked { .. })).collect();
        assert_eq!(locks.len(), 4, "four missiles armed, four fired: {locks:?}");
        assert!(locks.iter().all(|e| matches!(e, Event::MissileLocked { target: Some(s), .. } if *s == slot)), "{locks:?}");
        let at = with_tank(&game.world, enemy, |t| t.position);
        let blasts = blasts(&events);
        assert_eq!(blasts.len(), 4);
        // Spread by tube, but each well inside its blast of the hull.
        let reach = 1.5 * tuning().missile_impact_spread_px + 1.0;
        assert!(blasts.iter().all(|&b| b.distance_to(at) <= reach), "{blasts:?} vs {at:?}");
        let spread = blasts.iter().map(|b| b.distance_to(blasts[0])).fold(0.0f32, f32::max);
        assert!(spread >= 2.0 * tuning().missile_impact_spread_px, "the salvo lands spread out: {blasts:?}");
        assert!(with_tank(&game.world, enemy, |t| t.damage) > 0.0, "the enemy took the blasts");
        assert_eq!(with_tank(&game.world, player(&game), |t| t.damage), 0.0);
        assert_eq!(missiles_in_air(&game), 0, "all spent");
    }

    #[test]
    fn missiles_fly_over_walls() {
        // A brick wall straight between the shooter and the enemy: a shell
        // would stop on it; a missile goes over and the wall is untouched.
        let wall = "cells.\"9,5\" = { kind = \"wall\", material = \"brick\" }\ncells.\"9,6\" = { kind = \"wall\", material = \"brick\" }\ncells.\"9,7\" = { kind = \"wall\", material = \"brick\" }\n";
        let mut game = sandbox(wall);
        let enemy = parked_enemy(&mut game, Position::new(15.0 * 32.0 + 16.0, 6.0 * 32.0 + 16.0));
        let wall_health = |game: &Game| -> Vec<String> {
            game.world.query::<&Obstacle>().iter().filter(|o| o.material.is_wall()).map(|o| format!("{:?}", o.health)).collect()
        };
        let before = wall_health(&game);
        arm(&mut game, 4);
        fire(&mut game);
        idle(&mut game, 400);
        assert!(with_tank(&game.world, enemy, |t| t.damage) > 0.0, "reached the enemy behind the wall");
        assert_eq!(before, wall_health(&game), "flew over it");
    }

    #[test]
    fn with_nothing_in_range_they_come_down_on_the_aim_point() {
        let mut game = sandbox("");
        arm(&mut game, 4);
        let from = with_tank(&game.world, player(&game), |t| t.position);
        let mut events = fire(&mut game);
        events.extend(idle(&mut game, 400));
        assert!(events.iter().any(|e| matches!(e, Event::MissileLocked { target: None, .. })));
        let blasts = blasts(&events);
        assert_eq!(blasts.len(), 4);
        let aim = Position::new(from.x + tuning().missile_fallback_range, from.y);
        let reach = 1.5 * tuning().missile_impact_spread_px + 1.0;
        assert!(blasts.iter().all(|&b| b.distance_to(aim) <= reach), "{blasts:?} vs {aim:?}");
        assert_eq!(with_tank(&game.world, player(&game), |t| t.damage), 0.0, "never hurt by its own missiles");
    }

    #[test]
    fn a_tank_that_keeps_moving_can_slip_the_dive() {
        // The dive commits to where the target was: a missile does not
        // follow a tank out from under it.
        let mut game = sandbox("");
        let enemy = parked_enemy(&mut game, Position::new(12.0 * 32.0 + 16.0, 6.0 * 32.0 + 16.0));
        arm(&mut game, 1);
        fire(&mut game);
        let mut committed = None;
        for _ in 0..400 {
            idle(&mut game, 1);
            let dive = game.world.query::<&Missile>().iter().find(|m| m.stage == crate::missile::MissileStage::Dive).map(|m| m.aim);
            if let Some(aim) = dive {
                committed = Some(aim);
                break;
            }
        }
        let aim = committed.expect("reached the dive");
        // Pull the enemy well clear the moment the dive commits.
        let far = Position::new(aim.x, aim.y + 200.0);
        let slot = with_tank(&game.world, enemy, |t| t.owner_slot());
        game.debug_teleport(slot, far, None).expect("teleports");
        let events = idle(&mut game, 200);
        let blasts = blasts(&events);
        assert_eq!(blasts.len(), 1);
        assert!(blasts[0].distance_to(aim) < 1.0, "burst on the committed spot");
        assert_eq!(with_tank(&game.world, enemy, |t| t.damage), 0.0, "and missed");
    }

    #[test]
    fn a_missile_volley_replays_bit_for_bit() {
        let run = || {
            let mut game = sandbox("");
            parked_enemy(&mut game, Position::new(15.0 * 32.0 + 16.0, 4.0 * 32.0 + 16.0));
            arm(&mut game, 8);
            let mut events = fire(&mut game);
            events.extend(idle(&mut game, 300));
            let tanks: Vec<(usize, f32, f32, f32)> =
                game.tank_snapshots().iter().map(|t| (t.slot, t.position.x, t.position.y, t.damage)).collect();
            (format!("{events:?}"), format!("{tanks:?}"))
        };
        assert_eq!(run(), run());
    }
}
