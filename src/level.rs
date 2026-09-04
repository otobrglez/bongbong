//! What a map asks of a round beyond its terrain (docs/maps-to-levels.md):
//! the **mission** (what ends the round) and the **spawn plan** (how
//! enemies arrive). Both live in the map file as small optional tables,
//! can be overridden per run from the CLI (`main.rs`, `bin/probe.rs`) or
//! the dev server, and are resolved once in `Game::init` with the
//! precedence CLI > map > default. Pure data: nothing here touches the
//! world or the RNG.

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::tuning::tuning;

/// What ends the round.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Mission {
    /// Keep the frog alive until every enemy is a wreck.
    #[default]
    Protect,
    /// Kill the enemy frog before the enemies kill yours.
    Hunt,
    /// No frog at all: wreck every enemy.
    Destroy,
}

impl Mission {
    /// The big white text the round opens with.
    pub fn banner(self) -> &'static str {
        match self {
            Mission::Protect => "PROTECT THE FROG!",
            Mission::Hunt => "HUNT THE FROG!",
            Mission::Destroy => "DESTROY!",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mission::Protect => "protect",
            Mission::Hunt => "hunt",
            Mission::Destroy => "destroy",
        }
    }

    /// Whether the player's frog is on the field.
    pub fn has_player_frog(self) -> bool {
        !matches!(self, Mission::Destroy)
    }

    /// Whether an enemy frog is on the field.
    pub fn has_enemy_frog(self) -> bool {
        matches!(self, Mission::Hunt)
    }
}

/// How enemies arrive.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum SpawnKind {
    /// Every enemy placed in the spawn band before the first frame.
    #[default]
    Band,
    /// Enemies roll in through edge gates, wave after wave.
    Waves,
}

impl SpawnKind {
    pub fn name(self) -> &'static str {
        match self {
            SpawnKind::Band => "band",
            SpawnKind::Waves => "waves",
        }
    }
}

/// Chassis power class for wave composition - see `TANK_TIER_BY_ROW` in
/// `lib.rs` for which rows belong to which tier. Declaration order is the
/// ladder: `index` walks it and `from_index` clamps back onto it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Light,
    Medium,
    Heavy,
    Super,
}

impl Tier {
    pub const ALL: [Tier; 4] = [Tier::Light, Tier::Medium, Tier::Heavy, Tier::Super];

    pub fn index(self) -> usize {
        self as usize
    }

    /// The tier at `i` on the ladder, clamped to the top.
    pub fn from_index(i: usize) -> Tier {
        Tier::ALL[i.min(Tier::ALL.len() - 1)]
    }

    pub fn name(self) -> &'static str {
        match self {
            Tier::Light => "light",
            Tier::Medium => "medium",
            Tier::Heavy => "heavy",
            Tier::Super => "super",
        }
    }

    /// The sprite rows (`Tank::row`) that belong to this tier, in row order.
    pub fn rows(self) -> Vec<i32> {
        crate::TANK_TIER_BY_ROW
            .iter()
            .enumerate()
            .filter(|(_, t)| **t == self)
            .map(|(row, _)| row as i32)
            .collect()
    }
}

/// The map file's `[mission]` table. Every field optional so a map without
/// the table is a Protect map.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MissionConfig {
    pub kind: Mission,
}

/// The map file's `[spawn]` table. Fields left `None` fall back to the
/// `waves` tuning group's defaults when the plan is resolved.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SpawnConfig {
    pub kind: SpawnKind,
    pub waves: Option<u32>,
    pub size: Option<u32>,
    pub growth: Option<u32>,
    pub tier_start: Option<Tier>,
    pub tier_end: Option<Tier>,
}

/// Per-run overrides of the map's level tables: one field per CLI flag /
/// dev-server `restart` parameter. `None` means "the map decides".
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LevelOverrides {
    pub mission: Option<Mission>,
    pub spawn: Option<SpawnKind>,
    pub waves: Option<u32>,
    pub wave_size: Option<u32>,
    pub wave_growth: Option<u32>,
    pub tier_start: Option<Tier>,
    pub tier_end: Option<Tier>,
}

impl LevelOverrides {
    pub fn resolve_mission(&self, map: &MissionConfig) -> Mission {
        self.mission.unwrap_or(map.kind)
    }

    /// The plan a round runs: CLI over map over the `waves` tuning group.
    /// `enemies` is `--enemies`, which only the band plan reads (its count
    /// falls through to the map's `tanks`, then a random roll, in
    /// `Game::init`).
    pub fn resolve_spawn(&self, map: &SpawnConfig, enemies: Option<usize>) -> SpawnPlan {
        match self.spawn.unwrap_or(map.kind) {
            SpawnKind::Band => SpawnPlan::Band { count: enemies },
            SpawnKind::Waves => {
                let t = tuning();
                let tier_start = self.tier_start.or(map.tier_start).unwrap_or(Tier::Light);
                let tier_end = self.tier_end.or(map.tier_end).unwrap_or(Tier::Super);
                SpawnPlan::Waves {
                    waves: self.waves.or(map.waves).unwrap_or(t.wave_count_default as u32).max(1),
                    size: self.wave_size.or(map.size).unwrap_or(t.wave_size_default as u32).max(1),
                    growth: self.wave_growth.or(map.growth).unwrap_or(t.wave_growth_default as u32),
                    tier_start,
                    tier_end,
                }
            }
        }
    }
}

