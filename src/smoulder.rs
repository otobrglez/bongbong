//! What a fight leaves burning (docs/aftermath.md): the headless half of
//! the battlefield's aftermath - every remain still giving off heat, the
//! stages it cools through, and the pictures composed from it in the
//! effects language (docs/effects.md).
//!
//! A remain is a `Hot`: a wreck, the ember bed a ground fire leaves, the
//! charred rubble of a tile fire finished, a burnt tree's stump, a tower's
//! ruin, a rod's crater. Each cools through the same `Stage`s - burning,
//! smouldering, wisps, cold - on spans its kind and the sky set
//! (`spans`): rain keeps `rain_douse_factor` of every smoulder and turns
//! its smoke to steam. `Game::smouldering` gathers them each frame from
//! state the round and its replicas already keep, so nothing here goes on
//! the wire.
//!
//! The pictures are pure functions of a remain and the round clock and
//! draw no RNG: a column's puffs are born on fixed ticks and kept by a
//! hash against the stage's rate at their birth, as the volcano's plume
//! is (`volcano::plume`), so the column is one picture for one clock on
//! every client and uses none of the particle budget. Every choice hashes
//! from the remain's seed.
//!
//! The battle's haze (`step_haze`) is the one thing with memory: a load
//! the hot remains feed and the sky drains, kept as a presentation number
//! by `Game::battle_haze`. Nothing a round plays reads it.
//!
//! Two of the rules play, each behind a knob that defaults off: thick
//! smoke hides a hull (`smoke_cells`, `smoke_cover`) and a gust over a
//! hot ember bed throws embers (`ember_bed_relight`, `Game::relight_beds`).

use std::collections::BTreeSet;

use crate::map::Weather;
use crate::math::{self, Color, Rectangle};
use crate::pyro::{self, Puff, Shape, CHAR, FIRE, SMOKE};
use crate::tuning::Tuning;
use crate::{Position, OBSTACLE_GRID_SIZE};

/// What a remain is: it sets how long each stage lasts, how big its smoke
/// is and how much it feeds the haze.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A tank's wreck (`Tank::wreck_age`).
    Wreck,
    /// The scar a ground fire left (`Scorch::ember`).
    Bed,
    /// A tile's charred rubble (`Decal::hot`).
    Ruin,
    /// A burnt tree's stump (`Decal::upright`).
    Stump,
    /// A defence tower's ruin (`tower::TowerRuin`).
    Tower,
    /// A rod's crater (`rod::Crater`).
    Crater,
}

/// How far a remain has cooled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// Its own fire still burns - drawn by what already draws that fire
    /// (`damage_stage`, the particle layer, the rod's crater smoke).
    Burning,
    /// A thick column, flare-ups, glowing embers.
    Smouldering,
    /// A thin pale thread.
    Wisps,
    /// Nothing rises; a cold hulk rusts.
    Cold,
}

/// Seconds a remain of `kind` spends burning, smouldering and giving off
/// wisps, under a sky that keeps `douse` of every smoulder (`douse`).
/// Measured from when the remain appeared: a wreck from its kill, a bed
/// from its fire going out, rubble from its tile's death.
pub fn spans(kind: Kind, douse: f32, t: &Tuning) -> (f32, f32, f32) {
    let ruin = t.ruin_smoulder_seconds * douse;
    match kind {
        Kind::Wreck => (t.wreck_burn_seconds, t.wreck_smoulder_seconds * douse, t.wreck_wisp_seconds * douse),
        Kind::Bed => (0.0, t.ember_bed_seconds * douse, 0.0),
        Kind::Ruin => (0.0, ruin, ruin * 0.5),
        Kind::Stump => (0.0, ruin * 0.6, ruin * 0.35),
        Kind::Tower => (t.tower_ruin_smoke_seconds, ruin, ruin * 0.5),
        Kind::Crater => (t.rod_crater_smoke_seconds, t.crater_smoulder_seconds * douse, 0.0),
    }
}

/// The stage a remain of `kind` is in at `age`, and its heat: 1 the moment
/// it starts smouldering, 0.3 as it turns to wisps, 0 cold. A burning one
/// is 1.
pub fn stage_at(kind: Kind, age: f32, douse: f32, t: &Tuning) -> (Stage, f32) {
    let (burn, smoulder, wisps) = spans(kind, douse, t);
    if age < burn {
        return (Stage::Burning, 1.0);
    }
    let a = age - burn;
    if a < smoulder {
        return (Stage::Smouldering, 1.0 - 0.7 * a / smoulder.max(1e-3));
    }
    let a = a - smoulder;
    if a < wisps {
        return (Stage::Wisps, 0.3 * (1.0 - a / wisps.max(1e-3)));
    }
    (Stage::Cold, 0.0)
}

