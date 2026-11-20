//! The particle layer's store (`fx.rs` owns the particles and what they
//! do): every live particle oldest first in one run of memory, which is
//! the order they are drawn in, with the oldest dropped by moving the
//! run's start rather than the run - a queue that slides along its buffer
//! and is moved back to the front once half of it is behind the start.
//!
//! At the cap (`fx_max_particles`) a new particle takes the place of the
//! oldest one that may go. Each particle counts against a `Budget`, and a
//! budget's `floor` is how many of its particles the others cannot take:
//! the smoke off a damaged hull is a tell a player reads, so a
//! flamethrower and a missile volley flooding the cap eat into each
//! other's sparks and old smoke and leave it standing. Under the cap
//! nothing is dropped and the budgets decide nothing.

use crate::fx::ParticleKind;

/// How many kinds of particle there are (`ParticleKind`'s variants).
const KINDS: usize = 10;

/// What a particle counts against at the cap: its kind, or - whatever its
/// kind - the smoke and embers a damaged hull gives off.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Budget {
    Kind(ParticleKind),
    Damage,
}

impl Budget {
    fn index(self) -> usize {
        match self {
            Budget::Kind(kind) => match kind {
                ParticleKind::Spark => 0,
                ParticleKind::Chip => 1,
                ParticleKind::Dust => 2,
                ParticleKind::Smoke => 3,
                ParticleKind::Ember => 4,
                ParticleKind::Spray => 5,
                ParticleKind::Trail => 6,
                ParticleKind::Plume => 7,
                ParticleKind::Kick => 8,
                ParticleKind::Foam => 9,
            },
            Budget::Damage => KINDS,
        }
    }
}

/// How many particles of each budget the others cannot take at the cap:
/// `damage` for a damaged hull's (`fx_damage_smoke_reserve`), none for a
/// kind's own.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Floors {
    pub damage: usize,
}

impl Floors {
    fn of(&self, budget: usize) -> usize {
        if budget == KINDS { self.damage } else { 0 }
    }
}

pub(crate) struct Particles<T> {
    /// `slots[start..]` are the live particles, oldest first; what is
    /// before `start` was dropped and waits to be written over.
    slots: Vec<T>,
    /// The budget each of `slots` counts against (`Budget::index`).
    budgets: Vec<u8>,
    start: usize,
    /// How many live particles count against each budget.
    counts: [usize; KINDS + 1],
}

impl<T> Default for Particles<T> {
    fn default() -> Self {
        Particles { slots: Vec::new(), budgets: Vec::new(), start: 0, counts: [0; KINDS + 1] }
    }
}

impl<T> std::ops::Deref for Particles<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.slots[self.start..]
    }
}

impl<T> std::ops::DerefMut for Particles<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.slots[self.start..]
    }
}

impl<T> Particles<T> {
    /// Drop every particle.
    pub fn clear(&mut self) {
        self.slots.clear();
        self.budgets.clear();
        self.start = 0;
        self.counts = [0; KINDS + 1];
    }

    /// One more particle, the newest, counted against `budget`. With `cap`
    /// live already, the oldest that may go under `floors` goes first -
    /// and more than one where the cap came down since; a cap of 0 takes
    /// none.
    pub fn push(&mut self, p: T, budget: Budget, cap: usize, floors: Floors) {
        if cap == 0 {
            return;
        }
        while self.len() >= cap {
            self.drop_one(floors);
        }
        // The run slid half way along its buffer: back to the front.
        if self.start > 0 && self.start >= self.len() {
            self.slots.drain(..self.start);
            self.budgets.drain(..self.start);
            self.start = 0;
        }
        let budget = budget.index();
        self.counts[budget] += 1;
        self.budgets.push(budget as u8);
        self.slots.push(p);
    }

    /// Drop the oldest particle whose budget holds more than its floor, or
    /// the oldest of all where every budget is at its floor. The ones older
    /// than it move up one place, so the order stands.
    fn drop_one(&mut self, floors: Floors) {
        if self.is_empty() {
            return;
        }
        let found = self.budgets[self.start..].iter().position(|&b| self.counts[b as usize] > floors.of(b as usize)).unwrap_or(0);
        let at = self.start + found;
        self.counts[self.budgets[at] as usize] -= 1;
        self.slots[self.start..=at].rotate_right(1);
        self.budgets[self.start..=at].rotate_right(1);
        self.start += 1;
    }

    /// Count the newest `n` particles against `budget` from now on.
    pub fn count_newest_as(&mut self, n: usize, budget: Budget) {
        let from = self.slots.len() - n.min(self.len());
        for b in &mut self.budgets[from..] {
            self.counts[*b as usize] -= 1;
            *b = budget.index() as u8;
            self.counts[*b as usize] += 1;
        }
    }

    /// Keep the particles `keep` answers true for, in their order, handing
    /// it each to change as it decides.
    pub fn retain_mut(&mut self, mut keep: impl FnMut(&mut T) -> bool) {
        let mut kept = 0;
        for at in self.start..self.slots.len() {
            if keep(&mut self.slots[at]) {
                self.slots.swap(kept, at);
                self.budgets.swap(kept, at);
                kept += 1;
            } else {
                self.counts[self.budgets[at] as usize] -= 1;
            }
        }
        self.slots.truncate(kept);
        self.budgets.truncate(kept);
        self.start = 0;
    }