/// The resolved spawn plan a round runs under.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SpawnPlan {
    /// `count` enemies placed in the spawn band at init (`None`: the map's
    /// `tanks`, else a random `enemy_count_min..=enemy_count_max` roll).
    Band { count: Option<usize> },
    /// `waves` waves; wave `i` (0-based) brings `size + i * growth` tanks
    /// from the tier interpolated between `tier_start` and `tier_end`.
    Waves { waves: u32, size: u32, growth: u32, tier_start: Tier, tier_end: Tier },
}

impl Default for SpawnPlan {
    fn default() -> Self {
        SpawnPlan::Band { count: None }
    }
}

impl SpawnPlan {
    pub fn kind(&self) -> SpawnKind {
        match self {
            SpawnPlan::Band { .. } => SpawnKind::Band,
            SpawnPlan::Waves { .. } => SpawnKind::Waves,
        }
    }

    /// How many tanks wave `i` (0-based) brings, before the live cap.
    pub fn wave_size(&self, i: u32) -> u32 {
        match *self {
            SpawnPlan::Band { .. } => 0,
            SpawnPlan::Waves { size, growth, .. } => size + i * growth,
        }
    }

    /// The tier wave `i` (0-based) draws from: linear between the plan's
    /// start and end tiers, rounded to the nearest rung.
    pub fn wave_tier(&self, i: u32) -> Tier {
        match *self {
            SpawnPlan::Band { .. } => Tier::Light,
            SpawnPlan::Waves { waves, tier_start, tier_end, .. } => {
                let span = tier_end.index() as f32 - tier_start.index() as f32;
                let steps = waves.saturating_sub(1).max(1) as f32;
                let t = i.min(waves.saturating_sub(1)) as f32 / steps;
                let idx = tier_start.index() as f32 + span * t;
                Tier::from_index(idx.round().max(0.0) as usize)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_tables_mean_protect_and_band() {
        let overrides = LevelOverrides::default();
        assert_eq!(overrides.resolve_mission(&MissionConfig::default()), Mission::Protect);
        assert_eq!(overrides.resolve_spawn(&SpawnConfig::default(), Some(4)), SpawnPlan::Band { count: Some(4) });
    }

    #[test]
    fn cli_beats_map_beats_default() {
        let map = SpawnConfig { kind: SpawnKind::Waves, waves: Some(3), size: Some(2), ..Default::default() };
        let overrides = LevelOverrides { waves: Some(7), tier_end: Some(Tier::Heavy), ..Default::default() };
        let plan = overrides.resolve_spawn(&map, None);
        assert_eq!(
            plan,
            SpawnPlan::Waves {
                waves: 7,
                size: 2,
                growth: Tuning::DEFAULT.wave_growth_default as u32,
                tier_start: Tier::Light,
                tier_end: Tier::Heavy
            }
        );
        let overrides = LevelOverrides { spawn: Some(SpawnKind::Band), ..Default::default() };
        assert_eq!(overrides.resolve_spawn(&map, None), SpawnPlan::Band { count: None });
    }

    use crate::tuning::Tuning;

    #[test]
    fn wave_tiers_interpolate_between_start_and_end() {
        let plan = SpawnPlan::Waves { waves: 4, size: 3, growth: 1, tier_start: Tier::Light, tier_end: Tier::Super };
        let tiers: Vec<Tier> = (0..4).map(|i| plan.wave_tier(i)).collect();
        assert_eq!(tiers, [Tier::Light, Tier::Medium, Tier::Heavy, Tier::Super]);
        assert_eq!(plan.wave_tier(99), Tier::Super, "past the last wave stays at the end tier");
        assert_eq!((0..4).map(|i| plan.wave_size(i)).collect::<Vec<_>>(), [3, 4, 5, 6]);
        let flat = SpawnPlan::Waves { waves: 1, size: 3, growth: 1, tier_start: Tier::Medium, tier_end: Tier::Super };
        assert_eq!(flat.wave_tier(0), Tier::Medium, "a single wave is the start tier");
    }

    #[test]
    fn every_row_belongs_to_exactly_one_tier() {
        let total: usize = Tier::ALL.iter().map(|t| t.rows().len()).sum();
        assert_eq!(total, crate::TANK_TIER_BY_ROW.len());
        assert!(Tier::ALL.iter().all(|t| !t.rows().is_empty()));
    }
}
