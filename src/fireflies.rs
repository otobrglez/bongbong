//! Warm fireflies (docs/field-life.md "Fireflies"): at night - and fainter
//! at dusk (`firefly_dusk`) - slow specks drifting over the grass and the
//! water near trees, tall grass and bushes, each glowing in and out at its
//! own hashed pace: a FIRE_PALE core while it is bright, GOLD_BRIGHT
//! otherwise, in a GOLD_BRIGHT glow of `firefly_glow_px` whose falloff is
//! stepped through the Bayer pattern (docs/effects.md). A blast within
//! `firefly_blast_px` pushes the near ones away and puts them out; they
//! drift back over `firefly_return_seconds` and light again over
//! `firefly_relight_seconds`.
//!
//! Drawn over the lit field (`render::fauna::draw_glowing`), so a firefly
//! is as bright in the dark as its own light. Presentation only, like
//! everything in `fauna.rs`: each one's place, drift and pace are hashed
//! from the round's seed and its cell, and its drift and pulse are pure in
//! the round's clock.

use crate::fauna::{self, Stir};
use crate::map::Weather;
use crate::math::{Color, Rectangle, Vec2};
use crate::pyro::{self, Shape};
use crate::simulation::Game;
use crate::tuning::Tuning;
use crate::Position;

/// The core (FIRE_PALE) and the glow round it (GOLD_BRIGHT).
pub const CORE: Color = Color::new(0xFF, 0xE2, 0xA0, 255);
pub const GLOW: Color = Color::new(0xEE, 0xA3, 0x43, 255);

/// How dark a blast leaves a firefly: past 1, so it stays out a while
/// before it starts to light again.
const OUT: f32 = 1.3;

/// One firefly.
#[derive(Clone, Debug, PartialEq)]
pub struct Firefly {
    /// The point it drifts round.
    pub home: Position,
    seed: u32,
    /// Where a blast has pushed it from its drift, easing back to nothing.
    push: Vec2,
    /// How far out it is: 0 lit, `OUT` just put out.
    dark: f32,
}

/// Every firefly of a round.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fireflies {
    flies: Vec<Firefly>,
    /// The round's clock at the last step.
    clock: f32,
}

/// How strongly fireflies show in `game`'s sky now, 0 to 1: full at night
/// (or once a map's nightfall has come), `firefly_dusk` at dusk, none
/// under any other sky or with `firefly_enabled` off.
pub fn shown(game: &Game, t: &Tuning) -> f32 {
    if !t.firefly_enabled {
        return 0.0;
    }
    let sky = match game.weather() {
        Weather::Night => 1.0,
        Weather::Dusk => t.firefly_dusk.clamp(0.0, 1.0),
        _ => 0.0,
    };
    sky.max(game.nightfall_mix())
}

impl Firefly {
    /// Where it is at round time `time`: its home, two slow hashed sines
    /// across and down `firefly_drift_px` wide, and what a blast pushed.
    pub fn at(&self, time: f32, t: &Tuning) -> Position {
        let u = |k| pyro::unit(self.seed, k);
        let (wx, wy) = (0.6 + 0.6 * u(5), 0.8 + 0.6 * u(6));
        let drift = t.firefly_drift_px;
        let x = crate::math::sin(time * wx + u(7) * 9.0) * drift;
        let y = crate::math::sin(time * wy + u(8) * 5.0) * drift * 0.7;
        self.home + Vec2::new(x, y) + self.push
    }

    /// How bright it glows at round time `time`, in quarters: its own
    /// pulse at about `firefly_pulse_hz`, less how far a blast put it out.
    pub fn level(&self, time: f32, t: &Tuning) -> f32 {
        let hz = t.firefly_pulse_hz.max(0.0) * (0.6 + 0.8 * pyro::unit(self.seed, 9));
        let pulse = 0.5 + 0.5 * crate::math::sin(std::f32::consts::TAU * (time * hz + pyro::unit(self.seed, 10)));
        ((pulse * (1.0 - self.dark.min(1.0))) * 4.0).round() / 4.0
    }
}

