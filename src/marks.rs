//! The marks a round leaves on its floor - burn marks (`blast::Scorch`) and
//! rubble (`decal::Decal`) - kept oldest first and found by where they lie:
//! `Marks` is the list `Game` holds, with every mark also filed under the
//! `BLOCK_PX` block its anchor is in, so a painter over part of the field
//! (`Marks::shown`) visits the blocks its view crosses rather than every
//! mark of the round. The order a painter draws in is the list's, whatever
//! it asked for. Nothing that plays reads the blocks.

use crate::math::Rectangle;
use crate::Position;
use std::collections::HashMap;

/// The side of a block of the floor, in px: a few to a screen, so a view
/// crosses a handful of them.
pub const BLOCK_PX: f32 = 256.0;

/// The block `at` lies in.
pub fn block_of(at: Position) -> (i32, i32) {
    ((at.x / BLOCK_PX).floor() as i32, (at.y / BLOCK_PX).floor() as i32)
}

/// Every key of `blocks` that `rect` crosses, in no order: the blocks from
/// its corner's to its far corner's, or the kept keys when those are fewer.
pub fn blocks_over<'a, V>(blocks: &'a HashMap<(i32, i32), V>, rect: Rectangle) -> impl Iterator<Item = &'a V> + 'a {
    let (x0, y0) = block_of(Position::new(rect.x, rect.y));
    let (x1, y1) = block_of(Position::new(rect.x + rect.width, rect.y + rect.height));
    let across = (x1 as i64 - x0 as i64 + 1).max(0) * (y1 as i64 - y0 as i64 + 1).max(0);
    let kept = across > blocks.len() as i64;
    let by_key = blocks.iter().filter(move |_| kept).filter(move |&(&(x, y), _)| (x0..=x1).contains(&x) && (y0..=y1).contains(&y)).map(|(_, v)| v);
    let by_block = (y0..=y1)
        .filter(move |_| !kept)
        .flat_map(move |y| (x0..=x1).map(move |x| (x, y)))
        .filter_map(move |key| blocks.get(&key));
    by_key.chain(by_block)
}

/// Something lying on the floor at a point a painter culls it by.
pub trait Anchored {
    fn anchor(&self) -> Position;
}

impl Anchored for crate::blast::Scorch {
    fn anchor(&self) -> Position {
        self.center
    }
}

impl Anchored for crate::decal::Decal {
    fn anchor(&self) -> Position {
        self.center
    }
}

/// A round's marks of one kind, oldest first (it reads as the slice it
/// holds). Each mark has a number - how many came before it this round -
/// and its block lists that number, so dropping the oldest moves nothing:
/// numbers under `dropped` are gone, and `tidy` sweeps them out of the
/// blocks once they outnumber the marks kept. A mark whose anchor is moved
/// in place is filed again with `moved`.
pub struct Marks<T> {
    items: Vec<T>,
    /// The block each of `items` is filed under.
    homes: Vec<(i32, i32)>,
    /// How many marks the round has dropped: the number of `items[0]`.
    dropped: u64,
    /// Each block's marks by number, rising.
    blocks: HashMap<(i32, i32), Vec<u64>>,
    /// Numbers still filed that are under `dropped`.
    stale: usize,
}

impl<T> Default for Marks<T> {
    fn default() -> Self {
        Marks { items: Vec::new(), homes: Vec::new(), dropped: 0, blocks: HashMap::new(), stale: 0 }
    }
}

impl<T> std::ops::Deref for Marks<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.items
    }
}

impl<T> std::ops::DerefMut for Marks<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.items
    }
}

impl<'a, T> IntoIterator for &'a Marks<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut Marks<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter_mut()
    }
}

impl<T: Anchored> Marks<T> {
    /// One more mark, the newest.
    pub fn push(&mut self, item: T) {
        let home = block_of(item.anchor());
        self.blocks.entry(home).or_default().push(self.dropped + self.items.len() as u64);
        self.items.push(item);
        self.homes.push(home);
    }

    /// Every mark of `more`, in its order.
    pub fn extend(&mut self, more: impl IntoIterator<Item = T>) {
        for item in more {
            self.push(item);
        }
    }

    /// Keep the newest `cap` marks.
    pub fn hold(&mut self, cap: usize) {
        if self.items.len() <= cap {
            return;
        }
        let excess = self.items.len() - cap;
        self.items.drain(..excess);
        self.homes.drain(..excess);
        self.dropped += excess as u64;
        self.stale += excess;
        if self.stale > self.items.len() {
            self.tidy();
        }
    }

    /// Drop the numbers of dropped marks from every block, and the blocks
    /// left empty.
    fn tidy(&mut self) {
        let dropped = self.dropped;
        self.blocks.retain(|_, numbers| {
            let gone = numbers.partition_point(|&n| n < dropped);
            numbers.drain(..gone);
            !numbers.is_empty()
        });
        self.stale = 0;
    }

    /// No marks: a new round's floor.
    pub fn clear(&mut self) {
        *self = Marks::default();
    }

