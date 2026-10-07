//! The grenades' world half (`grenade.rs` is the ball): rolling every
//! grenade on the ground among the tiles, the walls and the hulls each
//! tick, and setting off the ones whose fuse ran out - the side opposing
//! the launcher hurt, everything in range shoved, tiles cracked, and a
//! shockwave. No RNG outside the blast's damage rolls, which are drawn
//! only for what is inside the radius, so a round with no grenade on the
//! ground draws exactly what it did before.

use hecs::Entity;

use crate::ai::Ai;
use crate::blast::{BlastFx, BlastKind, BlastShape, Scorch};
use crate::ground::Depth;
use crate::grenade::{Grenade, Ground, Surroundings};
use crate::math::Vec2;
use crate::shell::Owner;
use crate::shockwave::Shockwave;
use crate::tank::Tank;
use crate::tuning::tuning;
use crate::Position;

use super::combat::BlastParams;
use super::{Event, Frame, Game, Spectacle};

impl BlastParams {
    /// One grenade's blast, launched by `by`.
    pub fn grenade(by: Owner) -> Self {
        let t = tuning();
        BlastParams {
            radius: t.grenade_blast_radius,
            damage: (t.grenade_blast_damage_min, t.grenade_blast_damage_max),
            knockback: t.grenade_blast_knockback_speed,
            by: Some(by),
        }
    }
}

impl Game {
    /// One tick for every grenade (`Grenade::roll`): in the air over
    /// everything but the field's walls, on the ground among this frame's
    /// tiles and walls and every tank with a body - a live one moving at its body's velocity, a
    /// wreck still - after the world stepped, so a hull that drove into a
    /// grenade this tick hands it its motion. Also runs on the end screen,
    /// where a grenade keeps rolling and its blast hurts nothing.
    pub(super) fn roll_grenades(&mut self, f: &mut Frame) {
        if self.world.query::<&Grenade>().iter().next().is_none() {
            return;
        }
        let mut around = Surroundings { solids: f.terrain.tile_boxes(), edges: f.terrain.edge_boxes(), hulls: Vec::new() };
        let mut add = |tank: &Tank, physics: &crate::physics::Physics| {
            let Some(body) = tank.body else { return };
            let moving = if tank.is_wreck() { Vec2::zero() } else { physics.velocity(body) };
            let (center, half) = tank.hull_bbox_world();
            around.hulls.push((center, half, moving));
        };
        for player in self.players().into_iter().flatten() {
            super::with_tank(&self.world, player, |t| add(t, &self.physics));
        }
        for tank in self.world.query::<&Tank>().with::<&Ai>().iter() {
            add(tank, &self.physics);
        }
        let t = crate::tuning::tuning();
        let (wells, now) = (&f.wells, self.time);
        let zones = &self.zones;
        for grenade in self.world.query::<&mut Grenade>().iter() {
            // On a well's ring (docs/gravity-well.md "Grenades"): placed by
            // the clock, its fuse burning on; a ring whose well is gone
            // lets it go where it is.
            if let Some(orbit) = grenade.orbit {
                if let Some(zone) = zones.iter().find(|z| z.id == orbit.well) {
                    let to = crate::well::orbit_at(&orbit, zone.centre, now, &t);
                    let moved = to - grenade.position;
                    let distance = moved.length();
                    if distance > 0.01 {
                        grenade.heading = moved * (1.0 / distance);
                        grenade.roll += distance / t.grenade_radius;
                    }
                    grenade.position = to;
                    grenade.velocity = Vec2::zero();
                    grenade.height = 0.0;
                    grenade.climb = 0.0;
                    grenade.fuse -= f.dt;
                    continue;
                }
                grenade.orbit = None;
            }
            if !wells.is_empty() {
                if let Some((id, centre, _)) = wells.strongest(grenade.position, &t)
                    && grenade.position.distance_to(centre) <= t.well_ring_px
                {
                    let off = grenade.position - centre;
                    let bearing = off.y.atan2(off.x);
                    grenade.orbit = Some(crate::well::GrenadeOrbit { well: id, bearing, since: now });
                    grenade.velocity = Vec2::zero();
                    grenade.height = 0.0;
                    grenade.climb = 0.0;
                    grenade.fuse -= f.dt;
                    continue;
                }
            }
            let pull = if wells.is_empty() { Vec2::zero() } else { wells.pull(grenade.position, &t) * t.well_grenade_pull };
            let ground = match self.water.depth_at(grenade.position) {
                Depth::Dry => Ground::Dry,
                Depth::Ice => Ground::Ice,
                Depth::Shallow | Depth::Deep => Ground::Water,
            };
            grenade.roll_pulled(f.dt, &around, ground, pull);
        }
    }

