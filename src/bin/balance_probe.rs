//! Seeded combat experiments using the same simulation and inputs as play.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::PathBuf;

use bongbong::ai::Intent;
use bongbong::bong::Side;
use bongbong::level::Mission;
use bongbong::map::{MapFile, cell_to_world};
use bongbong::math::Vec2;
use bongbong::pickup::PickupKind;
use bongbong::simulation::debug::TankPatch;
use bongbong::simulation::{Event, Game, HitCause, HitTarget, Input, Outcome, TankSnapshot};
use bongbong::tank::{Dir, TankKind};
use bongbong::tuning::{self, tuning};
use bongbong::{PHYSICS_FIXED_DT, Position};
use clap::{Parser, ValueEnum};
use serde::Serialize;
use serde_json::Value;

const DUEL_MAP: &str = r#"
version = 1
view = "whole"
size = [24, 16]
tanks = 0
tank = "scout"
weather = "clear"

[cells]
"3,7" = { kind = "start" }
"#;
const SHELL_PRESS: u32 = 12;
const GRENADE_SHIELD_FRAME: u32 = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
enum Case {
    ShellOpen,
    ShellShield,
    ShellPeekShield,
    EmpOpen,
    EmpShield,
    GrenadeOpen,
    GrenadeShield,
    GrenadeCover,
    ProtectGuard,
    ProtectCrate,
}

impl Case {
    fn is_protect(self) -> bool {
        matches!(self, Self::ProtectGuard | Self::ProtectCrate)
    }

    fn is_grenade(self) -> bool {
        matches!(
            self,
            Self::GrenadeOpen | Self::GrenadeShield | Self::GrenadeCover
        )
    }

    fn is_emp(self) -> bool {
        matches!(self, Self::EmpOpen | Self::EmpShield)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
enum ProtectMap {
    #[default]
    Practice,
    Crossfire,
}

impl ProtectMap {
    fn source(self) -> &'static str {
        match self {
            Self::Practice => include_str!("../../maps/balance/protect.toml"),
            Self::Crossfire => include_str!("../../maps/balance/crossfire.toml"),
        }
    }
}

#[derive(Parser)]
#[command(about = "Run one seeded BongBong balance experiment and emit one JSON record per round")]
struct Args {
    #[arg(
        long,
        value_enum,
        required_unless_present = "schema",
        conflicts_with = "schema"
    )]
    case: Option<Case>,
    #[arg(long, default_value = "42000", value_parser = bongbong::parse_seed)]
    seed: u64,
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
    rounds: u32,
    /// Overlay this tuning JSON object before initializing the round.
    #[arg(long)]
    tuning: Option<PathBuf>,
    /// Which built-in map Protect cases run on.
    #[arg(long, value_enum, default_value_t = ProtectMap::Practice)]
    protect_map: ProtectMap,
    /// Print the tuning schema for experiment runners.
    #[arg(long)]
    schema: bool,
}

#[derive(Default, Serialize)]
struct Hits {
    count: u32,
    reported_damage: f32,
}

#[derive(Default, Serialize)]
struct Metrics {
    seat_pickups: BTreeMap<&'static str, u32>,
    enemy_pickups: BTreeMap<&'static str, u32>,
    seat_trigger_pulls: BTreeMap<&'static str, u32>,
    enemy_trigger_pulls: BTreeMap<&'static str, u32>,
    /// Event totals, grouped by the affected side/target and cause. These
    /// events omit continuous damage ticks and do not credit the shooter.
    hit_events: BTreeMap<&'static str, Vec<(HitCause, Hits)>>,
    seat_shield_deflections: u32,
    enemy_shield_deflections: u32,
    seat_shield_breaks: u32,
    first_target_shield_break_frame: Option<u32>,
    first_target_hull_damage_frame: Option<u32>,
    target_wreck_frame: Option<u32>,
    target_disabled_frame: Option<u32>,
    target_shield_hp_before_grenade_blast_tick: Option<f32>,
    grenade_blast_frame: Option<u32>,
    seat_peak_damage: f32,
    bong_healed: f32,
    first_bong_damage_frame: Option<u32>,
}