    /// File the mark at `index` under the block its anchor is in now: call
    /// it after moving one in place.
    pub fn moved(&mut self, index: usize) {
        let home = block_of(self.items[index].anchor());
        let old = std::mem::replace(&mut self.homes[index], home);
        if old == home {
            return;
        }
        let number = self.dropped + index as u64;
        if let Some(numbers) = self.blocks.get_mut(&old) {
            if let Ok(at) = numbers.binary_search(&number) {
                numbers.remove(at);
            }
        }
        let numbers = self.blocks.entry(home).or_default();
        let at = numbers.partition_point(|&n| n < number);
        numbers.insert(at, number);
    }

    /// The marks a painter culling to `cull` draws (`view::culled` on each
    /// anchor), oldest first: every mark with no rectangle, else the ones
    /// anchored inside it, found through the blocks it crosses.
    pub fn shown(&self, cull: Option<Rectangle>) -> impl Iterator<Item = &T> + '_ {
        let (all, picked) = match cull {
            None => (Some(self.items.iter()), Vec::new()),
            Some(rect) => {
                let mut picked: Vec<usize> = blocks_over(&self.blocks, rect)
                    .flat_map(|numbers| numbers[numbers.partition_point(|&n| n < self.dropped)..].iter())
                    .map(|&n| (n - self.dropped) as usize)
                    .filter(|&i| rect.contains(self.items[i].anchor()))
                    .collect();
                picked.sort_unstable();
                (None, picked)
            }
        };
        all.into_iter().flatten().chain(picked.into_iter().map(move |i| &self.items[i]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blast::Scorch;

    /// A scatter of points over and round a field of a few screens, some
    /// on block edges, hashed so the test is the same on every run.
    fn scatter(n: u32) -> Vec<Position> {
        (0..n)
            .map(|i| {
                let h = i.wrapping_mul(0x9E37_79B9) ^ (i << 7);
                let (x, y) = ((h % 4000) as f32 - 300.0, ((h >> 12) % 2600) as f32 - 300.0);
                if i % 17 == 0 { Position::new((x / BLOCK_PX).round() * BLOCK_PX, (y / BLOCK_PX).round() * BLOCK_PX) } else { Position::new(x, y) }
            })
            .collect()
    }

    fn views() -> Vec<Rectangle> {
        vec![
            Rectangle::new(0.0, 0.0, 1280.0, 720.0),
            Rectangle::new(256.0, 512.0, 256.0, 256.0),
            Rectangle::new(-1000.0, -1000.0, 300.0, 300.0),
            Rectangle::new(900.5, 310.25, 733.0, 411.0),
            Rectangle::new(-1.0e7, -1.0e7, 2.0e7, 2.0e7),
            Rectangle::new(500.0, 500.0, 0.0, 0.0),
        ]
    }

    /// What `shown` promises, against the walk it stands in for.
    fn assert_as_a_walk(marks: &Marks<Scorch>, what: &str) {
        for view in views() {
            let walked: Vec<u32> = marks.iter().filter(|s| !crate::view::culled(Some(view), s.center)).map(|s| s.seed).collect();
            let shown: Vec<u32> = marks.shown(Some(view)).map(|s| s.seed).collect();
            assert_eq!(shown, walked, "{what}: {view:?}");
        }
        let all: Vec<u32> = marks.shown(None).map(|s| s.seed).collect();
        assert_eq!(all, marks.iter().map(|s| s.seed).collect::<Vec<_>>(), "{what}: no rectangle is every mark");
    }

    fn scorch(at: Position, seed: u32) -> Scorch {
        Scorch { seed, ..Scorch::new(at) }
    }

    #[test]
    fn a_view_shows_the_marks_a_walk_would_in_its_order() {
        let mut marks = Marks::default();
        for (i, at) in scatter(900).into_iter().enumerate() {
            marks.push(scorch(at, i as u32));
        }
        assert_as_a_walk(&marks, "as pushed");
    }

    #[test]
    fn dropping_the_oldest_and_moving_one_keep_the_views_right() {
        let mut marks = Marks::default();
        let points = scatter(2400);
        for (i, at) in points.iter().enumerate() {
            marks.push(scorch(*at, i as u32));
            // A ceiling of 300, as `Game::show` holds its lists.
            marks.hold(300);
            if i % 211 == 0 {
                assert_as_a_walk(&marks, "held to its ceiling");
            }
        }
        assert_eq!(marks.len(), 300);
        assert_eq!(marks[0].seed, 2100, "the oldest went");
        assert!(marks.blocks.values().map(Vec::len).sum::<usize>() <= 600, "the dropped numbers are swept out");
        // A mark thrown across the field in place, as a blast rethrows rubble.
        for i in [0usize, 7, 150, 299] {
            marks[i].center = Position::new(1000.0 + i as f32, 400.0);
            marks.moved(i);
        }
        assert_as_a_walk(&marks, "after moving four");
        marks.clear();
        assert!(marks.is_empty() && marks.shown(Some(views()[0])).next().is_none());
    }
}