/// How much of every smoulder and wisp `sky` keeps: `rain_douse_factor`
/// in rain and a storm, four fifths under snow, all of it otherwise.
pub fn douse(sky: Weather, t: &Tuning) -> f32 {
    match sky {
        Weather::Rain | Weather::Storm => t.rain_douse_factor,
        Weather::Snow => 0.8,
        _ => 1.0,
    }
}

/// Whether the sun is out under `sky` to cast smoke's shadow on the floor:
/// a clear sky, dusk, heat haze and between a sandstorm's gusts.
pub fn sunlit(sky: Weather) -> bool {
    matches!(sky, Weather::Clear | Weather::Dusk | Weather::HeatHaze | Weather::Sandstorm)
}

/// Whether `sky` turns a remain's smoke to steam: rain and a storm.
pub fn steams(sky: Weather) -> bool {
    matches!(sky, Weather::Rain | Weather::Storm)
}

/// One remain still giving off heat, as `Game::smouldering` gathers it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hot {
    pub kind: Kind,
    /// Where its smoke rises from and its embers lie.
    pub at: Position,
    /// Seconds since it appeared (`spans`).
    pub age: f32,
    /// What the sky keeps of its smoulder (`douse`).
    pub douse: f32,
    /// Its smoke is steam (`steams`).
    pub steam: bool,
    /// Every cosmetic choice: a hash of where it is and what it was.
    pub seed: u32,
}

impl Hot {
    pub fn stage(&self, t: &Tuning) -> (Stage, f32) {
        stage_at(self.kind, self.age, self.douse, t)
    }

    /// What it feeds the battle's haze a second, by its stage (`step_haze`).
    pub fn load(&self, t: &Tuning) -> f32 {
        let (stage, heat) = self.stage(t);
        let (burning, smouldering) = match self.kind {
            Kind::Wreck => (1.0, 0.6),
            Kind::Tower => (0.8, 0.55),
            Kind::Ruin => (0.6, 0.55),
            Kind::Bed => (0.5, 0.2),
            Kind::Stump => (0.4, 0.15),
            Kind::Crater => (0.4, 0.2),
        };
        match stage {
            Stage::Burning => burning,
            Stage::Smouldering => smouldering * (0.4 + 0.6 * heat),
            Stage::Wisps => if self.kind == Kind::Wreck { 0.14 } else { 0.08 },
            Stage::Cold => if self.kind == Kind::Wreck { t.battle_haze_hulk_load } else { 0.0 },
        }
    }

    /// How hard it is flaring up now, 0 to about 0.45: in a hashed half of
    /// some of its three-second stretches while it smoulders
    /// (`smoulder_flare_chance`), never in the rain.
    pub fn flare(&self, t: &Tuning) -> f32 {
        let (stage, heat) = self.stage(t);
        if stage != Stage::Smouldering || self.steam {
            return 0.0;
        }
        let stretch = self.age / 3.0;
        let k = stretch.floor();
        if pyro::unit(self.seed, 0xF1A0 ^ k as u32) >= t.smoulder_flare_chance || stretch - k >= 0.5 {
            return 0.0;
        }
        0.45 * heat.max(0.3)
    }

    /// How bright its embers are, 0 to 1: its heat while it smoulders, a
    /// faint glow through its wisps.
    pub fn ember(&self, t: &Tuning) -> f32 {
        match self.stage(t) {
            (Stage::Smouldering, heat) => heat,
            (Stage::Wisps, heat) => heat * 0.6,
            _ => 0.0,
        }
    }

    /// Its seed's salt `k`, for a choice of its own.
    fn salt(&self, k: u32) -> u32 {
        self.seed ^ k.wrapping_mul(0x9E37_79B9)
    }
}

