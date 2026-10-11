//! The world half of the ground's memory (docs/ground-memory.md): every
//! hull's runs pressed into the wear grid (`crate::wear`) from the poses
//! the drive and the replica's walk leave, the weather wearing the marks
//! away, the silt drifting down the streams, and what the particle layer
//! reads off a hull's tread (`Game::driving`). No RNG: every choice is a
//! hash of a position or a hull's odometer, so a seeded replay presses the
//! same ground and nothing that plays reads it.

use rand::rngs::SmallRng;
use rand::RngExt;

use super::Game;
use crate::ground::{Depth, GroundFloor, GroundGrid, WaterLayout};
use crate::math::{self, Vec2};
use crate::tank::{Tank, Tread};
use crate::tuning::tuning;
use crate::wear::{Look, Roll, Sky, Stamp, Surface, WearGrid};
use crate::{Position, TANK_TRACK_FRAMES, TREAD_BY_ROW};

/// What lies under a hull, as the marks see it: the ground's tiles, the
/// water and lava over them and the rods' craters in them.
pub(crate) struct Underfoot<'a> {
    pub(crate) ground: &'a GroundGrid,
    pub(crate) water: &'a WaterLayout,
    pub(crate) lava: &'a crate::lava::LavaLayout,
    pub(crate) craters: &'a crate::rod::Craters,
}

impl Underfoot<'_> {
    pub(crate) fn surface(&self, pos: Position) -> Surface {
        match self.water.depth_at(pos) {
            Depth::Deep => Surface::Deep,
            Depth::Shallow => Surface::Ford,
            Depth::Ice => Surface::Ice,
            Depth::Dry if self.lava.depth_at(pos) != Depth::Dry => Surface::Deep,
            Depth::Dry if self.craters.under(pos) => Surface::Road,
            Depth::Dry => match self.ground.floor_at(pos) {
                GroundFloor::Road => Surface::Road,
                GroundFloor::Sand => Surface::Sand,
                GroundFloor::Grass | GroundFloor::Water => Surface::Grass,
            },
        }
    }
}

/// One hull's tread, for the particle layer (`Game::driving`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Driving {
    pub slot: usize,
    pub position: Position,
    /// Its drawn heading, degrees, 0 up and clockwise.
    pub heading: f32,
    /// Its chassis (`Tank::row`) and `Tank::scale`: where its runs are.
    pub row: usize,
    pub scale: f32,
    /// Its undamaged top speed (px/s), what its speed is measured against.
    pub top_speed: f32,
    /// Its chassis's press per pass (`wear_chassis_press`): how much a
    /// heavier hull throws.
    pub weight: f32,
    pub tread: Tread,
}

impl Game {
    /// What lies under `pos`, as the marks and the dust see it.
    pub fn surface_at(&self, pos: Position) -> Surface {
        self.underfoot().surface(pos)
    }