impl Fireflies {
    /// The fireflies of a round over `glades` (`fauna::Ground::glades`):
    /// each cell holds one at `firefly_density`, hashed from the round's
    /// `seed`, and at most `firefly_max` are kept, the hashed first.
    pub fn new(glades: &[(i32, i32)], seed: u64, t: &Tuning) -> Fireflies {
        let salt = (seed ^ (seed >> 32)) as u32 ^ 0xF1AE_F1E5;
        let mut out = Fireflies::default();
        if !t.firefly_enabled || t.firefly_max <= 0 {
            return out;
        }
        let mut kept: Vec<(f32, (i32, i32), u32)> = glades
            .iter()
            .map(|&cell| {
                let fseed = crate::blast::seed_at(fauna::centre(cell), salt);
                (pyro::unit(fseed, 1), cell, fseed)
            })
            .filter(|&(roll, _, _)| roll < t.firefly_density)
            .collect();
        kept.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        kept.truncate(t.firefly_max as usize);
        kept.sort_by_key(|&(_, cell, _)| cell);
        for (_, cell, fseed) in kept {
            let home = fauna::centre(cell) + Vec2::new((pyro::unit(fseed, 3) - 0.5) * 28.0, (pyro::unit(fseed, 4) - 0.5) * 28.0);
            out.flies.push(Firefly { home, seed: fseed, push: Vec2::new(0.0, 0.0), dark: 0.0 });
        }
        out
    }

    pub fn flies(&self) -> &[Firefly] {
        &self.flies
    }

    /// Every blast of this step's `stir` pushes the fireflies within
    /// `firefly_blast_px` straight away from it - the nearer, the further,
    /// up to `firefly_push_px` - and puts them out.
    pub fn stir(&mut self, stir: &Stir, t: &Tuning) {
        let reach = t.firefly_blast_px;
        if reach <= 0.0 || stir.blasts.is_empty() {
            return;
        }
        let time = self.clock;
        for f in &mut self.flies {
            for &b in &stir.blasts {
                let at = f.at(time, t);
                let d = at.distance_to(b);
                if d >= reach {
                    continue;
                }
                let away = fauna::unit_of(at - b).unwrap_or(Vec2::new(0.0, -1.0));
                f.push += away * (t.firefly_push_px * (1.0 - d / reach));
                f.dark = OUT;
            }
        }
    }

    /// `dt` seconds on: each pushed firefly eases back toward its drift and
    /// each one put out lights again.
    pub fn step(&mut self, dt: f32, t: &Tuning) {
        self.clock += dt;
        let ease = crate::math::exp(-dt / t.firefly_return_seconds.max(0.05));
        let relight = dt / t.firefly_relight_seconds.max(0.05);
        for f in &mut self.flies {
            f.push *= ease;
            f.dark = (f.dark - relight).max(0.0);
        }
    }

    /// The fireflies to draw at round time `time`, `shown` (0..1, from
    /// `shown`) strong, within `cull` where there is one: each one's glow,
    /// then its core.
    pub fn shapes(&self, time: f32, shown: f32, cull: Option<Rectangle>, t: &Tuning) -> Vec<Shape> {
        let mut out = Vec::new();
        if shown <= 0.0 {
            return out;
        }
        let radius = t.firefly_glow_px.max(0.0);
        for f in &self.flies {
            let at = f.at(time, t);
            if fauna::off_view(cull, at, radius) {
                continue;
            }
            let level = f.level(time, t) * shown;
            if level <= 0.0 {
                continue;
            }
            glow(&mut out, at, radius, level * t.firefly_opacity);
            out.push(Shape::Mark { pos: at, size: 2, color: if level > 0.5 { CORE } else { GLOW } });
        }
        out
    }
}

