//! Life on the field (docs/field-life.md): crows on open grass
//! (`crows.rs`), mallards on a lake's open water (`ducks.rs`) and warm
//! fireflies at night (`fireflies.rs`). Built the way the fish are
//! (`fish.rs`): scenery that reacts to the fight - a hull coming near, a
//! shot flying past, a blast - and nothing else.
//!
//! **Presentation only.** Nothing in the simulation reads a creature,
//! nothing travels on the wire, and no creature draws the round's RNG or
//! `rand::rng()`: every choice is hashed (`pyro::unit`) from the round's
//! seed, a cell or the creature's own seed. `Fauna` lives in `fx::Fx` (so
//! `app.rs` owns it) and is stepped on the round's clock after every
//! simulation step (`Fauna::observe`), in ticks of at most
//! `PHYSICS_FIXED_DT`: the same steps from the same seed give the same
//! field, so a dev-server lockstep replays it and a replica, which reads
//! the same `Game`, draws the same creatures.
//!
//! **How they look** (docs/effects.md): whole 2 px blocks on the field's
//! grid in Puny Palette steps. A creature is a small grid of blocks drawn
//! facing up (`compose_grid`), turned to one of the sixteen headings - the
//! diagonals left out (`drawn_heading`) - by sampling the grid at each
//! block's centre turned back into it, the fish's way, so a turned shape
//! keeps whole blocks and no gaps. What sits
//! or swims is drawn on the floor under everything that stands; what flies
//! is drawn over everything in the air stage with its shadow on the ground
//! under it; fireflies shine over the lit field.

use std::collections::BTreeSet;

use crate::math::{Color, Rectangle, Vec2};
use crate::pyro::Shape;
use crate::simulation::{Event, Game, HitTarget};
use crate::tuning::{tuning, Tuning};
use crate::{OBSTACLE_GRID_SIZE, PHYSICS_FIXED_DT, Position};

/// The longest step a creature is moved in one go: time that arrives in a
/// lump (a replica's frame) is split into ticks, so the field moves alike
/// however the round's clock reaches it.
pub const MAX_STEP: f32 = PHYSICS_FIXED_DT;

/// The longest gap in the round's clock the field is stepped across; a
/// longer one (a stall) is taken as this.
pub const MAX_GAP: f32 = 0.5;

/// The centre of map cell `(col, row)`.
pub fn centre((col, row): (i32, i32)) -> Position {
    crate::map::cell_to_world(col, row)
}

/// `v` as a unit vector, `None` for a zero one.
pub fn unit_of(v: Vec2) -> Option<Vec2> {
    let l = v.length();
    (l > 1e-4).then(|| v / l)
}

/// `v` turned `a` radians clockwise on the y-down field.
pub fn turned(v: Vec2, a: f32) -> Vec2 {
    let (s, c) = crate::math::sin_cos(a);
    Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c)
}

/// `dir` turned toward `want` by at most `most` radians (both unit
/// vectors).
pub fn steer(dir: Vec2, want: Vec2, most: f32) -> Vec2 {
    let cross = dir.x * want.y - dir.y * want.x;
    let angle = crate::math::atan2(cross, dir.dot(want));
    if angle.abs() <= most {
        return want;
    }
    turned(dir, most * angle.signum())
}

/// One heading of the sixteen a creature is drawn in, as a unit vector
/// (`fish::HEADINGS`).
pub fn heading_of(i: usize) -> Vec2 {
    let (c, s) = crate::fish::HEADINGS[i % 16];
    Vec2::new(c, s)
}

/// Which of the sixteen headings a creature facing `v` is drawn in: the
/// nearest, but never a diagonal - a grid three blocks wide turned 45
/// degrees falls apart into a checkerboard - so a diagonal one goes to
/// whichever of its two neighbours `v` leans toward.
pub fn drawn_heading(v: Vec2) -> usize {
    let i = crate::fish::heading_index(v);
    if i % 4 != 2 {
        return i;
    }
    let (a, b) = (heading_of(i + 15), heading_of(i + 1));
    if v.dot(a) > v.dot(b) { (i + 15) % 16 } else { i + 1 }
}