/// The battle's haze `dt` seconds on from `haze`, the hot remains feeding
/// it `load` a second (`Hot::load`, summed) and the sky draining it: by a
/// factor e every `battle_haze_seconds` under a clear sky,
/// `battle_haze_rain_seconds` in the rain, faster in a sandstorm. 0 to 1.
pub fn step_haze(haze: f32, load: f32, dt: f32, sky: Weather, t: &Tuning) -> f32 {
    let drain = match sky {
        Weather::Rain | Weather::Storm => t.battle_haze_rain_seconds,
        Weather::Sandstorm => t.battle_haze_seconds * 0.37,
        _ => t.battle_haze_seconds,
    };
    let next = haze + (t.battle_haze_gain * load * (1.0 - haze) - haze / drain.max(1.0)) * dt;
    next.clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// The column.

/// How often a column may put up a puff (seconds), and the longest a puff
/// lives.
const PUFF_EVERY: f32 = 1.0 / 12.0;
const PUFF_LIFE_MAX: f32 = 4.8;

/// How a remain's smoke looks for a puff born while it was in `stage` at
/// `heat`: puffs a second, life, start and end radius (px), rise (px/s),
/// its three shades and how much of it is drawn. `None` for a stage that
/// puts up no composed smoke - a burning remain's is its fire's.
fn smoke_of(kind: Kind, stage: Stage, heat: f32, steam: bool, t: &Tuning) -> Option<(f32, f32, f32, f32, f32, [Color; 3], f32)> {
    let (rate_k, size) = match kind {
        Kind::Wreck => (1.0, 1.0),
        Kind::Tower => (0.9, 1.1),
        Kind::Ruin => (0.9, 1.0),
        Kind::Bed => (0.35, 0.6),
        Kind::Stump => (0.4, 0.55),
        Kind::Crater => (0.5, 0.8),
    };
    let (rate, life, r0, r1, alpha_k) = match stage {
        Stage::Smouldering => (t.smoulder_column_rate * rate_k * (0.35 + 0.65 * heat), 4.6, 3.0 * size, 12.0 * size * (0.6 + 0.4 * heat), 0.875),
        Stage::Wisps if kind != Kind::Bed => (1.7 * rate_k, 3.6, 2.0 * size, 6.0 * size, 0.625),
        _ => return None,
    };
    if steam {
        // Steam: pale, quick and short-lived.
        return Some((rate, life * 0.4, r0, r1 * 0.8, 34.0, [SMOKE[4], SMOKE[5], SMOKE[6]], alpha_k * 0.85));
    }
    // Hot smoke is dark; it pales as the remain cools and in its wisps.
    let s = if stage == Stage::Wisps { 4 } else if heat > 0.65 { 2 } else { 3 };
    Some((rate, life, r0, r1, 22.0, [SMOKE[s - 1], SMOKE[s], SMOKE[s + 1]], alpha_k))
}

/// A remain's column at round `time`, appended to `out` back to front:
/// shaded puffs leaning with the grass's wind (`pyro::smoke_lean`) and
/// thrown along by a sandstorm's gust (`weather::gust_at`), each born on a
/// fixed tick and kept by a hash against its stage's rate at its birth.
/// With `shadows`, each puff's shadow on the floor goes there as a
/// `Shape::Disc`, down and to the right by its height
/// (`smoke_shadow_offset`). Returns the puffs drawn.
pub fn column(hot: &Hot, time: f32, sky: Weather, t: &Tuning, out: &mut Vec<Shape>, mut shadows: Option<&mut Vec<Shape>>) -> usize {
    let first = ((time - PUFF_LIFE_MAX) / PUFF_EVERY).floor() as i64;
    let last = (time / PUFF_EVERY).floor() as i64;
    let shadow = pyro::alpha(SMOKE[0], ((1.0 - t.smoke_shadow_strength) * 1.4).clamp(0.0, 1.0));
    let mut drawn = 0;
    for k in first..=last {
        let born = k as f32 * PUFF_EVERY;
        let age = time - born;
        let at_birth = hot.age - age;
        if age < 0.0 || at_birth < 0.0 {
            continue;
        }
        let (stage, heat) = stage_at(hot.kind, at_birth, hot.douse, t);
        let Some((rate, life, r0, r1, rise_speed, shades, alpha_k)) = smoke_of(hot.kind, stage, heat, hot.steam, t) else {
            continue;
        };
        let ki = k as u32;
        if pyro::unit(hot.salt(ki), 1) >= rate * PUFF_EVERY {
            continue;
        }
        let life = life * (0.7 + 0.3 * pyro::unit(hot.salt(ki), 2));
        let f = age / life;
        if f >= 1.0 {
            continue;
        }
        let rise = age * rise_speed * (1.0 - 0.25 * f);
        let lean = pyro::smoke_lean(t, hot.at, born);
        let gust = crate::weather::gust_at(sky, hot.at, born, t);
        let wobble = math::sin(age * 1.7 + k as f32) * 1.5 * f;
        let x = hot.at.x + (pyro::unit(hot.salt(ki), 3) - 0.5) * 6.0 + lean * rise + gust.x * age * t.gust_tears_smoke * 0.5 + wobble;
        let y = hot.at.y - 8.0 - rise + gust.y * age * t.gust_tears_smoke * 0.5;
        let mut r = r0 + (r1 - r0) * (f * 1.6).min(1.0);
        if f > 0.62 {
            r *= 1.0 - (f - 0.62) / 0.38 * 0.5;
        }
        let cover = if f < 0.75 { 1.0 } else { 1.0 - (f - 0.75) / 0.25 };
        let [shade, body, lit] = shades.map(|c| pyro::alpha(c, alpha_k));
        out.push(Shape::Puff(Puff { pos: Position::new(x, y), radius: r, body, shadow: Some(shade), lit: Some(lit), core: None, cover }));
        drawn += 1;
        if let Some(shadows) = shadows.as_deref_mut()
            && !hot.steam
            && cover > 0.5
        {
            let height = hot.at.y - y;
            let off = height * t.smoke_shadow_offset;
            shadows.push(Shape::Disc { center: Position::new(x + off, hot.at.y + off * 0.8), radius: r * 0.9, color: shadow, cover: 0.5 });
        }
    }
    drawn
}

// ---------------------------------------------------------------------------
// Embers.

/// The glints of a remain lying on the ground - an ember bed, charred
/// rubble, a stump's foot - within `radius` px of it at `time`: a hashed
/// share of the blocks (`ember_bed_glint` at its hottest) each breathing
/// through the fire ramp's low steps on its own beat, and one glow over
/// the lot (drawn in the additive pass).
pub fn glints(hot: &Hot, time: f32, radius: f32, t: &Tuning, out: &mut Vec<Shape>) {
    let ember = hot.ember(t);
    if ember <= 0.0 || radius < pyro::BLOCK {
        return;
    }
    let share = t.ember_bed_glint * ember;
    let (cx, cy) = pyro::block_of(hot.at.x, hot.at.y);
    let m = (radius / pyro::BLOCK).ceil() as i32;
    for by in -m..=m {
        for bx in -m..=m {
            if (bx * bx + by * by) as f32 > (m * m) as f32 {
                continue;
            }
            let h = pyro::unit(hot.seed, ((bx + 64) as u32) << 8 | (by + 64) as u32);
            if h >= share {
                continue;
            }
            // Each glint breathes on its own beat between the fire ramp's
            // darkest steps and, at a hot bed's peak, its red; the bed dims
            // as it cools.
            let beat = (math::sin(time * (1.5 + h * 6.0) + h * 40.0) + 1.0) * 0.5;
            let step = ((ember * 1.6 + beat * 2.0) as usize).min(3);
            let pos = Position::new(((cx + bx) as f32 + 0.5) * pyro::BLOCK, ((cy + by) as f32 + 0.5) * pyro::BLOCK);
            out.push(Shape::Mark { pos, size: 2, color: FIRE[step] });
        }
    }
    out.push(Shape::Glow { pos: hot.at, radius: radius * 0.9, color: pyro::alpha(FIRE[3], 0.14 * ember) });
}

/// A smouldering remain's flare-up (`Hot::flare`): a few small tongues of
/// flame standing on it, leaning with the wind, and their light. Empty
/// while it is not flaring.
pub fn flare(hot: &Hot, time: f32, spread: f32, t: &Tuning, out: &mut Vec<Shape>) {
    let strength = hot.flare(t);
    if strength <= 0.0 {
        return;
    }
    let lean = pyro::smoke_lean(t, hot.at, time);
    pyro::tongues(out, Position::new(hot.at.x, hot.at.y + 4.0), spread, 18.0, 3, hot.seed, time, lean, strength);
    out.push(Shape::Glow { pos: Position::new(hot.at.x, hot.at.y - 4.0), radius: 22.0, color: pyro::alpha(FIRE[5], 0.2 * strength / 0.45) });
}

// ---------------------------------------------------------------------------
// The air.

/// Spacing (px) of the lattice the haze's banks drift along: one bank per
/// cell, so a field of any size carries the same density.
const BANK_SPACING: f32 = 240.0;

/// Puffs a bank of the haze is drawn as (`haze_banks`).
const BANK_PUFFS: u32 = 9;

/// The battle's haze over the part of the field `view` shows, at `haze`
/// (0 to 1, `Game::battle_haze`): banks of smoke drifting with the wind
/// along a lattice anchored to the field, so a field of any size carries
/// the same density and a bank never jumps. They drift at the wind's mean
/// (`pyro::mean_smoke_lean`), a steady speed whatever the gusts and
/// wherever the view is. A bank is a loose cluster of
/// small dithered discs, densest in its middle, so it has no hard edge;
/// each disc thins to nothing within `battle_haze_clear_px` of a seat in
/// `seats`, so a hull is never lost under it, and none covers more than
/// `battle_haze_max`. One flat step of the smoke ramp, its blocks fading
/// in eighths (effects rule 3); `pale` (under a dark sky) a lighter step,
/// so the night makes smoke a pale veil rather than a black blot.
pub fn haze_banks(view: Rectangle, seats: &[Position], time: f32, haze: f32, pale: bool, t: &Tuning, out: &mut Vec<Shape>) {
    let strength = haze.clamp(0.0, 1.0) * t.battle_haze_max;
    if strength <= 0.02 {
        return;
    }
    let drift = time * (4.0 + 10.0 * pyro::mean_smoke_lean(t).max(0.0));
    let s = BANK_SPACING;
    let (i0, i1) = (((view.x - drift - s) / s).floor() as i32, ((view.x + view.width - drift + s) / s).ceil() as i32);
    let (j0, j1) = (((view.y - s) / s).floor() as i32, ((view.y + view.height + s) / s).ceil() as i32);
    let clear = t.battle_haze_clear_px.max(1.0);
    let color = pyro::alpha(if pale { SMOKE[5] } else { SMOKE[3] }, 0.5);
    for j in j0..=j1 {
        for i in i0..=i1 {
            let seed = crate::blast::seed_at(Position::new(i as f32 * 31.0, j as f32 * 17.0), 0x4A2E);
            let dens = strength * (0.55 + 0.45 * pyro::unit(seed, 1));
            let cx = i as f32 * s + drift + pyro::unit(seed, 2) * s * 0.6;
            let cy = j as f32 * s + pyro::unit(seed, 3) * s * 0.6 + math::sin(time * 0.06 + i as f32) * 6.0;
            for p in 0..BANK_PUFFS {
                // Strung out along the wind, thicker at the middle.
                let angle = pyro::unit(seed, 40 + p) * std::f32::consts::TAU;
                let out_by = pyro::unit(seed, 50 + p).sqrt();
                let pos = Position::new(cx + math::cos(angle) * out_by * 80.0, cy + math::sin(angle) * out_by * 40.0);
                let r = 12.0 + 16.0 * pyro::unit(seed, 10 + p);
                let near = seats.iter().map(|seat| seat.distance_to(pos)).fold(f32::INFINITY, f32::min);
                let thin = ((near - clear - r) / (clear * 1.5)).clamp(0.0, 1.0);
                let cover = dens * thin * (1.0 - 0.6 * out_by);
                if cover <= 0.02 {
                    continue;
                }
                out.push(Shape::Disc { center: pos, radius: r, color, cover: cover.min(1.0) });
            }
        }
    }
}

/// What a full haze grades the whole field toward: a warm grey at
/// `battle_haze_grade` of `haze`. `None` while there is too little haze to
/// show.
pub fn haze_grade(haze: f32, t: &Tuning) -> Option<Color> {
    let a = haze.clamp(0.0, 1.0) * t.battle_haze_grade;
    (a > 0.01).then(|| pyro::alpha(Color::new(0x5C, 0x50, 0x42, 0xFF), a))
}

/// Spacing (px) of the lattice ash falls on.
const ASH_CELL: f32 = 100.0;

/// Ash falling over `view` under `haze`: none below `ash_from_haze`, then
/// up to `ash_density` flakes per 10,000 px^2, each a block of the smoke
/// ramp's palest steps drifting down and with the wind on a lattice
/// anchored to the field. Hashed, so a paused frame holds still.
pub fn ash(view: Rectangle, time: f32, haze: f32, t: &Tuning, out: &mut Vec<Shape>) {
    let from = t.ash_from_haze.clamp(0.0, 0.99);
    if haze <= from || t.ash_density <= 0.0 {
        return;
    }
    let per_cell = t.ash_density * (haze - from) / (1.0 - from);
    let lean = pyro::mean_smoke_lean(t);
    let (i0, i1) = ((view.x / ASH_CELL).floor() as i32, ((view.x + view.width) / ASH_CELL).ceil() as i32);
    let (j0, j1) = ((view.y / ASH_CELL).floor() as i32, ((view.y + view.height) / ASH_CELL).ceil() as i32);
    for j in j0..j1 {
        for i in i0..i1 {
            let seed = crate::blast::seed_at(Position::new(i as f32 * 13.0, j as f32 * 7.0), 0xA5);
            let n = per_cell.floor() as u32 + u32::from(pyro::unit(seed, 0) < per_cell.fract());
            for k in 0..n {
                let speed = 0.06 + 0.06 * pyro::unit(seed, 10 + k);
                let fall = (time * speed + pyro::unit(seed, 20 + k)).fract();
                let sway = (pyro::unit(seed, 30 + k) + time * lean * 0.08 + math::sin(time * 0.9 + k as f32) * 0.03).rem_euclid(1.0);
                let pos = Position::new((i as f32 + sway) * ASH_CELL, (j as f32 + fall) * ASH_CELL);
                out.push(Shape::Mark { pos, size: 2, color: pyro::alpha(SMOKE[4 + (k % 3) as usize], 0.875) });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The ground.

/// The snow's own white, as the weather's ground pass lays it.
pub const SNOW: Color = Color::new(0xF6, 0xF9, 0xFC, 0xFF);

/// How much of a cold mark snowfall has covered `cold` seconds after it
/// went cold: up to 0.85 over `snow_cover_seconds`, so a buried scorch
/// still shows through.
pub fn snow_cover(cold: f32, t: &Tuning) -> f32 {
    (cold / t.snow_cover_seconds.max(1.0)).clamp(0.0, 0.85)
}

/// The wet ring a hot remain melts in the snow round it: dark wet earth
/// in the middle, its edge dissolving into the snow. Empty once it is
/// cold.
pub fn melt(hot: &Hot, t: &Tuning, out: &mut Vec<Shape>) {
    let (stage, heat) = hot.stage(t);
    if stage == Stage::Cold || t.snow_melt_px <= 0.0 {
        return;
    }
    let r = t.snow_melt_px * (0.6 + 0.4 * heat);
    out.push(Shape::Disc { center: hot.at, radius: r, color: pyro::alpha(pyro::DUST[0], 0.875), cover: 0.6 });
    out.push(Shape::Disc { center: hot.at, radius: r * 0.7, color: CHAR[3], cover: 1.0 });
}

// ---------------------------------------------------------------------------
// Smoke that hides (opt-in).

/// The aftermath's rules that play, each off by default
/// (`Game::aftermath_rules`, read once by `init`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rules {
    /// Thick smoke hides a hull (`smoke_cells`).
    pub smoke_cover: bool,
    /// Under a dark sky a tank by a fire is seen at full range
    /// (`Game::is_lit`).
    pub fire_reveals: bool,
    /// A gust over a hot ember bed throws embers (`Game::relight_beds`).
    pub ember_relight: bool,
}

impl Rules {
    pub fn of(t: &Tuning) -> Rules {
        Rules { smoke_cover: t.smoke_cover, fire_reveals: t.fire_reveals, ember_relight: t.ember_bed_relight }
    }
}

/// The cells a remain's thick smoke hides a hull in, under `smoke_cover`:
/// for every wreck, ruin and tower ruin still burning, or smouldering at
/// `smoke_cover_strength` or more, its own cell and `smoke_cover_cells`
/// more downwind (`grass_gust_heading_deg`). Sorted.
pub fn smoke_cells(hots: &[Hot], t: &Tuning) -> BTreeSet<(i32, i32)> {
    let mut cells = BTreeSet::new();
    let heading = t.grass_gust_heading_deg.to_radians();
    let (dx, dy) = (math::cos(heading), math::sin(heading));
    for hot in hots.iter().filter(|h| matches!(h.kind, Kind::Wreck | Kind::Ruin | Kind::Tower)) {
        let strength = match hot.stage(t) {
            (Stage::Burning, _) => 1.0,
            (Stage::Smouldering, heat) => heat,
            _ => 0.0,
        };
        if strength < t.smoke_cover_strength {
            continue;
        }
        for i in 0..=t.smoke_cover_cells.max(0) {
            let d = i as f32 * OBSTACLE_GRID_SIZE;
            cells.insert(crate::map::world_to_cell(Position::new(hot.at.x + dx * d, hot.at.y + dy * d)));
        }
    }
    cells
}

/// Whether the grass's wind at `at` is in a strong gust at `time`: what a
/// hot ember bed throws embers on under `ember_bed_relight`. A pure
/// function of the clock, like the wind.
pub fn gust_strong(at: Position, time: f32, sky: Weather, t: &Tuning) -> bool {
    let calm = t.grass_wind_px + 0.75 * t.grass_gust_px;
    let along = crate::trig::cos(t.grass_gust_heading_deg.to_radians()).abs().max(0.1);
    if crate::grass::wind_at(t, at, time) / along >= calm {
        return true;
    }
    let gust = crate::weather::gust_at(sky, at, time, t);
    gust.x * gust.x + gust.y * gust.y >= (0.5 * t.sand_gust_speed).powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Color;
    use crate::pyro::Blocks;

    #[derive(Default)]
    struct Rects(Vec<(i32, i32, i32, i32, Color)>);

    impl Blocks for Rects {
        fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Color) {
            self.0.push((x, y, w, h, c));
        }
    }

    fn hot(kind: Kind, age: f32) -> Hot {
        Hot { kind, at: Position::new(320.0, 320.0), age, douse: 1.0, steam: false, seed: 77 }
    }

    #[test]
    fn a_wreck_burns_then_smoulders_then_wisps_then_goes_cold() {
        let t = Tuning::default();
        let burn = t.wreck_burn_seconds;
        assert_eq!(stage_at(Kind::Wreck, 1.0, 1.0, &t).0, Stage::Burning);
        assert_eq!(stage_at(Kind::Wreck, burn + 1.0, 1.0, &t).0, Stage::Smouldering);
        assert_eq!(stage_at(Kind::Wreck, burn + t.wreck_smoulder_seconds + 1.0, 1.0, &t).0, Stage::Wisps);
        assert_eq!(stage_at(Kind::Wreck, burn + t.wreck_smoulder_seconds + t.wreck_wisp_seconds + 1.0, 1.0, &t).0, Stage::Cold);
        let (_, hot) = stage_at(Kind::Wreck, burn + 1.0, 1.0, &t);
        let (_, cool) = stage_at(Kind::Wreck, burn + t.wreck_smoulder_seconds - 1.0, 1.0, &t);
        assert!(hot > cool, "it cools as it smoulders");
    }

    #[test]
    fn rain_shortens_every_smoulder_and_turns_it_to_steam() {
        let t = Tuning::default();
        let age = t.wreck_burn_seconds + t.wreck_smoulder_seconds * 0.5;
        assert_eq!(stage_at(Kind::Wreck, age, douse(Weather::Clear, &t), &t).0, Stage::Smouldering);
        assert_ne!(stage_at(Kind::Wreck, age, douse(Weather::Rain, &t), &t).0, Stage::Smouldering);
        assert!(steams(Weather::Storm) && !steams(Weather::Snow));
        let wet = Hot { steam: true, ..hot(Kind::Wreck, t.wreck_burn_seconds + 5.0) };
        assert_eq!(wet.flare(&t), 0.0, "nothing flares in the rain");
    }

    #[test]
    fn a_column_rises_from_a_smouldering_remain_and_none_from_a_cold_one() {
        let t = Tuning::default();
        let time = 500.0;
        let mut out = Vec::new();
        let n = column(&hot(Kind::Wreck, t.wreck_burn_seconds + 20.0), time, Weather::Clear, &t, &mut out, None);
        assert!(n > 10, "a thick column: {n}");
        let rises = out.iter().all(|s| matches!(s, Shape::Puff(p) if p.pos.y < 320.0));
        assert!(rises, "every puff above its source");
        let mut cold = Vec::new();
        assert_eq!(column(&hot(Kind::Wreck, 10_000.0), time, Weather::Clear, &t, &mut cold, None), 0);
        let mut burning = Vec::new();
        assert_eq!(column(&hot(Kind::Wreck, 1.0), time, Weather::Clear, &t, &mut burning, None), 0, "a burning wreck's smoke is its fire's");
    }

    #[test]
    fn the_same_clock_draws_the_same_column() {
        let t = Tuning::default();
        let h = hot(Kind::Ruin, 30.0);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        column(&h, 123.4, Weather::Clear, &t, &mut a, None);
        column(&h, 123.4, Weather::Clear, &t, &mut b, None);
        assert_eq!(a, b);
    }

    #[test]
    fn a_column_casts_its_shadow_down_and_right() {
        let t = Tuning::default();
        let (mut puffs, mut shadows) = (Vec::new(), Vec::new());
        column(&hot(Kind::Wreck, t.wreck_burn_seconds + 10.0), 200.0, Weather::Clear, &t, &mut puffs, Some(&mut shadows));
        assert!(!shadows.is_empty());
        assert!(shadows.iter().all(|s| matches!(s, Shape::Disc { center, .. } if center.y >= 320.0)), "on the floor at or below the source");
        let mut steam = Vec::new();
        let wet = Hot { steam: true, ..hot(Kind::Wreck, t.wreck_burn_seconds + 10.0) };
        column(&wet, 200.0, Weather::Rain, &t, &mut Vec::new(), Some(&mut steam));
        assert!(steam.is_empty(), "no sun to cast it in the rain");
    }

    #[test]
    fn an_ember_bed_glints_on_the_grid_and_dims_as_it_cools() {
        let t = Tuning::default();
        let count = |age| {
            let mut out = Vec::new();
            glints(&hot(Kind::Bed, age), 3.0, 20.0, &t, &mut out);
            out.iter().filter(|s| matches!(s, Shape::Mark { .. })).count()
        };
        assert!(count(2.0) > count(t.ember_bed_seconds * 0.9), "fewer glints as it cools");
        assert_eq!(count(t.ember_bed_seconds + 1.0), 0, "a cold bed is only its scar");
        let mut out = Vec::new();
        glints(&hot(Kind::Bed, 2.0), 3.0, 20.0, &t, &mut out);
        let mut r = Rects::default();
        pyro::draw(&mut r, &out);
        assert!(r.0.iter().all(|q| q.0 % 2 == 0 && q.1 % 2 == 0), "on the block grid");
    }

    #[test]
    fn the_haze_builds_under_a_fight_and_the_rain_drains_it() {
        let t = Tuning::default();
        let mut clear = 0.0;
        let mut rain = 0.0;
        for _ in 0..(180 * 60) {
            clear = step_haze(clear, 4.0, 1.0 / 60.0, Weather::Clear, &t);
            rain = step_haze(rain, 4.0, 1.0 / 60.0, Weather::Rain, &t);
        }
        assert!(clear > 0.4, "three minutes of four hot remains: {clear}");
        assert!(rain < clear * 0.5, "the rain keeps it down: {rain}");
        let mut after = clear;
        for _ in 0..(600 * 60) {
            after = step_haze(after, 0.0, 1.0 / 60.0, Weather::Clear, &t);
        }
        assert!(after < clear * 0.5, "it drains once the fires are out");
    }

    #[test]
    fn the_haze_thins_round_a_seat() {
        let t = Tuning::default();
        let view = Rectangle::new(0.0, 0.0, 1200.0, 800.0);
        let seat = Position::new(600.0, 400.0);
        let mut out = Vec::new();
        haze_banks(view, &[seat], 50.0, 1.0, false, &t, &mut out);
        assert!(!out.is_empty());
        for shape in &out {
            let Shape::Disc { center, radius, cover, .. } = *shape else { panic!("a bank is discs") };
            assert!(center.distance_to(seat) > t.battle_haze_clear_px + radius, "no bank disc reaches into a seat's clear ring");
            assert!(cover <= t.battle_haze_max + 1e-4);
        }
        let mut none = Vec::new();
        haze_banks(view, &[seat], 50.0, 0.0, false, &t, &mut none);
        assert!(none.is_empty());
    }

    #[test]
    fn the_haze_drifts_steadily_downwind_however_long_the_round() {
        let t = Tuning::default();
        let view = Rectangle::new(0.0, 0.0, 1200.0, 800.0);
        let banks = |time: f32| {
            let mut out = Vec::new();
            haze_banks(view, &[], time, 1.0, false, &t, &mut out);
            out.into_iter()
                .map(|s| match s {
                    Shape::Disc { center, radius, .. } => (center, radius),
                    _ => panic!("a bank is discs"),
                })
                .collect::<Vec<_>>()
        };
        // Through gusts and lulls, early in a round and late: every disc in
        // view is a frame later just downwind of where it was.
        for second in [10.0, 61.0, 242.0, 601.0] {
            for frame in (0..180).step_by(7) {
                let time = second + frame as f32 / 60.0;
                let (now, next) = (banks(time), banks(time + 1.0 / 60.0));
                for &(at, radius) in now.iter().filter(|(at, _)| view.contains(*at)) {
                    let then = next.iter().filter(|&&(_, r)| r == radius).map(|&(p, _)| p).min_by(|a, b| a.distance_to(at).total_cmp(&b.distance_to(at))).expect("the disc is still there");
                    let (dx, dy) = (then.x - at.x, then.y - at.y);
                    assert!(dx > 0.0 && dx < 0.5 && dy.abs() < 0.5, "a disc at {at:?} moved ({dx}, {dy}) in a frame at {time} s");
                }
            }
        }
    }

    #[test]
    fn ash_falls_only_under_a_heavy_haze() {
        let t = Tuning::default();
        let view = Rectangle::new(0.0, 0.0, 1000.0, 600.0);
        let mut light = Vec::new();
        ash(view, 10.0, t.ash_from_haze * 0.5, &t, &mut light);
        assert!(light.is_empty());
        let mut heavy = Vec::new();
        ash(view, 10.0, 0.9, &t, &mut heavy);
        assert!(heavy.len() > 30, "{}", heavy.len());
    }

    #[test]
    fn ash_drifts_steadily_however_long_the_round() {
        let t = Tuning::default();
        let view = Rectangle::new(0.0, 0.0, 1000.0, 600.0);
        let flakes = |time: f32| {
            let mut out = Vec::new();
            ash(view, time, 0.9, &t, &mut out);
            out.into_iter()
                .map(|s| match s {
                    Shape::Mark { pos, .. } => pos,
                    _ => panic!("ash is marks"),
                })
                .collect::<Vec<_>>()
        };
        for second in [10.0, 61.0, 242.0, 601.0] {
            for frame in (0..180).step_by(7) {
                let time = second + frame as f32 / 60.0;
                let (now, next) = (flakes(time), flakes(time + 1.0 / 60.0));
                assert_eq!(now.len(), next.len());
                for (a, b) in now.iter().zip(&next) {
                    let dx = b.x - a.x;
                    // Wrapping round its lattice cell is the one jump allowed.
                    assert!(dx.abs() < 0.5 || dx.abs() > ASH_CELL - 1.0, "a flake at {a:?} jumped {dx} px in a frame at {time} s");
                }
            }
        }
    }

    #[test]
    fn smoke_hides_downwind_of_a_hot_wreck() {
        let t = Tuning::default();
        assert_eq!(Rules::of(&t), Rules::default(), "every rule that plays is off by default");
        let hots = [hot(Kind::Wreck, 1.0), hot(Kind::Bed, 1.0)];
        let cells = smoke_cells(&hots, &t);
        assert_eq!(cells.len(), 1 + t.smoke_cover_cells as usize, "the wreck's cell and the ones downwind; a bed hides nobody");
        assert!(smoke_cells(&[hot(Kind::Wreck, 10_000.0)], &t).is_empty(), "a cold hulk hides nobody");
    }

    #[test]
    fn a_hot_remain_melts_the_snow_until_it_is_cold() {
        let t = Tuning::default();
        let mut out = Vec::new();
        melt(&hot(Kind::Wreck, t.wreck_burn_seconds + 5.0), &t, &mut out);
        assert!(!out.is_empty());
        let mut cold = Vec::new();
        melt(&hot(Kind::Wreck, 10_000.0), &t, &mut cold);
        assert!(cold.is_empty());
        assert!(snow_cover(0.0, &t) == 0.0 && snow_cover(10_000.0, &t) <= 0.85);
    }
}
