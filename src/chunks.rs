//! A wall tile's chunks (docs/WALLS_SPEC.md §10, BB-80): a brick or wood
//! tile is a 4×4 grid of 8 px chunks, each with its own health, so a shot
//! breaks the piece it strikes rather than wearing down one pool for the
//! whole tile. What stands decides what a shot can pass (`Terrain::sweep`
//! tests the standing chunks), what the tile's body covers
//! (`solid_box`) and when the tile gives way (`Obstacle::strike`).
//!
//! Plain data and geometry: no RNG, no raylib. The damage it spreads is a
//! pure function of the blow, so a replay breaks the same chunks.

use crate::math::Vec2;
use crate::{OBSTACLE_GRID_SIZE, Position};

/// Chunks along a tile's side.
pub const CHUNK_SIDE: usize = 4;

/// Chunks in a tile.
pub const CHUNKS: usize = CHUNK_SIDE * CHUNK_SIDE;

/// A chunk's side in world px: a quarter of the 32 px cell, four design
/// pixels.
pub const CHUNK_PX: f32 = OBSTACLE_GRID_SIZE / CHUNK_SIDE as f32;

/// Every chunk standing.
pub const ALL: u16 = u16::MAX;

/// The steps a chunk's wear travels in (`wear`), and in which it goes on
/// the wire: 0 gone, 1..=3 standing, 3 whole or nearly.
pub const WEAR_STEPS: u8 = 3;

/// A blow at least this large breaks every chunk it reaches: the callers
/// that mean "this tile is gone" pass `f32::MAX`.
pub const LETHAL: f32 = 1.0e6;

/// One tile's chunks: health per chunk, row-major from the tile's top-left
/// corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chunks {
    hp: [f32; CHUNKS],
    max: f32,
}

impl Chunks {
    /// Every chunk whole at `max` health.
    pub fn new(max: f32) -> Self {
        Chunks { hp: [max; CHUNKS], max }
    }

    /// The chunks as the wire carried them (`quantised`): each standing
    /// one at its wear step's share of `max`.
    pub fn from_quantised(q: u32, max: f32) -> Self {
        let mut hp = [0.0; CHUNKS];
        for (i, h) in hp.iter_mut().enumerate() {
            let step = ((q >> (i * 2)) & 0b11) as u8;
            *h = max * step as f32 / WEAR_STEPS as f32;
        }
        Chunks { hp, max }
    }

    /// Each chunk's wear in two bits, chunk 0 lowest.
    pub fn quantised(&self) -> u32 {
        (0..CHUNKS).fold(0u32, |q, i| q | (self.wear(i) as u32) << (i * 2))
    }

    /// One chunk's health.
    pub fn hp(&self, i: usize) -> f32 {
        self.hp[i]
    }

    /// A whole chunk's health.
    pub fn max(&self) -> f32 {
        self.max
    }

    /// Whether chunk `i` still stands.
    pub fn stands(&self, i: usize) -> bool {
        self.hp[i] > 0.0
    }

    /// The standing chunks, bit `i` for chunk `i`.
    pub fn standing(&self) -> u16 {
        (0..CHUNKS).filter(|&i| self.stands(i)).fold(0u16, |m, i| m | 1 << i)
    }

    /// How many chunks stand.
    pub fn count(&self) -> u32 {
        self.standing().count_ones()
    }

    /// Nothing broken and nothing worn.
    pub fn is_whole(&self) -> bool {
        (0..CHUNKS).all(|i| self.wear(i) == WEAR_STEPS)
    }

    /// Chunk `i`'s wear step: 0 gone, `WEAR_STEPS` whole (or worn by less
    /// than a step), rounding up so a scratched chunk still draws whole.
    pub fn wear(&self, i: usize) -> u8 {
        if !self.stands(i) {
            return 0;
        }
        let frac = (self.hp[i] / self.max).clamp(0.0, 1.0);
        ((frac * WEAR_STEPS as f32).ceil() as u8).clamp(1, WEAR_STEPS)
    }

    /// The share of the tile's health left, 0..=1.
    pub fn fraction(&self) -> f32 {
        self.hp.iter().sum::<f32>() / (self.max * CHUNKS as f32)
    }

    /// Break every chunk; returns the ones that stood.
    pub fn break_all(&mut self) -> u16 {
        let was = self.standing();
        self.hp = [0.0; CHUNKS];
        was
    }

