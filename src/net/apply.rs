//! The wire into a client replica (docs/online-coop-prd.md §4.5): `welcome`
//! builds a `Game` the way the server built its round - `Game::init` on
//! the same map, seed and overrides, so every roll `init` makes (wall and
//! prop variants, the frog's colour, the band's tanks) comes out the same -
//! then strips the enemies' `Ai` and applies the welcome's snapshot;
//! `snapshot` writes one authoritative snapshot into the entities, matched
//! by id. The replica is never `update`d: a tank in the snapshot but not
//! the world is spawned (an enemy with a `Tank` and no `Ai`, like a rolling
//! in wave tank; a seat with its chassis), a world tank missing from the
//! snapshot is removed, likewise projectiles by id; frogs, tiles, fires,
//! pickups and the round's scalars are overwritten. Tanks keep physics
//! bodies so the drawn hull follows the snapshot through the same
//! `Tank::position`/body pair the renderer reads. No RNG anywhere.
//!
//! What the wire does not say the replica fills from its own rules, all
//! cosmetic: a wreck's art variant and a spawned enemy's damage overlay
//! hash from the slot, a shot's shadow height from its id, a snapshot's
//! flag becomes the full timer it stands for (a boost, an afterburn, the
//! hit window), a frog's secondary clips run at full length, the round
//! clock is the tick at the fixed step. Between two snapshots the whole
//! picture is `Game::tick_presentation`'s, which is why a hull's *drawn*
//! angles are left where they stand here while its facing snaps.
//! `tuning_json` is the transport's to stage (`tuning::submit_json`,
//! applied at the frame boundary) before `welcome` runs, since `init`
//! reads the knobs.
//!
//! Two things a snapshot states by omission: a fire is out once the
//! list stops carrying its cell (burning out is the only way one ever
//! leaves), and the tile a `DrumLaunched` names is in the air rather than
//! anywhere on the field.
//!
//! The spectacle a round lays down from inside its phases - a kill's
//! fireball, mushroom cloud, shockwave, screen flash, scorch and thrown
//! hull parts, a drum's or a missile's blast, cook-off pops, impact and
//! muzzle flashes, rubble - comes to a replica off the events that
//! announce each cause (`apply_spectacle`), through the same `*_show`
//! methods the phases call (docs/online-coop-prd.md section 4.16). The
//! interpolator hands a snapshot's events over exactly once, so each is
//! drawn once; a welcome draws none, since a late joiner has missed the
//! moment, and neither does a prediction sandbox, which nobody draws
//! (`Show`). What a client drew itself on its own press - its shots'
//! muzzle ripples, its laser beams - the replica does not draw again.

use std::collections::{BTreeMap, BTreeSet};

use hecs::Entity;

use crate::bullet::{Bullet, BulletState};
use crate::frog::{Facing, Frog, Side};
use crate::map::{self, MapFile};
use crate::math::Vec2;
use crate::net::PROTOCOL_VERSION;
use crate::net::encode::{cell_from_index, cell_index, field_cols};
use crate::net::events::WireEvent;
use crate::net::events::WireHitTarget;
use crate::net::wire::{
    MissileState, ShotKind, ShotState, Snapshot, TankState, WeaponKind, Welcome, dequantise_heading, dequantise_pos, dequantise_seconds, dequantise_velocity, dir_from_index, crate_flags, dequantise_health, frog_flags, tank_flags, tile_flags,
};
use crate::laser::{LaserBeam, LaserVariant};
use crate::obstacle::{Drum, Fuse, Obstacle};
use crate::pickup::{Pickup, PickupKind};
use crate::missile::Missile;
use crate::plasma::{Plasma, PlasmaState};
use crate::shell::{Owner, Shell, ShellState};
use crate::simulation::replica::plasma_variant_from_index;
use crate::blast::BlastShape;
use crate::shockwave::Shockwave;
use crate::simulation::{Game, GroundFire, PlayerCount, SHOCK_FROG, SHOCK_SHIELD_BREAK, SHOCK_TELEPORT, Spectacle, tile_rubble};
use crate::tank::{ActiveWeapon, Dir, Tank};
use crate::tuning::tuning;
use crate::{DAMAGE_VARIANTS, MAX_DAMAGE, PHYSICS_FIXED_DT, Position, TANK_SHELL_VARIANT_BY_ROW, TANK_WRECK_COLS};

/// The owner a replica's projectile carries: the wire names none, and the
/// replica never resolves a hit, so the slot only has to be one no tank
/// ever holds.
const REPLICA_OWNER: Owner = Owner::Enemy(usize::MAX);

/// How far (px) a hull may move between two applies before the replica
/// reads it as a jump - a portal, a seat driven back in at a gate, the
/// room placing a hull - rather than as driving: past it the tread marks
/// start again from where the hull landed and the ring follower is put
/// under it, so nothing is drawn across the gap. Half again a cell: a
/// hull at full boost covers a few pixels a frame, and the interpolator
/// blends every other move.
const JUMP_PX: f32 = 48.0;

/// Where the wire last put a replica's hull: the room's own track of it,
/// which a jump is read from. Not `Tank::position`, because the local
/// seat's hull is drawn somewhere else - `net::round` writes the
/// prediction over it every frame, ahead of the track by the picture's
/// delay and the round trip, and that gap is no jump. Only `write_tank`
/// adds one, so a local round's tanks never carry it.
struct WireTrack(Position);

/// How much of the moment a snapshot's events are drawn with. The state -
/// every hull, shot, tile, fire and the round's scalars - is written
/// whatever the show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Show {
    /// Every cause's show, every beam and every drum in the air: a client
    /// replica's picture.
    All,
    /// `All`, less what `seat`'s client drew itself on the press
    /// (docs/online-coop-prd.md section 4.16): the muzzle ripple and the
    /// turret's kick of its shots, which `net::round` puts on as a
    /// provisional shot leaves, and the laser beams it drew from the predicted muzzle and claimed by
    /// their `Fired` (`net::predict::Predictor::confirm_beam`) - bit `k` of
    /// `beams` for the seat's `k`th laser `Fired` in the snapshot. A beam
    /// whose `Fired` the client did not claim is one it never drew, and is
    /// drawn as the room's.
    OwnShotsDrawn { seat: u8, beams: u8 },
    /// The state and nothing of the moment: a welcome, whose joiner has
    /// missed it, and a prediction sandbox (`net::predict`), which nobody
    /// draws and nothing ages - a fireball or a beam put on it would stay
    /// for the round.
    Quiet,
}

impl Show {
    /// Whether anything of the moment is drawn at all.
    fn drawn(self) -> bool {
        self != Show::Quiet
    }

    /// Whether the client drew the shots of the tank in owner slot `slot`
    /// itself.
    fn client_drew(self, slot: usize) -> bool {
        matches!(self, Show::OwnShotsDrawn { seat, .. } if seat as usize == slot)
    }

    /// Which of `s`'s events are beams the client drew itself, by index:
    /// for each of its seat's laser `Fired`s whose bit is set in `beams`,
    /// the seat's first `LaserBeam` after it and before the seat's next
    /// laser `Fired` - the beam that shot put up, which the room logs after
    /// its `Fired` in the same tick. A `Fired` whose beam is missing (kept
    /// from a snapshot handed over too late to draw) drops nothing.
    fn beams_drawn(self, s: &Snapshot) -> BTreeSet<usize> {
        let Show::OwnShotsDrawn { seat, beams } = self else { return BTreeSet::new() };
        let mut out = BTreeSet::new();
        let (mut fired, mut claimed) = (0u32, false);
        for (i, event) in s.events.iter().enumerate() {
            match *event {
                WireEvent::Fired { slot, weapon: WeaponKind::Laser, .. } if slot == seat as u16 => {
                    claimed = fired < u8::BITS && beams & (1 << fired) != 0;
                    fired += 1;
                }
                WireEvent::LaserBeam { seat: by, .. } if by == seat && claimed => {
                    out.insert(i);
                    claimed = false;
                }
                _ => {}
            }
        }
        out
    }
}

/// A replica of the round `w` describes, at the tick its snapshot was
/// cut. Refuses a welcome from another protocol version or with a map
/// that does not parse.
pub fn welcome(w: &Welcome) -> Result<Game, String> {
    if w.protocol != PROTOCOL_VERSION {
        return Err(format!("server speaks protocol {} but this build speaks {PROTOCOL_VERSION}", w.protocol));
    }
    let mut game = Game::default();
    game.map = MapFile::from_toml_str(&w.map_toml)?;
    game.seed_override = Some(w.seed);
    game.level_overrides = w.overrides.into();
    game.enemy_count_override = w.enemy_count.map(|n| n as usize);
    // As many seats as the roster's highest, so the replica's enemies
    // count from the same slot the room's do.
    let seats = w.roster.iter().map(|s| s.seat as usize + 1).max().unwrap_or(1);
    game.players = PlayerCount::from_count(seats).unwrap_or(PlayerCount::MAX);
    let chassis = |seat: u8| w.roster.iter().find(|s| s.seat == seat).map(|s| s.chassis as i32);
    game.player_row_override = chassis(0);
    game.player2_row_override = chassis(1);
    // The banner's time left travels in the snapshot.
    game.show_intro = false;
    // The room's sky, which is its map's and its seed's alone: the
    // window's `weather_override` knob never reaches a round it does not
    // simulate, so the replica and the sandbox draw and drive under the
    // sky the room fights under.
    game.weather_from_map = true;
    let (width, height) = game.map.field_size();
    game.init(width, height);
    game.strip_ai();
    let cols = field_cols(&game);
    game.oil_cells = w.oil_cells.iter().map(|&i| cell_from_index(cols, i)).collect();
    remove_tiles(&mut game, &w.dead_cells.iter().copied().collect(), cols);
    apply(&mut game, &w.snapshot, Show::Quiet);
    // The lanterns each seat has set down already are spent; one a blast
    // broke before this client came in is forgotten.
    for seat in 0..game.lamps_left.len() {
        let set = game.lanterns.iter().filter(|l| l.seat as usize == seat).count();
        game.lamps_left[seat] = game.lamps_left[seat].saturating_sub(set.min(u8::MAX as usize) as u8);
    }
    Ok(game)
}

/// The lanterns the snapshot lists: kept where the replica already has
/// them (their flame's age with them), new ones lit now, the rest gone.
fn apply_lamps(game: &mut Game, s: &Snapshot) {
    let now = game.time;
    let lanterns = s
        .lamps
        .iter()
        .map(|l| {
            let lit_at = game.lanterns.iter().find(|k| k.id == l.id).map_or(now, |k| k.lit_at);
            crate::lamp::Lantern {
                id: l.id,
                position: Position::new(dequantise_pos(l.x), dequantise_pos(l.y)),
                seat: l.seat,
                lit_at,
            }
        })
        .collect();
    game.lanterns = lanterns;
}

/// Take the tiles in `dead` out of the world, bodies included, and recap
/// the walls around the holes.
fn remove_tiles(game: &mut Game, dead: &BTreeSet<u16>, cols: u16) {
    if dead.is_empty() {
        return;
    }
    let gone: Vec<(Entity, _)> = game
        .world
        .query::<(Entity, &Obstacle)>()
        .iter()
        .filter(|(_, o)| dead.contains(&cell_index(cols, o.cell())))
        .map(|(e, o)| (e, o.body))
        .collect();
    let removed = !gone.is_empty();
    for (entity, body) in gone {
        game.physics_mut().remove_body(body);
        game.world.despawn(entity).ok();
    }
    if removed {
        game.refresh_edge_masks();
    }
}

/// Write `s` into `game`: the frame's events first (a tile death there
/// removes the tile) and the spectacle they announce, then every family.
pub fn snapshot(game: &mut Game, s: &Snapshot) {
    apply(game, s, Show::All);
}

/// `snapshot`, with as much of the moment drawn as `show` says.
pub fn snapshot_with(game: &mut Game, s: &Snapshot, show: Show) {
    apply(game, s, show);
}

fn apply(game: &mut Game, s: &Snapshot, show: Show) {
    let cols = field_cols(game);
    let own_beams = show.beams_drawn(s);
    let dead_tiles = apply_events(game, s, cols, show, &own_beams);
    let mut spectacle = Spectacle::default();
    let drawn = show.drawn();
    if drawn {
        // Before the families move anything: a dying tile is still
        // standing, a bursting missile still in the air and a firing
        // hull where it is drawn.
        apply_spectacle(game, s, show, &own_beams, &mut spectacle);
    }
    apply_tiles(game, s, cols, dead_tiles);
    apply_tanks(game, s);
    apply_shots(game, s);
    apply_missiles(game, s, drawn.then_some(&mut spectacle));
    apply_frogs(game, s, drawn.then_some(&mut spectacle));
    apply_pickups(game, s, cols);
    apply_fires(game, s, cols);
    apply_lamps(game, s);
    apply_round(game, s);
    game.show(spectacle);
    game.frame = s.tick as u64;
    game.time = s.tick as f32 * PHYSICS_FIXED_DT;
}