/// A creature's picture: rows of one character a block, its head at the
/// top facing up; `.` is no block.
pub type Grid = [&'static str];

/// The blocks of `grid` at `pos` facing `heading`, each coloured by
/// `color` (its character's colour, `None` for none): the grid is centred
/// on the block `pos` falls in, turned to the heading `drawn_heading`
/// gives, and every block round it takes the grid cell its centre lands on
/// once turned back - so a turned creature stays whole blocks with no
/// gaps.
pub fn compose_grid(out: &mut Vec<Shape>, pos: Position, heading: Vec2, grid: &Grid, color: impl Fn(u8) -> Option<Color>) {
    let d = heading_of(drawn_heading(heading));
    let rows = grid.len() as i32;
    let cols = grid.first().map_or(0, |r| r.len()) as i32;
    let (cx0, cy0) = ((cols - 1) as f32 / 2.0, (rows - 1) as f32 / 2.0);
    let (ax, ay) = ((pos.x / crate::pyro::BLOCK).floor() as i32, (pos.y / crate::pyro::BLOCK).floor() as i32);
    let reach = rows.max(cols) / 2 + 2;
    for j in -reach..=reach {
        for i in -reach..=reach {
            let (x, y) = (i as f32, j as f32);
            // Ahead along the heading, and to its right.
            let ahead = x * d.x + y * d.y;
            let right = -x * d.y + y * d.x;
            let gx = (right + cx0 + 0.5).floor() as i32;
            let gy = (-ahead + cy0 + 0.5).floor() as i32;
            if gx < 0 || gy < 0 || gx >= cols || gy >= rows {
                continue;
            }
            let ch = grid[gy as usize].as_bytes()[gx as usize];
            if ch == b'.' {
                continue;
            }
            if let Some(c) = color(ch) {
                out.push(Shape::Mark { pos: Position::new(((ax + i) * 2) as f32, ((ay + j) * 2) as f32), size: 2, color: c });
            }
        }
    }
}

/// The shadow a creature `alt` px up throws on the ground under `ground`:
/// its grid's blocks in black at `opacity`, stepped to eighths, along the
/// field's light (`shadow_dir_x`/`_y`) by half its height.
pub fn compose_shadow(out: &mut Vec<Shape>, ground: Position, alt: f32, heading: Vec2, grid: &Grid, opacity: f32, t: &Tuning) {
    let shade = crate::pyro::alpha(Color::new(0, 0, 0, 255), opacity);
    if shade.a == 0 {
        return;
    }
    let at = ground + Vec2::new(t.shadow_dir_x, t.shadow_dir_y) * (alt * 0.5);
    compose_grid(out, at, heading, grid, |_| Some(shade));
}

/// What stirs the field this step: where every live hull is, every shot
/// in the air and every hit, every blast, and every tree a hull struck, a
/// shot hit or that fell.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stir {
    pub hulls: Vec<Position>,
    pub shots: Vec<Position>,
    pub blasts: Vec<Position>,
    pub trees: Vec<Position>,
}

impl Stir {
    /// This step of `game`: its hulls, shells, bullets, plasma bolts,
    /// missiles and orbs; the hits, ricochets and beams of its events; its
    /// blasts, bursts, wrecks and impacts; the trees struck, hit or felled.
    pub fn of(game: &Game) -> Stir {
        let mut s = Stir::default();
        for tank in game.world.query::<&crate::tank::Tank>().iter().filter(|tank| !tank.is_dead()) {
            s.hulls.push(tank.position);
        }
        for shell in game.world.query::<&crate::shell::Shell>().iter() {
            s.shots.push(shell.position);
        }
        for b in game.world.query::<&crate::bullet::Bullet>().iter() {
            s.shots.push(b.position);
        }
        for p in game.world.query::<&crate::plasma::Plasma>().iter() {
            s.shots.push(p.position);
        }
        for m in game.world.query::<&crate::missile::Missile>().iter().filter(|m| !m.is_dead()) {
            s.shots.push(m.position);
        }
        for o in game.orbs() {
            s.shots.push(o.position);
        }
        for e in game.events() {
            match *e {
                Event::Blast { x, y, .. }
                | Event::MissileBlast { x, y, .. }
                | Event::GrenadeBlast { x, y, .. }
                | Event::Wreck { x, y, .. }
                | Event::CookOff { x, y }
                | Event::WellCollapsed { x, y, .. }
                | Event::AirStrike { x, y, .. }
                | Event::DroneBurst { x, y, .. } => s.blasts.push(Position::new(x, y)),
                Event::RodImpact { cell, .. } => s.blasts.push(crate::map::cell_to_world(cell.0, cell.1)),
                Event::Hit { target, x, y, .. } => {
                    s.shots.push(Position::new(x, y));
                    if matches!(target, HitTarget::Obstacle { material } if material.is_tree()) {
                        s.trees.push(Position::new(x, y));
                    }
                }
                Event::HullStruck { material: Some(material), x, y, .. } if material.is_tree() => s.trees.push(Position::new(x, y)),
                Event::ObstacleDestroyed { material, x, y } if material.is_tree() => s.trees.push(Position::new(x, y)),
                Event::Ricochet { x, y, .. } | Event::DroneCrashed { x, y, .. } => s.shots.push(Position::new(x, y)),
                Event::LaserBeam { x0, y0, x1, y1, .. } | Event::RailSlug { x0, y0, x1, y1, .. } => {
                    // Along the beam or the slug, a point every cell, its
                    // end included.
                    let (a, b) = (Position::new(x0, y0), Position::new(x1, y1));
                    let n = (a.distance_to(b) / OBSTACLE_GRID_SIZE).ceil().max(1.0) as u32;
                    for i in 1..=n {
                        s.shots.push(a + (b - a) * (i as f32 / n as f32));
                    }
                }
                _ => {}
            }
        }
        s
    }