    /// Set off every grenade whose fuse ran out and remove it, in id
    /// order. `live` is false on the end screen, where the blast plays out
    /// without touching anything.
    pub(super) fn resolve_grenades(&mut self, f: &mut Frame, live: bool) {
        let mut spent: Vec<(u32, Entity, Position, Owner)> = self
            .world
            .query::<(Entity, &Grenade)>()
            .iter()
            .filter(|(_, g)| g.spent())
            .map(|(e, g)| (g.id, e, g.position, g.owner))
            .collect();
        spent.sort_by_key(|&(id, ..)| id);
        for (_, entity, center, owner) in spent {
            self.world.despawn(entity).ok();
            f.events.push(Event::GrenadeBlast { slot: owner.slot(), x: center.x, y: center.y });
            if live {
                self.side_blast(f, center, owner, &BlastParams::grenade(owner));
            }
            let mut show = Spectacle::default();
            self.grenade_show(&mut show, center);
            f.stage(show);
        }
    }

    /// The grenades laying smoke, for the presentation: (a stable key,
    /// where the ball is drawn) for each one in the air or rolling faster
    /// than `grenade_trail_min_speed`.
    pub fn grenade_trails(&self) -> Vec<(u32, Position)> {
        let fast = tuning().grenade_trail_min_speed;
        self.world
            .query::<&Grenade>()
            .iter()
            .filter(|g| g.airborne() || g.velocity.length() > fast)
            .map(|g| (g.id, g.draw_pos()))
            .collect()
    }