/// The cosmetics the round laid down for each event in `s`, from the code
/// that laid them: `Wreck` is a kill's whole show, `Blast` a drum's,
/// `MissileBlast` a missile's (leaning the way the replica's copy of it
/// was heading), `CookOff` a pop; every hit that flashed on the server
/// flashes here, a laser flashes at its muzzle, a shield break and a
/// portal ripple, and a tile that died drops its rubble. `Fired` puts a
/// ripple at the drawn muzzle of the tank that fired and kicks its turret
/// back (`kick_turret`) - except for a seat whose shots the client drew
/// itself, which rippled and kicked on the press (`net::round`).
///
/// A zero-damage hit on a tank or a frog is a flame's contact, which
/// never flashes - unless it lies where a beam in the same snapshot
/// stopped: a laser on a shield deals nothing and flashes all the same,
/// as `resolve_lasers` flashes it. The two land on one point exactly,
/// being the same position quantised the same way.
///
/// What the client drew itself (`Show::OwnShotsDrawn`) is left out: its
/// shots' muzzle ripples, and the muzzle flash of each beam it drew
/// (`own_beams`, indices into `s.events`). A beam's end still counts for
/// the hit it stopped on.
///
/// What the wire does not carry is taken plain: a drum's blast has no
/// cause on the wire, so its fireball is the hashed pick without a lean.
fn apply_spectacle(game: &mut Game, s: &Snapshot, mode: Show, own_beams: &BTreeSet<usize>, show: &mut Spectacle) {
    let field = game.map.field_size();
    let at = |x: i16, y: i16| Position::new(dequantise_pos(x), dequantise_pos(y));
    let beam_ends: BTreeSet<(i16, i16)> = s
        .events
        .iter()
        .filter_map(|e| match *e {
            WireEvent::LaserBeam { x1, y1, .. } => Some((x1, y1)),
            _ => None,
        })
        .collect();
    for (i, event) in s.events.iter().enumerate() {
        match *event {
            WireEvent::Wreck { x, y, .. } => game.wreck_show(show, at(x, y)),
            WireEvent::Blast { x, y, drum, .. } => game.blast_show(show, at(x, y), drum, BlastShape::Plain, field),
            WireEvent::MissileBlast { x, y, .. } => {
                let center = at(x, y);
                let dir = landing_dir(game, center);
                game.missile_show(show, center, dir);
            }
            WireEvent::CookOff { x, y } => Game::cookoff_show(show, at(x, y)),
            WireEvent::CrateBroken { x, y, cooked: true, .. } => game.crate_cookoff_show(show, at(x, y)),
            WireEvent::Hit { target, damage, x, y, .. } => {
                let contact = damage == 0.0
                    && matches!(target, WireHitTarget::Player { .. } | WireHitTarget::Enemy { .. } | WireHitTarget::Frog { .. })
                    && !beam_ends.contains(&(x, y));
                if !contact {
                    show.impact_flashes.push(Shockwave::new(at(x, y)));
                }
            }
            WireEvent::Deflected { x, y, .. } | WireEvent::Ricochet { x, y, .. } | WireEvent::ShellsCollided { x, y } => {
                show.impact_flashes.push(Shockwave::new(at(x, y)));
            }
            WireEvent::LaserBeam { x0, y0, .. } if !own_beams.contains(&i) => {
                show.muzzle_flashes.push(Shockwave::new(at(x0, y0)));
            }
            WireEvent::Fired { slot, weapon, .. } if !mode.client_drew(slot as usize) => {
                kick_turret(game, slot as usize, weapon);
                if let Some(muzzle) = drawn_muzzle(game, slot as usize, weapon) {
                    show.muzzle_flashes.push(Shockwave::new(muzzle));
                }
            }
            WireEvent::ShieldBroken { x, y, .. } => show.shocks.push(Shockwave::scaled(at(x, y), SHOCK_SHIELD_BREAK)),
            WireEvent::Teleported { x, y, to_x, to_y, .. } => {
                show.shocks.push(Shockwave::scaled(at(x, y), SHOCK_TELEPORT));
                show.shocks.push(Shockwave::scaled(at(to_x, to_y), SHOCK_TELEPORT));
            }
            WireEvent::ObstacleDestroyed { material, x, y } => {
                let center = at(x, y);
                let cell = map::world_to_cell(center);
                // A tile that was burning when it died is the charred
                // plank a fire leaves; the tile is still standing here.
                let charred = game.world.query::<&Obstacle>().iter().any(|o| o.burning && o.cell() == cell);
                if let Some(decal) = tile_rubble(material, center, charred) {
                    show.decals.push(decal);
                }
            }
            _ => {}
        }
    }
}

/// Which way the missile bursting at `center` was coming down: the
/// heading of the replica's nearest missile, which the snapshot about to
/// drop it still holds. Straight onto the spot, with no lean, when there
/// is none.
fn landing_dir(game: &Game, center: Position) -> Vec2 {
    game.world
        .query::<&Missile>()
        .iter()
        .min_by(|a, b| a.position.distance_to(center).total_cmp(&b.position.distance_to(center)))
        .map_or(Vec2::new(0.0, 0.0), |m| m.dir)
}

/// The turret of the tank that fired kicks back through its recoil cells
/// (`Tank::kick`) - a shell or a plasma bolt from the main gun - or its
/// laser lens flashes. Presentation only, like the ripple.
fn kick_turret(game: &mut Game, slot: usize, weapon: WeaponKind) {
    for tank in game.world.query::<&mut Tank>().iter() {
        if tank.owner_slot() != slot {
            continue;
        }
        match weapon {
            WeaponKind::Shell => tank.kick(false),
            WeaponKind::Plasma => tank.kick(true),
            WeaponKind::Laser => tank.kick_laser(),
            WeaponKind::Minigun | WeaponKind::Missiles | WeaponKind::Flamethrower => {}
        }
        break;
    }
}

/// Where a shot of `weapon` leaves `slot`'s tank as it is drawn: the spawn
/// point the round's own `Shell::spawn`/`Plasma::spawn`/`Bullet::spawn`
/// give, turned to the drawn turret (a twin barrel's first, as the round
/// fires it). `None` for the weapons whose muzzle comes another way - a
/// laser's rides its `LaserBeam`, a missile's is its own puff when it
/// first shows (`apply_missiles`) - and for the flamethrower, which has
/// none.
fn drawn_muzzle(game: &mut Game, slot: usize, weapon: WeaponKind) -> Option<Position> {
    let entity = game.world.query::<(Entity, &Tank)>().iter().find(|(_, t)| t.owner_slot() == slot).map(|(e, _)| e)?;
    let mut q = game.world.query_one::<&mut Tank>(entity);
    let tank = q.get().ok()?;
    let facing = tank.rotation;
    tank.rotation = tank.turret_visual_rotation;
    let lateral = tuning().tank_barrel_lateral_offset[tank.row as usize];
    let owner = tank.owner();
    let muzzle = match weapon {
        WeaponKind::Shell => Some(Shell::spawn(tank, owner, 0.0, -lateral).position),
        WeaponKind::Plasma => Some(Plasma::spawn(tank, owner, tank.plasma_variant, 0.0, -lateral).position),
        WeaponKind::Minigun => Some(Bullet::spawn(tank, owner, 0.0).position),
        WeaponKind::Laser | WeaponKind::Missiles | WeaponKind::Flamethrower => None,
    };
    tank.rotation = facing;
    muzzle
}

/// Hand the frame's events to the replica's presentation (`fx.rs` reads
/// `Game::events`) and collect the cells whose tile died. A `RoundStarted`
/// among them is the server's round restarting (the end screen ran out):
/// the replica starts over the same way, `init` on the event's seed, since
/// a snapshot alone cannot put back what the old round destroyed. A
/// welcome's frame-0 snapshot carries the same event for the round
/// `welcome` has just built, so a replica already standing at frame 0 on
/// that seed keeps the world it has instead of building an identical one.
///
/// A `DrumLaunched` puts the drum in the air here, since a snapshot has
/// no family for something that is neither a tile nor a shot; its arc is
/// derived from an age `tick_presentation` advances, and the blast it
/// sets off when it lands arrives as the server's own `Blast`. A
/// `LavaBombLaunched` puts a volcano's bomb in the air the same way. A
/// `LaserBeam` is drawn likewise. Both are the moment's, so a quiet
/// apply puts neither up, and a beam the client drew itself (`own_beams`,
/// indices into `s.events`) is neither drawn nor handed on.
fn apply_events(game: &mut Game, s: &Snapshot, cols: u16, show: Show, own_beams: &BTreeSet<usize>) -> BTreeSet<u16> {
    let restarted = s.events.iter().rev().find_map(|e| match *e {
        WireEvent::RoundStarted { seed, .. } => Some(seed),
        _ => None,
    });
    if let Some(seed) = restarted {
        if game.frame() != 0 || game.round_seed() != seed {
            game.seed_override = Some(seed);
            let (width, height) = game.map.field_size();
            game.init(width, height);
            game.strip_ai();
        }
    }
    let mut dead = BTreeSet::new();
    for (i, event) in s.events.iter().enumerate() {
        match *event {
            WireEvent::ObstacleDestroyed { x, y, .. } => {
                let at = Position::new(dequantise_pos(x), dequantise_pos(y));
                dead.insert(cell_index(cols, map::world_to_cell(at)));
            }
            WireEvent::DrumLaunched { x, y, to_x, to_y } if show.drawn() => {
                let from = Position::new(dequantise_pos(x), dequantise_pos(y));
                let to = Position::new(dequantise_pos(to_x), dequantise_pos(to_y));
                // Only a fuel drum ever launches (`props::tick_fuses`).
                game.drum_in_flight(from, to, Drum::Fuel as i32);
            }
            // A seat's lantern is spent the moment it is set down, however
            // late the news: the HUD counts what is left.
            WireEvent::LanternSet { seat, .. } => {
                if let Some(left) = game.lamps_left.get_mut(seat as usize) {
                    *left = left.saturating_sub(1);
                }
            }
            // A lava bomb flies the same way, and bursts on the room's
            // `Blast`.
            WireEvent::LavaBombLaunched { x, y, to_x, to_y } if show.drawn() => {
                let from = Position::new(dequantise_pos(x), dequantise_pos(y));
                let to = Position::new(dequantise_pos(to_x), dequantise_pos(to_y));
                game.bomb_in_flight(from, to);
            }
            // An instant hit: the beam is the only trace, and the replica's
            // `tick_effects` fades it as a local round's does.
            WireEvent::LaserBeam { x0, y0, x1, y1, variant, .. } if show.drawn() && !own_beams.contains(&i) => {
                let start = Position::new(dequantise_pos(x0), dequantise_pos(y0));
                let end = Position::new(dequantise_pos(x1), dequantise_pos(y1));
                let variant = LaserVariant::ALL.get(variant as usize).copied().unwrap_or(LaserVariant::Red);
                game.laser_beams.push(LaserBeam::new(start, end, variant));
            }
            _ => {}
        }
    }
    game.events = s
        .events
        .iter()
        .enumerate()
        .filter(|(i, _)| !own_beams.contains(i))
        .filter_map(|(_, e)| WireEvent::to_event(e))
        .collect();
    dead
}

fn apply_tiles(game: &mut Game, s: &Snapshot, cols: u16, mut dead: BTreeSet<u16>) {
    let listed: BTreeMap<u16, _> = s
        .tiles
        .iter()
        .inspect(|t| {
            if t.flags & tile_flags::DESTROYED != 0 {
                dead.insert(t.cell);
            }
        })
        .filter(|t| t.flags & tile_flags::DESTROYED == 0)
        .map(|t| (t.cell, *t))
        .collect();
    remove_tiles(game, &dead, cols);
    let live: Vec<(Entity, u16)> = game
        .world
        .query::<(Entity, &Obstacle)>()
        .iter()
        .filter(|(_, o)| !o.destroyed)
        .map(|(e, o)| (e, cell_index(cols, o.cell())))
        .collect();
    for (entity, cell) in live {
        let mut q = game.world.query_one::<&mut Obstacle>(entity);
        let Ok(o) = q.get() else { continue };
        match listed.get(&cell) {
            Some(t) => {
                o.health = t.hp as f32;
                o.burning = t.flags & tile_flags::BURNING != 0;
                let fused = t.flags & tile_flags::FUSED != 0;
                if o.burning || fused {
                    o.health = 0.0;
                }
                if fused && o.fuse.is_none() {
                    let total = tuning().barrel_fuse_seconds;
                    o.fuse = Some(Fuse { left: total, total, from: None });
                } else if !fused {
                    o.fuse = None;
                }
                o.scorched = t.faces;
                o.set_lean_strength((t.flags & tile_flags::LEAN_MASK) >> (tile_flags::LEAN_SHIFT + 2));
            }
            // Absent from the list: the tile is as the map made it.
            None => {
                o.health = o.max_health;
                o.burning = false;
                o.fuse = None;
                o.scorched = 0;
                o.ram_timer = 0.0;
            }
        }
    }
}

fn apply_tanks(game: &mut Game, s: &Snapshot) {
    let first_enemy = game.first_enemy_slot();
    let by_slot: BTreeMap<usize, Entity> =
        game.world.query::<(Entity, &Tank)>().iter().map(|(e, t)| (t.owner_slot(), e)).collect();
    let wanted: BTreeSet<usize> = s.tanks.iter().map(|t| t.id as usize).collect();
    // A seat's tank never leaves the picture: the lobby, not the
    // snapshot, says who is playing.
    for (&slot, &entity) in by_slot.iter().filter(|(slot, _)| **slot >= first_enemy) {
        if !wanted.contains(&slot) {
            let body = game.world.get::<&Tank>(entity).ok().and_then(|t| t.body);
            if let Some(body) = body {
                game.physics_mut().remove_body(body);
            }
            game.world.despawn(entity).ok();
        }
    }
    for t in &s.tanks {
        let slot = t.id as usize;
        let entity = match by_slot.get(&slot) {
            Some(&e) => e,
            None => spawn_tank(game, t, slot < first_enemy),
        };
        write_tank(game, entity, t);
    }
}