    /// The nearest point of `points` within `reach` of `at`, if any.
    pub fn near(points: &[Position], at: Position, reach: f32) -> Option<Position> {
        let mut best: Option<(f32, Position)> = None;
        for &p in points {
            let d = p.distance_to(at);
            if d < reach && best.is_none_or(|(b, _)| d < b) {
                best = Some((d, p));
            }
        }
        best.map(|(_, p)| p)
    }
}

/// The ground a round's creatures live on, read once when it starts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ground {
    /// Open grass, sorted: a cell inside the field with nothing the map
    /// put on it and nothing grown on it, whose floor is grass edge to
    /// edge.
    pub open: Vec<(i32, i32)>,
    /// The trees, sorted by cell: each one's cell and where its crown is
    /// drawn.
    pub trees: Vec<((i32, i32), Position)>,
    /// Where fireflies live, sorted: open grass, water and soft cover
    /// within two cells of a tree, tall grass or a bush.
    pub glades: Vec<(i32, i32)>,
}

impl Ground {
    /// `game`'s field as the creatures see it.
    pub fn of(game: &Game) -> Ground {
        let (width, height) = game.map.field_size();
        let (cols, rows) = ((width / OBSTACLE_GRID_SIZE).round() as i32, (height / OBSTACLE_GRID_SIZE).round() as i32);
        let taken: BTreeSet<(i32, i32)> = game.map.iter_cells().map(|(c, r, _)| (c, r)).collect();
        let grown: BTreeSet<(i32, i32)> = game.grass.iter().map(|tuft| crate::map::world_to_cell(tuft.base)).collect();
        let grass = |c: i32, r: i32| {
            let o = centre((c, r));
            [(-8.0, -8.0), (8.0, -8.0), (-8.0, 8.0), (8.0, 8.0)]
                .iter()
                .all(|&(dx, dy)| game.ground.floor_at(Position::new(o.x + dx, o.y + dy)) == crate::ground::GroundFloor::Grass)
        };
        let mut open = Vec::new();
        for r in 1..rows {
            for c in 1..cols {
                if !taken.contains(&(c, r)) && !grown.contains(&(c, r)) && grass(c, r) {
                    open.push((c, r));
                }
            }
        }
        open.sort();
        let mut trees: Vec<((i32, i32), Position)> = game
            .world
            .query::<&crate::obstacle::Obstacle>()
            .iter()
            .filter(|o| o.material.is_tree())
            .map(|o| (o.cell(), crate::obstacle::crown_centre(o)))
            .collect();
        trees.sort_by_key(|&(cell, _)| cell);
        // What fireflies gather round: the trees, the map's tall grass and
        // bushes, and the undergrowth grown round its woods.
        let cover: BTreeSet<(i32, i32)> = game.grass_cells.iter().map(|&p| crate::map::world_to_cell(p)).collect();
        let mut green: BTreeSet<(i32, i32)> = trees.iter().map(|&(cell, _)| cell).collect();
        green.extend(cover.iter().copied());
        green.extend(grown.iter().copied());
        let wet = |c: i32, r: i32| game.water.depth_of_cell(c, r).is_wet();
        let soft = |c: i32, r: i32| grown.contains(&(c, r)) || cover.contains(&(c, r));
        let mut glades = Vec::new();
        for r in 1..rows {
            for c in 1..cols {
                let tree = trees.binary_search_by_key(&(c, r), |&(cell, _)| cell).is_ok();
                let fit = !tree && (open.binary_search(&(c, r)).is_ok() || wet(c, r) || soft(c, r));
                if fit && (-2..=2).any(|dr| (-2..=2).any(|dc| green.contains(&(c + dc, r + dr)))) {
                    glades.push((c, r));
                }
            }
        }
        Ground { open, trees, glades }
    }
}