    /// The show a grenade going off at `center` puts on: a big round fireball,
    /// its shockwave rippling the screen, the impact flash, the screen's
    /// flash, a scorch on dry ground and flattened grass. Everything but
    /// the damage, which is why a replica can call it off
    /// `Event::GrenadeBlast`. No RNG.
    pub(crate) fn grenade_show(&mut self, show: &mut Spectacle, center: Position) {
        let t = tuning();
        let mut fx = BlastFx::shaped(center, BlastKind::Oil, BlastShape::Plain);
        fx.scale *= t.grenade_blast_fx_scale;
        show.blast_fx.push(fx);
        show.shocks.push(Shockwave::scaled(center, t.grenade_shock));
        show.impact_flashes.push(Shockwave::new(center));
        if self.water.depth_at(center) == Depth::Dry {
            show.scorches.push(Scorch::with(center, t.grenade_blast_fx_scale, None));
        }
        self.flash_screen();
        crate::grass::flatten(&mut self.grass, center, t.grenade_blast_radius * t.blast_grass_flatten);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::Mission;
    use crate::map::MapFile;
    use crate::simulation::{with_tank, with_tank_mut, Input};

    const W: f32 = 1280.0;
    const H: f32 = 720.0;
    const DT: f32 = crate::PHYSICS_FIXED_DT;

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

    fn player(game: &Game) -> Entity {
        game.player().expect("player")
    }

    /// Hand player 1 a drum of grenades, facing right.
    fn arm(game: &mut Game) {
        with_tank_mut(&game.world, player(game), |t| {
            t.disarm();
            t.take_weapon(crate::tank::ActiveWeapon::Grenades);
            t.rotation = 90.0;
        });
    }

    fn step(game: &mut Game, fire: bool) -> Vec<Event> {
        let mut input = Input::default();
        input.seats[0].fire = fire;
        game.update(input, DT, W, H);
        game.events().to_vec()
    }

    fn grenades(game: &Game) -> Vec<(Position, Vec2)> {
        game.world.query::<&Grenade>().iter().map(|g| (g.position, g.velocity)).collect()
    }

    fn parked_enemy(game: &mut Game, pos: Position) -> Entity {
        let slot = game.debug_spawn_enemy(pos, None, None).expect("spawns");
        let entity = game.tank_entity_by_slot(slot).expect("exists");
        with_tank_mut(&game.world, entity, |t| {
            t.speed_scale = 0.0;
            t.shells_ammo = 0;
            t.disarm();
        });
        entity
    }

    /// A map places a grenade crate like any other pickup, and driving
    /// over it loads the drum.
    #[test]
    fn a_map_places_a_grenade_crate_and_driving_over_it_loads_the_drum() {
        let mut game = sandbox("cells.\"5,6\" = { kind = \"pickup\", pickup = \"grenades\" }\n");
        let text = game.map.to_toml_string().expect("writes back");
        assert!(text.contains("grenades"), "{text}");
        with_tank_mut(&game.world, player(&game), |t| t.rotation = 90.0);
        let mut taken = false;
        for _ in 0..120 {
            let mut input = Input::default();
            input.seats[0].move_dir = Some(crate::tank::Dir::Right);
            game.update(input, DT, W, H);
            taken |= game.events().iter().any(|e| matches!(e, Event::PickupCollected { kind: crate::pickup::PickupKind::Grenades, .. }));
        }
        assert!(taken, "drove over the crate");
        assert_eq!(with_tank(&game.world, player(&game), |t| t.grenade_ammo), tuning().grenade_ammo_per_pickup);
    }

    #[test]
    fn a_crate_loads_six_and_each_press_lobs_one() {
        let mut game = sandbox("");
        arm(&mut game);
        assert_eq!(with_tank(&game.world, player(&game), |t| t.grenade_ammo), tuning().grenade_ammo_per_pickup);
        assert_eq!(tuning().grenade_ammo_per_pickup, 6);
        assert_eq!(with_tank(&game.world, player(&game), |t| t.active_weapon()), crate::tank::ActiveWeapon::Grenades);
        let launched = |events: &[Event]| events.iter().filter(|e| matches!(e, Event::Fired { weapon: "grenades", .. })).count();
        let mut fired = launched(&step(&mut game, true));
        assert_eq!(fired, 1);
        // Holding the trigger does not lob another.
        for _ in 0..60 {
            fired += launched(&step(&mut game, true));
        }
        assert_eq!(fired, 1, "one per press");
        // Press after press, a press a second: a crate's worth, then shells.
        for _ in 0..8 {
            fired += launched(&step(&mut game, false));
            for _ in 0..60 {
                fired += launched(&step(&mut game, true));
            }
        }
        assert_eq!(fired, 6, "a crate's worth");
        assert_eq!(with_tank(&game.world, player(&game), |t| (t.grenade_ammo, t.active_weapon())), (0, crate::tank::ActiveWeapon::Shell));
    }

    #[test]
    fn it_is_lobbed_ahead_and_goes_off_when_the_fuse_runs_out() {
        let mut game = sandbox("");
        let from = with_tank(&game.world, player(&game), |t| t.position);
        arm(&mut game);
        step(&mut game, true);
        let (at, v) = grenades(&game)[0];
        assert!(at.x > from.x && v.x > 0.0, "lobbed ahead: {at:?} {v:?}");
        assert!(game.world.query::<&Grenade>().iter().all(|g| g.airborne()), "up in the air");
        let fuse = (tuning().grenade_fuse_seconds / DT).ceil() as usize;
        let mut blasts = Vec::new();
        for frame in 0..fuse + 2 {
            for e in step(&mut game, false) {
                if let Event::GrenadeBlast { x, y, .. } = e {
                    blasts.push((frame, Position::new(x, y)));
                }
            }
        }
        assert_eq!(blasts.len(), 1, "{blasts:?}");
        assert!(blasts[0].0 + 3 >= fuse, "not before its fuse: {blasts:?}");
        assert!(grenades(&game).is_empty());
        assert_eq!(with_tank(&game.world, player(&game), |t| t.damage), 0.0, "its own blast spares the launcher");
    }

    #[test]
    fn the_blast_hurts_and_shoves_an_enemy_beside_it() {
        let mut game = sandbox("");
        let enemy = parked_enemy(&mut game, Position::new(12.0 * 32.0 + 16.0, 6.0 * 32.0 + 16.0));
        let near = with_tank(&game.world, enemy, |t| t.position) + Vec2::new(-30.0, 0.0);
        let mut g = Grenade::launch(near, Vec2::new(1.0, 0.0), Vec2::zero(), Owner::Player(0));
        (g.velocity, g.height, g.climb) = (Vec2::zero(), 0.0, 0.0);
        g.fuse = DT * 0.5;
        game.world.spawn((g,));
        let events = step(&mut game, false);
        assert!(events.iter().any(|e| matches!(e, Event::GrenadeBlast { .. })));
        assert!(with_tank(&game.world, enemy, |t| t.damage) > 0.0, "the enemy took the blast");
    }

    #[test]
    fn it_flies_over_a_near_wall_and_bounces_off_a_far_one() {
        // A wall right ahead of the launcher, and another beyond where the
        // lob comes down: the first is flown over, the second rolled into.
        let mut walls = String::new();
        for row in 4..=8 {
            walls.push_str(&format!("cells.\"5,{row}\" = {{ kind = \"wall\", material = \"iron\" }}\n"));
            walls.push_str(&format!("cells.\"11,{row}\" = {{ kind = \"wall\", material = \"iron\" }}\n"));
        }
        let mut game = sandbox(&walls);
        arm(&mut game);
        step(&mut game, true);
        let far_face = 11.0 * 32.0;
        let mut furthest: f32 = 0.0;
        let mut came_back = false;
        for _ in 0..240 {
            step(&mut game, false);
            let (at, v) = grenades(&game)[0];
            furthest = furthest.max(at.x);
            came_back |= v.x < 0.0 && at.x > 6.0 * 32.0;
        }
        assert!(furthest > 6.0 * 32.0, "flew over the near wall: {furthest}");
        assert!(furthest < far_face, "never through the far wall: {furthest}");
        assert!(came_back, "bounced back off the far wall");
    }

    #[test]
    fn a_tank_driving_into_a_grenade_shoves_it_along() {
        let mut game = sandbox("");
        let from = with_tank(&game.world, player(&game), |t| t.position);
        let mut g = Grenade::launch(from + Vec2::new(48.0, 0.0), Vec2::new(1.0, 0.0), Vec2::zero(), Owner::Player(0));
        (g.velocity, g.height, g.climb) = (Vec2::zero(), 0.0, 0.0);
        let start = g.position;
        game.world.spawn((g,));
        for _ in 0..90 {
            let mut input = Input::default();
            input.seats[0].move_dir = Some(crate::tank::Dir::Right);
            game.update(input, DT, W, H);
        }
        let (at, _) = grenades(&game)[0];
        let hull = with_tank(&game.world, player(&game), |t| t.position);
        assert!(at.x > start.x + 20.0, "pushed along: {start:?} -> {at:?}");
        assert!(at.x > hull.x, "still ahead of the hull: {at:?} vs {hull:?}");
    }
}