/// A fresh tank for `t`, a seat's or an enemy's: `Tank` and body, no
/// `Ai`. The cosmetic rolls `init` would have made hash from the slot.
fn spawn_tank(game: &mut Game, t: &TankState, player: bool) -> Entity {
    let slot = t.id as usize;
    let owner = if player { Owner::Player(slot as u8) } else { Owner::Enemy(slot) };
    let row = (t.row as i32).clamp(0, TANK_SHELL_VARIANT_BY_ROW.len() as i32 - 1);
    let rotation = dir_from_index(t.dir).unwrap_or(Dir::Up).rotation();
    let position = Position::new(dequantise_pos(t.x), dequantise_pos(t.y));
    let mut tank = Tank {
        row,
        shell_variant: TANK_SHELL_VARIANT_BY_ROW[row as usize],
        damage_variant: (slot % DAMAGE_VARIANTS as usize) as i32,
        position,
        rotation,
        visual_rotation: rotation,
        turret_visual_rotation: rotation,
        ring_position: position,
        owner,
        ..Tank::default()
    };
    let half = tank.move_half_extents(tank.facing_along_x());
    let mass = tank.mass();
    tank.body = Some(game.physics_mut().spawn_tank(position, half, mass));
    let entity = game.world.spawn((tank,));
    if player {
        if let Some(held) = game.seats.get_mut(slot) {
            *held = Some(entity);
        }
    }
    entity
}

/// Write `t` into the tank `entity` and put its body where the hull is.
///
/// A jump is read off the hull's wire track (`WireTrack`), the last place
/// the wire put it, rather than off where it was drawn. With no track yet
/// the hull stands where `init` or `spawn_tank` put it, which nothing has
/// drawn over.
fn write_tank(game: &mut Game, entity: Entity, t: &TankState) {
    let position = Position::new(dequantise_pos(t.x), dequantise_pos(t.y));
    let velocity = Position::new(dequantise_velocity(t.vx), dequantise_velocity(t.vy));
    let rotation = dir_from_index(t.dir).unwrap_or(Dir::Up).rotation();
    let on = |bit: u8| t.flags & bit != 0;
    let track = game.world.get::<&WireTrack>(entity).ok().map(|w| w.0);
    let (body, half_extents, turned) = {
        let mut q = game.world.query_one::<&mut Tank>(entity);
        let Ok(tank) = q.get() else { return };
        if track.unwrap_or(tank.position).distance_to(position) > JUMP_PX {
            // A jump, not a drive: no marks across it, and the ring put
            // under the hull rather than left to chase it.
            tank.track_from = None;
            tank.ring_position = position;
            tank.ring_velocity = Vec2::new(0.0, 0.0);
        }
        tank.position = position;
        // The hull's facing snaps the way `Tank::control` snaps it; the
        // drawn angles stay where they are and swing across in
        // `Game::tick_presentation`, which is the turn the player sees.
        let turned = tank.rotation != rotation;
        tank.rotation = rotation;
        let wreck = on(tank_flags::WRECK);
        tank.damage = if wreck { MAX_DAMAGE } else { MAX_DAMAGE - t.hp as f32 };
        if wreck && tank.wreck_col.is_none() {
            tank.wreck_col = Some(TANK_WRECK_COLS[t.id as usize % TANK_WRECK_COLS.len()]);
        }
        tank.shield_hp = if on(tank_flags::SHIELD) { t.shield.max(1) as f32 } else { 0.0 };
        let knobs = tuning();
        set_timer(&mut tank.speed_boost_timer, on(tank_flags::BOOST), knobs.speed_boost_duration_seconds);
        set_timer(&mut tank.burn_timer, on(tank_flags::BURNING), knobs.flame_afterburn_seconds);
        set_timer(&mut tank.hit_flash_timer, on(tank_flags::HIT), knobs.health_ring_hit_seconds);
        set_timer(&mut tank.heat_shield_timer, on(tank_flags::HEAT_SHIELD), knobs.heat_shield_seconds);
        tank.flame_held = on(tank_flags::FLAME);
        let weapon: ActiveWeapon = t.weapon.into();
        tank.weapon_queue = if weapon == ActiveWeapon::Shell { Vec::new() } else { vec![weapon] };
        // The wire names the live weapon alone, so that is the one a
        // replica carries: the other stocks are cleared, or a weapon the
        // tank has spent would stay on its turret (`tank::module_cols`) at
        // the last count this replica heard.
        tank.laser_charges = 0;
        tank.plasma_ammo = 0;
        tank.minigun_ammo = 0;
        tank.missile_ammo = 0;
        tank.flame_fuel = 0.0;
        let ammo = t.ammo as i32;
        match weapon {
            ActiveWeapon::Shell => tank.shells_ammo = ammo,
            ActiveWeapon::Laser => tank.laser_charges = ammo,
            ActiveWeapon::Plasma => tank.plasma_ammo = ammo,
            ActiveWeapon::Minigun => tank.minigun_ammo = ammo,
            ActiveWeapon::Missiles => tank.missile_ammo = ammo,
            ActiveWeapon::Flamethrower => tank.flame_fuel = ammo as f32,
        }
        (tank.body, tank.move_half_extents(tank.facing_along_x()), turned)
    };
    let tracked = game.world.get::<&mut WireTrack>(entity).map(|mut w| w.0 = position).is_ok();
    if !tracked {
        game.world.insert_one(entity, WireTrack(position)).ok();
    }
    let Some(body) = body else { return };
    let physics = game.physics_mut();
    physics.set_position(body, position);
    physics.set_velocity(body, velocity);
    if turned {
        let collider = physics.collider_of(body);
        physics.resize_collider(collider, half_extents);
    }
}

/// A flag into the timer it stands for: a running window keeps running
/// (or starts at its full length), a cleared one stops.
fn set_timer(timer: &mut f32, on: bool, full: f32) {
    if !on {
        *timer = 0.0;
    } else if *timer <= 0.0 {
        *timer = full.max(f32::EPSILON);
    }
}

/// The seeker missiles the snapshot lists, spawned, moved or dropped.
///
/// A replica never flies one: no seek, no lock, no dive, no burst. It holds
/// the pose the server sent and `render/missile.rs` draws it, exactly as a
/// shot is held. What the wire leaves out stays at its spawn value - the
/// aim, the target, the stage timers - because nothing here reads them.
///
/// A missile seen for the first time puts the puff its launch put at the
/// tube into `show`, when the spectacle is on.
fn apply_missiles(game: &mut Game, s: &Snapshot, mut show: Option<&mut Spectacle>) {
    let mut existing: BTreeMap<u16, Entity> = BTreeMap::new();
    for (e, m) in game.world.query::<(Entity, &Missile)>().iter() {
        existing.insert((m.id & 0xFFFF) as u16, e);
    }
    let wanted: BTreeMap<u16, &MissileState> = s.missiles.iter().map(|m| (m.id, m)).collect();
    for (id, entity) in &existing {
        if !wanted.contains_key(id) {
            game.world.despawn(*entity).ok();
        }
    }
    for (id, ms) in wanted {
        let ground = Position::new(dequantise_pos(ms.x), dequantise_pos(ms.y));
        let height = dequantise_pos(ms.height);
        let unit = |deg: f32| {
            let rad = deg.to_radians();
            Vec2::new(rad.sin(), -rad.cos())
        };
        let facing = unit(dequantise_heading(ms.facing));
        let dir = unit(dequantise_heading(ms.heading));
        match existing.get(&id) {
            Some(&entity) => {
                let mut q = game.world.query_one::<&mut Missile>(entity);
                if let Ok(m) = q.get() {
                    m.position = ground;
                    m.height = height;
                    m.facing = facing;
                    m.dir = dir;
                }
            }
            None => {
                // The aim is the ground point it is over: a replica never
                // steers, so this only has to be finite.
                let mut m = Missile::spawn(ground, dir, REPLICA_OWNER, ms.tube, ground);
                m.set_id(ms.id as u32);
                m.position = ground;
                m.height = height;
                m.facing = facing;
                m.dir = dir;
                game.world.spawn((m,));
                if let Some(show) = show.as_deref_mut() {
                    show.muzzle_flashes.push(Shockwave::new(ground));
                }
            }
        }
    }
}

fn apply_shots(game: &mut Game, s: &Snapshot) {
    let mut existing: BTreeMap<u16, (ShotKind, Entity)> = BTreeMap::new();
    for (e, sh) in game.world.query::<(Entity, &Shell)>().iter() {
        existing.insert((sh.id & 0xFFFF) as u16, (ShotKind::Shell, e));
    }
    for (e, b) in game.world.query::<(Entity, &Bullet)>().iter() {
        existing.insert((b.id & 0xFFFF) as u16, (ShotKind::Bullet, e));
    }
    for (e, p) in game.world.query::<(Entity, &Plasma)>().iter() {
        existing.insert((p.id & 0xFFFF) as u16, (ShotKind::Plasma, e));
    }
    let wanted: BTreeMap<u16, &ShotState> = s.shots.iter().map(|sh| (sh.id, sh)).collect();
    for (id, (kind, entity)) in &existing {
        if wanted.get(id).is_none_or(|sh| sh.kind != *kind) {
            game.world.despawn(*entity).ok();
        }
    }
    for (id, sh) in wanted {
        let position = Position::new(dequantise_pos(sh.x), dequantise_pos(sh.y));
        let rotation = dequantise_heading(sh.heading);
        let rad = rotation.to_radians();
        let dir = Vec2::new(rad.sin(), -rad.cos());
        let knobs = tuning();
        match existing.get(&id) {
            Some(&(kind, entity)) if kind == sh.kind => match sh.kind {
                ShotKind::Shell => {
                    let mut q = game.world.query_one::<&mut Shell>(entity);
                    if let Ok(shell) = q.get() {
                        shell.prev_position = shell.position;
                        shell.position = position;
                        shell.rotation = rotation;
                        shell.velocity = dir * knobs.shell_speed;
                        let state = ShellState::from_col(sh.state as i32).unwrap_or(ShellState::Flying);
                        if shell.state != state {
                            shell.state = state;
                            shell.timer = 0.0;
                        }
                    }
                }
                ShotKind::Bullet => {
                    let mut q = game.world.query_one::<&mut Bullet>(entity);
                    if let Ok(bullet) = q.get() {
                        bullet.prev_position = bullet.position;
                        bullet.position = position;
                        bullet.rotation = rotation;
                        bullet.velocity = dir * knobs.minigun_bullet_speed;
                        let state = BulletState::from_col(sh.state as i32).unwrap_or(BulletState::Flying);
                        if bullet.state != state {
                            bullet.state = state;
                            bullet.timer = 0.0;
                        }
                    }
                }
                ShotKind::Plasma => {
                    let mut q = game.world.query_one::<&mut Plasma>(entity);
                    if let Ok(plasma) = q.get() {
                        plasma.prev_position = plasma.position;
                        plasma.position = position;
                        plasma.rotation = rotation;
                        plasma.velocity = dir * knobs.plasma_speed;
                        let state = PlasmaState::from_col(sh.state as i32).unwrap_or(PlasmaState::Flying);
                        if plasma.state != state {
                            plasma.state = state;
                            plasma.timer = 0.0;
                        }
                    }
                }
            },
            _ => spawn_shot(game, sh, position, rotation, dir),
        }
    }
}

/// A shot's shadow height, hashed from its id between the kind's knobs:
/// the server rolled one, the picture only needs variety.
fn shadow_offset(id: u16, min: f32, max: f32) -> f32 {
    let h = (id as u32).wrapping_mul(2_654_435_761) >> 22;
    min + (max - min) * (h % 1000) as f32 / 1000.0
}

/// Who fired `sh`, as the replica holds it: the seat the wire names, or
/// the replica's stand-in for an enemy.
fn shot_owner(sh: &ShotState) -> Owner {
    if sh.owner == crate::net::wire::NO_SEAT { REPLICA_OWNER } else { Owner::Player(sh.owner) }
}

fn spawn_shot(game: &mut Game, sh: &ShotState, position: Position, rotation: f32, dir: Vec2) {
    let knobs = tuning();
    let id = sh.id as u32;
    let owner = shot_owner(sh);
    match sh.kind {
        ShotKind::Shell => {
            game.world.spawn((Shell {
                state: ShellState::from_col(sh.state as i32).unwrap_or(ShellState::Flying),
                position,
                velocity: dir * knobs.shell_speed,
                rotation,
                timer: 0.0,
                done: false,
                owner,
                variant: sh.variant as i32,
                shooter_row: 0,
                shadow_offset: shadow_offset(sh.id, knobs.shell_shadow_offset_min, knobs.shell_shadow_offset_max),
                prev_position: position,
                bounces_left: 0,
                passed_over: Vec::new(),
                id,
            },));
        }
        ShotKind::Bullet => {
            game.world.spawn((Bullet {
                state: BulletState::from_col(sh.state as i32).unwrap_or(BulletState::Flying),
                position,
                velocity: dir * knobs.minigun_bullet_speed,
                rotation,
                timer: 0.0,
                done: false,
                owner,
                shooter_row: 0,
                shadow_offset: shadow_offset(
                    sh.id,
                    knobs.minigun_bullet_shadow_offset_min,
                    knobs.minigun_bullet_shadow_offset_max,
                ),
                prev_position: position,
                passed_over: Vec::new(),
                id,
            },));
        }
        ShotKind::Plasma => {
            game.world.spawn((Plasma {
                state: PlasmaState::from_col(sh.state as i32).unwrap_or(PlasmaState::Flying),
                position,
                velocity: dir * knobs.plasma_speed,
                rotation,
                timer: 0.0,
                done: false,
                owner,
                variant: plasma_variant_from_index(sh.variant as i32),
                shooter_row: 0,
                shadow_offset: shadow_offset(sh.id, knobs.plasma_shadow_offset_min, knobs.plasma_shadow_offset_max),
                prev_position: position,
                passed_over: Vec::new(),
                id,
            },));
        }
    }
}