    /// The chunk a shot arriving at `at` along `dir` strikes, on the tile
    /// centred on `center`: the first standing chunk on its way in, else the
    /// standing one nearest to where it struck. `None` when nothing stands.
    pub fn struck(&self, center: Position, at: Position, dir: Option<Vec2>) -> Option<usize> {
        if let Some(d) = dir {
            let len = d.length();
            if len > 1e-6 {
                let step = Vec2::new(d.x / len, d.y / len);
                // Walk in from just outside the face: the hit point sits on
                // the box's edge, which may be a seam-closed face a few px
                // outside the cell.
                for s in 0..(OBSTACLE_GRID_SIZE as i32 + 8) {
                    let p = Position::new(at.x + step.x * s as f32, at.y + step.y * s as f32);
                    if let Some(i) = index_at(center, p) {
                        if self.stands(i) {
                            return Some(i);
                        }
                    }
                }
            }
        }
        (0..CHUNKS)
            .filter(|&i| self.stands(i))
            .min_by(|&a, &b| chunk_center(center, a).distance_to(at).total_cmp(&chunk_center(center, b).distance_to(at)).then(a.cmp(&b)))
    }

    /// A blow of `amount` on chunk `i`: the chunk takes all of it, its four
    /// side neighbours `ring` of it and its corners `corner` of it. Returns
    /// the chunks it broke.
    pub fn strike(&mut self, i: usize, amount: f32, ring: f32, corner: f32) -> u16 {
        let was = self.standing();
        if amount >= LETHAL {
            return self.break_all();
        }
        let (col, row) = ((i % CHUNK_SIDE) as i32, (i / CHUNK_SIDE) as i32);
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (c, r) = (col + dx, row + dy);
                if !(0..CHUNK_SIDE as i32).contains(&c) || !(0..CHUNK_SIDE as i32).contains(&r) {
                    continue;
                }
                let share = match dx.abs() + dy.abs() {
                    0 => 1.0,
                    1 => ring,
                    _ => corner,
                };
                let j = r as usize * CHUNK_SIDE + c as usize;
                self.hp[j] = (self.hp[j] - amount * share).max(0.0);
            }
        }
        was & !self.standing()
    }

    /// A blast of `amount` from `from` on the tile centred on `center`:
    /// the chunks facing it take all of it, the far side half. Returns the
    /// chunks it broke.
    pub fn blast(&mut self, center: Position, from: Position, amount: f32) -> u16 {
        let was = self.standing();
        if amount >= LETHAL {
            return self.break_all();
        }
        let near = (0..CHUNKS).map(|i| chunk_center(center, i).distance_to(from)).fold(f32::MAX, f32::min);
        for i in 0..CHUNKS {
            let back = (chunk_center(center, i).distance_to(from) - near) / OBSTACLE_GRID_SIZE;
            let share = 1.0 - 0.5 * back.clamp(0.0, 1.0);
            self.hp[i] = (self.hp[i] - amount * share).max(0.0);
        }
        was & !self.standing()
    }

    /// The same share of every chunk's health gone: a blow with no place,
    /// worth `fraction` of the whole tile. Returns the chunks it broke.
    pub fn wear_evenly(&mut self, fraction: f32) -> u16 {
        let was = self.standing();
        let loss = self.max * fraction.max(0.0);
        for h in &mut self.hp {
            *h = (*h - loss).max(0.0);
        }
        was & !self.standing()
    }

    /// The box the tile's body covers, in px from the tile's centre, as
    /// `(offset, half extents)`: the bounds of its standing quadrants, a
    /// 16 px quadrant standing while two of its four chunks do. `None` when
    /// no quadrant stands.
    pub fn solid_box(&self) -> Option<(Vec2, Vec2)> {
        let half = OBSTACLE_GRID_SIZE / 2.0;
        let mut lo = Vec2::new(f32::MAX, f32::MAX);
        let mut hi = Vec2::new(f32::MIN, f32::MIN);
        for q in 0..4 {
            let (qc, qr) = (q % 2, q / 2);
            let n = (0..4).filter(|k| self.stands((qr * 2 + k / 2) * CHUNK_SIDE + qc * 2 + k % 2)).count();
            if n >= 2 {
                let x0 = qc as f32 * half - half;
                let y0 = qr as f32 * half - half;
                lo = Vec2::new(lo.x.min(x0), lo.y.min(y0));
                hi = Vec2::new(hi.x.max(x0 + half), hi.y.max(y0 + half));
            }
        }
        (lo.x <= hi.x).then(|| (Vec2::new((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0), Vec2::new((hi.x - lo.x) / 2.0, (hi.y - lo.y) / 2.0)))
    }
}

/// The top-left corner of the tile centred on `center`.
pub fn origin(center: Position) -> Position {
    Position::new(center.x - OBSTACLE_GRID_SIZE / 2.0, center.y - OBSTACLE_GRID_SIZE / 2.0)
}

