//! Tall grass: cover a tank can sit in, and the only vegetation that is
//! *not* an obstacle.
//!
//! **Why this is not an `Obstacle`.** `Game::nav_grid` feeds every
//! `Obstacle` into pathfinding with no material filter, and `maplint`
//! reasons the same way, so anything that is one is unconditionally
//! impassable to the AI and a wall to the linter. Grass you drive through
//! cannot be that. It is modelled on `decal::Decal` instead - owned by the
//! simulation, seeded from a position hash so it never draws RNG, and
//! rebuilt only when a round starts.
//!
//! **What it looks like.** A map marks whole cells as tall grass; each cell
//! scatters several small tufts at hashed offsets. That scatter is the
//! point: the reference (docs/grass-improvements.md, `grass-hidding.gif`)
//! never hides a unit completely - at worst about 44% of it is covered,
//! bottom-weighted, because the unit walks *between* discrete tufts rather
//! than into a solid block. One sprite per cell would either occlude
//! everything or nothing.
//!
//! **How it moves.** Wind crosses the field (`wind_at`): every tuft leans a
//! little along it, and gust fronts roll over the meadow as a travelling
//! wave on a slower envelope, so neighbouring tufts bend together and a
//! gust is seen moving across the field rather than every tuft swaying in
//! place. On top of that each tuft flutters on a sine of its own hashed
//! phase, which keeps a field in a gust from moving as one sheet. All of
//! it is a draw-time offset computed on the CPU from the tuft's position
//! and the clock - there is no shader, because every shader here needs a
//! hand-ported GLSL ES 100 twin for the web build (see `main.rs`'s
//! `shader_path`) and a few sines do not justify that.
//!
//! **How a tank goes through it.** `tick` carries two more values per tuft,
//! both pure functions of where the tanks are - no RNG, so a seeded replay
//! is unaffected:
//!
//! - `crush`, how flat the tuft is lying. It goes to 1 under a hull and
//!   recovers over `grass_crush_recover_seconds`, which is the whole trail
//!   effect: grass stays matted behind a tank and stands back up, the same
//!   shape `track.rs` gives a tread mark. Persistence is also *why* the
//!   wake does not need to be shaped by the direction of travel - the trail
//!   is left behind by time, not by geometry.
//! - `push`, the sideways shove. It is stronger for grass *ahead* of a
//!   moving tank than behind it, so a hull drives a bow wave rather than
//!   a symmetric ring.
//!
//! Both are additions of ours: the reference gif sways ambiently and does
//! not react to the unit at all.
//!
//! **Depth.** `game.rs` draws tufts interleaved with tanks by ground-contact
//! y, so a tuft rooted behind a tank is drawn behind it. Drawing all grass
//! last buries a tank in grass that is rooted well past it.
//!
//! **Tiles.** A tuft's art is wider than half a cell and taller than a
//! whole one, and the standing pass comes after the tiles, so a tuft rooted
//! by a wall would be drawn over it. `keep_off` places every tuft so it
//! is not: the root moves away from a solid tile beside or below its cell
//! as far as the tuft's art (`TUFT_EXTENTS`) and its steady lean need, its
//! lean toward a tile beside it is capped at the room left
//! (`GrassTuft::lean`), and under a wall to the north the root moves down
//! until the art overlaps the wall's foot by no more than
//! `grass_wall_overlap_px` - grass growing in front of the wall, never on
//! it. A tuft too tall or too wide for the room takes the next of the
//! sheet's tufts that fits, and a cell with room for none (walled in on
//! three sides) grows none. Every tuft stands in the y-sorted walk among
//! the tanks. Trees are not counted: they are drawn over every tuft
//! anyway.

use crate::math::{Color, Rectangle, Vec2};

use crate::canvas::{Canvas, Sheet};
use crate::map::Theme;
use crate::tuning::{tuning, Tuning};
use crate::{GRASS_SPECIES, GRASS_TEXTURE_SIZE, GRASS_VARIANTS, OBSTACLE_GRID_SIZE, Position};