/// Every creature of a round, stepped on the round's clock. Rebuilt when
/// a new round starts.
#[derive(Clone, Debug, Default)]
pub struct Fauna {
    /// The round it was made for (`Game::round_seed`), `None` before the
    /// first.
    round: Option<u64>,
    /// The last `Game::frame` observed: a frame counter going back is a
    /// new round.
    last_frame: u64,
    /// The round's clock at the last step.
    clock: f32,
    crows: crate::crows::Crows,
    ducks: crate::ducks::Ducks,
    fireflies: crate::fireflies::Fireflies,
}

impl Fauna {
    /// The creatures of a round of `game` seeded `seed`.
    pub fn new(game: &Game, t: &Tuning) -> Fauna {
        let seed = game.round_seed();
        let ground = Ground::of(game);
        Fauna {
            round: Some(seed),
            last_frame: game.frame(),
            clock: game.time,
            crows: crate::crows::Crows::new(&ground, seed, t),
            ducks: crate::ducks::Ducks::new(game.water(), seed, t),
            fireflies: crate::fireflies::Fireflies::new(&ground.glades, seed, t),
        }
    }

    pub fn crows(&self) -> &crate::crows::Crows {
        &self.crows
    }

    pub fn ducks(&self) -> &crate::ducks::Ducks {
        &self.ducks
    }

    pub fn fireflies(&self) -> &crate::fireflies::Fireflies {
        &self.fireflies
    }

    /// One simulation step of `game` seen: rebuild for a new round, stir
    /// the creatures near this step's hulls, shots and blasts, and move
    /// them on to the round's clock. A frame already seen does nothing, so
    /// a frozen round's creatures hold still.
    pub fn observe(&mut self, game: &Game) {
        let frame = game.frame();
        let restarted = game.events().iter().any(|e| matches!(e, Event::RoundStarted { .. }));
        if self.round != Some(game.round_seed()) || frame < self.last_frame || (restarted && frame != self.last_frame) {
            *self = Fauna::new(game, &tuning());
            return;
        }
        if frame == self.last_frame {
            return;
        }
        self.last_frame = frame;
        let t = tuning();
        let stir = Stir::of(game);
        let water = game.water();
        self.crows.stir(&stir, &t);
        self.ducks.stir(&stir, water, &t);
        self.fireflies.stir(&stir, &t);
        self.advance(water, game.time, &t);
    }

    /// Move every creature on to round time `clock`, in steps of at most a
    /// tick. A clock that went back starts the count again from there.
    pub fn advance(&mut self, water: &crate::ground::WaterLayout, clock: f32, t: &Tuning) {
        let mut gap = (clock - self.clock).clamp(0.0, MAX_GAP);
        self.clock = clock - gap;
        while gap > 1e-6 {
            let dt = gap.min(MAX_STEP);
            gap -= dt;
            self.clock += dt;
            self.crows.step(dt, t);
            self.ducks.step(water, dt, t);
            self.fireflies.step(dt, t);
        }
        self.clock = clock;
    }
}