/// The frogs as the snapshot has them. A frog that dies here puts the
/// ripple its death set off on the server into `show`, when the
/// spectacle is on.
fn apply_frogs(game: &mut Game, s: &Snapshot, mut show: Option<&mut Spectacle>) {
    for f in &s.frogs {
        let position = Position::new(dequantise_pos(f.x), dequantise_pos(f.y));
        let entity = match f.side {
            Side::Player => game.frog,
            Side::Enemy => game.enemy_frog,
        };
        let entity = match entity {
            Some(e) => e,
            None => {
                let e = game.spawn_frog(f.side, position, 0);
                match f.side {
                    Side::Player => game.frog = Some(e),
                    Side::Enemy => game.enemy_frog = Some(e),
                }
                e
            }
        };
        let on = |bit: u8| f.state & bit != 0;
        let body = {
            let mut q = game.world.query_one::<&mut Frog>(entity);
            let Ok(frog) = q.get() else { continue };
            frog.position = position;
            let dead = on(frog_flags::DEAD);
            if dead && !frog.is_dead() {
                if let Some(show) = show.as_deref_mut() {
                    show.shocks.push(Shockwave::scaled(position, SHOCK_FROG));
                }
            }
            let hopping = on(frog_flags::HOPPING);
            frog.health = if dead { 0.0 } else { f.hp.max(1) as f32 };
            if hopping {
                frog.hop_start = position;
                frog.hop_end = Position::new(dequantise_pos(f.hop_x), dequantise_pos(f.hop_y));
                if let Some(facing) = Facing::from_dx(frog.hop_end.x - position.x) {
                    frog.facing = facing;
                }
            }
            frog.set_clips(dead, hopping, on(frog_flags::BITING), on(frog_flags::HURT), f.phase);
            frog.body
        };
        game.physics_mut().set_position(body, position);
    }
}

/// The pickups the snapshot lists, made afresh. A crate that was already on
/// the replica keeps when it came down, so its air drop plays on; one the
/// replica has not seen before came down now, at the snapshot's tick
/// (`Pickup::dropped_at`) - which is every pickup the room drops in, and
/// none of the ones a `Welcome`'s `init` already stood on their slots.
fn apply_pickups(game: &mut Game, s: &Snapshot, cols: u16) {
    let old: Vec<(Entity, PickupKind, Position, Option<f32>)> =
        game.world.query::<(Entity, &Pickup)>().iter().map(|(e, p)| (e, p.kind, p.position, p.dropped_at)).collect();
    for &(entity, ..) in &old {
        game.world.despawn(entity).ok();
    }
    let now = s.tick as f32 * PHYSICS_FIXED_DT;
    let dropped_at = |kind: PickupKind, position: Position| match old.iter().find(|o| o.1 == kind && o.2.distance_to(position) < 0.5) {
        Some(&(_, _, _, at)) => at,
        None => Some(now),
    };
    let slots: Vec<_> = game.pickup_slots().to_vec();
    for (i, (position, kind)) in slots.into_iter().enumerate().take(64) {
        if s.pickups & (1 << i) != 0 {
            game.world.spawn((Pickup::dropped(kind, position, dropped_at(kind, position)),));
        }
    }
    for bonus in &s.bonus_pickups {
        let (col, row) = cell_from_index(cols, bonus.cell);
        let position = map::cell_to_world(col, row);
        game.world.spawn((Pickup::dropped(bonus.kind, position, dropped_at(bonus.kind, position)),));
    }
    // The crates that are not whole: hurt, burning, or broken and lying
    // loose (`crate_breakable`).
    for state in &s.crates {
        let (col, row) = cell_from_index(cols, state.cell);
        let position = map::cell_to_world(col, row);
        for pickup in game.world.query_mut::<&mut Pickup>() {
            if pickup.position.distance_to(position) < 0.5 {
                pickup.health = dequantise_health(state.hp);
                let left = dequantise_seconds(state.left);
                pickup.burn = (state.flags & crate_flags::BURNING != 0).then_some(left);
                pickup.loose = (state.flags & crate_flags::LOOSE != 0).then_some(left);
            }
        }
    }
}

fn apply_fires(game: &mut Game, s: &Snapshot, cols: u16) {
    let trail_seconds = tuning().oil_trail_burn_seconds;
    let fires: Vec<GroundFire> = s
        .fires
        .iter()
        .map(|f| {
            let cell = cell_from_index(cols, f.cell);
            let left = dequantise_seconds(f.left);
            let known = game.fires.iter().find(|g| g.cell == cell);
            GroundFire {
                cell,
                left,
                total: known.map_or(left.max(trail_seconds), |g| g.total),
                spread_at: None,
                pool: known.map_or(!game.oil_cells.contains(&cell), |g| g.pool),
                lava: f.lava,
            }
        })
        .collect();
    // Grass in a burning cell chars, as `tick_fires` does on the server.
    let burning: BTreeSet<(i32, i32)> = fires.iter().map(|f| f.cell).collect();
    if !burning.is_empty() {
        game.grass_cells.retain(|c| !burning.contains(&map::world_to_cell(*c)));
        for tuft in game.grass.iter_mut() {
            if burning.contains(&map::world_to_cell(tuft.base)) {
                tuft.burnt = true;
            }
        }
    }
    // A cell only ever leaves the list by burning out, so the snapshot
    // dropping it is the event: the oil spent and a burn scar, once each
    // (docs/online-coop-prd.md section 4.5).
    let out: Vec<GroundFire> = game.fires.iter().filter(|f| !burning.contains(&f.cell)).copied().collect();
    for fire in out {
        let scar = game.fire_burnt_out(&fire);
        game.show(crate::simulation::Spectacle { scorches: vec![scar], ..Default::default() });
    }
    game.fires = fires;
}