/// One drawn tuft. Several of these make up a single grass cell.
pub struct GrassTuft {
    /// Where its base sits, in world pixels.
    pub base: Position,
    /// Sheet row (species) and column (variant), both hashed from `base`.
    pub row: i32,
    pub col: i32,
    /// Cosmetic seed - drives the sway phase and the horizontal mirror.
    pub seed: u32,
    /// How flat this tuft is lying: 0 standing, 1 fully matted. Driven to 1
    /// under a hull and recovered over `grass_crush_recover_seconds` - see
    /// the module comment on why that recovery is the trail.
    pub crush: f32,
    /// Sideways shove from a passing hull, in px of travel at the tip.
    /// Signed; positive is to the right. Added to the ambient sway.
    pub push: f32,
    /// Charred by a burning cell (`Game::tick_fires`): drawn as a stub for
    /// the rest of the round, never standing back up. The cell it grew in
    /// leaves `Game::grass_cells` at the same moment, so it no longer
    /// conceals.
    pub burnt: bool,
    /// How far the tip may lean left and right (`bend`'s units, px of
    /// travel at the sprite's top), both positive: the room left before
    /// the art crosses into a solid tile beside its cell. Unbounded on a
    /// side with no tile.
    pub lean: [f32; 2],
}

/// Each tuft's art in sheet pixels about its root, the bottom centre it is
/// drawn from: how far it reaches left of the root, right of it and above
/// it, by species (row) and variant (column), unmirrored. The larger of
/// the two themes' sheets for each cell, so one table serves both
/// (`tuft_extents_cover_both_sheets` holds it against the PNGs).
pub const TUFT_EXTENTS: [[(u8, u8, u8); GRASS_VARIANTS as usize]; GRASS_SPECIES as usize] = [
    [(6, 1, 18), (4, 13, 17), (13, 6, 18), (6, 13, 18), (11, 4, 17), (6, 10, 17), (13, 2, 17), (13, 6, 17)],
    [(0, 6, 23), (0, 11, 23), (2, 3, 22), (5, 7, 22), (7, 5, 21), (5, 5, 23), (2, 9, 22), (2, 11, 22)],
    [(8, 11, 14), (8, 8, 14), (7, 12, 14), (12, 10, 14), (6, 12, 14), (6, 10, 13), (7, 10, 14), (10, 5, 14)],
];

/// Px of slack round a tuft's art when it is kept off a tile: the root is
/// not on the 2 px grid, so its art can land a pixel over.
const TILE_SLACK_PX: f32 = 1.0;

/// A tank as the grass sees it. Velocity is the *body's*, not
/// `Tank::velocity` - that one is the commanded cardinal vector and reads
/// as zero the moment the driver lets go, while the hull is still moving.
pub struct Mover {
    pub position: Position,
    pub velocity: Position,
    /// Half-width of the hull's footprint. Flattening is measured from this
    /// *box*, not from the tank's centre: a radial falloff from the centre
    /// leaves the grass under the tracks standing, because the hull is
    /// wider than any radius small enough to look right.
    pub half: f32,
}

/// The tufts one grass cell scatters. Everything - count, offsets, species,
/// variant, phase - comes out of `blast::seed_at` salted per tuft, so a
/// grass field is identical on replay and costs the round RNG nothing.
/// `solid` names the cells (`map::world_to_cell`) holding a tile a tuft
/// must not be drawn over; see `keep_off`.
pub fn tufts_for_cell(t: &Tuning, center: Position, solid: impl Fn((i32, i32)) -> bool) -> Vec<GrassTuft> {
    let per_cell = t.grass_tufts_per_cell.max(0);
    let half = OBSTACLE_GRID_SIZE / 2.0;
    (0..per_cell)
        .filter_map(|i| {
            let h = crate::blast::seed_at(center, 60 + i as u32 * 5);
            // Spread across the cell, but keep the base inside it so a
            // tuft belongs to the cell that owns it.
            let ox = (h % 1000) as f32 / 1000.0 * OBSTACLE_GRID_SIZE - half;
            let oy = ((h >> 10) % 1000) as f32 / 1000.0 * OBSTACLE_GRID_SIZE - half;
            let tuft = GrassTuft {
                base: Position::new(center.x + ox, center.y + oy),
                row: ((h >> 20) % GRASS_SPECIES as u32) as i32,
                col: ((h >> 24) % GRASS_VARIANTS as u32) as i32,
                seed: h,
                crush: 0.0,
                push: 0.0,
                burnt: false,
                lean: [f32::INFINITY; 2],
            };
            keep_off(t, tuft, center, &solid)
        })
        .collect()
}