/// Whether `at` is outside `cull` by more than `margin` px.
pub fn off_view(cull: Option<Rectangle>, at: Position, margin: f32) -> bool {
    cull.is_some_and(|r| at.x < r.x - margin || at.y < r.y - margin || at.x > r.x + r.width + margin || at.y > r.y + r.height + margin)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUCK: [&str; 6] = [".k.", ".H.", "BBB", "BWB", "BWB", ".B."];

    #[test]
    fn a_grid_facing_up_is_drawn_as_written() {
        let mut out = Vec::new();
        compose_grid(&mut out, Position::new(101.0, 51.0), Vec2::new(0.0, -1.0), &DUCK, |_| Some(Color::new(1, 2, 3, 255)));
        let blocks = DUCK.iter().map(|r| r.bytes().filter(|&b| b != b'.').count()).sum::<usize>();
        assert_eq!(out.len(), blocks);
        // Head on top: the beak's block is the topmost.
        let top = out.iter().map(|s| if let Shape::Mark { pos, .. } = s { pos.y } else { unreachable!() }).fold(f32::MAX, f32::min);
        let beak: Vec<&Shape> = out.iter().filter(|s| matches!(s, Shape::Mark { pos, .. } if pos.y == top)).collect();
        assert_eq!(beak.len(), 1);
    }

    #[test]
    fn a_turned_grid_stays_whole_blocks_on_the_grid() {
        for k in 0..16 {
            let heading = heading_of(k);
            let mut out = Vec::new();
            compose_grid(&mut out, Position::new(77.3, 41.9), heading, &DUCK, |_| Some(Color::new(1, 2, 3, 255)));
            assert!(out.len() >= 10, "heading {k}: {} blocks", out.len());
            let mut seen = BTreeSet::new();
            for s in &out {
                let Shape::Mark { pos, size, .. } = *s else { panic!() };
                assert_eq!(size, 2);
                assert!(pos.x % 2.0 == 0.0 && pos.y % 2.0 == 0.0);
                assert!(seen.insert((pos.x as i32, pos.y as i32)), "one block once");
                assert!(pos.distance_to(Position::new(77.0, 41.0)) < 12.0);
            }
        }
    }

    /// A round on a shipped lake map: flocks on its open grass only, ducks
    /// on its lake, fireflies by its trees and reeds, all the same for the
    /// same seed.
    #[test]
    fn a_lake_map_comes_alive() {
        let text = std::fs::read_to_string("maps/lotus-lagoon.toml").expect("the map");
        let make = || {
            let mut game = Game::default();
            game.seed_override = Some(7);
            game.map = crate::map::MapFile::from_toml_str(&text).expect("parses");
            let (w, h) = game.map.field_size();
            game.init(w, h);
            game
        };
        let game = make();
        let t = Tuning::DEFAULT;
        let ground = Ground::of(&game);
        let fauna = Fauna::new(&game, &t);
        let crows = fauna.crows().crows();
        assert!(crows.len() >= 10, "{} crows", crows.len());
        for c in crows.iter().filter(|c| c.state == crate::crows::State::Ground) {
            assert!(ground.open.binary_search(&crate::map::world_to_cell(c.pos)).is_ok(), "{:?}", c.pos);
        }
        // A flock spreads out: not every bird on one spot.
        let first = crows[0].pos;
        assert!(crows.iter().take(5).any(|c| c.pos.distance_to(first) > 6.0));
        assert!(fauna.ducks().ducks().len() >= 3, "{} ducks", fauna.ducks().ducks().len());
        assert!(!fauna.fireflies().flies().is_empty());
        assert_eq!(Fauna::new(&make(), &t).crows(), fauna.crows());
    }

    /// Every creature in every heading, to look at:
    /// `cargo test --lib fauna::tests::preview -- --ignored` writes
    /// target/fauna-preview.png - a crow sitting, pecking and in both beats
    /// of its wings, a duck swimming, dabbling and flying, a row each, over
    /// grass and water.
    #[test]
    #[ignore]
    #[cfg(feature = "render")] // write_png is raylib's PNG encoder
    fn preview() {
        use crate::canvas::{Canvas, CpuCanvas};
        let mut c = CpuCanvas::blank(16 * 24, 8 * 24);
        c.fill_rect(0, 0, 16 * 24, 4 * 24, Color::new(0x5D, 0x9C, 0x3E, 255));
        c.fill_rect(0, 4 * 24, 16 * 24, 4 * 24, Color::new(0x04, 0xA0, 0xB4, 255));
        for k in 0..16 {
            for row in 0..8 {
                let mut out = Vec::new();
                let at = Position::new(k as f32 * 24.0 + 12.0, row as f32 * 24.0 + 12.0);
                crate::crows::preview(&mut out, row, at, heading_of(k));
                crate::ducks::preview(&mut out, row, at, heading_of(k));
                crate::pyro::draw(&mut c, &out);
            }
        }
        c.write_png(std::path::Path::new("target/fauna-preview.png"), 4).unwrap();
    }

    #[test]
    fn steering_turns_at_most_its_share() {
        let up = Vec2::new(0.0, -1.0);
        let right = Vec2::new(1.0, 0.0);
        let d = steer(up, right, 0.1);
        assert!((d.length() - 1.0).abs() < 1e-4);
        assert!(d.x > 0.0 && d.x < 0.11);
        assert_eq!(steer(up, right, 2.0), right);
    }

    #[test]
    fn a_stir_finds_the_nearest() {
        let pts = [Position::new(0.0, 0.0), Position::new(30.0, 0.0)];
        assert_eq!(Stir::near(&pts, Position::new(25.0, 0.0), 40.0), Some(Position::new(30.0, 0.0)));
        assert_eq!(Stir::near(&pts, Position::new(100.0, 0.0), 40.0), None);
    }
}
