//! The supply crates' shows (docs/CRATES_SPEC.md), in the effects language
//! (docs/effects.md): the air drop a crate comes down in, the glint it
//! idles with and the crack-open it is taken with.
//!
//! Pure functions of what they draw and its age - nothing rolled, nothing
//! kept - so a replica, a paused frame and a test draw the same picture.
//! `pickup::draw_pickup` paints the drop and the glint in the field's
//! standing stage; the opening plays on the particle layer's clock
//! (`fx::CrateOpen`, started by `Event::PickupCollected`) and
//! `render/game.rs` paints it over the tanks, because the hull that takes a
//! crate is over it the frame it is taken.

use crate::math::Color;
use crate::pyro::{self, Shape};
use crate::tuning::Tuning;
use crate::{Position, CRATE_CELL, CRATE_COL_GLINT, CRATE_COL_INTACT, CRATE_GLINT_FRAMES};

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::new(r, g, b, 255)
}

/// The study's drop lands at this age; `crate_drop_seconds` stretches the
/// whole drop to its own landing.
const DROP_LANDS: f32 = 0.62;
/// How long the landing's dust takes to settle, after the landing.
const DROP_DUST_SECONDS: f32 = 0.7;
/// The study's opening puts the symbol in the tank at this age;
/// `crate_open_seconds` stretches the whole opening to its own.
const OPEN_ARRIVES: f32 = 0.75;
/// Chips thrown by the opening, and how long they lie before they go.
const OPEN_CHIPS: u32 = 12;
const CHIP_LIE_SECONDS: f32 = 1.0;
/// The ring round the tank that took the crate, after the symbol arrives.
const RING_SECONDS: f32 = 0.45;

/// One frame of a crate's air drop: a shadow gathers on the cell, the crate
/// falls into it - drawn larger while it is high - lands with a squash and
/// a stretch, and a ring of dust goes out from under it.
#[derive(Clone, Debug, PartialEq)]
pub struct Drop {
    /// Whether the crate is in the frame yet; the shadow comes first.
    pub visible: bool,
    /// How far above its cell the crate is drawn, px.
    pub lift: f32,
    /// The crate's drawn size, px: `CRATE_CELL` at rest, larger while
    /// high, squashed and stretched as it lands.
    pub w: f32,
    pub h: f32,
    /// The shadow's side, px, and its strength as a fraction of an
    /// obstacle's (`obstacle_shadow_opacity`).
    pub shadow: f32,
    pub shadow_alpha: f32,
    /// The landing's dust, drawn behind the crate.
    pub dust: Vec<Shape>,
}

/// The drop `age` seconds after a crate appeared at `at`, or `None` once it
/// has landed and its dust has settled (or with `crate_drop_seconds` 0).
pub fn drop(age: f32, at: Position, t: &Tuning) -> Option<Drop> {
    let lands = t.crate_drop_seconds;
    if lands <= 0.0 || age < 0.0 || age >= lands + DROP_DUST_SECONDS {
        return None;
    }
    let k = lands / DROP_LANDS;
    let gather = (age / (0.6 * k)).clamp(0.0, 1.0);
    let falls = 0.3 * k;
    let even = |v: f32| (v / pyro::BLOCK).round() * pyro::BLOCK;
    let mut d = Drop {
        visible: age >= falls,
        lift: 0.0,
        w: CRATE_CELL,
        h: CRATE_CELL,
        shadow: CRATE_CELL,
        shadow_alpha: 1.0,
        dust: Vec::new(),
    };
    if age < lands {
        let f = ((age - falls) / (lands - falls)).clamp(0.0, 1.0);
        let height = t.crate_drop_height_px * (1.0 - f * f);
        let grow = (4.0 * height / t.crate_drop_height_px.max(1.0)).round() * 4.0;
        d.lift = even(height);
        d.w = CRATE_CELL + grow;
        d.h = CRATE_CELL + grow;
        d.shadow = CRATE_CELL + (3.0 * (1.0 - gather)).round() * 4.0;
        d.shadow_alpha = 0.2 + 0.8 * gather;
    } else if age < lands + 0.08 {
        (d.w, d.h) = (CRATE_CELL + 4.0, CRATE_CELL - 6.0);
    } else if age < lands + 0.16 {
        (d.w, d.h, d.lift) = (CRATE_CELL - 2.0, CRATE_CELL + 2.0, 4.0);
    }
    let since = age - lands;
    if since >= 0.0 {
        let f = since / DROP_DUST_SECONDS;
        let seed = crate::blast::seed_at(at, 41);
        let n = 9;
        for i in 0..n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU + pyro::unit(seed, i) * 0.6;
            let reach = (16.0 + 22.0 * pyro::ease_out(f)) * (0.8 + 0.4 * pyro::unit(seed, i + 16));
            let pos = Position::new(at.x + a.cos() * reach, at.y + 8.0 + a.sin() * reach * 0.55 - f * 4.0);
            let radius = 4.5 - 2.5 * f;
            d.dust.push(Shape::Puff(pyro::dust_puff(pos, radius, 0.6, 1.0 - f * f)));
        }
    }
    Some(d)
}