/// The tuft's art about its root in world px at `grass_scale`: left,
/// right and up, mirrored the way `draw_tuft` mirrors it.
pub fn art_extents(t: &Tuning, tuft: &GrassTuft) -> (f32, f32, f32) {
    let (l, r, up) = TUFT_EXTENTS[tuft.row as usize][tuft.col as usize];
    let (l, r) = if tuft.seed & 1 != 0 { (r, l) } else { (l, r) };
    let s = t.grass_scale;
    (l as f32 * s, r as f32 * s, up as f32 * s)
}

/// The box `tuft`'s art can cover at any lean it is allowed, in world px:
/// left, top, right, bottom. A lean swings the art about its root, so the
/// top travels sideways and the lower corners dip a little below it.
pub fn reach(t: &Tuning, tuft: &GrassTuft) -> (f32, f32, f32, f32) {
    let (l, r, up) = art_extents(t, tuft);
    let size = GRASS_TEXTURE_SIZE * t.grass_scale;
    // The most `tick` and the wind can bend a tuft: the steady lean, a
    // gust, its flutter and a hull's bow wave.
    let most = t.grass_wind_px + t.grass_gust_px + t.grass_sway_px + t.grass_part_px * 1.2;
    let (left, right) = (most.min(tuft.lean[0]), most.min(tuft.lean[1]));
    let travel = up / size;
    let dip = l.max(r) * (left.max(right) / size).atan().sin();
    (
        tuft.base.x - l - left * travel,
        tuft.base.y - up,
        tuft.base.x + r + right * travel,
        tuft.base.y + dip,
    )
}

/// Place `tuft` so its art keeps off the solid tiles round its cell, or
/// `None` where no tuft of the sheet fits. The hashed tuft is tried first,
/// then the rest of the sheet in a fixed order from it (`place`), so most
/// tufts keep the art and spot they hashed and only a crowded cell changes
/// any. Pure in its inputs, no RNG.
fn keep_off(t: &Tuning, tuft: GrassTuft, center: Position, solid: &impl Fn((i32, i32)) -> bool) -> Option<GrassTuft> {
    let all = GRASS_SPECIES * GRASS_VARIANTS;
    let first = tuft.row * GRASS_VARIANTS + tuft.col;
    (0..all).find_map(|k| {
        let i = (first + k) % all;
        let mut tried = GrassTuft { row: i / GRASS_VARIANTS, col: i % GRASS_VARIANTS, ..tuft };
        place(t, &mut tried, center, solid).then_some(tried)
    })
}