    /// How many live particles count against `budget`.
    #[cfg(test)]
    pub fn counted(&self, budget: Budget) -> usize {
        self.counts[budget.index()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Floors = Floors { damage: 0 };
    const SPARK: Budget = Budget::Kind(ParticleKind::Spark);
    const EMBER: Budget = Budget::Kind(ParticleKind::Ember);

    #[test]
    fn every_kind_has_a_budget_of_its_own() {
        let kinds = [
            ParticleKind::Spark,
            ParticleKind::Chip,
            ParticleKind::Dust,
            ParticleKind::Smoke,
            ParticleKind::Ember,
            ParticleKind::Spray,
            ParticleKind::Trail,
            ParticleKind::Plume,
            ParticleKind::Kick,
            ParticleKind::Foam,
        ];
        let mut seen: Vec<usize> = kinds.iter().map(|&k| Budget::Kind(k).index()).collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..KINDS).collect::<Vec<_>>(), "ten kinds, ten budgets, the damage budget after them");
        assert_eq!(Budget::Damage.index(), KINDS);
    }

    /// Under the cap the store is the list it was pushed as, and at the
    /// cap with no floor it drops the oldest: what a plain list drained
    /// from its front does, push for push.
    #[test]
    fn with_no_floor_it_is_a_list_that_drops_its_oldest() {
        let mut store: Particles<u32> = Particles::default();
        let mut list: Vec<u32> = Vec::new();
        let cap = 40;
        for n in 0..500u32 {
            store.push(n, if n % 3 == 0 { EMBER } else { SPARK }, cap, NONE);
            if n % 5 == 0 {
                store.count_newest_as(1, Budget::Damage);
            }
            list.push(n);
            if list.len() > cap {
                list.remove(0);
            }
            assert_eq!(store[..], list[..], "after push {n}");
            // Some age out of the middle, as `Fx::tick` drops them.
            if n % 37 == 0 {
                store.retain_mut(|m| *m % 4 != 1);
                list.retain(|m| m % 4 != 1);
                assert_eq!(store[..], list[..], "after the retain at {n}");
            }
        }
        assert!(store.slots.len() <= 2 * cap, "the buffer is moved back, not grown: {}", store.slots.len());
        assert_eq!(store.counts.iter().sum::<usize>(), store.len(), "every live particle is counted once");
    }

    /// A flood at the cap leaves the damage smoke under its floor standing
    /// and takes the oldest of everything else, in order.
    #[test]
    fn a_full_store_keeps_the_damage_smoke_under_its_floor() {
        let floors = Floors { damage: 6 };
        let cap = 20;
        let mut store: Particles<u32> = Particles::default();
        // Four wisps off a damaged hull among old sparks, oldest first.
        for n in 0..12u32 {
            store.push(n, Budget::Kind(ParticleKind::Smoke), cap, floors);
            if n % 3 == 0 {
                store.count_newest_as(1, Budget::Damage);
            }
        }
        assert_eq!(store.counted(Budget::Damage), 4);
        // A flamethrower's stream floods the cap several times over.
        for n in 100..200u32 {
            store.push(n, EMBER, cap, floors);
            assert!(store.len() <= cap);
        }
        assert_eq!(store.len(), cap, "the cap holds");
        assert_eq!(store[..4], [0, 3, 6, 9], "the hull's smoke is still there, oldest first");
        assert_eq!(store[4..], (184..200).collect::<Vec<_>>()[..], "and the newest of the flood, in order");
        assert_eq!((store.counted(Budget::Damage), store.counted(EMBER)), (4, 16));

        // More smoke off the hull takes the flood's embers, until there is
        // more of it than its floor.
        for n in 200..203u32 {
            store.push(n, Budget::Kind(ParticleKind::Smoke), cap, floors);
            store.count_newest_as(1, Budget::Damage);
        }
        assert_eq!((store[..5].to_vec(), store.counted(Budget::Damage), store.len()), (vec![0, 3, 6, 9, 187], 7, cap));
        // Over its floor the oldest wisp is the oldest particle and goes
        // as one; back at its floor it is kept again.
        store.push(300, SPARK, cap, floors);
        assert_eq!((store[..4].to_vec(), store.counted(Budget::Damage)), (vec![3, 6, 9, 187], 6));
        store.push(301, SPARK, cap, floors);
        assert_eq!((store[..4].to_vec(), store.counted(Budget::Damage)), (vec![3, 6, 9, 188], 6));

        // With no floor the same flood takes the smoke with everything else.
        let mut plain: Particles<u32> = Particles::default();
        for n in 0..12u32 {
            plain.push(n, Budget::Damage, cap, NONE);
        }
        for n in 100..200u32 {
            plain.push(n, EMBER, cap, NONE);
        }
        assert_eq!(plain.counted(Budget::Damage), 0, "which is what the floor is for");
    }

    /// A store of nothing but damage smoke at its floor still holds its
    /// cap: the oldest goes.
    #[test]
    fn the_cap_holds_when_everything_is_under_its_floor() {
        let floors = Floors { damage: 100 };
        let mut store: Particles<u32> = Particles::default();
        for n in 0..30u32 {
            store.push(n, Budget::Damage, 10, floors);
        }
        assert_eq!(store[..], (20..30).collect::<Vec<_>>()[..]);
        // A cap that came down drops the oldest until it holds.
        store.push(99, SPARK, 4, floors);
        assert_eq!(store[..], [27, 28, 29, 99]);
        store.push(100, SPARK, 0, floors);
        assert_eq!(store.len(), 4, "a cap of 0 takes no particle");
        store.clear();
        assert!(store.is_empty() && store.counted(Budget::Damage) == 0);
    }
}