/// The crate sheet column a standing crate shows at `time` on the round
/// clock: now and then a glint sweeps its lid (`CRATE_COL_GLINT` and the
/// frames after it), each crate on its own beat - the phase is hashed from
/// where it stands, so a field of crates never glints in step.
pub fn glint_col(time: f32, at: Position, t: &Tuning) -> usize {
    let period = t.crate_glint_period_seconds;
    let frame = t.crate_glint_frame_seconds;
    if period <= 0.0 || frame <= 0.0 {
        return CRATE_COL_INTACT;
    }
    let phase = (time + pyro::unit(crate::blast::seed_at(at, 43), 0) * period).rem_euclid(period);
    let i = (phase / frame) as usize;
    if i < CRATE_GLINT_FRAMES { CRATE_COL_GLINT + i } else { CRATE_COL_INTACT }
}

/// What rises out of an opened crate: the symbol on its own
/// (`pickup::draw_glyph`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Symbol {
    /// Its centre, already lifted.
    pub pos: Position,
    /// Its side, px (20 is the sheet's own scale).
    pub size: f32,
    /// Where its shadow falls while it is in the air.
    pub shadow: Option<Position>,
    /// How much white it blinks with, 0..1.
    pub flash: f32,
}

/// One frame of a crate being taken: it flashes, splits into planks that
/// fly off, its symbol rises, blinks and drops into the tank that took it,
/// and a ring goes round that tank.
#[derive(Clone, Debug, PartialEq)]
pub struct Open {
    /// How much of the crate still stands, 0..1 (in eighths when drawn).
    pub crate_alpha: f32,
    /// Its flash, drawn additively over it, 0..1.
    pub crate_flash: f32,
    /// Chips and dust, alpha-blended, then the light (`Shape::Glow`).
    pub shapes: Vec<Shape>,
    pub symbol: Option<Symbol>,
}

/// How long an opening plays from start to finish.
pub fn open_seconds(t: &Tuning) -> f32 {
    let k = t.crate_open_seconds / OPEN_ARRIVES;
    (0.08 * k + CHIP_LIE_SECONDS + 0.25).max(t.crate_open_seconds + RING_SECONDS)
}