/// Move `tuft`'s root within its cell so its art keeps off the tiles round
/// it, and say whether it could:
///
/// - a tile in the cell's row, beside it or a cell further: the root moves
///   away from it by the art's reach and the steady lean (wind, gust and
///   flutter - never a passing hull's shove), and the lean toward it is
///   capped at the room left;
/// - a tile in the row below, beside it or not: the root moves up by what
///   a lean dips the art's lower corners;
/// - a tile in either row above that the art spans: the root moves down
///   until the art's top overlaps that tile's bottom edge by no more than
///   `grass_wall_overlap_px`, standing in front of the wall's foot.
fn place(t: &Tuning, tuft: &mut GrassTuft, center: Position, solid: &impl Fn((i32, i32)) -> bool) -> bool {
    let (l, r, up) = art_extents(t, tuft);
    let half = OBSTACLE_GRID_SIZE / 2.0;
    let size = GRASS_TEXTURE_SIZE * t.grass_scale;
    let travel = up / size;
    let steady = (t.grass_wind_px + t.grass_gust_px + t.grass_sway_px) * travel;
    let (col, row) = crate::map::world_to_cell(center);
    // The near edge of the nearest tile in the cell's row on each side, a
    // cell or two away: the art is never wider than that.
    let edge = |dir: i32| {
        (1..=2).find(|&k| solid((col + dir * k, row))).map(|k| center.x + dir as f32 * (half + (k - 1) as f32 * OBSTACLE_GRID_SIZE))
    };
    let (west, east) = (edge(-1), edge(1));
    let lo = west.map_or(f32::NEG_INFINITY, |w| w + l + steady + TILE_SLACK_PX);
    let hi = east.map_or(f32::INFINITY, |e| e - r - steady - TILE_SLACK_PX);
    if lo > hi {
        return false;
    }
    tuft.base.x = tuft.base.x.clamp(lo, hi);
    tuft.lean = [f32::INFINITY; 2];
    if let Some(w) = west {
        tuft.lean[0] = ((tuft.base.x - l - w - TILE_SLACK_PX) / travel).max(0.0);
    }
    if let Some(e) = east {
        tuft.lean[1] = ((e - tuft.base.x - r - TILE_SLACK_PX) / travel).max(0.0);
    }
    // The columns the art can reach at any lean it is allowed. A pixel is
    // drawn where its centre is covered, so an edge less than half a pixel
    // into the next cell draws nothing there.
    let (x0, _, x1, bottom) = reach(t, tuft);
    let dip = bottom - tuft.base.y;
    let cell = |v: f32| ((v + half) / OBSTACLE_GRID_SIZE).floor() as i32;
    let cols = cell(x0 + 0.5)..=cell(x1 - 0.5);
    let walled = |row: i32| cols.clone().any(|c| solid((c, row)));
    let top_edge = center.y - half;
    let mut lowest = top_edge;
    let mut highest = center.y + half;
    if walled(row + 1) {
        highest = highest.min(center.y + half - dip - TILE_SLACK_PX);
    }
    for k in 1..=2 {
        if walled(row - k) {
            // That row's bottom edge, which the art may overlap only so far.
            let edge = top_edge - (k - 1) as f32 * OBSTACLE_GRID_SIZE;
            lowest = lowest.max(edge - t.grass_wall_overlap_px.max(0.0) + up);
        }
    }
    if lowest > highest {
        return false;
    }
    tuft.base.y = tuft.base.y.clamp(lowest, highest);
    true
}

/// Is `p` inside a tall-grass cell?
///
/// A point query against the *cells*, not the tufts: cover is a property of
/// the ground a tank is standing on, and testing against scattered sprites
/// would make concealment depend on exactly which way the wind blew.
pub fn conceals(cells: &[Position], p: Position) -> bool {
    let half = OBSTACLE_GRID_SIZE / 2.0;
    cells
        .iter()
        .any(|c| (p.x - c.x).abs() <= half && (p.y - c.y).abs() <= half)
}

/// Advance every tuft against the tanks driving through it.
///
/// Called once per simulation frame (`Game::tick_grass`) rather than at
/// draw time, so the crush recovery is frame-rate independent and the trail
/// behind a tank is the same length however fast the game renders. Pure in
/// the tank positions - it draws no RNG and touches nothing a seeded replay
/// can see.
pub fn tick(tufts: &mut [GrassTuft], movers: &[Mover], dt: f32) {
    let t = tuning();
    let (crush_r, wake_r) = (t.grass_crush_radius, t.grass_part_radius);
    let recover = t.grass_crush_recover_seconds.max(0.05);
    for tuft in tufts.iter_mut() {
        let mut target = 0.0f32;
        let mut push = 0.0f32;
        for m in movers {
            let dx = tuft.base.x - m.position.x;
            let dy = tuft.base.y - m.position.y;
            let d = (dx * dx + dy * dy).sqrt();
            // Distance to the hull box rather than to its centre: zero
            // anywhere under the tank, then growing outward.
            let outside = {
                let ox = (dx.abs() - m.half).max(0.0);
                let oy = (dy.abs() - m.half).max(0.0);
                (ox * ox + oy * oy).sqrt()
            };
            if crush_r > 0.0 && outside < crush_r {
                // Smoothstepped so the flattened patch has no hard rim.
                let f = 1.0 - outside / crush_r;
                target = target.max(f * f * (3.0 - 2.0 * f));
            }
            if wake_r > 0.0 && d < wake_r && d > 0.001 {
                let speed = (m.velocity.x * m.velocity.x + m.velocity.y * m.velocity.y).sqrt();
                // How far in front of the hull this tuft is, 0 behind to 1
                // dead ahead: a moving tank drives a bow wave, a parked one
                // just parts what it is sitting in.
                let ahead = if speed > 8.0 {
                    ((dx * m.velocity.x + dy * m.velocity.y) / (d * speed)).max(0.0)
                } else {
                    0.5
                };
                push += (dx / d) * (1.0 - d / wake_r) * t.grass_part_px * (0.45 + 0.75 * ahead);
            }
        }
        // Flattening is immediate - a hull does not ease grass down - but
        // standing back up takes the whole recovery, which is the trail.
        tuft.crush = if target > tuft.crush { target } else { (tuft.crush - dt / recover).max(0.0) };
        tuft.push = push;
    }
}