impl Metrics {
    fn observe(&mut self, events: &[Event], frame: u32, target: Option<usize>) {
        for event in events {
            match *event {
                Event::PickupCollected { slot, kind, .. } => {
                    let totals = if slot == 0 {
                        &mut self.seat_pickups
                    } else {
                        &mut self.enemy_pickups
                    };
                    *totals.entry(kind.name()).or_default() += 1;
                }
                Event::Fired { slot, weapon } => {
                    let totals = if slot == 0 {
                        &mut self.seat_trigger_pulls
                    } else {
                        &mut self.enemy_trigger_pulls
                    };
                    *totals.entry(weapon).or_default() += 1;
                }
                Event::Hit {
                    target,
                    damage,
                    cause,
                    ..
                } => {
                    let key = match target {
                        HitTarget::Player { .. } => "seat",
                        HitTarget::Enemy { .. } => "enemy",
                        HitTarget::Bong { side: Side::Player } => "player_bong",
                        HitTarget::Bong { side: Side::Enemy } => "enemy_bong",
                        HitTarget::Obstacle { .. } | HitTarget::Wall => "cover",
                    };
                    let causes = self.hit_events.entry(key).or_default();
                    let index = causes
                        .iter()
                        .position(|(seen, _)| *seen == cause)
                        .unwrap_or_else(|| {
                            causes.push((cause, Hits::default()));
                            causes.len() - 1
                        });
                    let hit = &mut causes[index].1;
                    hit.count += 1;
                    hit.reported_damage += damage;
                }
                Event::ShieldBroken { slot: 0, .. } => self.seat_shield_breaks += 1,
                Event::ShieldBroken { slot, .. } if Some(slot) == target => {
                    self.first_target_shield_break_frame.get_or_insert(frame);
                }
                Event::Deflected { slot: 0, .. } => self.seat_shield_deflections += 1,
                Event::Deflected { .. } => self.enemy_shield_deflections += 1,
                Event::Disabled { slot, .. } if Some(slot) == target => {
                    self.target_disabled_frame.get_or_insert(frame);
                }
                Event::GrenadeBlast { slot: 0, .. } => {
                    self.grenade_blast_frame.get_or_insert(frame);
                }
                Event::BongHealed {
                    side: Side::Player,
                    amount,
                    ..
                } => self.bong_healed += amount,
                _ => {}
            }
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum EndReason {
    TargetWrecked,
    MissionEnded,
    TimeLimit,
}

#[derive(Serialize)]
struct Round {
    v: u32,
    case: Case,
    protect_map: Option<ProtectMap>,
    seed: String,
    frames: u32,
    frame_limit: u32,
    outcome: Outcome,
    end_reason: EndReason,
    seat_end_damage: f32,
    target_end_damage: Option<f32>,
    target_end_shield_hp: Option<f32>,
    bong_start_health: Option<f32>,
    bong_end_health: Option<f32>,
    bong_damage: Option<f32>,
    wrecks: u32,
    metrics: Metrics,
}

fn prepare(case: Case, map: ProtectMap, seed: u64) -> Result<(Game, Option<usize>), String> {
    let text = if case.is_protect() {
        map.source().to_owned()
    } else if case == Case::GrenadeCover {
        format!("{DUEL_MAP}\n\"10,7\" = {{ kind = \"wall\", material = \"iron\" }}\n")
    } else {
        DUEL_MAP.to_owned()
    };
    let mut game = Game::default();
    game.map = MapFile::from_toml_str(&text)?;
    game.level_overrides.mission = Some(if case.is_protect() {
        Mission::Protect
    } else {
        Mission::Destroy
    });
    game.seed_override = Some(seed);
    game.weather_from_map = true;
    game.show_intro = false;
    let (width, height) = game.map.field_size();
    game.init(width, height);
    if case.is_protect() {
        return Ok((game, None));
    }

    game.debug_teleport(0, cell_to_world(3, 7), Some(Dir::Right.rotation()))?;
    game.debug_set_shield(0, false)?;
    let target_cell = if case.is_grenade() { 11 } else { 6 };
    let target = game.debug_spawn_enemy(
        cell_to_world(target_cell, 7),
        Some(TankKind::Scout.row()),
        None,
    )?;
    game.debug_park_enemy(target)?;
    game.debug_set_shield(
        target,
        matches!(
            case,
            Case::ShellShield | Case::ShellPeekShield | Case::EmpShield
        ),
    )?;
    let patch = TankPatch {
        emp_charges: case.is_emp().then_some(1),
        grenade_ammo: case.is_grenade().then_some(1),
        ..TankPatch::default()
    };
    game.debug_set_tank(0, &patch)?;
    Ok((game, Some(target)))
}

fn protect_intent(
    case: Case,
    tanks: &[TankSnapshot],
    bong: Position,
    metrics: &Metrics,
    frame: u32,
) -> Intent {
    let seat = &tanks[0];
    let at = seat.position;
    if case == Case::ProtectCrate && frame <= 150 {
        for (kind, col) in [(PickupKind::Minigun, 10), (PickupKind::Shield, 8)] {
            if !metrics.seat_pickups.contains_key(kind.name()) {
                let pickup = cell_to_world(col, 10);
                return Intent {
                    move_dir: Some(Dir::toward(at, pickup)),
                    ..Intent::default()
                };
            }
        }
    }

    let enemy = tanks
        .iter()
        .filter(|t| !t.is_player && !t.is_wreck && !t.entering)
        .min_by(|a, b| {
            a.position
                .distance_to(bong)
                .total_cmp(&b.position.distance_to(bong))
        });
    let Some(enemy) = enemy else {
        return Intent::default();
    };
    if at.distance_to(bong) > 112.0 {
        return Intent {
            move_dir: Some(Dir::toward(at, bong)),
            ..Intent::default()
        };
    }

    let to_enemy = enemy.position - at;
    let face = Dir::toward(at, enemy.position);
    let direction = face.vec();
    let cross = to_enemy - direction * to_enemy.dot(direction);
    if cross.length() > 12.0 {
        return Intent {
            move_dir: Some(Dir::toward(Vec2::zero(), cross)),
            ..Intent::default()
        };
    }
    // A shot through the bong can hurt it, including when fired by the seat.
    let to_bong = bong - at;
    let along = to_bong.dot(direction);
    let cross_bong = to_bong - direction * along;
    let clear_shot = along <= 0.0 || along >= to_enemy.dot(direction) || cross_bong.length() > 24.0;
    Intent {
        face: Some(face),
        fire: clear_shot && frame.is_multiple_of(SHELL_PRESS),
        ..Intent::default()
    }
}

fn run_round(case: Case, map: ProtectMap, seed: u64) -> Result<Round, String> {
    let (mut game, target) = prepare(case, map, seed)?;
    let (width, height) = game.map.field_size();
    let frame_limit = if case.is_emp() {
        120
    } else if case.is_grenade() {
        (tuning().grenade_fuse_seconds / PHYSICS_FIXED_DT).ceil() as u32 + 60
    } else if case.is_protect() {
        1800
    } else {
        600
    };
    let bong_start_health = game.debug_bong_health();
    let mut bong_end_health = bong_start_health;
    let mut metrics = Metrics::default();
    let mut tanks = game.tank_snapshots();
    let mut frames = 0;
    let mut end_reason = EndReason::TimeLimit;
    for frame in 1..=frame_limit {
        let mut before_blast_shield = tanks
            .iter()
            .find(|t| Some(t.slot) == target)
            .map(|t| t.shield_hp);
        let intent = if let Some(target) = target {
            if case == Case::GrenadeShield && frame == GRENADE_SHIELD_FRAME {
                game.debug_set_shield(target, true)?;
                before_blast_shield = Some(tuning().shield_capacity);
            }
            let fire = if case.is_emp() || case.is_grenade() {
                frame == 1
            } else {
                let interval = if case == Case::ShellPeekShield {
                    150
                } else {
                    SHELL_PRESS
                };
                (frame - 1).is_multiple_of(interval)
            };
            Intent {
                face: Some(Dir::Right),
                fire,
                ..Intent::default()
            }
        } else {
            protect_intent(
                case,
                &tanks,
                game.bong_position().expect("live Protect bong"),
                &metrics,
                frame,
            )
        };
        game.update(Input::single(intent), PHYSICS_FIXED_DT, width, height);
        frames = frame;
        tanks = game.tank_snapshots();
        let healed_before = metrics.bong_healed;
        metrics.observe(game.events(), frame, target);
        metrics.seat_peak_damage = metrics.seat_peak_damage.max(tanks[0].damage);
        if metrics.grenade_blast_frame == Some(frame) {
            metrics.target_shield_hp_before_grenade_blast_tick = before_blast_shield;
        }
        if let Some(tank) = tanks.iter().find(|t| Some(t.slot) == target) {
            if tank.damage > 0.0 {
                metrics.first_target_hull_damage_frame.get_or_insert(frame);
            }
            if tank.is_wreck {
                metrics.target_wreck_frame.get_or_insert(frame);
            }
        }
        if let (Some(before), Some(after)) = (bong_end_health, game.debug_bong_health()) {
            if before + (metrics.bong_healed - healed_before) - after > 0.001 {
                metrics.first_bong_damage_frame.get_or_insert(frame);
            }
            bong_end_health = Some(after);
        }
        if metrics.target_wreck_frame.is_some() {
            end_reason = EndReason::TargetWrecked;
            break;
        }
        if game.outcome() != Outcome::Playing {
            end_reason = EndReason::MissionEnded;
            break;
        }
    }
    let target = tanks.iter().find(|t| Some(t.slot) == target);
    Ok(Round {
        v: 1,
        case,
        protect_map: case.is_protect().then_some(map),
        seed: format!("0x{seed:016x}"),
        frames,
        frame_limit,
        outcome: game.outcome(),
        end_reason,
        seat_end_damage: tanks[0].damage,
        target_end_damage: target.map(|t| t.damage),
        target_end_shield_hp: target.map(|t| t.shield_hp),
        bong_start_health,
        bong_end_health,
        bong_damage: bong_start_health
            .zip(bong_end_health)
            .map(|(start, end)| (start + metrics.bong_healed - end).max(0.0)),
        wrecks: game.round_stats().destroyed,
        metrics,
    })
}

#[derive(Serialize)]
struct Record<'a> {
    #[serde(flatten)]
    round: Round,
    tuning: &'a Value,
}

fn main() -> Result<(), String> {
    let args = Args::parse();
    if args.schema {
        println!("{}", tuning::schema_json());
        return Ok(());
    }
    let case = args.case.expect("clap requires --case or --schema");
    args.seed
        .checked_add(u64::from(args.rounds - 1))
        .ok_or("seed range exceeds u64")?;
    if let Some(path) = args.tuning {
        tuning::submit_file(&path)?;
        tuning::apply_pending();
    }
    let patch: Value =
        serde_json::from_str(&tuning::diff_json()).map_err(|error| error.to_string())?;
    let mut stdout = io::stdout().lock();
    for offset in 0..args.rounds {
        let round = run_round(case, args.protect_map, args.seed + u64::from(offset))?;
        serde_json::to_writer(
            &mut stdout,
            &Record {
                round,
                tuning: &patch,
            },
        )
        .map_err(|error| error.to_string())?;
        writeln!(stdout).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controlled_cases_exercise_the_weapon_and_its_counter() {
        let round = |case| run_round(case, ProtectMap::Practice, 42000).unwrap();
        let shell_open = round(Case::ShellOpen);
        let shell_shield = round(Case::ShellShield);
        assert_eq!(shell_open.metrics.enemy_shield_deflections, 0);
        assert!(shell_shield.metrics.enemy_shield_deflections > 0);
        assert!(shell_shield.seat_end_damage > shell_open.seat_end_damage);
        assert!(
            shell_shield.metrics.target_wreck_frame.unwrap()
                > shell_open.metrics.target_wreck_frame.unwrap()
        );

        let emp_open = round(Case::EmpOpen);
        let emp_shield = round(Case::EmpShield);
        assert!(emp_open.metrics.target_disabled_frame.is_some());
        assert!(emp_open.metrics.first_target_shield_break_frame.is_none());
        assert!(emp_shield.metrics.target_disabled_frame.is_some());
        assert_eq!(
            emp_shield.metrics.target_disabled_frame,
            emp_shield.metrics.first_target_shield_break_frame
        );

        let grenade_open = round(Case::GrenadeOpen);
        let grenade_shield = round(Case::GrenadeShield);
        let grenade_cover = round(Case::GrenadeCover);
        assert!(grenade_open.target_end_damage.unwrap() > 0.0);
        assert!(
            grenade_shield
                .metrics
                .target_shield_hp_before_grenade_blast_tick
                .unwrap()
                > 0.0
        );
        assert_eq!(grenade_shield.target_end_damage, Some(0.0));
        assert!(
            grenade_shield.target_end_shield_hp
                < grenade_shield
                    .metrics
                    .target_shield_hp_before_grenade_blast_tick
        );
        assert!(grenade_cover.target_end_damage < grenade_open.target_end_damage);
    }

    #[test]
    fn protect_cases_exercise_pickups_and_replay() {
        let guard = run_round(Case::ProtectGuard, ProtectMap::Practice, 42000).unwrap();
        let raid = run_round(Case::ProtectCrate, ProtectMap::Practice, 42000).unwrap();
        assert!(!guard.metrics.seat_pickups.contains_key("minigun"));
        assert!(raid.metrics.seat_pickups.contains_key("shield"));
        assert!(raid.metrics.seat_trigger_pulls.contains_key("minigun"));
        assert!(raid.metrics.bong_healed > 0.0);
        assert!(raid.bong_damage > raid.bong_start_health);

        let replay = run_round(Case::ProtectCrate, ProtectMap::Practice, 42000).unwrap();
        assert_eq!(
            serde_json::to_value(raid).unwrap(),
            serde_json::to_value(replay).unwrap()
        );

        let crossfire = run_round(Case::ProtectCrate, ProtectMap::Crossfire, 84000).unwrap();
        assert!(crossfire.metrics.seat_pickups.contains_key("minigun"));
        assert!(crossfire.metrics.seat_trigger_pulls.contains_key("minigun"));
    }
}