    pub(crate) fn underfoot(&self) -> Underfoot<'_> {
        Underfoot { ground: &self.ground, water: &self.water, lava: &self.lava, craters: &self.craters }
    }

    /// The round's ground and sky, as the marks see them.
    pub fn wear_look(&self) -> Look {
        Look { theme: self.map.theme, sky: Sky::of(self.weather) }
    }

    /// The ground's memory of the round's tread marks.
    pub fn wear(&self) -> &WearGrid {
        &self.wear
    }

    /// Every live hull's tread, in owner-slot order: what the particle
    /// layer throws dust, mud, spray and exhaust by.
    pub fn driving(&self) -> Vec<Driving> {
        let t = tuning();
        let mut out: Vec<Driving> = self
            .world
            .query::<&Tank>()
            .iter()
            .filter(|tank| !tank.is_wreck())
            .map(|tank| {
                let row = tank.row.clamp(0, 11) as usize;
                Driving {
                    slot: tank.owner_slot(),
                    position: tank.position,
                    heading: tank.visual_rotation,
                    row,
                    scale: tank.scale,
                    top_speed: tank.base_speed(),
                    weight: t.wear_chassis_press[row] * tank.tread_press,
                    tread: tank.tread,
                }
            })
            .collect();
        out.sort_by_key(|d| d.slot);
        out
    }

    /// The weather's wear on the marks and the streams' on their silt, one
    /// step of `dt` - a round's frame and a replica's presentation tick
    /// alike, since both are pure functions of the round clock.
    pub(crate) fn tick_wear(&mut self, dt: f32) {
        let t = tuning();
        let look = self.wear_look();
        let (sky, time) = (self.weather, self.time);
        let floor = Underfoot { ground: &self.ground, water: &self.water, lava: &self.lava, craters: &self.craters };
        if sky == crate::map::Weather::Sandstorm && !self.wear.is_empty() {
            let (width, height) = self.map.field_size();
            if crate::weather::gust_on_field(time, width, height, &t).is_some() {
                let full = t.sand_gust_speed.max(1.0);
                self.wear.scour(
                    |p| crate::weather::gust_at(sky, p, time, &t).length() / full,
                    |p| crate::wear::erodes(look, floor.surface(p)),
                    dt,
                    &t,
                );
            }
        }
        self.wear.drift_silt(dt, |p| self.water.pushes_south(p), |p| self.water.depth_at(p) == Depth::Shallow, &t);
    }

    /// A gravity well's drain on the marks (docs/gravity-well.md "The
    /// drain"): within each pulling well's reach they lose their pattern
    /// to a swirled smear and fade, faster the nearer its core. Run by
    /// `well_phase`'s round and by a replica's `tick_presentation`, from
    /// the field alike.
    pub(crate) fn drain_marks(&mut self, dt: f32, field: &crate::well::WellField) {
        if field.is_empty() {
            return;
        }
        let t = tuning();
        for &(_, centre) in &field.sources {
            self.wear.scrub(centre, t.well_radius_px, |d| crate::well::strength(d, &t), dt, &t);
        }
    }

    /// Burn the marks round a wreck or a drum's blast into the ground, so
    /// the place stays legible after the wreck or the drum is gone.
    pub(crate) fn char_marks(&mut self, center: Position) {
        self.wear.char_around(center, tuning().wear_char_radius_px);
    }

    /// Tall grass on a beaten lane never quite stands back up
    /// (`wear_grass_crush_floor`). Cosmetic: cover is the cells', not the
    /// tufts'.
    pub(crate) fn hold_worn_grass(&mut self) {
        if self.grass.is_empty() || self.wear.is_empty() {
            return;
        }
        let t = tuning();
        for tuft in self.grass.iter_mut().filter(|g| !g.burnt) {
            let floor = self.wear.crush_floor(tuft.base, &t);
            if floor > tuft.crush {
                tuft.crush = floor;
            }
        }
    }
}

/// Roll a spawning hull's tread: where its grouser ladder starts and how
/// hard it presses against its chassis. Four draws, two of them setting
/// nothing: they keep every seed's stream - and every pinned replay - where
/// it was.
pub(super) fn roll_tread(tank: &mut Tank, rng: &mut SmallRng) {
    let _: f32 = rng.random_range(1.5..6.0);
    let _: f32 = rng.random_range(40.0..120.0);
    let period = TREAD_BY_ROW[tank.row.clamp(0, 11) as usize].period as f32;
    tank.tread_phase = rng.random_range(0.0..std::f32::consts::TAU) / std::f32::consts::TAU * period;
    tank.tread_press = rng.random_range(0.85..1.15);
}

/// The signed turn from `from` to `to`, degrees in -180..180.
fn turn(from: f32, to: f32) -> f32 {
    (to - from + 540.0).rem_euclid(360.0) - 180.0
}