/// Flatten every tuft within `radius` of `center` at once - a blast's
/// pressure wave, using the same `crush` a hull drives so the ring
/// recovers over `grass_crush_recover_seconds` like a tank's trail. Pure
/// in its inputs, no RNG.
pub fn flatten(tufts: &mut [GrassTuft], center: Position, radius: f32) {
    if radius <= 0.0 {
        return;
    }
    for tuft in tufts.iter_mut() {
        if tuft.base.distance_to(center) <= radius {
            tuft.crush = 1.0;
        }
    }
}

/// The wind's lean at `base` at `time`, in px of travel at a tuft's tip:
/// a steady lean along the wind (`grass_wind_px`) plus gusts
/// (`grass_gust_px`) - a wave of bending grass riding a slower envelope,
/// both travelling along `grass_gust_heading_deg` at `grass_gust_speed`.
/// Every tuft on one front leans the same way at the same moment, which is
/// what makes a gust read as wind crossing the field. Only the wind's
/// sideways component shows, since a tuft bends about its root.
///
/// Pure in its inputs and drawing no RNG. Takes the table rather than
/// reading the global so a test can pass a local one.
pub fn wind_at(t: &Tuning, base: Position, time: f32) -> f32 {
    // `trig`, not libm: the CPU thumbnail of a grassy map is pinned by
    // hash and has to come out the same on every platform.
    let (dir_y, dir_x) = crate::trig::sin_cos(t.grass_gust_heading_deg.to_radians());
    // How far along the wind this point sits, less how far the fronts have
    // travelled: a front is a line of equal `along`, moving with the wind.
    let along = base.x * dir_x + base.y * dir_y - time * t.grass_gust_speed;
    let tau = std::f32::consts::TAU;
    let wave = crate::trig::sin(tau * along / t.grass_gust_wavelength.max(1.0));
    let gust = 0.5 + 0.5 * crate::trig::sin(tau * along / t.grass_gust_group.max(1.0));
    (t.grass_wind_px + t.grass_gust_px * gust * (0.55 + 0.45 * wave)) * dir_x
}

/// How far this tuft's tip leans right now, in world px: the wind, the
/// tuft's own flutter, and whatever `tick` last recorded. Matted grass
/// barely moves in the wind, so both are scaled down by the crush.
fn bend(tuft: &GrassTuft, time: f32) -> f32 {
    let t = tuning();
    // Per-tuft phase, so a field in a gust does not move as one sheet.
    let phase = (tuft.seed % 628) as f32 * 0.01;
    let flutter = crate::trig::sin(time * t.grass_sway_speed + phase) * t.grass_sway_px;
    let lean = (wind_at(&t, tuft.base, time) + flutter) * (1.0 - tuft.crush) + tuft.push;
    // Never over a tile beside the cell (`keep_off`).
    lean.clamp(-tuft.lean[0], tuft.lean[1])
}