fn apply_round(game: &mut Game, s: &Snapshot) {
    game.intro_timer = dequantise_seconds(s.round.intro);
    // The end screen's countdown is the server's: the replica takes the
    // number rather than running a clock of its own, so the two screens
    // read the same second and the restart lands with the round the
    // server starts.
    game.restart_timer = dequantise_seconds(s.round.restart);
    game.outcome = s.round.outcome.into();
    // Zero tenths of breather is no breather: the banner is either up or
    // it is not, and a last twentieth of a second of it changes nothing.
    let next_in = (s.round.next_wave > 0).then(|| dequantise_seconds(s.round.next_wave));
    game.set_wave_progress(s.round.wave as u32, s.round.pending as usize, next_in);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::Intent;
    use crate::level::{Mission, SpawnKind};
    use crate::net::MAX_SEATS;
    use crate::net::codec::{Msg, decode, encode};
    use crate::net::encode as enc;
    use crate::net::wire::Seat;
    use crate::simulation::Input;
    use crate::simulation::replica::DrawableState;

    const DEFAULT_MAP: &str = include_str!("../../maps/default.toml");
    const PROPS_MAP: &str = include_str!("../../maps/test/props.toml");

    /// A seeded band round with the seat's chassis pinned, as a room
    /// server runs one.
    fn authoritative(map: &str, seed: u64, enemies: usize) -> Game {
        let mut game = Game::default();
        game.seed_override = Some(seed);
        game.enemy_count_override = Some(enemies);
        game.player_row_override = Some(3);
        game.level_overrides.spawn = Some(SpawnKind::Band);
        game.map = MapFile::from_toml_str(map).expect("map parses");
        let (width, height) = game.map.field_size();
        game.init(width, height);
        game
    }

    /// Wreck `slot` on frame `at`, and insist the round is still running
    /// when it happens.
    ///
    /// A kill is queued into the frame and applied by the next `update`,
    /// but `update` deals no damage once the round is over - so a kill
    /// scheduled past the end is silently nothing, and the test that
    /// wanted a wreck fails with no clue why. Tuning moves where a seeded
    /// round ends (master's projectile speeds did), so the frame is early
    /// and the assumption is checked.
    fn kill_at(game: &mut Game, frame: u32, at: u32, slot: usize) {
        if frame != at {
            return;
        }
        assert_eq!(
            game.outcome(),
            crate::simulation::Outcome::Playing,
            "frame {at}: the round is already over, so killing slot {slot} would deal no damage - move the kill earlier"
        );
        game.debug_kill(slot).unwrap_or_else(|e| panic!("slot {slot}: {e}"));
    }

    /// A scripted drive: a new heading every 40 frames, the trigger every
    /// twelfth frame.
    fn intent(frame: u32) -> Intent {
        Intent { move_dir: Some(Dir::ALL[(frame / 40) as usize % 4]), fire: frame % 12 == 0, ..Intent::default() }
    }

    fn step(game: &mut Game, frame: u32) {
        let (width, height) = game.map.field_size();
        game.update(Input::single(intent(frame)), PHYSICS_FIXED_DT, width, height);
    }

    fn roster(game: &Game) -> Vec<Seat> {
        let chassis = game.player_chassis().expect("a running round has a player").row() as u8;
        vec![Seat { seat: 0, nick: "host".into(), chassis }]
    }

    fn welcome_through_the_codec(game: &Game) -> Game {
        let w = enc::welcome(game, 0, roster(game), "{}".into(), [0; MAX_SEATS]).expect("the map serialises");
        let Msg::Welcome(w) = decode(&encode(&Msg::Welcome(w))).expect("a welcome decodes") else { panic!("kind") };
        welcome(&w).expect("a welcome builds a replica")
    }

    /// The cosmetic state a replica carries forward on its own between two
    /// snapshots, sorted by key: the drawn hull angles, the burning tiles'
    /// flicker frames, the tread marks on the ground and the ages of the
    /// rubble in flight.
    #[derive(Default, PartialEq, Debug)]
    struct Cosmetics {
        angles: Vec<(usize, i32)>,
        burn_frames: Vec<((i32, i32), i32)>,
        marks: usize,
        decal_ages: Vec<i32>,
    }

    fn cosmetics(game: &Game) -> Cosmetics {
        let mut angles: Vec<(usize, i32)> = game
            .world
            .query::<&Tank>()
            .iter()
            .map(|t| (t.owner_slot(), (t.visual_rotation * 1000.0) as i32))
            .collect();
        angles.sort();
        let mut burn_frames: Vec<((i32, i32), i32)> = game
            .world
            .query::<&Obstacle>()
            .iter()
            .filter(|o| o.burning && !o.destroyed)
            .map(|o| (o.cell(), o.burn_frame))
            .collect();
        burn_frames.sort();
        Cosmetics {
            angles,
            burn_frames,
            marks: game.tracks.len(),
            decal_ages: game.decals.iter().map(|d| (d.age * 1000.0) as i32).collect(),
        }
    }

    /// The families a run touched, so a test can say it exercised them.
    #[derive(Default, Debug)]
    struct Seen {
        wrecks: usize,
        shots: usize,
        missiles: usize,
        changed_tiles: usize,
        fires: usize,
        pickups_taken: usize,
        frog_clips: usize,
        tiles_gone: usize,
        /// Frames on the end screen.
        ended: usize,
        /// Frames between snapshots on which the replica's own tick moved
        /// a hull's drawn angle, a tile's flicker, the tread marks or a
        /// piece of rubble in flight.
        eased: usize,
        flickered: usize,
        marked: usize,
        aged: usize,
    }

    impl Seen {
        fn record(&mut self, state: &DrawableState, first: &DrawableState) {
            self.wrecks += state.tanks.iter().filter(|t| t.wreck).count();
            self.shots += state.shots.len();
            self.missiles += state.missiles.len();
            self.changed_tiles += state
                .tiles
                .iter()
                .filter(|t| t.hp != t.material.max_health().round() as u8 || t.burning || t.fused || t.scorched != 0 || t.lean != 0)
                .count();
            self.fires += state.fires.len();
            self.pickups_taken += first.pickups.len().saturating_sub(state.pickups.len());
            self.frog_clips += state.frogs.iter().filter(|f| f.hopping || f.hurt || f.biting).count();
            self.tiles_gone += first.tiles.len().saturating_sub(state.tiles.len());
            self.ended += usize::from(state.outcome != crate::simulation::Outcome::Playing);
        }

        fn record_tick(&mut self, before: &Cosmetics, after: &Cosmetics) {
            self.eased += usize::from(before.angles != after.angles);
            self.flickered += usize::from(before.burn_frames != after.burn_frames);
            self.marked += usize::from(before.marks != after.marks);
            self.aged += usize::from(before.decal_ages != after.decal_ages);
        }
    }

    /// The state-only part of a snapshot: the frame's events and the tile
    /// deaths cut from them differ between a snapshot cut on the server
    /// and one re-encoded from the replica (which holds the interval's
    /// events, not the frame's).
    fn state_only(mut s: Snapshot) -> Snapshot {
        s.events.clear();
        s.tiles.retain(|t| t.flags & tile_flags::DESTROYED == 0);
        s
    }

    /// Run a round for `frames` frames, snapshot every third one (the
    /// interval's events accumulated the way a room server does), build
    /// the replica from a welcome at frame 60 and apply every later
    /// snapshot; on the two frames in between the replica runs
    /// `tick_presentation`, as a client does. After each apply the
    /// replica's picture must equal the round's, and re-encoding the
    /// replica must give the server's bytes - so nothing the replica
    /// animates on its own drifts out of what the wire pins.
    fn round_trip(map: &str, seed: u64, frames: u32, poke: impl Fn(&mut Game, u32)) -> Seen {
        let mut game = authoritative(map, seed, 6);
        let first = game.drawable_state();
        let mut replica: Option<Game> = None;
        let mut interval: Vec<WireEvent> = Vec::new();
        let mut seen = Seen::default();
        for frame in 1..=frames {
            poke(&mut game, frame);
            step(&mut game, frame);
            interval.extend(enc::wire_events(game.events()));
            if frame % 3 != 0 {
                if let Some(replica) = replica.as_mut() {
                    let before = cosmetics(replica);
                    replica.tick_presentation(PHYSICS_FIXED_DT);
                    seen.record_tick(&before, &cosmetics(replica));
                }
                continue;
            }
            let mut snap = enc::snapshot(&game, [0; MAX_SEATS]);
            snap.events = std::mem::take(&mut interval);
            let replica = match replica.as_mut() {
                None if frame >= 60 => replica.insert(welcome_through_the_codec(&game)),
                None => continue,
                Some(r) => {
                    let Msg::Snapshot(snap) = decode(&encode(&Msg::Snapshot(snap.clone()))).expect("decodes") else {
                        panic!("kind")
                    };
                    snapshot(r, &snap);
                    r
                }
            };
            let expected = game.drawable_state();
            assert_eq!(replica.drawable_state(), expected, "frame {frame}: the replica draws another picture");
            assert_eq!(
                state_only(enc::snapshot(replica, [0; MAX_SEATS])),
                state_only(snap),
                "frame {frame}: re-encoding the replica gives other bytes"
            );
            seen.record(&expected, &first);
        }
        seen
    }

    #[test]
    fn the_replica_draws_the_default_map_round() {
        // Frame 120, not 300: this map's round can be over well before
        // then - the mission is Protect and six enemies reach the frog
        // inside five seconds - and a kill on the end screen deals no
        // damage, so the wreck this test is about never happens. `kill_at`
        // says so out loud rather than leaving a bare "no wreck".
        let seen = round_trip(DEFAULT_MAP, 0xB0B5, 600, |game, frame| kill_at(game, frame, 120, 2));
        assert!(seen.shots > 0, "{seen:?}: nobody fired");
        assert!(seen.wrecks > 0, "{seen:?}: no wreck");
        assert!(seen.changed_tiles > 0, "{seen:?}: no tile was hit");
        assert!(seen.eased > 0, "{seen:?}: no hull swung its drawn angle between snapshots");
        assert!(seen.marked > 0, "{seen:?}: no tread mark was pressed between snapshots");
    }

    #[test]
    fn the_replica_draws_the_props_round_with_fires_and_a_launched_drum() {
        let seen = round_trip(PROPS_MAP, 0xC0FFEE, 900, |game, frame| {
            // The oil drum at the head of the trail: the fire runs the
            // trail to the fuel drum, which launches and blasts.
            if frame == 120 {
                game.debug_detonate(map::cell_to_world(17, 6)).expect("the oil drum at (17,6)");
            }
            if frame == 400 {
                game.debug_kill(1).expect("enemy in slot 1");
            }
            // Lose the round on purpose at 600, so the end screen (three
            // seconds of it) and the `RoundStarted` re-init both fall
            // inside the 900 frames whatever the battle does: a scripted
            // player whose shots land inside `player_shot_hit_pad_px`
            // keeps the frog alive past the horizon on its own.
            kill_at(game, frame, 600, 0);
        });
        assert!(seen.shots > 0, "{seen:?}: nobody fired");
        assert!(seen.wrecks > 0, "{seen:?}: no wreck");
        assert!(seen.fires > 0, "{seen:?}: nothing burned");
        assert!(seen.tiles_gone > 0, "{seen:?}: no tile died");
        assert!(seen.changed_tiles > 0, "{seen:?}: no tile changed");
        // The round is lost at 600 and restarts inside these 900 frames,
        // so the end screen and the `RoundStarted` re-init are covered too.
        assert!(seen.ended > 0, "{seen:?}: the round never ended");
        assert!(seen.eased > 0, "{seen:?}: no hull swung its drawn angle between snapshots");
        assert!(seen.aged > 0, "{seen:?}: no rubble aged between snapshots");
    }

    /// An `Obstacle` the replica has lost stays lost when the frame-0
    /// snapshot that built it is applied again: its `RoundStarted` names
    /// the round the replica is already standing in, and re-initing it
    /// would only build an identical world. A snapshot naming another seed
    /// is a real restart and does re-init.
    #[test]
    fn a_welcome_snapshot_does_not_rebuild_the_round_it_just_built() {
        let game = authoritative(DEFAULT_MAP, 0x5EED, 4);
        assert!(
            game.events().iter().any(|e| matches!(e, crate::simulation::Event::RoundStarted { .. })),
            "a round at frame 0 announces itself"
        );
        let snap = enc::snapshot(&game, [0; MAX_SEATS]);
        let mut replica = welcome_through_the_codec(&game);
        let doomed = replica.world.query::<(Entity, &Obstacle)>().iter().map(|(e, _)| e).next().expect("a tile");
        replica.world.despawn(doomed).ok();
        let tiles = replica.world.query::<&Obstacle>().iter().count();
        snapshot(&mut replica, &snap);
        assert_eq!(replica.world.query::<&Obstacle>().iter().count(), tiles, "the round was not rebuilt");
        assert_eq!(replica.round_seed(), game.round_seed());

        let other = authoritative(DEFAULT_MAP, 0xD1FF, 4);
        snapshot(&mut replica, &enc::snapshot(&other, [0; MAX_SEATS]));
        assert_eq!(replica.round_seed(), other.round_seed(), "another seed is a restart");
    }

    /// A map whose sky is `random` is drawn under one sky in the room and
    /// on every replica: the pick is the round seed's
    /// (`weather::random_sky`), and a replica stands on the room's seed -
    /// built on it by the `Welcome`, moved to the next by a rematch's
    /// `RoundStarted`. Nothing about the sky travels but the map's key.
    #[test]
    fn a_random_sky_is_the_rooms_on_every_replica() {
        use crate::map::Weather;
        use crate::weather::random_sky;
        let map = format!("weather = \"random\"\n{DEFAULT_MAP}");
        let mut skies = std::collections::BTreeSet::new();
        for seed in [0x5EED, 0xB0B5, 0xD1FF, 7, 8, 9] {
            let room = authoritative(&map, seed, 2);
            let replica = welcome_through_the_codec(&room);
            assert_eq!(replica.map.weather, Weather::Random, "the key rides the welcome's map");
            assert_ne!(room.weather(), Weather::Random);
            assert_eq!(replica.weather(), room.weather(), "seed {seed:#x}");
            skies.insert(room.weather());
        }
        assert!(skies.len() > 1, "six seeds brought one sky: {skies:?}");

        let (first, next) = (0x5EED, 0xD1FF);
        assert_ne!(random_sky(first), random_sky(next), "the rematch below has to change the sky to prove anything");
        let mut replica = welcome_through_the_codec(&authoritative(&map, first, 2));
        let rematch = authoritative(&map, next, 2);
        snapshot(&mut replica, &enc::snapshot(&rematch, [0; MAX_SEATS]));
        assert_eq!(replica.weather(), rematch.weather(), "a rematch's sky is its new seed's");
    }

    /// The sky is part of a room's rules (docs/weather.md "The rules"): a
    /// replica - and the prediction sandbox, built from the same welcome -
    /// fights under the room's sky, its water frozen where the room's is
    /// and its enemies' sight the room's, with nothing on the wire but the
    /// map's key.
    #[test]
    fn a_replica_plays_by_the_rooms_sky() {
        use crate::map::Weather;
        // A map with a river and a lake, the key put first: a key after
        // a `[table]` header would land in that table.
        const RIVER_MAP: &str = include_str!("../../maps/river.toml");
        let room_under = |sky: Weather| authoritative(&format!("weather = \"{}\"\n{RIVER_MAP}", sky.name()), 0x5EED, 2);
        for sky in Weather::ALL {
            let room = room_under(sky);
            let replica = welcome_through_the_codec(&room);
            assert!(replica.weather_from_map, "the window's override knob never reaches a room's round");
            assert_eq!(replica.weather(), room.weather(), "{sky:?}");
            assert_eq!(replica.water().is_frozen(), room.water().is_frozen(), "{sky:?}");
            assert_eq!(replica.water().deep_cells().count(), room.water().deep_cells().count(), "{sky:?}");
            assert_eq!(replica.enemy_sight(), room.enemy_sight(), "{sky:?}");
        }
        let snowy = welcome_through_the_codec(&room_under(Weather::Snow));
        assert!(snowy.water().is_frozen(), "a snowy room's lake is ice on the replica too");
        assert!(room_under(Weather::Clear).water().deep_cells().count() > 0, "and open water under any other sky");
    }

    /// The end screen's countdown belongs to the server: a replica takes
    /// the number off every snapshot and holds it in between, so the two
    /// screens read the same second and the restart lands with the round
    /// the server starts, rather than the replica counting down from zero.
    #[test]
    fn the_end_screens_countdown_travels() {
        let mut game = authoritative(DEFAULT_MAP, 0x5EED, 1);
        assert_eq!(enc::snapshot(&game, [0; MAX_SEATS]).round.restart, 0, "a live round has no countdown");
        game.debug_kill(0).expect("the player is in slot 0");
        let mut frame = 0;
        while game.outcome() == crate::simulation::Outcome::Playing {
            frame += 1;
            step(&mut game, frame);
            assert!(frame < 60, "the player's death never ended the round");
        }
        // The welcome's own snapshot puts the joiner on the end screen
        // with the seconds the server has left, not with a fresh clock.
        let mut replica = welcome_through_the_codec(&game);
        let started = replica.drawable_state().restart_tenths;
        assert!(started > 0, "the replica joined the end screen with no countdown");
        assert_eq!(replica.drawable_state(), game.drawable_state());

        let mut seen = vec![started];
        for _ in 0..40 {
            for _ in 0..3 {
                frame += 1;
                step(&mut game, frame);
            }
            let snap = enc::snapshot(&game, [0; MAX_SEATS]);
            let Msg::Snapshot(snap) = decode(&encode(&Msg::Snapshot(snap))).expect("decodes") else { panic!("kind") };
            snapshot(&mut replica, &snap);
            assert_eq!(replica.drawable_state(), game.drawable_state(), "frame {frame}");
            seen.push(replica.drawable_state().restart_tenths);
            // The two frames in between are the replica's own, and it
            // runs no clock of its own over them.
            let held = replica.drawable_state().restart_tenths;
            replica.tick_presentation(PHYSICS_FIXED_DT);
            replica.tick_presentation(PHYSICS_FIXED_DT);
            assert_eq!(replica.drawable_state().restart_tenths, held, "the replica ran the countdown itself");
        }
        assert!(seen.windows(2).all(|w| w[1] <= w[0]), "the countdown went back up: {seen:?}");
        assert!(seen.last() < seen.first(), "the countdown never moved: {seen:?}");
        assert!(game.outcome() != crate::simulation::Outcome::Playing, "the round restarted mid-test");
    }

/// **A seeker volley has to reach the replica.** Missiles are a keyed
    /// family of their own (`wire::MissileState`) because nothing else on
    /// the wire describes one: they are not shots, they have a height, and
    /// the server flies them alone. Before the family existed a volley was
    /// invisible in a room - the damage landed and the burst drew, but the
    /// missiles themselves only ever existed on the server.
    ///
    /// The round-trip assertions inside `round_trip` do the real checking,
    /// frame by frame; this test's job is to make sure a missile is
    /// actually *up* while they run, so a green result is never vacuous.
    #[test]
    fn a_seeker_volley_reaches_the_replica() {
        let seen = round_trip(DEFAULT_MAP, 0xB0B5, 420, |game, frame| {
            // Hand the seat a pod and pull the trigger: `intent` fires
            // every twelfth frame, and a pod puts a salvo of four in
            // the air per press.
            if frame == 30 {
                let patch = crate::simulation::debug::TankPatch { missile_ammo: Some(40), ..Default::default() };
                game.debug_set_tank(0, &patch).expect("the seat's tank");
            }
        });
        assert!(seen.missiles > 0, "{seen:?}: no missile was ever in the air, so nothing was checked");
    }

    #[test]
    fn a_late_joiner_sees_the_same_picture() {
        let mut game = authoritative(PROPS_MAP, 0xF406, 6);
        for frame in 1..=600 {
            if frame == 120 {
                game.debug_detonate(map::cell_to_world(20, 10)).expect("the barrel cluster");
            }
            kill_at(&mut game, frame, 150, 1);
            step(&mut game, frame);
        }
        let replica = welcome_through_the_codec(&game);
        let expected = game.drawable_state();
        assert_eq!(replica.drawable_state(), expected);
        assert!(expected.tanks.iter().any(|t| t.wreck), "the round should be busy: a wreck");
        assert!(expected.tiles.len() < authoritative(PROPS_MAP, 0xF406, 6).drawable_state().tiles.len(), "and tiles gone");
        assert_eq!(replica.frame(), game.frame());
        assert!(replica.world.query::<&crate::ai::Ai>().iter().count() == 0, "a replica's enemies have no Ai");
    }

    #[test]
    fn encoding_the_same_round_twice_gives_identical_bytes() {
        let mut game = authoritative(DEFAULT_MAP, 0xB0B5, 6);
        for frame in 1..=240 {
            step(&mut game, frame);
        }
        let a = encode(&Msg::Snapshot(enc::snapshot(&game, [1; MAX_SEATS])));
        let b = encode(&Msg::Snapshot(enc::snapshot(&game, [1; MAX_SEATS])));
        assert_eq!(a, b);
        let welcome = |g: &Game| enc::welcome(g, 0, roster(g), "{}".into(), [1; MAX_SEATS]).unwrap();
        assert_eq!(encode(&Msg::Welcome(welcome(&game))), encode(&Msg::Welcome(welcome(&game))));
        println!("default map, frame 240: snapshot {} B, welcome {} B", a.len(), encode(&Msg::Welcome(welcome(&game))).len());
    }

    #[test]
    fn a_welcome_from_another_protocol_is_refused() {
        let game = authoritative(DEFAULT_MAP, 1, 2);
        let mut w = enc::welcome(&game, 0, roster(&game), "{}".into(), [0; MAX_SEATS]).unwrap();
        w.protocol += 1;
        assert!(welcome(&w).is_err());
        w.protocol -= 1;
        w.map_toml = "not a map".into();
        assert!(welcome(&w).is_err());
    }

    /// An open waves round, the shape a room server runs when the map
    /// rolls its enemies in: one wave of one, no growth.
    fn waves_round() -> Game {
        let mut game = Game::default();
        game.seed_override = Some(0x5A1E);
        game.player_row_override = Some(3);
        game.level_overrides.mission = Some(Mission::Destroy);
        game.level_overrides.spawn = Some(SpawnKind::Waves);
        game.level_overrides.waves = Some(1);
        game.level_overrides.wave_size = Some(1);
        game.level_overrides.wave_growth = Some(0);
        game.map = MapFile::from_toml_str("version = 1\ntanks = 1\ncells.\"20,11\" = { kind = \"start\" }\n")
            .expect("the inline map parses");
        let (width, height) = game.map.field_size();
        game.init(width, height);
        game
    }

    /// `Tank::alpha` of the tank in `slot`, `None` once it is gone.
    fn wreck_alpha(game: &Game, slot: usize) -> Option<f32> {
        game.world.query::<&Tank>().iter().find(|t| t.owner_slot() == slot).map(|t| t.alpha())
    }

    /// A wave round's wreck fades out on the replica without the server
    /// sending its `despawn_timer`: the replica arms the timer the first
    /// frame it sees the wreck and runs it down, and taking the hull off
    /// the field stays the server's (`Event::WreckRemoved` plus a snapshot
    /// that no longer lists it).
    #[test]
    fn a_replicas_wreck_fades_out_on_its_own() {
        let mut game = waves_round();
        let mut slot = None;
        for frame in 1..=900 {
            step(&mut game, frame);
            if let Some(&crate::simulation::Event::TankEntered { slot: s }) =
                game.events().iter().find(|e| matches!(e, crate::simulation::Event::TankEntered { .. }))
            {
                slot = Some(s);
                break;
            }
        }
        let slot = slot.expect("a wave tank rolls in");
        game.debug_kill(slot).expect("the tank that rolled in");
        step(&mut game, 1);
        assert!(game.drawable_state().tanks.iter().any(|t| t.wreck), "the round has a wreck");

        let mut replica = welcome_through_the_codec(&game);
        assert_eq!(wreck_alpha(&replica, slot), Some(1.0), "a wreck starts at full opacity");
        let seconds = tuning().wave_wreck_despawn_seconds;
        let ticks = |s: f32| (s / PHYSICS_FIXED_DT).round() as u32;
        for _ in 0..ticks(seconds - 0.5) {
            replica.tick_presentation(PHYSICS_FIXED_DT);
        }
        let fading = wreck_alpha(&replica, slot).expect("the replica does not remove its own wrecks");
        assert!(fading > 0.0 && fading < 1.0, "half a second from removal it is fading: {fading}");
        for _ in 0..ticks(0.6) {
            replica.tick_presentation(PHYSICS_FIXED_DT);
        }
        assert_eq!(wreck_alpha(&replica, slot), Some(0.0), "faded out, and still on the field");
    }

    /// A ground fire that goes out leaves its burn scar on the replica's
    /// ground and spends its oil cell - driven by the snapshot that stops
    /// listing the cell, since burning out is the only way a fire ever
    /// leaves the list.
    #[test]
    fn a_replicas_fire_burns_out_when_the_snapshot_drops_it() {
        let mut game = authoritative(PROPS_MAP, 0xC0FFEE, 2);
        game.debug_detonate(map::cell_to_world(17, 6)).expect("the oil drum at (17,6)");
        for frame in 1..=30 {
            step(&mut game, frame);
        }
        let lit: Vec<(i32, i32)> = game.fires.iter().map(|f| f.cell).collect();
        assert!(!lit.is_empty(), "the drum leaves a burning pool");
        let mut replica = welcome_through_the_codec(&game);
        assert_eq!(replica.fires.len(), game.fires.len(), "the replica has the same pool");
        let scars = replica.scorches.len();

        let mut burnt = false;
        for frame in 31..=1800 {
            step(&mut game, frame);
            if frame % 3 == 0 {
                snapshot(&mut replica, &enc::snapshot(&game, [0; MAX_SEATS]));
            } else {
                replica.tick_presentation(PHYSICS_FIXED_DT);
            }
            if lit.iter().all(|cell| !game.fires.iter().any(|f| f.cell == *cell)) {
                snapshot(&mut replica, &enc::snapshot(&game, [0; MAX_SEATS]));
                burnt = true;
                break;
            }
        }
        assert!(burnt, "the pool never burned out");
        assert!(replica.fires.iter().all(|f| !lit.contains(&f.cell)), "the cells are out on the replica too");
        assert!(replica.scorches.len() > scars, "a burnt-out cell leaves a burn scar");
        assert_eq!(replica.oil_cells, game.oil_cells, "and the oil it burned is spent on both sides");
    }

    /// A tile the snapshot says is burning flickers on the replica, and
    /// only flickers: the charring that kills it is the server's, and the
    /// tile stays on the field until an `ObstacleDestroyed` takes it away,
    /// however long the replica runs.
    #[test]
    fn a_burning_tile_flickers_on_the_replica_without_charring_out() {
        const WOOD_MAP: &str =
            "version = 1\ntanks = 0\ncells.\"5,5\" = { kind = \"start\" }\ncells.\"10,8\" = { kind = \"wall\", material = \"wood\" }\n";
        let cell = (10, 8);
        let game = authoritative(WOOD_MAP, 1, 0);
        // Light the wall the way a fire beside it does.
        for obstacle in game.world.query::<&mut Obstacle>().iter() {
            if obstacle.cell() == cell {
                obstacle.health = 0.0;
                obstacle.burning = true;
            }
        }
        let mut replica = welcome_through_the_codec(&game);
        let tile = |g: &Game| {
            g.world.query::<&Obstacle>().iter().find(|o| o.cell() == cell).map(|o| (o.burning, o.burn_frame, o.burn_elapsed))
        };
        assert_eq!(tile(&replica), Some((true, 0, 0.0)), "the replica has the burning wall");
        let ticks = ((tuning().wood_burn_frame_seconds + PHYSICS_FIXED_DT) / PHYSICS_FIXED_DT).ceil() as u32;
        for _ in 0..ticks {
            replica.tick_presentation(PHYSICS_FIXED_DT);
        }
        assert_eq!(tile(&replica), Some((true, 1, 0.0)), "the flicker advanced, the charring did not");
        // Well past `wood_burn_seconds`, and the tile is still standing.
        for _ in 0..600 {
            replica.tick_presentation(PHYSICS_FIXED_DT);
        }
        assert!(tile(&replica).is_some(), "the replica never chars a tile out on its own");
    }

    /// `DrumLaunched` puts the drum in the air on the replica, which flies
    /// it from an age `tick_presentation` advances and drops it when it
    /// lands - the blast there is the server's own `Blast`.
    /// **A laser was invisible online**: an instant hit leaves nothing in
    /// the world for a snapshot to list, and `Fired` carries no
    /// geometry. The beam travels as its own event now, and a replica
    /// draws it and fades it as a local round does.
    #[test]
    fn a_laser_beam_arrives_as_an_event_and_is_drawn_on_the_replica() {
        let mut game = authoritative(DEFAULT_MAP, 0xB0B5, 0);
        let patch = crate::simulation::debug::TankPatch { laser_charges: Some(3), ..Default::default() };
        game.debug_set_tank(0, &patch).expect("the seat's tank");
        let mut replica = welcome_through_the_codec(&game);
        let (w, h) = game.map.field_size();
        game.update(Input::single(Intent { fire: true, ..Intent::default() }), PHYSICS_FIXED_DT, w, h);
        assert!(
            game.events().iter().any(|e| matches!(e, crate::simulation::Event::LaserBeam { .. })),
            "the authority fired a laser: {:?}",
            game.events()
        );
        let wire = enc::snapshot(&game, [0; MAX_SEATS]);
        assert!(wire.events.iter().any(|e| matches!(e, WireEvent::LaserBeam { .. })), "the beam is on the wire");
        snapshot(&mut replica, &wire);
        assert_eq!(replica.laser_beams.len(), 1, "the replica draws the beam");
        let beam = &replica.laser_beams[0];
        assert!(
            (beam.start.x - beam.end.x).abs() + (beam.start.y - beam.end.y).abs() > 32.0,
            "a beam with length: {:?}",
            (beam.start, beam.end)
        );
        // And it fades on the replica's own clock.
        for _ in 0..120 {
            replica.tick_presentation(PHYSICS_FIXED_DT);
        }
        assert!(replica.laser_beams.is_empty(), "the beam faded");
    }

    #[test]
    fn a_launched_drum_flies_on_the_replica() {
        let mut game = authoritative(PROPS_MAP, 0xC0FFEE, 2);
        let mut replica = welcome_through_the_codec(&game);
        // A cascade can launch more than one drum on the same frame.
        let mut launched: Vec<(f32, f32, f32, f32)> = Vec::new();
        for frame in 1..=900 {
            if frame == 120 {
                game.debug_detonate(map::cell_to_world(17, 6)).expect("the oil drum at (17,6)");
            }
            step(&mut game, frame);
            snapshot(&mut replica, &enc::snapshot(&game, [0; MAX_SEATS]));
            launched.extend(game.events().iter().filter_map(|e| match *e {
                crate::simulation::Event::DrumLaunched { x, y, to_x, to_y } => Some((x, y, to_x, to_y)),
                _ => None,
            }));
            if !launched.is_empty() {
                break;
            }
        }
        let (x, y, to_x, to_y) = *launched.first().expect("the trail reaches a fuel drum and launches it");
        assert_eq!(replica.flying_drums.len(), launched.len(), "the replica put every launched drum in the air");
        let drum = replica.flying_drums[0];
        // The launch travels in quarter pixels.
        assert!(drum.from.distance_to(Position::new(x, y)) <= 0.5, "from {:?} for {x},{y}", drum.from);
        assert!(drum.to.distance_to(Position::new(to_x, to_y)) <= 0.5, "to {:?} for {to_x},{to_y}", drum.to);

        let start = drum.draw_pos();
        replica.tick_presentation(PHYSICS_FIXED_DT);
        assert!(replica.flying_drums[0].draw_pos().distance_to(start) > 0.0, "the arc advances");
        let ticks = (tuning().debris_flight_seconds / PHYSICS_FIXED_DT).ceil() as u32 + 2;
        for _ in 0..ticks {
            replica.tick_presentation(PHYSICS_FIXED_DT);
        }
        assert!(replica.flying_drums.is_empty(), "and it lands");
    }

    /// A round with no mission banner, so the first `update` already
    /// plays: the frame a test's kill or detonation lands on is frame 1,
    /// and every effect list is empty before it.
    fn quiet_round(map: &str, seed: u64, enemies: usize) -> Game {
        let mut game = Game::default();
        game.show_intro = false;
        game.seed_override = Some(seed);
        game.enemy_count_override = Some(enemies);
        game.player_row_override = Some(3);
        game.level_overrides.spawn = Some(SpawnKind::Band);
        game.map = MapFile::from_toml_str(map).expect("map parses");
        let (width, height) = game.map.field_size();
        game.init(width, height);
        game
    }

    /// One idle frame of the authority, then its snapshot (that frame's
    /// events included) through the codec onto `replica`.
    fn step_and_apply(game: &mut Game, replica: &mut Game) {
        let (width, height) = game.map.field_size();
        game.update(Input::default(), PHYSICS_FIXED_DT, width, height);
        let snap = enc::snapshot(game, [0; MAX_SEATS]);
        let Msg::Snapshot(snap) = decode(&encode(&Msg::Snapshot(snap))).expect("decodes") else { panic!("kind") };
        snapshot(replica, &snap);
    }

    /// The show a game is putting on, in a form two games can be compared
    /// by: each list's entries sorted, positions rounded to whole pixels
    /// (the wire carries quarter pixels, so a float centre and its
    /// quantised twin agree to the pixel).
    #[derive(Debug, PartialEq)]
    struct Spectacles {
        /// (x, y, scale in thousandths, secondary, fuel)
        blasts: Vec<(i32, i32, i32, bool, bool)>,
        /// (x, y, strength in hundredths)
        shocks: Vec<(i32, i32, i32)>,
        scorches: Vec<(i32, i32)>,
        /// Each piece's landing spot.
        decals: Vec<(i32, i32)>,
        impacts: Vec<(i32, i32)>,
        flash: bool,
    }

    fn spectacles(game: &Game) -> Spectacles {
        let px = |p: Position| (p.x.round() as i32, p.y.round() as i32);
        let sorted = |mut v: Vec<(i32, i32)>| {
            v.sort();
            v
        };
        let mut blasts: Vec<_> = game
            .blast_fx
            .iter()
            .map(|b| {
                let (x, y) = px(b.center);
                (x, y, (b.scale * 1000.0).round() as i32, b.secondary, b.kind == crate::blast::BlastKind::Fuel)
            })
            .collect();
        blasts.sort();
        let mut shocks: Vec<_> = game
            .shocks
            .iter()
            .map(|s| {
                let (x, y) = px(s.center);
                (x, y, (s.strength * 100.0).round() as i32)
            })
            .collect();
        shocks.sort();
        Spectacles {
            blasts,
            shocks,
            scorches: sorted(game.scorches.iter().map(|s| px(s.center)).collect()),
            decals: sorted(game.decals.iter().map(|d| px(d.center)).collect()),
            impacts: sorted(game.impact_flashes.iter().map(|s| px(s.center)).collect()),
            flash: game.screen_flash.is_some(),
        }
    }

    /// A kill on the server is the same show on the replica: the kill's
    /// shockwave, the wreck's fireball (the mushroom cloud where the hash
    /// gives one), the impact flash, the screen flash, the scorch and the
    /// hull parts thrown to the same spots - all off the `Wreck` event,
    /// through the code the round itself ran.
    ///
    /// The victim stands on a whole pixel, which the wire carries exactly:
    /// the show hashes from the pixel the centre truncates to, and a
    /// centre just under a whole pixel can quantise onto the next one,
    /// which picks another fireball. Every replica reads the same wire,
    /// so they all agree with each other; only the headless server, which
    /// nobody watches, might have drawn it otherwise.
    #[test]
    fn a_kill_puts_on_the_same_show_on_the_replica() {
        let mut game = quiet_round(DEFAULT_MAP, 0xB0B5, 3);
        let slot = game.first_enemy_slot();
        let at = game.tank_snapshots().into_iter().find(|t| t.slot == slot).expect("the enemy").position;
        game.debug_teleport(slot, Position::new(at.x.round(), at.y.round()), None).expect("a whole pixel");
        let mut replica = welcome_through_the_codec(&game);
        game.debug_kill(slot).expect("an enemy to kill");
        step_and_apply(&mut game, &mut replica);
        assert!(game.events().iter().any(|e| matches!(e, crate::simulation::Event::Wreck { .. })), "the kill landed");

        let (server, client) = (spectacles(&game), spectacles(&replica));
        assert!(!server.blasts.is_empty() && !server.shocks.is_empty() && !server.scorches.is_empty(), "{server:?}");
        assert!(!server.decals.is_empty() && server.flash, "hull parts and a flash: {server:?}");
        assert_eq!(client, server, "the replica puts on the round's show");

        // The fireball is the wreck's own - the same hashed row and, where
        // the hash gives one, the same mushroom cloud.
        let fireball = |g: &Game| g.blast_fx.iter().map(|b| (b.row, b.turn, b.cloud.is_some())).collect::<Vec<_>>();
        assert_eq!(fireball(&replica), fireball(&game));
    }

    /// A drum's blast likewise: the barrel's ripple, flash quad, fireball,
    /// screen flash, scorch and thrown drum parts, off the `Blast` event -
    /// and none of the damage, which the snapshot already carries.
    #[test]
    fn a_barrel_blast_puts_on_the_same_show_on_the_replica() {
        let mut game = quiet_round(PROPS_MAP, 0xC0FFEE, 2);
        let mut replica = welcome_through_the_codec(&game);
        game.debug_detonate(map::cell_to_world(17, 6)).expect("the oil drum at (17,6)");
        step_and_apply(&mut game, &mut replica);
        assert!(game.events().iter().any(|e| matches!(e, crate::simulation::Event::Blast { .. })), "the drum went off");

        let (server, client) = (spectacles(&game), spectacles(&replica));
        assert!(server.blasts.iter().any(|b| !b.3), "a fireball: {server:?}");
        assert!(!server.shocks.is_empty() && !server.scorches.is_empty() && !server.decals.is_empty(), "{server:?}");
        assert_eq!(client, server, "the replica puts on the round's show");
    }

    /// Crates a blast breaks, with crates breakable (`simulation::crates`),
    /// are broken the same on the replica: the ammo crate's cook-off puts
    /// on its show off `CrateBroken`, the crates it broke lie loose, and a
    /// crate the blast only hurt stays hurt - and stays so as the clocks
    /// run.
    #[test]
    fn broken_crates_are_the_same_on_the_replica() {
        let map = r#"
version = 1
tanks = 1
cells."2,2" = { kind = "frog" }
cells."4,14" = { kind = "start" }
cells."10,10" = { kind = "barrel", drum = "oil" }
cells."11,10" = { kind = "pickup", pickup = "ammo" }
cells."12,10" = { kind = "pickup", pickup = "shield" }
cells."10,12" = { kind = "pickup", pickup = "speedup" }
"#;
        let mut game = quiet_round(map, 0xC4A7E, 1);
        game.crates_breakable = true;
        let mut replica = welcome_through_the_codec(&game);
        game.debug_detonate(map::cell_to_world(10, 10)).expect("the drum");
        step_and_apply(&mut game, &mut replica);
        let broke = |g: &Game| g.events().iter().filter(|e| matches!(e, crate::simulation::Event::CrateBroken { .. })).count();
        assert!(broke(&game) >= 2, "the ammo cooked off and broke the shield crate");
        assert_eq!(spectacles(&replica), spectacles(&game), "the replica puts on the round's show");
        assert_eq!(replica.drawable_state().pickups, game.drawable_state().pickups, "the same crates, hurt, burning and loose");
        let loose = game.drawable_state().pickups.iter().filter(|p| p.loose).count();
        assert!(loose >= 1, "something lies loose");
        for _ in 0..30 {
            step_and_apply(&mut game, &mut replica);
            assert_eq!(replica.drawable_state().pickups, game.drawable_state().pickups);
        }
    }

    /// The cook-offs a kill queues pop on the server seconds later, each
    /// an `Event::CookOff`; the replica pops them off that event rather
    /// than queuing its own, so each pops once.
    #[test]
    fn a_kills_cook_offs_pop_on_the_replica_off_their_events() {
        let mut game = quiet_round(DEFAULT_MAP, 0xB0B5, 3);
        let mut replica = welcome_through_the_codec(&game);
        game.debug_kill(game.first_enemy_slot()).expect("an enemy to kill");
        step_and_apply(&mut game, &mut replica);
        assert!(replica.cookoffs.is_empty(), "the replica queues nothing of its own");
        let mut popped = 0;
        for _ in 0..(tuning().cookoff_window_seconds / PHYSICS_FIXED_DT) as u32 + 2 {
            replica.tick_presentation(PHYSICS_FIXED_DT);
            step_and_apply(&mut game, &mut replica);
            let pops = game.events().iter().filter(|e| matches!(e, crate::simulation::Event::CookOff { .. })).count();
            popped += pops;
            let fresh = |g: &Game| g.blast_fx.iter().filter(|b| b.secondary && b.time == 0.0).count();
            assert_eq!(fresh(&replica), pops, "every pop the server made this frame, and no other");
            assert_eq!(fresh(&replica), fresh(&game));
        }
        assert!(popped > 0, "the wreck cooked off");
    }

    /// A replica carries the weapon the wire names and nothing else, so a
    /// module leaves the turret the snapshot its weapon is spent
    /// (`tank::module_cols`) rather than staying at its last count.
    #[test]
    fn a_spent_weapon_leaves_the_replica() {
        let mut game = quiet_round(DEFAULT_MAP, 0xB0B5, 0);
        let mut replica = welcome_through_the_codec(&game);
        let arms = |g: &Game| {
            let tank = g.world.get::<&Tank>(g.seat(0).expect("seat 0")).expect("a tank");
            (tank.active_weapon(), tank.minigun_ammo, tank.laser_charges)
        };
        let patch = |minigun: i32, laser: i32| crate::simulation::debug::TankPatch {
            minigun_ammo: Some(minigun),
            laser_charges: Some(laser),
            ..Default::default()
        };
        game.debug_set_tank(0, &patch(12, 3)).expect("the seat's tank");
        step_and_apply(&mut game, &mut replica);
        assert_eq!(arms(&replica), (ActiveWeapon::Minigun, 12, 0), "the live weapon, nothing else");
        game.debug_set_tank(0, &patch(0, 3)).expect("the seat's tank");
        step_and_apply(&mut game, &mut replica);
        assert_eq!(arms(&replica), (ActiveWeapon::Laser, 0, 3), "the spent minigun is gone");
    }

    /// A shell leaving a seat's gun kicks that tank's turret through its
    /// recoil cells on the replica as in the room (`Tank::kick`); a client
    /// that drew its own shots kicked on the press, so the room's `Fired`
    /// leaves its turret alone.
    #[test]
    fn a_fired_shell_kicks_the_turret_on_the_replica() {
        let mut game = quiet_round(DEFAULT_MAP, 0xB0B5, 0);
        let mut replica = welcome_through_the_codec(&game);
        let mut drew_own = welcome_through_the_codec(&game);
        let (width, height) = game.map.field_size();
        game.update(Input::single(Intent { fire: true, ..Intent::default() }), PHYSICS_FIXED_DT, width, height);
        assert!(game.events().iter().any(|e| matches!(e, crate::simulation::Event::Fired { slot: 0, .. })), "the seat fired");
        let snap = enc::snapshot(&game, [0; MAX_SEATS]);
        snapshot(&mut replica, &snap);
        snapshot_with(&mut drew_own, &snap, Show::OwnShotsDrawn { seat: 0, beams: 0 });
        let pose = |g: &Game| g.world.get::<&Tank>(g.seat(0).expect("seat 0")).expect("a tank").recoil_pose;
        assert_eq!(pose(&game), 1, "the room's turret kicked");
        assert_eq!(pose(&replica), 1, "and the replica's");
        assert_eq!(pose(&drew_own), 0, "not the one whose client kicked it on the press");
    }

    /// A seeker volley on the replica: a puff at each tube as its missile
    /// first shows, and each burst the server's own show - the small
    /// fireball leaning the way the missile came down, its ripple, flash
    /// and scorch - off `MissileBlast`, leaning by the heading the
    /// replica's copy of the missile last had, a tick before the dive
    /// ended. Where it lands (to the wire's quarter pixel) and which way it
    /// leans are compared; the size jitter hashes from the pixel the centre
    /// truncates to, which the wire's quarter pixel can move (see
    /// `a_kill_puts_on_the_same_show_on_the_replica`).
    #[test]
    fn a_missile_burst_puts_on_the_same_show_on_the_replica() {
        let mut game = quiet_round(DEFAULT_MAP, 0xB0B5, 2);
        let patch = crate::simulation::debug::TankPatch { missile_ammo: Some(1), ..Default::default() };
        game.debug_set_tank(0, &patch).expect("the seat's tank");
        let mut replica = welcome_through_the_codec(&game);
        let (width, height) = game.map.field_size();
        let fresh_blasts = |g: &Game| {
            let mut v: Vec<_> = g
                .blast_fx
                .iter()
                .filter(|b| b.time == 0.0)
                .map(|b| (b.center.x, b.center.y, b.offset.x.round() as i32, b.offset.y.round() as i32))
                .collect();
            v.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
            v
        };
        let (mut puffs, mut bursts) = (0, 0);
        for frame in 0..240 {
            let fire = Intent { fire: frame == 0, ..Intent::default() };
            let before = replica.muzzle_flashes.len();
            game.update(Input::single(fire), PHYSICS_FIXED_DT, width, height);
            let snap = enc::snapshot(&game, [0; MAX_SEATS]);
            snapshot(&mut replica, &snap);
            let launched = game.muzzle_flashes.iter().filter(|m| m.time == 0.0).count();
            let drawn = replica.muzzle_flashes.len() - before;
            puffs += launched;
            assert_eq!(drawn, launched, "frame {frame}: a puff per missile launched");
            if game.events().iter().any(|e| matches!(e, crate::simulation::Event::MissileBlast { .. })) {
                bursts += 1;
                let (client, server) = (fresh_blasts(&replica), fresh_blasts(&game));
                assert_eq!(client.len(), server.len(), "frame {frame}: a fireball per burst");
                for (c, s) in client.iter().zip(&server) {
                    let quarter = 0.125 + 1e-3;
                    assert!((c.0 - s.0).abs() <= quarter && (c.1 - s.1).abs() <= quarter, "frame {frame}: where it burst {c:?} for {s:?}");
                    // One 2 px step of lean at most: the heading the replica
                    // leans by is the wire's, a tick before the dive ended.
                    assert!((c.2 - s.2).abs() <= 2 && (c.3 - s.3).abs() <= 2, "frame {frame}: the lean {c:?} for {s:?}");
                }
            }
            replica.tick_presentation(PHYSICS_FIXED_DT);
        }
        assert!(puffs > 0 && bursts > 0, "a volley went up ({puffs}) and came down ({bursts})");
    }

    /// A shot that lands flashes where it landed; a flamethrower's
    /// contact, the zero-damage hit on a hull, never flashed on the server
    /// and does not here.
    #[test]
    fn a_hit_leaves_an_impact_flash_and_a_flames_contact_does_not() {
        let game = quiet_round(DEFAULT_MAP, 0xB0B5, 2);
        let mut replica = welcome_through_the_codec(&game);
        let mut snap = enc::snapshot(&game, [0; MAX_SEATS]);
        snap.events = vec![
            WireEvent::Hit { target: WireHitTarget::Wall, damage: 0.0, killed: false, x: 400, y: 800 },
            WireEvent::Hit { target: WireHitTarget::Enemy { slot: 1 }, damage: 0.0, killed: false, x: 1600, y: 1600 },
        ];
        snapshot(&mut replica, &snap);
        let at: Vec<Position> = replica.impact_flashes.iter().map(|s| s.center).collect();
        assert_eq!(at, vec![Position::new(100.0, 200.0)], "one flash, at the wall hit");
    }

    /// A trigger pull ripples at the drawn muzzle: the spawn point the
    /// round's own `Shell::spawn` gives, turned to where the turret is
    /// drawn.
    #[test]
    fn a_shot_ripples_at_the_drawn_muzzle() {
        let game = quiet_round(DEFAULT_MAP, 0xB0B5, 2);
        let mut replica = welcome_through_the_codec(&game);
        let mut snap = enc::snapshot(&game, [0; MAX_SEATS]);
        snap.events = vec![WireEvent::Fired { slot: 0, weapon: WeaponKind::Shell, input_tick: 1 }];
        let (expected, hull) = {
            let player = replica.player().expect("a seat");
            let tank = replica.world.get::<&Tank>(player).expect("its tank");
            let lateral = tuning().tank_barrel_lateral_offset[tank.row as usize];
            (Shell::spawn(&tank, tank.owner(), 0.0, -lateral).position, tank.position)
        };
        snapshot(&mut replica, &snap);
        let at: Vec<Position> = replica.muzzle_flashes.iter().map(|s| s.center).collect();
        assert_eq!(at, vec![expected]);
        assert!(expected.distance_to(hull) > 1.0, "at the barrel's tip, not the hull's centre");
    }

    /// A welcome builds a round already under way: it puts on no show for
    /// the tick it was cut on, since a late joiner has missed the moment.
    #[test]
    fn a_welcome_puts_on_no_show() {
        let mut game = quiet_round(DEFAULT_MAP, 0xB0B5, 3);
        game.debug_kill(game.first_enemy_slot()).expect("an enemy to kill");
        let (width, height) = game.map.field_size();
        game.update(Input::default(), PHYSICS_FIXED_DT, width, height);
        assert!(!game.blast_fx.is_empty(), "the server's frame has its fireball");
        let replica = welcome_through_the_codec(&game);
        assert!(replica.blast_fx.is_empty() && replica.shocks.is_empty() && replica.screen_flash.is_none());
    }

    /// Where the replica's hull stands and how many tread marks it has.
    fn player_marks(replica: &Game) -> (Position, Position, usize) {
        let player = replica.player().expect("a seat");
        let tank = replica.world.get::<&Tank>(player).expect("its tank");
        (tank.position, tank.ring_position, replica.tracks.len())
    }

    /// A hull that jumps - a portal, a gate, the room placing it - lays no
    /// tread marks across the gap and takes its ring with it, while one
    /// that drives lays marks along the way as ever.
    #[test]
    fn a_hull_that_jumps_lays_no_tread_marks_across_the_gap() {
        let game = quiet_round(DEFAULT_MAP, 0xB0B5, 0);
        let mut replica = welcome_through_the_codec(&game);
        replica.tick_presentation(PHYSICS_FIXED_DT);
        let base = enc::snapshot(&game, [0; MAX_SEATS]);
        let moved = |dx_px: i16| {
            let mut s = base.clone();
            s.events.clear();
            let seat = s.tanks.iter_mut().find(|t| t.id == 0).expect("the seat's tank");
            seat.x += dx_px * 4;
            s
        };

        let (_, _, marks) = player_marks(&replica);
        snapshot(&mut replica, &moved(40));
        replica.tick_presentation(PHYSICS_FIXED_DT);
        let (_, _, driven) = player_marks(&replica);
        assert!(driven > marks, "a drive of 40 px lays marks: {marks} -> {driven}");

        snapshot(&mut replica, &moved(40 + 200));
        replica.tick_presentation(PHYSICS_FIXED_DT);
        let (hull, ring, jumped) = player_marks(&replica);
        assert_eq!(jumped, driven, "no mark across a 200 px jump");
        assert_eq!(ring, hull, "the ring is under the hull, not flying after it");

        snapshot(&mut replica, &moved(40 + 200 + 40));
        replica.tick_presentation(PHYSICS_FIXED_DT);
        let (_, _, after) = player_marks(&replica);
        assert!(after > jumped, "and it lays marks again from where it landed");
    }

    /// The local seat's hull is drawn where its prediction has it
    /// (`net::round` writes it over the replica every frame, between the
    /// apply and the presentation tick), ahead of its wire track by the
    /// picture's delay and the round trip. That gap is no jump: a drive on
    /// the wire keeps the tread marks coming and the ring following. A
    /// jump on the wire itself still is one.
    #[test]
    fn a_hull_drawn_ahead_of_its_wire_track_is_no_jump() {
        let game = quiet_round(DEFAULT_MAP, 0xB0B5, 0);
        let mut replica = welcome_through_the_codec(&game);
        let base = enc::snapshot(&game, [0; MAX_SEATS]);
        let moved = |dx_px: i16| {
            let mut s = base.clone();
            s.events.clear();
            let seat = s.tanks.iter_mut().find(|t| t.id == 0).expect("the seat's tank");
            seat.x += dx_px * 4;
            s
        };
        let player = replica.player().expect("a seat");
        // The prediction, a hundred pixels up the road from the track.
        let draw_ahead = |replica: &mut Game| {
            let (wire, rotation) = {
                let tank = replica.world.get::<&Tank>(player).expect("its tank");
                (tank.position, tank.rotation)
            };
            replica.place_seat(0, Position::new(wire.x + 100.0, wire.y), rotation, Vec2::new(0.0, 0.0));
        };
        draw_ahead(&mut replica);
        replica.tick_presentation(PHYSICS_FIXED_DT);
        let (_, _, marks) = player_marks(&replica);

        for step in 1..=8 {
            snapshot(&mut replica, &moved(8 * step));
            {
                let tank = replica.world.get::<&Tank>(player).expect("its tank");
                assert!(tank.track_from.is_some(), "step {step}: an 8 px drive read as a jump - the marks start over");
                assert!(tank.ring_position != tank.position, "step {step}: the ring was put under the wire track");
            }
            draw_ahead(&mut replica);
            replica.tick_presentation(PHYSICS_FIXED_DT);
        }
        let (_, _, driven) = player_marks(&replica);
        assert!(driven > marks, "the drawn hull drove 64 px and laid marks: {marks} -> {driven}");

        snapshot(&mut replica, &moved(8 * 8 + 200));
        let (hull, ring, _) = player_marks(&replica);
        let tank = replica.world.get::<&Tank>(player).expect("its tank");
        assert!(tank.track_from.is_none(), "a 200 px jump on the wire lays no mark across it");
        assert_eq!(ring, hull, "and takes the ring with it");
    }

    /// A beam stopped by a shield deals nothing, and flashes where it
    /// stopped all the same, as `resolve_lasers` flashes it; the
    /// zero-damage hit a flame's contact makes still does not. A beam
    /// the client drew itself (`Show::OwnShotsDrawn`) is not drawn again,
    /// and still tells its hit apart.
    #[test]
    fn a_beam_stopped_by_a_shield_flashes_where_it_stopped() {
        let game = quiet_round(DEFAULT_MAP, 0xB0B5, 2);
        let mut snap = enc::snapshot(&game, [0; MAX_SEATS]);
        snap.events = vec![
            WireEvent::Fired { slot: 0, weapon: WeaponKind::Laser, input_tick: 1 },
            WireEvent::LaserBeam { x0: 400, y0: 400, x1: 1600, y1: 400, variant: 0, seat: 0 },
            WireEvent::Hit { target: WireHitTarget::Enemy { slot: 1 }, damage: 0.0, killed: false, x: 1600, y: 400 },
            WireEvent::Hit { target: WireHitTarget::Enemy { slot: 2 }, damage: 0.0, killed: false, x: 2000, y: 800 },
        ];
        let flashes = |g: &Game| g.impact_flashes.iter().map(|s| s.center).collect::<Vec<_>>();

        let mut replica = welcome_through_the_codec(&game);
        snapshot(&mut replica, &snap);
        assert_eq!(flashes(&replica), vec![Position::new(400.0, 100.0)], "the beam's hit flashes, the flame's contact does not");
        assert_eq!(replica.laser_beams.len(), 1);
        assert_eq!(replica.muzzle_flashes.len(), 1, "the beam's muzzle");

        let mut own = welcome_through_the_codec(&game);
        snapshot_with(&mut own, &snap, Show::OwnShotsDrawn { seat: 0, beams: 1 });
        assert_eq!(flashes(&own), vec![Position::new(400.0, 100.0)], "the client's own beam still flashes its hit");
        assert!(own.laser_beams.is_empty() && own.muzzle_flashes.is_empty(), "the client drew its own beam and muzzle");
        assert!(
            !own.events().iter().any(|e| matches!(e, crate::simulation::Event::LaserBeam { .. })),
            "and the beam is not handed on as the replica's"
        );
    }

    /// Only the beams the client claimed are left out, each by the `Fired`
    /// before it: with two of the seat's beams in a snapshot and only the
    /// second `Fired` claimed, the first beam - a press the local gate
    /// refused and the room fired - is drawn and handed on as the room's,
    /// and the second is not; a seat whose client claimed none gets both.
    #[test]
    fn only_the_beams_the_client_drew_are_left_out() {
        let game = quiet_round(DEFAULT_MAP, 0xB0B5, 2);
        let mut snap = enc::snapshot(&game, [0; MAX_SEATS]);
        let enemy = game.first_enemy_slot() as u16;
        snap.events = vec![
            WireEvent::Fired { slot: 0, weapon: WeaponKind::Laser, input_tick: 3 },
            WireEvent::LaserBeam { x0: 400, y0: 400, x1: 1600, y1: 400, variant: 0, seat: 0 },
            WireEvent::Fired { slot: enemy, weapon: WeaponKind::Shell, input_tick: 0 },
            WireEvent::Fired { slot: 0, weapon: WeaponKind::Laser, input_tick: 5 },
            WireEvent::LaserBeam { x0: 400, y0: 800, x1: 1600, y1: 800, variant: 0, seat: 0 },
        ];
        let beams = |g: &Game| g.events().iter().filter(|e| matches!(e, crate::simulation::Event::LaserBeam { .. })).count();

        let mut second = welcome_through_the_codec(&game);
        snapshot_with(&mut second, &snap, Show::OwnShotsDrawn { seat: 0, beams: 0b10 });
        assert_eq!(second.laser_beams.len(), 1, "the beam the client did not draw is drawn");
        let drawn = second.laser_beams[0].start;
        assert_eq!((drawn.x, drawn.y), (100.0, 100.0), "and it is the first `Fired`'s");
        assert_eq!(beams(&second), 1, "and handed on");
        assert_eq!(second.muzzle_flashes.len(), 2, "its muzzle flashes, and the enemy's `Fired` ripples");

        let mut none = welcome_through_the_codec(&game);
        snapshot_with(&mut none, &snap, Show::OwnShotsDrawn { seat: 0, beams: 0 });
        assert_eq!((none.laser_beams.len(), beams(&none)), (2, 2), "a client that drew no beam draws both of the room's");
    }

    /// The same, off a real round: a seat's laser into an enemy under a
    /// shield. The room flashes the hit and the replica flashes it where
    /// the room did.
    #[test]
    fn a_laser_on_a_shield_flashes_on_the_replica_as_on_the_server() {
        let mut game = quiet_round(DEFAULT_MAP, 0xB0B5, 2);
        let seat = game.tank_snapshots().into_iter().find(|t| t.slot == 0).expect("the seat");
        let rad = seat.rotation.to_radians();
        let ahead = Position::new(seat.position.x + rad.sin() * 90.0, seat.position.y - rad.cos() * 90.0);
        let enemy = game.first_enemy_slot();
        game.debug_teleport(enemy, ahead, Some(seat.rotation)).expect("an enemy");
        let shield = crate::simulation::debug::TankPatch { shield_hp: Some(500.0), ..Default::default() };
        game.debug_set_tank(enemy, &shield).expect("the enemy's tank");
        let laser = crate::simulation::debug::TankPatch { laser_charges: Some(3), ..Default::default() };
        game.debug_set_tank(0, &laser).expect("the seat's tank");
        let mut replica = welcome_through_the_codec(&game);
        let (width, height) = game.map.field_size();
        game.update(Input::single(Intent { fire: true, ..Intent::default() }), PHYSICS_FIXED_DT, width, height);
        assert!(
            game.events().iter().any(|e| matches!(e, crate::simulation::Event::Hit { damage, .. } if *damage == 0.0)),
            "the beam stopped on the shield: {:?}",
            game.events()
        );
        let snap = enc::snapshot(&game, [0; MAX_SEATS]);
        let Msg::Snapshot(snap) = decode(&encode(&Msg::Snapshot(snap))).expect("decodes") else { panic!("kind") };
        snapshot(&mut replica, &snap);
        let centres = |g: &Game| g.impact_flashes.iter().map(|s| s.center).collect::<Vec<_>>();
        let (server, client) = (centres(&game), centres(&replica));
        assert!(!server.is_empty(), "the room flashed the hit");
        assert_eq!(client.len(), server.len(), "a flash for each of the room's: {client:?} vs {server:?}");
        // The wire carries quarter pixels.
        for at in &server {
            assert!(client.iter().any(|c| c.distance_to(*at) <= 0.25), "no flash at {at:?} on the replica: {client:?}");
        }
    }

    /// A seat whose client drew its own shots (`Show::OwnShotsDrawn`) gets
    /// no ripple from its `Fired` - the client put one on at the press -
    /// while every other tank's `Fired` ripples as ever, and the event
    /// itself is handed on either way.
    #[test]
    fn a_fired_the_client_drew_itself_ripples_nothing() {
        let game = quiet_round(DEFAULT_MAP, 0xB0B5, 2);
        let enemy = game.first_enemy_slot() as u16;
        let mut snap = enc::snapshot(&game, [0; MAX_SEATS]);
        snap.events = vec![
            WireEvent::Fired { slot: 0, weapon: WeaponKind::Shell, input_tick: 1 },
            WireEvent::Fired { slot: enemy, weapon: WeaponKind::Shell, input_tick: 0 },
        ];
        let mut replica = welcome_through_the_codec(&game);
        snapshot_with(&mut replica, &snap, Show::OwnShotsDrawn { seat: 0, beams: 0 });
        assert_eq!(replica.muzzle_flashes.len(), 1, "the enemy's ripple and not the seat's");
        let fired = replica.events().iter().filter(|e| matches!(e, crate::simulation::Event::Fired { .. })).count();
        assert_eq!(fired, 2, "both `Fired` are handed on");

        let mut plain = welcome_through_the_codec(&game);
        snapshot(&mut plain, &snap);
        assert_eq!(plain.muzzle_flashes.len(), 2, "a replica that predicts nothing ripples both");
    }

    #[test]
    fn cell_indices_round_trip() {
        for cols in [1u16, 34, 40, 200] {
            for (col, row) in [(0, 0), (3, 7), (33, 16)] {
                let i = cell_index(cols, (col.min(cols as i32 - 1), row));
                assert_eq!(cell_from_index(cols, i), (col.min(cols as i32 - 1), row));
            }
        }
    }
}