/// Press a hull's runs into the ground for the step that moved it from
/// `before` (over `dt` seconds), and advance its tread's animation frame
/// off the same travel. Must only run on steps the hull really moved on -
/// a stationary `before` reads as idle and resets the animation. Runs at
/// every live damage tier and stops once the hull is a wreck.
///
/// What it presses follows how the hull moved: rolling prints its
/// grouser ladder; turning while slow is a pivot and churns; moving across
/// its runs (a drift through a turn, a knock, a well's pull) smears; a
/// start that spins its tracks (ice, rain) churns too. Open water takes no
/// mark but wets the tracks, which then print wet for `wear_wet_carry_px`
/// of travel, dripping between the runs; mud flies off them under rain.
pub(super) fn press_treads(wear: &mut WearGrid, tank: &mut Tank, before: Position, dt: f32, floor: &Underfoot, look: Look, now: f32) {
    if tank.is_wreck() {
        return;
    }
    let t = tuning();
    let moved = tank.position.distance_to(before);
    if moved > 0.0 {
        tank.hull_anim_accum += moved;
        while tank.hull_anim_accum >= t.tank_hull_track_frame_distance {
            tank.hull_anim_accum -= t.tank_hull_track_frame_distance;
            tank.hull_frame = (tank.hull_frame + 1) % TANK_TRACK_FRAMES;
        }
    } else {
        tank.hull_frame = 0;
    }
    let heading = tank.visual_rotation;
    let turned = tank.tread.heading.is_some_and(|last| turn(last, heading).abs() > 0.5);
    let surface = floor.surface(tank.position);
    let speed = if dt > 0.0 { moved / dt } else { 0.0 };
    let velocity = if dt > 0.0 { (tank.position - before) * (1.0 / dt) } else { Vec2::new(0.0, 0.0) };
    let speeding_up = speed > tank.tread.speed + 1.0;
    let mut tread = Tread { surface, speed, velocity, speeding_up, heading: Some(heading), ..tank.tread };
    if moved <= 0.0 && !turned {
        tread.roll = Roll::Rolling;
        tank.tread = tread;
        return;
    }

    let (sin, cos) = math::sin_cos(heading.to_radians());
    let along = Vec2::new(sin, -cos);
    let step = tank.position - before;
    let side = (step.x * along.y - step.y * along.x).abs();
    let slip = if moved > 0.0 { side / moved } else { 0.0 };
    let top = tank.base_speed().max(1.0);
    let slippery = surface == Surface::Ice || look.sky == Sky::Rain;
    tread.roll = if turned && speed < t.wear_pivot_speed_fraction * top {
        Roll::Pivoting
    } else if tank.skid > 0.0 || (slip > t.wear_slip_fraction && speed > t.wear_slide_min_speed) {
        Roll::Sliding
    } else if speeding_up && slippery && speed < 0.3 * top {
        Roll::Pivoting
    } else {
        Roll::Rolling
    };
    tread.wet = if surface == Surface::Ford { 1.0 } else { (tread.wet - moved / t.wear_wet_carry_px.max(1.0)).max(0.0) };

    let row = tank.row.clamp(0, 11) as usize;
    let surfaces = |p: Position| floor.surface(p);
    let stamp = Stamp {
        at: tank.position,
        heading,
        profile: &TREAD_BY_ROW[row],
        scale: tank.scale,
        press: t.wear_chassis_press[row] * tank.tread_press,
        roll: tread.roll,
        // Half in a ford the runs still on the bank are dry yet.
        wet: if surface == Surface::Ford { 0.0 } else { tread.wet },
        phase: tank.tread_phase,
    };
    wear.press(&stamp, look, &surfaces, now, &t);

    if surface == Surface::Ford && moved > 0.0 {
        wear.stir(tank.position, tank.hull_size() * 0.5, t.wear_silt_stir * dt, now, &surfaces);
    }

    // Drops and mud off the rear of the hull, by distance rolled, at
    // places hashed from the odometer: the same on every replay.
    tread.odometer += moved;
    let slot = tank.owner_slot() as f32;
    let behind = |reach: f32, across: f32| tank.position - along * reach + Vec2::new(cos, sin) * across;
    let rear = tank.hull_size() * 0.5;
    if tread.wet > 0.05 && surface != Surface::Ford {
        tread.drip_due += moved;
        let spacing = t.wear_drip_spacing_px.max(1.0);
        while tread.drip_due >= spacing {
            tread.drip_due -= spacing;
            let h = crate::blast::seed_at(Position::new(tread.odometer, slot), 61);
            if (h % 1000) as f32 / 1000.0 < tread.wet * 0.7 {
                let across = (((h >> 10) % 1000) as f32 / 1000.0 - 0.5) * rear * 0.6;
                wear.drip(behind(rear + 2.0, across), now, &surfaces);
            }
        }
    }
    let muddy = !surface.is_water() && surface != Surface::Ice && ((look.sky == Sky::Rain && surface != Surface::Sand) || tread.wet > 0.15);
    if muddy {
        tread.splat_due += moved;
        let spacing = t.wear_splat_spacing_px.max(1.0);
        while tread.splat_due >= spacing {
            tread.splat_due -= spacing;
            let h = crate::blast::seed_at(Position::new(tread.odometer, slot), 67);
            let reach = rear + 6.0 + ((h % 1000) as f32 / 1000.0) * t.drive_mud_throw_px;
            let across = (((h >> 10) % 1000) as f32 / 1000.0 - 0.5) * rear * 2.4;
            wear.splat(behind(reach, across), now, &surfaces, &t);
        }
    }
    tank.tread = tread;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_is_the_short_way_round() {
        assert_eq!(turn(350.0, 10.0), 20.0);
        assert_eq!(turn(10.0, 350.0), -20.0);
        assert_eq!(turn(0.0, 180.0), -180.0);
        assert_eq!(turn(90.0, 90.0), 0.0);
    }
}