/// Draw one tuft, leaning and squashed by however flat it is lying, from
/// `theme`'s sheet (`Sheet::Grass`).
pub fn draw_tuft(c: &mut impl Canvas, tuft: &GrassTuft, theme: Theme, time: f32) {
    let cell = GRASS_TEXTURE_SIZE;
    let t = tuning();
    if tuft.burnt {
        // A charred stub: two blocks of ash where the tuft stood. Not
        // nothing - a burnt meadow should read as burnt, not as mown.
        let x = (tuft.base.x / 2.0).floor() as i32 * 2;
        let y = (tuft.base.y / 2.0).floor() as i32 * 2;
        c.fill_rect(x - 2, y - 4, 2, 4, Color::new(0x37, 0x37, 0x37, 255));
        c.fill_rect(x, y - 2, 2, 2, Color::new(0x25, 0x25, 0x25, 255));
        return;
    }
    let size = cell * t.grass_scale;
    let flip = if tuft.seed & 1 != 0 { -1.0 } else { 1.0 };
    let src = Rectangle::new(tuft.col as f32 * cell, tuft.row as f32 * cell, cell * flip, cell);
    // Matted grass is *foreshortened*, not cropped: the blades bend over
    // rather than being mown, so the sprite squashes toward its own root.
    // Never all the way - `grass_crush_flatten` leaves a stub, because a
    // tuft that vanishes entirely reads as a hole in the field.
    let height = size * (1.0 - tuft.crush * t.grass_crush_flatten);
    // Rotating about the base is what makes the tip move and the root stay
    // put; raylib rotates about `origin`, so the origin sits at the bottom
    // centre of the sprite.
    let rotation = crate::trig::atan2(bend(tuft, time), size).to_degrees();
    let dest = Rectangle::new(tuft.base.x, tuft.base.y, size, height);
    let origin = Vec2::new(size / 2.0, height);
    c.blit(Sheet::Grass(theme), src, dest, origin, rotation, Color::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heading(t: &Tuning) -> (f32, f32) {
        let (y, x) = crate::trig::sin_cos(t.grass_gust_heading_deg.to_radians());
        (x, y)
    }

    #[test]
    fn a_gust_front_travels_with_the_wind() {
        let t = Tuning::DEFAULT;
        let (dx, dy) = heading(&t);
        for &(x, y, time) in &[(40.0, 60.0, 0.0), (700.0, 300.0, 3.5), (1000.0, 20.0, 41.0)] {
            let here = wind_at(&t, Position::new(x, y), time);
            // Where the same front is half a second later.
            let step = t.grass_gust_speed * 0.5;
            let there = wind_at(&t, Position::new(x + dx * step, y + dy * step), time + 0.5);
            assert!((here - there).abs() < 1e-2, "front at ({x}, {y}) t={time}: {here} vs {there}");
        }
    }

    #[test]
    fn tufts_on_one_front_lean_together() {
        let t = Tuning::DEFAULT;
        let (dx, dy) = heading(&t);
        // Along the front: perpendicular to the way it travels.
        let (px, py) = (-dy, dx);
        let a = wind_at(&t, Position::new(300.0, 200.0), 2.0);
        let b = wind_at(&t, Position::new(300.0 + px * 250.0, 200.0 + py * 250.0), 2.0);
        assert!((a - b).abs() < 1e-2, "{a} vs {b}");
    }

    #[test]
    fn the_grass_leans_the_way_the_wind_blows() {
        let east = Tuning::DEFAULT;
        let west = Tuning { grass_gust_heading_deg: east.grass_gust_heading_deg + 180.0, ..Tuning::DEFAULT };
        for i in 0..40 {
            let p = Position::new(37.0 * i as f32, 13.0 * i as f32);
            let time = 0.37 * i as f32;
            assert!(wind_at(&east, p, time) > 0.0, "an east wind leans right at {p:?}");
            assert!(wind_at(&west, p, time) < 0.0, "a west wind leans left at {p:?}");
        }
        let still = Tuning { grass_wind_px: 0.0, grass_gust_px: 0.0, ..Tuning::DEFAULT };
        assert_eq!(wind_at(&still, Position::new(100.0, 100.0), 7.0), 0.0);
    }

    /// Cells round the origin cell (0, 0), as `tufts_for_cell` names them.
    fn around(solid: &[(i32, i32)]) -> impl Fn((i32, i32)) -> bool + '_ {
        move |cell| solid.contains(&cell)
    }

    /// Does the box (left, top, right, bottom) cover a pixel centre of
    /// `cell`?
    fn covers(b: (f32, f32, f32, f32), (col, row): (i32, i32)) -> bool {
        let half = OBSTACLE_GRID_SIZE / 2.0 - 0.5;
        let (x, y) = (col as f32 * OBSTACLE_GRID_SIZE, row as f32 * OBSTACLE_GRID_SIZE);
        b.0 < x + half && b.2 > x - half && b.1 < y + half && b.3 > y - half
    }

    /// How far the box (left, top, right, bottom) reaches up into `cell`
    /// from its bottom edge, 0 where it covers no pixel centre of it.
    fn over_foot(b: (f32, f32, f32, f32), (col, row): (i32, i32)) -> f32 {
        let bottom = row as f32 * OBSTACLE_GRID_SIZE + OBSTACLE_GRID_SIZE / 2.0;
        if covers(b, (col, row)) { (bottom - b.1).max(0.0) } else { 0.0 }
    }

    #[test]
    fn a_tuft_never_covers_a_tile_past_a_walls_foot() {
        let t = Tuning { grass_tufts_per_cell: 6, ..Tuning::DEFAULT };
        let neighbours = [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1), (0, -2), (-2, 0), (2, 0), (-1, -2), (1, -2)];
        let (mut seeded, mut grown, mut at_a_foot) = (0, 0, 0);
        // Every set of the first eight neighbours, on cells all over the
        // field so the hashed spots and variants vary.
        for mask in 0u32..256 {
            let mut solid: Vec<(i32, i32)> = (0..8).filter(|b| mask & (1 << b) != 0).map(|b| neighbours[b]).collect();
            if mask % 3 == 0 {
                solid.extend_from_slice(&neighbours[8..]);
            }
            for k in 0..6 {
                // `tufts_for_cell` names cells by `world_to_cell` of the
                // centre, so the solid cells move with it.
                let (col, row) = (3 + mask as i32 % 29 + k * 7, 2 + k * 5);
                let shifted: Vec<(i32, i32)> = solid.iter().map(|&(c, r)| (c + col, r + row)).collect();
                let center = crate::map::cell_to_world(col, row);
                seeded += t.grass_tufts_per_cell as usize;
                for tuft in tufts_for_cell(&t, center, around(&shifted)) {
                    grown += 1;
                    let half = OBSTACLE_GRID_SIZE / 2.0;
                    assert!((tuft.base.x - center.x).abs() <= half && (tuft.base.y - center.y).abs() <= half, "rooted in its cell");
                    let b = reach(&t, &tuft);
                    for &cell in &shifted {
                        if cell.1 >= row {
                            assert!(!covers(b, cell), "a tuft at {:?} reaches the tile at {cell:?} beside or below it: {b:?}", tuft.base);
                        } else {
                            let over = over_foot(b, cell);
                            assert!(over <= t.grass_wall_overlap_px + 0.5, "a tuft at {:?} covers {over} px of the wall at {cell:?}", tuft.base);
                            if over > 0.0 {
                                at_a_foot += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(at_a_foot > 0, "some tufts stand in front of a wall's foot");
        assert!(grown * 10 > seeded * 9, "few cells are too crowded for a tuft: {grown} of {seeded}");
        // A wall along one side of a meadow takes none of its tufts away.
        for side in [(-1, 0), (1, 0), (0, 1), (0, -1)] {
            for i in 0..200 {
                let tufts = tufts_for_cell(&t, crate::map::cell_to_world(i, 3), |(c, r)| (c - i, r - 3) == side);
                assert_eq!(tufts.len(), t.grass_tufts_per_cell as usize, "a wall at {side:?}");
            }
        }
    }

    #[test]
    fn grass_under_a_wall_stands_at_its_foot() {
        let t = Tuning::DEFAULT;
        let foot = |center: Position, t: &Tuning| center.y - OBSTACLE_GRID_SIZE / 2.0 - t.grass_wall_overlap_px;
        for i in 0..200 {
            let center = crate::map::cell_to_world(i, 5);
            let open = tufts_for_cell(&t, center, |_| false);
            let walled = tufts_for_cell(&t, center, |c| c == (i, 4));
            for (a, b) in open.iter().zip(&walled) {
                // Short enough, a tuft stays as it grew; too tall, it moves
                // down or gives way to a shorter one.
                if a.base.y - art_extents(&t, a).2 >= foot(center, &t) {
                    assert_eq!((a.base.x, a.base.y, a.row, a.col), (b.base.x, b.base.y, b.row, b.col));
                }
                assert!(b.base.y - art_extents(&t, b).2 >= foot(center, &t) - 1e-3);
            }
        }
        // No overlap allowed: the grass stops at the wall's edge.
        let flush = Tuning { grass_wall_overlap_px: 0.0, ..t };
        for i in 0..200 {
            let center = crate::map::cell_to_world(i, 5);
            for b in tufts_for_cell(&flush, center, |c| c == (i, 4)) {
                assert!(b.base.y - art_extents(&flush, &b).2 >= foot(center, &flush) - 1e-3);
            }
        }
    }

    #[test]
    fn a_tuft_with_no_tile_round_it_is_left_alone() {
        let t = Tuning { grass_tufts_per_cell: 8, ..Tuning::DEFAULT };
        for i in 0..40 {
            let center = crate::map::cell_to_world(i * 3 + 1, i % 17);
            let open = tufts_for_cell(&t, center, |_| false);
            // A tile two cells off is out of every tuft's reach.
            let far = tufts_for_cell(&t, center, |(c, r)| (c - (i * 3 + 1)).abs() > 2 || (r - i % 17).abs() > 2);
            for (a, b) in open.iter().zip(&far) {
                assert!(a.lean == [f32::INFINITY; 2]);
                assert_eq!((a.base.x, a.base.y, a.row, a.col), (b.base.x, b.base.y, b.row, b.col));
            }
        }
    }

    #[test]
    fn a_tile_beside_the_cell_caps_the_lean_toward_it() {
        let t = Tuning::DEFAULT;
        for i in 0..60 {
            let center = crate::map::cell_to_world(i, 4);
            let tufts = tufts_for_cell(&t, center, |c| c == (i - 1, 4));
            for tuft in &tufts {
                assert!(tuft.lean[0].is_finite() && tuft.lean[1].is_infinite());
                // The steady lean always fits: wind, gust and flutter.
                let (_, _, up) = art_extents(&t, tuft);
                let steady = (t.grass_wind_px + t.grass_gust_px + t.grass_sway_px) * up / (GRASS_TEXTURE_SIZE * t.grass_scale);
                assert!(tuft.lean[0] * up / (GRASS_TEXTURE_SIZE * t.grass_scale) >= steady - 1e-3);
            }
        }
    }

    /// `TUFT_EXTENTS` covers every tuft's opaque pixels in every theme's
    /// sheet (the moon's generator clips its crystals to it). Decoding is raylib's, so this needs the `render` feature.
    #[cfg(feature = "render")]
    #[test]
    fn tuft_extents_cover_both_sheets() {
        use crate::canvas::Pixels;
        let cell = GRASS_TEXTURE_SIZE as usize;
        for theme in Theme::ALL {
            let sheet = Pixels::load(&Sheet::Grass(theme).path()).unwrap();
            for row in 0..GRASS_SPECIES as usize {
                for col in 0..GRASS_VARIANTS as usize {
                    let (l, r, up) = TUFT_EXTENTS[row][col];
                    for y in 0..cell {
                        for x in 0..cell {
                            if sheet.data[(row * cell + y) * sheet.width + col * cell + x].a == 0 {
                                continue;
                            }
                            let root = cell as i32 / 2;
                            let (x, y) = (x as i32, y as i32);
                            assert!(
                                x >= root - l as i32 && x < root + r as i32 && y >= cell as i32 - up as i32,
                                "{theme:?} tuft ({row}, {col}) has a pixel at ({x}, {y}) past {:?}",
                                (l, r, up)
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn gusts_come_and_go() {
        let t = Tuning::DEFAULT;
        let p = Position::new(500.0, 250.0);
        let leans: Vec<f32> = (0..400).map(|i| wind_at(&t, p, i as f32 * 0.05)).collect();
        let lo = leans.iter().cloned().fold(f32::MAX, f32::min);
        let hi = leans.iter().cloned().fold(f32::MIN, f32::max);
        let (dx, _) = heading(&t);
        assert!(lo < (t.grass_wind_px + 0.5) * dx, "the field settles between gusts: {lo}");
        assert!(hi > (t.grass_wind_px + 0.8 * t.grass_gust_px) * dx, "a gust bends it well past the steady lean: {hi}");
    }
}