/// A firefly's glow round `at`: the blocks within `radius`, each at
/// `strength` falling off as the 1.5th power of its distance, stepped to
/// eighths, kept where the Bayer pattern is under 1.6 times that and drawn
/// at it (at most 0.85) - a soft disc whose edge breaks up in steps.
fn glow(out: &mut Vec<Shape>, at: Position, radius: f32, strength: f32) {
    if radius < pyro::BLOCK || strength <= 0.0 {
        return;
    }
    let r = radius / pyro::BLOCK;
    let (cx, cy) = (at.x / pyro::BLOCK, at.y / pyro::BLOCK);
    for by in (cy - r).floor() as i32 - 1..=(cy + r).floor() as i32 + 1 {
        for bx in (cx - r).floor() as i32 - 1..=(cx + r).floor() as i32 + 1 {
            let d = crate::math::hypot(bx as f32 + 0.5 - cx, by as f32 + 0.5 - cy) / r;
            if d >= 1.0 {
                continue;
            }
            let q = pyro::eighths(strength * crate::math::powf(1.0 - d, 1.5));
            if q <= 0.0 || pyro::bayer(bx, by) > (q * 1.6).min(1.0) {
                continue;
            }
            out.push(Shape::Mark { pos: Position::new((bx * 2) as f32, (by * 2) as f32), size: 2, color: pyro::alpha(GLOW, q.min(0.85)) });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PHYSICS_FIXED_DT;

    fn glades() -> Vec<(i32, i32)> {
        let mut g: Vec<(i32, i32)> = (1..20).flat_map(|c| (1..12).map(move |r| (c, r))).collect();
        g.sort();
        g
    }

    fn quick() -> Tuning {
        Tuning { firefly_density: 0.3, ..Tuning::DEFAULT }
    }

    #[test]
    fn fireflies_live_over_glades_hashed_from_the_seed() {
        let t = quick();
        let g = glades();
        let f = Fireflies::new(&g, 7, &t);
        assert!(f.flies.len() >= 30, "{} fireflies", f.flies.len());
        for fly in &f.flies {
            let cell = crate::map::world_to_cell(fly.home);
            assert!(g.iter().any(|&c| (c.0 - cell.0).abs() <= 1 && (c.1 - cell.1).abs() <= 1), "{:?}", fly.home);
        }
        assert_eq!(f, Fireflies::new(&g, 7, &t));
        assert_ne!(f.flies, Fireflies::new(&g, 8, &t).flies);
        let few = Fireflies::new(&g, 7, &Tuning { firefly_max: 5, ..t });
        assert_eq!(few.flies.len(), 5);
        assert!(Fireflies::new(&g, 7, &Tuning { firefly_enabled: false, ..t }).flies.is_empty());
        assert!(Fireflies::new(&[], 7, &t).flies.is_empty());
    }

    #[test]
    fn they_show_only_when_it_is_dark() {
        let t = quick();
        let f = Fireflies::new(&glades(), 3, &t);
        assert!(f.shapes(2.0, 0.0, None, &t).is_empty(), "nothing by day");
        let night = f.shapes(2.0, 1.0, None, &t);
        assert!(!night.is_empty());
        for s in &night {
            let Shape::Mark { pos, size, color } = *s else { panic!() };
            assert_eq!(size, 2);
            let c = (color.r, color.g, color.b);
            assert!(c == (CORE.r, CORE.g, CORE.b) || c == (GLOW.r, GLOW.g, GLOW.b), "{color:?}");
            let _ = pos;
        }
        let mut game = Game::default();
        game.weather = Weather::Clear;
        assert_eq!(shown(&game, &t), 0.0);
        game.weather = Weather::Night;
        assert_eq!(shown(&game, &t), 1.0);
        game.weather = Weather::Dusk;
        assert_eq!(shown(&game, &t), t.firefly_dusk);
        assert_eq!(shown(&game, &Tuning { firefly_enabled: false, ..t }), 0.0);
    }

    #[test]
    fn a_blast_scatters_and_puts_them_out_and_they_come_back() {
        let t = quick();
        let mut f = Fireflies::new(&glades(), 5, &t);
        let fly = f.flies[f.flies.len() / 2].clone();
        let at = fly.at(0.0, &t);
        let blast = at + Vec2::new(20.0, 0.0);
        f.stir(&Stir { blasts: vec![blast], ..Stir::default() }, &t);
        let i = f.flies.iter().position(|g| g.seed == fly.seed).unwrap();
        assert!(f.flies[i].at(0.0, &t).distance_to(blast) > at.distance_to(blast) + 10.0, "pushed away");
        assert_eq!(f.flies[i].level(0.0, &t), 0.0, "put out");
        let far = f.flies.iter().find(|g| g.at(0.0, &t).distance_to(blast) > t.firefly_blast_px + 30.0).cloned();
        for _ in 0..((t.firefly_relight_seconds * 2.0 + t.firefly_return_seconds * 4.0) / PHYSICS_FIXED_DT) as usize {
            f.step(PHYSICS_FIXED_DT, &t);
        }
        assert!(f.flies[i].push.length() < 2.0, "back: {:?}", f.flies[i].push);
        assert_eq!(f.flies[i].dark, 0.0, "lit again");
        if let Some(far) = far {
            let now = f.flies.iter().find(|g| g.seed == far.seed).unwrap();
            assert_eq!(now.home, far.home);
        }
    }
}