/// The opening `age` seconds after a crate at `at` was taken, the symbol
/// flying to `toward` (the hull that took it, wherever that is now), in the
/// symbol's `ink` (`PickupKind::ink`). `None` once it is over.
pub fn open(age: f32, at: Position, toward: Position, ink: [Color; 3], t: &Tuning) -> Option<Open> {
    if t.crate_open_seconds <= 0.0 || age < 0.0 || age >= open_seconds(t) {
        return None;
    }
    let k = t.crate_open_seconds / OPEN_ARRIVES;
    let u = age / k;
    let seed = crate::blast::seed_at(at, 47);
    let mut o = Open { crate_alpha: 0.0, crate_flash: 0.0, shapes: Vec::new(), symbol: None };
    if u < 0.08 {
        o.crate_alpha = 1.0;
        o.crate_flash = u / 0.08 * 0.55;
    } else if u < 0.2 {
        o.crate_alpha = 1.0 - (u - 0.08) / 0.12;
    }

    // The planks: thrown from the lid, tumbling, and lying where they fall
    // for a moment before they go.
    let wood = [rgb(0x68, 0x47, 0x1D), rgb(0x99, 0x65, 0x24), rgb(0xCA, 0x8A, 0x3B), rgb(0xB5, 0x7A, 0x28)];
    let thrown = age - 0.08 * k;
    if thrown >= 0.0 {
        for i in 0..OPEN_CHIPS {
            let a = pyro::unit(seed, i) * std::f32::consts::TAU;
            let speed = 50.0 + 70.0 * pyro::unit(seed, i + 32);
            let up = 60.0 + 70.0 * pyro::unit(seed, i + 64);
            let lands = 2.0 * up / 300.0;
            let tt = thrown.min(lands);
            let x = at.x + a.cos() * speed * tt;
            let y = at.y - 6.0 + a.sin() * speed * tt * 0.7;
            let z = (up * tt - 150.0 * tt * tt).max(0.0);
            let fade = if thrown < CHIP_LIE_SECONDS { 1.0 } else { 1.0 - (thrown - CHIP_LIE_SECONDS) / 0.25 };
            if fade <= 0.0 {
                continue;
            }
            let color = if i == OPEN_CHIPS - 1 { ink[1] } else { wood[i as usize % wood.len()] };
            if z > 0.0 {
                o.shapes.push(Shape::Mark { pos: Position::new(x, y), size: 2, color: pyro::alpha(Color::new(0, 0, 0, 255), 0.3) });
            }
            let size = if i % 3 == 0 { 4 } else { 2 };
            o.shapes.push(Shape::Mark { pos: Position::new(x, y - z), size, color: pyro::alpha(color, fade) });
        }
    }
    // A puff of wood dust off the lid.
    if (0.05..0.6).contains(&u) {
        let f = (u - 0.05) / 0.55;
        for i in 0..3 {
            let pos = Position::new(at.x - 8.0 + i as f32 * 8.0, at.y - 6.0 - f * 20.0);
            o.shapes.push(Shape::Puff(pyro::Puff {
                pos,
                radius: 6.0 - 3.5 * f,
                body: rgb(0x99, 0x65, 0x24),
                shadow: Some(rgb(0x68, 0x47, 0x1D)),
                lit: Some(rgb(0xD8, 0xBF, 0x8E)),
                core: None,
                cover: 1.0 - f,
            }));
        }
    }

    // The symbol: up out of the crate, a blink, then into the tank.
    let lift_top = 26.0;
    if (0.06..0.5).contains(&u) {
        let f = ((u - 0.06) / 0.25).clamp(0.0, 1.0);
        let lift = lift_top * pyro::ease_out(f);
        let blink = u > 0.32 && ((u / 0.05) as i32) % 2 == 0;
        o.symbol = Some(Symbol {
            pos: Position::new(at.x, at.y - lift),
            size: 20.0 + (f * 2.0).round() * 4.0,
            shadow: Some(Position::new(at.x + 2.0, at.y + 4.0)),
            flash: if blink { 0.7 } else { 0.0 },
        });
        o.shapes.push(Shape::Glow { pos: Position::new(at.x, at.y - lift), radius: 16.0, color: pyro::alpha(ink[1], 0.6) });
    } else if (0.5..OPEN_ARRIVES).contains(&u) {
        let f = (u - 0.5) / (OPEN_ARRIVES - 0.5);
        let path = |g: f32| {
            let e = if g < 0.5 { 2.0 * g * g } else { 1.0 - 2.0 * (1.0 - g) * (1.0 - g) };
            Position::new(at.x + (toward.x - at.x) * e, at.y - lift_top + (toward.y - 6.0 - at.y + lift_top) * e)
        };
        let pos = path(f);
        o.symbol = Some(Symbol { pos, size: (28.0 - 20.0 * f).max(8.0), shadow: None, flash: 0.0 });
        for i in 1..=3 {
            let g = (f - i as f32 * 0.06).max(0.0);
            o.shapes.push(Shape::Mark { pos: path(g), size: 2, color: pyro::alpha(ink[2], 0.75 - i as f32 * 0.2) });
        }
    }
    // The ring round the tank that took it.
    let ring = u - OPEN_ARRIVES;
    if ring >= 0.0 && ring < RING_SECONDS / k {
        let f = ring / (RING_SECONDS / k);
        let radius = 16.0 + 18.0 * pyro::ease_out(f);
        let n = (radius * 1.2) as u32;
        for i in 0..n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            let pos = Position::new(toward.x + a.cos() * radius, toward.y + a.sin() * radius);
            let (bx, by) = pyro::block_of(pos.x, pos.y);
            if pyro::bayer(bx, by) < 1.0 - f {
                o.shapes.push(Shape::Mark { pos, size: 2, color: ink[2] });
            }
        }
        o.shapes.push(Shape::Glow { pos: toward, radius: radius + 6.0, color: pyro::alpha(ink[1], 0.5 * (1.0 - f)) });
    }
    Some(o)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pickup::PickupKind;
    use crate::tuning::Tuning;

    fn every_age(until: f32) -> impl Iterator<Item = f32> {
        (0..=(until * 120.0) as i32).map(|i| i as f32 / 120.0)
    }

    fn finite(p: Position) -> bool {
        p.x.is_finite() && p.y.is_finite()
    }

    #[test]
    fn a_drop_lands_on_its_cell_and_then_is_over() {
        let t = Tuning::DEFAULT;
        let at = Position::new(80.0, 80.0);
        let lands = t.crate_drop_seconds;
        let first = drop(0.0, at, &t).expect("a drop at its start");
        assert!(!first.visible, "the shadow gathers before the crate is in the frame");
        let falling = drop(lands * 0.8, at, &t).expect("mid-fall");
        assert!(falling.visible && falling.lift > 0.0 && falling.w > CRATE_CELL, "high and drawn larger: {falling:?}");
        let landed = drop(lands + 0.2, at, &t).expect("dust still settling");
        assert_eq!((landed.lift, landed.w, landed.h), (0.0, CRATE_CELL, CRATE_CELL), "at rest on its cell");
        assert!(!landed.dust.is_empty());
        assert!(drop(lands + DROP_DUST_SECONDS, at, &t).is_none(), "over once the dust settles");
        let mut off = Tuning::DEFAULT;
        off.crate_drop_seconds = 0.0;
        assert!(drop(0.1, at, &off).is_none(), "0 turns the drop off");
    }

    #[test]
    fn a_drop_only_ever_falls() {
        let t = Tuning::DEFAULT;
        let at = Position::new(80.0, 80.0);
        let mut last = f32::INFINITY;
        for age in every_age(t.crate_drop_seconds) {
            let d = drop(age, at, &t).unwrap();
            if d.visible && age < t.crate_drop_seconds {
                assert!(d.lift <= last, "the crate never climbs while it falls ({age})");
                assert!(d.lift % pyro::BLOCK == 0.0, "lifted by whole blocks");
                last = d.lift;
            }
        }
    }

    #[test]
    fn the_same_drop_and_opening_are_the_same_picture() {
        let t = Tuning::DEFAULT;
        let at = Position::new(144.0, 48.0);
        let ink = PickupKind::Plasma.ink();
        for age in every_age(1.5) {
            assert_eq!(drop(age, at, &t), drop(age, at, &t));
            assert_eq!(open(age, at, Position::new(100.0, 48.0), ink, &t), open(age, at, Position::new(100.0, 48.0), ink, &t));
        }
    }

    #[test]
    fn an_opening_puts_the_symbol_in_the_tank_and_ends() {
        let t = Tuning::DEFAULT;
        let at = Position::new(144.0, 80.0);
        let tank = Position::new(100.0, 80.0);
        let ink = PickupKind::Health.ink();
        let start = open(0.0, at, tank, ink, &t).unwrap();
        assert_eq!(start.crate_alpha, 1.0, "the crate is there when it is taken");
        let rising = open(0.2, at, tank, ink, &t).unwrap();
        let symbol = rising.symbol.expect("the symbol rises");
        assert!(symbol.pos.y < at.y, "above the crate");
        let arriving = open(t.crate_open_seconds * 0.99, at, tank, ink, &t).unwrap();
        let symbol = arriving.symbol.expect("still flying");
        assert!(symbol.pos.distance_to(tank) < at.distance_to(tank), "on its way to the tank");
        assert!(open(open_seconds(&t), at, tank, ink, &t).is_none());
        for age in every_age(open_seconds(&t)) {
            let o = open(age, at, tank, ink, &t).unwrap();
            assert!((0.0..=1.0).contains(&o.crate_alpha));
            for s in &o.shapes {
                match *s {
                    Shape::Mark { pos, .. } | Shape::Glow { pos, .. } => assert!(finite(pos)),
                    Shape::Puff(p) => assert!(finite(p.pos) && p.radius >= 0.0),
                    Shape::Line { from, to, .. } => assert!(finite(from) && finite(to)),
                }
            }
        }
    }

    #[test]
    fn crates_glint_now_and_then_and_not_in_step() {
        let t = Tuning::DEFAULT;
        let a = Position::new(48.0, 48.0);
        let b = Position::new(176.0, 112.0);
        let glints = |at| every_age(t.crate_glint_period_seconds).filter(|&s| glint_col(s, at, &t) != CRATE_COL_INTACT).count();
        let n = glints(a);
        assert!(n > 0, "a crate glints once a period");
        assert!(n < 120, "and stands still most of the time");
        let differ = every_age(t.crate_glint_period_seconds).any(|s| glint_col(s, a, &t) != glint_col(s, b, &t));
        assert!(differ, "two crates glint on their own beats");
        for s in every_age(t.crate_glint_period_seconds) {
            let col = glint_col(s, a, &t);
            assert!(col == CRATE_COL_INTACT || (CRATE_COL_GLINT..CRATE_COL_GLINT + CRATE_GLINT_FRAMES).contains(&col));
        }
    }
}
