//! The world half of the aftermath (docs/aftermath.md; the headless rules
//! and pictures are `crate::smoulder`): gathering what is still hot from
//! the state the round and its replicas already keep, the battle's haze,
//! a wave round's hulk scar, and the two rules that play - smoke that
//! hides a hull and a gust relighting an ember bed - each behind a knob
//! that defaults off. Nothing here draws the round's RNG.

use super::{Frame, Game};
use crate::decal::Decal;
use crate::smoulder::{self, Hot, Kind, Stage};
use crate::tank::Tank;
use crate::tuning::tuning;
use crate::{Position, RUBBLE_ROW_TANK};

impl Game {
    /// `tank`'s wreck as a hot remain, its seed hashed from its slot and
    /// rolled damage variant (`damage_stage`'s), so the round and a
    /// replica draw it the same; `None` for a live tank.
    pub fn wreck_hot(&self, tank: &Tank) -> Option<Hot> {
        if !tank.is_wreck() {
            return None;
        }
        let t = tuning();
        let seed = crate::blast::seed_at(Position::new(tank.owner_slot() as f32 * 32.0, tank.damage_variant as f32 * 32.0), 0x5A0);
        Some(Hot {
            kind: Kind::Wreck,
            at: tank.position,
            age: tank.wreck_age,
            douse: smoulder::douse(self.weather, &t),
            steam: smoulder::steams(self.weather),
            seed,
        })
    }

    /// Every remain on the field still giving off heat, and every wreck
    /// (a cold hulk still rusts and feeds the haze a little): the wrecks in
    /// owner-slot order, then the ember beds and the hot rubble oldest
    /// first, the towers' ruins and the rods' craters. Gathered from what
    /// the round and a replica both keep, so both gather the same list.
    pub fn smouldering(&self) -> Vec<Hot> {
        let t = tuning();
        let douse = smoulder::douse(self.weather, &t);
        let steam = smoulder::steams(self.weather);
        let mut wrecks: Vec<(usize, Hot)> =
            self.world.query::<&Tank>().iter().filter_map(|tank| self.wreck_hot(tank).map(|hot| (tank.owner_slot(), hot))).collect();
        wrecks.sort_by_key(|&(slot, _)| slot);
        let mut out: Vec<Hot> = wrecks.into_iter().map(|(_, hot)| hot).collect();
        let warm = |kind, at: Position, age: f32, salt| {
            let hot = Hot { kind, at, age, douse, steam, seed: crate::blast::seed_at(at, salt) };
            (hot.stage(&t).0 != Stage::Cold).then_some(hot)
        };
        out.extend(self.scorches.iter().filter(|s| s.ember).filter_map(|s| warm(Kind::Bed, s.center, s.age, 0x5A1)));
        out.extend(
            self.decals
                .iter()
                .filter(|d| d.hot && d.landed())
                .filter_map(|d| warm(if d.upright { Kind::Stump } else { Kind::Ruin }, d.center, d.age, 0x5A2)),
        );
        out.extend(self.tower_ruins.iter().filter_map(|r| warm(Kind::Tower, r.position, r.age, 0x5A3)));
        out.extend(
            self.craters
                .list()
                .iter()
                .filter_map(|c| warm(Kind::Crater, crate::map::cell_to_world(c.cell.0, c.cell.1), self.time - c.at, 0x5A4)),
        );
        out
    }

    /// The battle's smoke load, 0 to 1 (`smoulder::step_haze`): what the
    /// haze over the field, its grade and the ash are drawn at.
    pub fn battle_haze(&self) -> f32 {
        self.battle_haze
    }

    /// Feed the battle's haze from every hot remain and let the sky drain
    /// it, `dt` seconds on. Off (`battle_haze`), it holds at none. Runs
    /// with the effects, on the round and on a replica alike.
    pub(super) fn tick_haze(&mut self, dt: f32) {
        let t = tuning();
        if !t.battle_haze {
            self.battle_haze = 0.0;
            return;
        }
        let load: f32 = self.smouldering().iter().map(|hot| hot.load(&t)).sum();
        self.battle_haze = smoulder::step_haze(self.battle_haze, load, dt, self.weather, &t);
    }

    /// The cells thick smoke hides a hull in this tick (`smoke_cover`,
    /// `smoulder::smoke_cells`), for `cover_cells`. The round's own; a
    /// replica takes the room's off the wire. With the rule off the set
    /// stays empty and nothing else is read.
    pub(super) fn tick_smoke_cover(&mut self) {
        if !self.aftermath_rules.smoke_cover {
            self.smoke_cells.clear();
            return;
        }
        self.smoke_cells = smoulder::smoke_cells(&self.smouldering(), &tuning());
    }

    /// `ember_bed_relight`: every `ember_bed_relight_seconds`, each ember
    /// bed still smouldering that a strong gust is over throws an ember
    /// downwind (`throw_ember`), which lights the wood, tree or board it
    /// lands on `ember_catch_chance` of the time. With the rule off, or
    /// embers off, it reads nothing.
    pub(super) fn relight_beds(&mut self, f: &mut Frame) {
        let t = tuning();
        if !self.aftermath_rules.ember_relight || !t.embers_enabled {
            return;
        }
        let every = t.ember_bed_relight_seconds.max(0.2);
        let (was, now) = (((self.time - f.dt) / every).floor(), (self.time / every).floor());
        if now <= was {
            return;
        }
        let douse = smoulder::douse(self.weather, &t);
        let steam = smoulder::steams(self.weather);
        let throws: Vec<(Position, u32)> = self
            .scorches
            .iter()
            .filter(|s| s.ember)
            .map(|s| Hot { kind: Kind::Bed, at: s.center, age: s.age, douse, steam, seed: crate::blast::seed_at(s.center, 0x5A1) })
            .filter(|hot| !hot.steam && hot.stage(&t).0 == Stage::Smouldering)
            .filter(|hot| smoulder::gust_strong(hot.at, self.time, self.weather, &t))
            .map(|hot| (hot.at, hot.seed))
            .collect();
        for (from, seed) in throws {
            self.throw_ember(f, from, seed, now as u32);
        }
    }

    /// What a wave round's wreck leaves when its body is taken off the
    /// field at `center` (`despawn_wrecks`): `wreck_scar_parts` blackened
    /// parts lying where it stood, beside the scorch its kill left. A
    /// replica lays the same off `Event::WreckRemoved`.
    pub(crate) fn wreck_scar(&self, center: Position) -> Vec<Decal> {
        let parts = tuning().wreck_scar_parts.max(0) as u32;
        (0..parts)
            .map(|i| {
                let h = crate::blast::seed_at(center, 0x5C0 + i * 7);
                let angle = (h % 3600) as f32 / 3600.0 * std::f32::consts::TAU;
                let dist = 6.0 + 16.0 * ((h >> 12) % 100) as f32 / 100.0;
                let at = Position::new(center.x + crate::math::cos(angle) * dist, center.y + crate::math::sin(angle) * dist);
                let mut part = Decal::thrown(RUBBLE_ROW_TANK, at, at, 0x5C0 + i * 7);
                part.arc = 0.0;
                part
            })
            .collect()
    }
}