/// The chunk of the tile centred on `center` that `p` lies in, if any.
pub fn index_at(center: Position, p: Position) -> Option<usize> {
    let o = origin(center);
    let (lx, ly) = (p.x - o.x, p.y - o.y);
    if lx < 0.0 || ly < 0.0 || lx >= OBSTACLE_GRID_SIZE || ly >= OBSTACLE_GRID_SIZE {
        return None;
    }
    Some((ly / CHUNK_PX) as usize * CHUNK_SIDE + (lx / CHUNK_PX) as usize)
}

/// Chunk `i`'s top-left corner in px from the tile's top-left.
pub fn chunk_offset(i: usize) -> Vec2 {
    Vec2::new((i % CHUNK_SIDE) as f32 * CHUNK_PX, (i / CHUNK_SIDE) as f32 * CHUNK_PX)
}

/// Chunk `i`'s centre in world px, on the tile centred on `center`.
pub fn chunk_center(center: Position, i: usize) -> Position {
    let o = origin(center);
    let off = chunk_offset(i);
    Position::new(o.x + off.x + CHUNK_PX / 2.0, o.y + off.y + CHUNK_PX / 2.0)
}

/// The chunk beside `i` one step along `(dx, dy)` on the same tile, if the
/// step stays on it.
pub fn neighbour(i: usize, dx: i32, dy: i32) -> Option<usize> {
    let (c, r) = ((i % CHUNK_SIDE) as i32 + dx, (i / CHUNK_SIDE) as i32 + dy);
    ((0..CHUNK_SIDE as i32).contains(&c) && (0..CHUNK_SIDE as i32).contains(&r)).then(|| r as usize * CHUNK_SIDE + c as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: Position = Position { x: 64.0, y: 64.0 };

    #[test]
    fn a_blow_breaks_the_struck_chunk_and_wears_its_ring() {
        let mut ch = Chunks::new(10.0);
        let broken = ch.strike(5, 20.0, 0.6, 0.3);
        // 5 and its side neighbours 1, 4, 6, 9 take 20 and 12: gone. The
        // corners take 6 and stand.
        assert_eq!(broken, (1 << 5) | (1 << 1) | (1 << 4) | (1 << 6) | (1 << 9));
        assert!(ch.stands(0) && ch.stands(10));
        assert_eq!(ch.wear(0), 2);
        assert_eq!(ch.count(), 11);
    }

    #[test]
    fn a_lethal_blow_breaks_everything() {
        let mut ch = Chunks::new(10.0);
        assert_eq!(ch.strike(0, f32::MAX, 0.6, 0.3), ALL);
        assert_eq!(ch.count(), 0);
    }

    #[test]
    fn a_shot_strikes_the_first_standing_chunk_on_its_way_in() {
        let mut ch = Chunks::new(10.0);
        // From the left along the second row: chunk 4 first.
        let at = Position::new(C.x - 16.0, C.y - 4.0);
        assert_eq!(ch.struck(C, at, Some(Vec2::new(1.0, 0.0))), Some(4));
        // With 4 and 5 broken it reaches 6.
        ch.strike(4, 100.0, 0.0, 0.0);
        ch.strike(5, 100.0, 0.0, 0.0);
        assert_eq!(ch.struck(C, at, Some(Vec2::new(1.0, 0.0))), Some(6));
    }

    #[test]
    fn wear_round_trips_through_the_wire() {
        let mut ch = Chunks::new(10.0);
        ch.strike(5, 20.0, 0.6, 0.3);
        let back = Chunks::from_quantised(ch.quantised(), 10.0);
        assert_eq!(back.standing(), ch.standing());
        assert!((0..CHUNKS).all(|i| back.wear(i) == ch.wear(i)));
        assert!(Chunks::from_quantised(Chunks::new(4.0).quantised(), 4.0).is_whole());
    }

    #[test]
    fn the_body_covers_the_standing_quadrants() {
        let mut ch = Chunks::new(10.0);
        assert_eq!(ch.solid_box(), Some((Vec2::new(0.0, 0.0), Vec2::new(16.0, 16.0))));
        // The left column of quadrants gone: the body is the right half.
        for i in [0, 1, 4, 5, 8, 9, 12, 13] {
            ch.strike(i, 100.0, 0.0, 0.0);
        }
        assert_eq!(ch.solid_box(), Some((Vec2::new(8.0, 0.0), Vec2::new(8.0, 16.0))));
        ch.break_all();
        assert_eq!(ch.solid_box(), None);
    }

    #[test]
    fn a_blast_hits_its_near_face_hardest() {
        let mut ch = Chunks::new(10.0);
        let broken = ch.blast(C, Position::new(C.x - 40.0, C.y), 12.0);
        // The left column breaks; the right column takes half and stands.
        for r in 0..4 {
            assert!(broken & (1 << (r * 4)) != 0);
            assert!(ch.stands(r * 4 + 3));
        }
    }
}
