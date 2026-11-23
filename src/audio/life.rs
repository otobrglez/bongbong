//! The field's creatures heard (docs/field-life.md): crows cawing now and
//! then where they sit, bursting up in a clatter of wings and alarm and
//! giving a last caw as they wheel away; mallards quacking and dabbling on
//! the water, splashing up in alarm, quacking at a hull on the bank and
//! splashing down again; and at night a faint shimmer where the fireflies
//! gather.
//!
//! The creatures are presentation (`fauna.rs`): hashed from the round's
//! seed and stepped on its clock after every step by `Fauna::observe`. The
//! window draws the copy `fx::Fx` keeps; the director keeps its own
//! (`Audio::life`), stepped by the same call at the same moments - after
//! each step (`Audio::observe_events`) and once a frame (`Audio::observe`),
//! as `Fx` does - so it is the very field the window draws, while the sound
//! stays a function of the `Game` it reads: the offline render, the tests
//! and an online replica hear the creatures with nothing handed over from
//! the window.
//!
//! Sparse by rule. A call - a caw, a quack, a dabble - falls at a moment
//! hashed on the round's clock (`calls`), so a replica hears the same; it
//! plays only within earshot of an ear, while the mix is calm (the HDR
//! window's top under `audio_critter_calm_db`), and no sooner than
//! `audio_critter_gap_s` after the last call or alarm anywhere, waiting
//! its turn behind the calls due before it for that long at most. An alarm
//! (a flock flushed, a group put up, a wary quack) and a splash-down play
//! on their edges, and an alarm hushes the calls waiting. Every creature's
//! sound sits `audio_critters_db` off its cue's level, on the world bus,
//! which explosions duck.

use std::collections::BTreeMap;

use crate::crows::State as CrowState;
use crate::ducks::State as DuckState;
use crate::fauna::Fauna;
use crate::math::Vec2;
use crate::simulation::Game;
use crate::tuning::Tuning;

use super::bind::{At, Play};
use super::cues::{CueId, ids};
use super::dsp::db_to_gain;
use super::watch::Wish;

/// The keys of the creatures' sounds: a flock's or a group's cooldown
/// source, a swarm's loop.
const CROW: u64 = 60 << 56;
const DUCK: u64 = 61 << 56;
const FIREFLY: u64 = 62 << 56;

/// The side of a firefly swarm, in map cells: the fireflies whose homes
/// fall in one square of this many cells shimmer as one bed.
const SWARM_CELLS: f32 = 6.0;

/// The longest gap in the round's clock (s) the call moments are looked
/// for across; a longer one (a stall, a jump) is skipped.
const MAX_CALL_GAP: f32 = 1.0;

/// What the creatures' watcher keeps between frames.
#[derive(Default)]
pub struct Critters {
    /// Each crow flock as last seen, by flock.
    flocks: BTreeMap<usize, FlockEar>,
    /// Each duck group as last seen, by group.
    groups: BTreeMap<usize, GroupEar>,
    /// Each duck as last seen, in the ducks' order.
    ducks: Vec<DuckEar>,
    /// The round's clock at the last frame.
    clock: Option<f32>,
    /// When the next call may play, on the director's clock.
    next_call: f64,
    /// The calls due and waiting for the gap, oldest first: one a caller
    /// at most, each kept `audio_critter_gap_s` at most.
    waiting: Vec<(f64, Call)>,
    /// The fireflies gathered into swarms.
    swarms: Swarms,
}

#[derive(Clone, Copy, Debug, Default)]
struct FlockEar {
    /// Its birds in the air.
    flying: usize,
    /// Whether this flight's far caw has been heard.
    wheeled: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct GroupEar {
    /// Running to take off or in the air, any of it.
    up: bool,
    /// The most frightened of its ducks on the water.
    fright: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct DuckEar {
    flying: bool,
    dabbling: bool,
}

/// A flock of crows this frame.
#[derive(Default)]
struct FlockNow {
    members: Vec<usize>,
    /// Its birds startled or in the air, and where they are summed.
    roused: usize,
    roused_at: Vec2,
    flying: usize,
    /// The bird longest in the air: its age and index.
    oldest: Option<(f32, usize)>,
}

/// A duck group this frame.
#[derive(Default)]
struct GroupNow {
    members: Vec<usize>,
    /// Its ducks running on the water (past their stagger) or flying, and
    /// where they are summed.
    up: usize,
    up_at: Vec2,
    /// The most frightened duck on the water: its fright and index.
    wary: Option<(f32, usize)>,
    calm: bool,
}

/// A call due, waiting for the gap.
struct Call {
    cue: CueId,
    at: Vec2,
    pitch: f32,
    source: u64,
}

/// The fireflies of a round gathered into swarms of `SWARM_CELLS` square,
/// gathered again when the fireflies change (a new round).
#[derive(Default)]
struct Swarms {
    /// How many fireflies, and the first one's home, it was gathered from.
    from: Option<(usize, Vec2)>,
    list: Vec<Swarm>,
}

struct Swarm {
    key: u64,
    /// The middle of its fireflies' homes.
    at: Vec2,
    members: Vec<usize>,
    pitch: f32,
}

impl Swarms {
    fn gather(&mut self, flies: &[crate::fireflies::Firefly], salt: u32) {
        let from = flies.first().map(|f| (flies.len(), f.home));
        if self.from == from {
            return;
        }
        self.from = from;
        let side = SWARM_CELLS * crate::OBSTACLE_GRID_SIZE;
        let mut buckets: BTreeMap<(i32, i32), (Vec2, Vec<usize>)> = BTreeMap::new();
        for (i, f) in flies.iter().enumerate() {
            let b = ((f.home.x / side).floor() as i32, (f.home.y / side).floor() as i32);
            let e = buckets.entry(b).or_insert((Vec2::new(0.0, 0.0), Vec::new()));
            e.0 += f.home;
            e.1.push(i);
        }
        self.list = buckets
            .into_iter()
            .map(|((bx, by), (sum, members))| {
                let key = FIREFLY | (((bx as u32 as u64) & 0xFF_FFFF) << 24) | ((by as u32 as u64) & 0xFF_FFFF);
                let pitch = 0.94 + 0.12 * crate::pyro::unit(salt ^ (key as u32) ^ ((key >> 24) as u32).rotate_left(7), 1);
                Swarm { key, at: sum * (1.0 / members.len() as f32), members, pitch }
            })
            .collect();
    }
}

/// The round's salt for the creatures' sounds: its seed folded to 32 bits.
fn salt_of(game: &Game) -> u32 {
    let s = game.round_seed();
    (s ^ (s >> 32)) as u32
}

/// The call of a caller hashed `seed` that falls after round time `from`
/// and by `to`, if one does: the round's clock is cut into windows of
/// `every` seconds (offset by the caller), and each window holds a call at
/// a hashed moment in `share` of them. Returns the window's number.
pub fn calls(seed: u32, every: f32, share: f32, from: f32, to: f32) -> Option<u32> {
    if to <= from || every <= 0.0 || from < 0.0 {
        return None;
    }
    let unit = |k: i64, salt: u32| crate::pyro::unit(seed ^ (k as u32).wrapping_mul(0x27D4_EB2F), salt);
    let phase = crate::pyro::unit(seed, 1) * every;
    let first = ((from + phase) / every).floor() as i64;
    let last = ((to + phase) / every).floor() as i64;
    (first..=last).find(|&k| {
        let at = k as f32 * every - phase + unit(k, 2) * every;
        at > from && at <= to && unit(k, 3) < share
    })
    .map(|k| k as u32)
}

impl Critters {
    pub fn reset(&mut self) {
        *self = Critters::default();
    }

    /// The creatures of `life`, the field the window draws, once a frame.
    /// `ears` are the listeners; `now` the director's clock; `calm` whether
    /// the mix is quiet enough for a call.
    #[allow(clippy::too_many_arguments)]
    pub fn sample(&mut self, life: &Fauna, game: &Game, ears: &[Vec2], now: f64, calm: bool, t: &Tuning, wishes: &mut Vec<Wish>, plays: &mut Vec<Play>) {
        let time = game.time;
        let span = self.clock.replace(time).filter(|&from| time > from && time - from <= MAX_CALL_GAP).map(|from| (from, time));
        let level = db_to_gain(t.audio_critters_db);
        let salt = salt_of(game);
        let reach = t.audio_class_far_px[super::Class::Small.index()];
        let heard = |p: Vec2| ears.iter().any(|e| e.distance_to(p) < reach);
        let mut calls: Vec<Call> = Vec::new();
        let mut alarmed = false;
        self.crows(life, salt, span, level, t, plays, &mut calls, &mut alarmed, &heard);
        self.ducks(life, game, salt, span, level, t, plays, &mut calls, &mut alarmed, &heard);
        self.fireflies(life, game, salt, ears, level, t, wishes);
        let gap = t.audio_critter_gap_s as f64;
        for call in calls {
            if heard(call.at) && !self.waiting.iter().any(|(_, w)| w.source == call.source) {
                self.waiting.push((now, call));
            }
        }
        // A fright or a fight hushes the field: what was waiting is let go.
        if alarmed || !calm {
            self.waiting.clear();
        }
        if alarmed {
            self.next_call = self.next_call.max(now + gap);
        }
        self.waiting.retain(|&(due, _)| now - due <= gap);
        if now >= self.next_call && !self.waiting.is_empty() {
            let (_, call) = self.waiting.remove(0);
            plays.push(Play::new(call.cue.0, At::Point(call.at)).strength(level).pitch(call.pitch).source(call.source));
            self.next_call = now + gap;
        }
    }

    /// The crows: a flock lifting off flushes (wings and alarm), its first
    /// bird a while in the air caws once more as the flock wheels away, and
    /// a flock all down - on the grass or in a crown - has its calls.
    #[allow(clippy::too_many_arguments)]
    fn crows(
        &mut self,
        life: &Fauna,
        salt: u32,
        span: Option<(f32, f32)>,
        level: f32,
        t: &Tuning,
        plays: &mut Vec<Play>,
        calls: &mut Vec<Call>,
        alarmed: &mut bool,
        heard: &dyn Fn(Vec2) -> bool,
    ) {
        let crows = life.crows().crows();
        let mut flocks: BTreeMap<usize, FlockNow> = BTreeMap::new();
        for (i, c) in crows.iter().enumerate() {
            let f = flocks.entry(c.flock()).or_default();
            f.members.push(i);
            match c.state {
                CrowState::Ground | CrowState::Perched(_) => {}
                CrowState::Startled { .. } => {
                    f.roused += 1;
                    f.roused_at += c.pos;
                }
                CrowState::Flying { age } => {
                    f.roused += 1;
                    f.roused_at += c.pos;
                    f.flying += 1;
                    if f.oldest.is_none_or(|(a, _)| age > a) {
                        f.oldest = Some((age, i));
                    }
                }
            }
        }
        self.flocks.retain(|fi, _| flocks.contains_key(fi));
        for (&fi, f) in &flocks {
            let fseed = salt ^ (fi as u32).wrapping_add(1).wrapping_mul(0x9E37_79B9) ^ 0xC20E_CA11;
            let pitch = 0.94 + 0.12 * crate::pyro::unit(fseed, 5);
            let source = CROW | fi as u64;
            let was = self.flocks.get(&fi).copied();
            let mut ear = FlockEar { flying: f.flying, wheeled: was.map_or(f.flying > 0, |w| w.wheeled && f.flying > 0) };
            if was.is_some_and(|w| w.flying == 0) && f.flying > 0 {
                let at = f.roused_at * (1.0 / f.roused.max(1) as f32);
                let strength = libm::sqrtf(f.roused as f32 / 8.0).clamp(0.5, 1.0);
                plays.push(Play::new(ids::CROW_FLUSH.0, At::Point(at)).strength(strength * level).pitch(pitch).source(source));
                *alarmed |= heard(at);
            }
            if let Some((age, i)) = f.oldest
                && !ear.wheeled
                && age >= t.audio_crow_wheel_s
            {
                ear.wheeled = true;
                plays.push(Play::new(ids::CROW_CAW_FAR.0, At::Point(crows[i].pos)).strength(level).pitch(pitch).source(source));
            }
            self.flocks.insert(fi, ear);
            if f.roused == 0
                && let Some((from, to)) = span
                && let Some(k) = calls_of(fseed, t.audio_crow_caw_s, t, from, to)
            {
                let who = f.members[((crate::pyro::unit(fseed ^ k.wrapping_mul(0x2545_F491), 4) * f.members.len() as f32) as usize).min(f.members.len() - 1)];
                calls.push(Call { cue: ids::CROW_CAW, at: crows[who].pos, pitch: pitch * (0.97 + 0.06 * crate::pyro::unit(fseed ^ k, 6)), source });
            }
        }
    }

    /// The mallards: a group putting up splashes off in alarm, a hull on
    /// the bank sets it quacking, each duck splashes down where it lands,
    /// and a calm group on the water has its quacks and its ducks' dips
    /// (`Duck::dabbling`, the dip the window draws). Nothing on ice.
    #[allow(clippy::too_many_arguments)]
    fn ducks(
        &mut self,
        life: &Fauna,
        game: &Game,
        salt: u32,
        span: Option<(f32, f32)>,
        level: f32,
        t: &Tuning,
        plays: &mut Vec<Play>,
        calls: &mut Vec<Call>,
        alarmed: &mut bool,
        heard: &dyn Fn(Vec2) -> bool,
    ) {
        let ducks = life.ducks().ducks();
        if game.water().is_frozen() || ducks.is_empty() {
            self.groups.clear();
            self.ducks.clear();
            return;
        }
        let time = game.time;
        let fresh = self.ducks.len() != ducks.len();
        if fresh {
            self.groups.clear();
            self.ducks = vec![DuckEar::default(); ducks.len()];
        }
        let mut groups: BTreeMap<usize, GroupNow> = BTreeMap::new();
        for (i, d) in ducks.iter().enumerate() {
            let g = groups.entry(d.group()).or_insert_with(|| GroupNow { calm: true, ..GroupNow::default() });
            g.members.push(i);
            match d.state {
                DuckState::Swim => {
                    if g.wary.is_none_or(|(f, _)| d.fright > f) {
                        g.wary = Some((d.fright, i));
                    }
                    g.calm &= d.fright <= 0.0;
                }
                DuckState::Run { age } => {
                    g.calm = false;
                    if age >= 0.0 {
                        g.up += 1;
                        g.up_at += d.pos;
                    }
                }
                DuckState::Fly { .. } => {
                    g.calm = false;
                    g.up += 1;
                    g.up_at += d.pos;
                }
            }
            let now = DuckEar { flying: matches!(d.state, DuckState::Fly { .. }), dabbling: d.dabbling(time) };
            let was = std::mem::replace(&mut self.ducks[i], now);
            if fresh {
                continue;
            }
            let dseed = salt ^ (i as u32).wrapping_add(1).wrapping_mul(0x2545_F491) ^ 0x0DAB_B1E5;
            if was.flying && d.state == DuckState::Swim {
                plays.push(Play::new(ids::DUCK_LAND.0, At::Point(d.pos)).strength(0.8 * level).pitch(0.95 + 0.1 * crate::pyro::unit(dseed, 1)).source(DUCK | d.group() as u64));
            }
            let dip = (time * 10.0).floor() as i64 as u32;
            if now.dabbling && !was.dabbling && crate::pyro::unit(dseed ^ dip.wrapping_mul(0x9E37_79B9), 2) < t.audio_duck_dabble_share {
                calls.push(Call { cue: ids::DUCK_DABBLE, at: d.pos, pitch: 0.9 + 0.2 * crate::pyro::unit(dseed ^ dip, 3), source: DUCK | d.group() as u64 });
            }
        }
        self.groups.retain(|g, _| groups.contains_key(g));
        for (&g, now) in &groups {
            let gseed = salt ^ (g as u32).wrapping_add(1).wrapping_mul(0x85EB_CA6B) ^ 0xD0C5_0A4C;
            let pitch = 0.94 + 0.12 * crate::pyro::unit(gseed, 5);
            let source = DUCK | g as u64;
            let fright = now.wary.map_or(0.0, |(f, _)| f);
            let ear = GroupEar { up: now.up > 0, fright };
            let was = self.groups.insert(g, ear);
            let Some(was) = was.filter(|_| !fresh) else { continue };
            if ear.up && !was.up {
                let at = now.up_at * (1.0 / now.up.max(1) as f32);
                let strength = libm::sqrtf(now.up as f32 / 5.0).clamp(0.5, 1.0);
                plays.push(Play::new(ids::DUCK_TAKEOFF.0, At::Point(at)).strength(strength * level).pitch(pitch).source(source));
                *alarmed |= heard(at);
            } else if let Some((f, i)) = now.wary
                && f > 0.9
                && was.fright < 0.5
            {
                plays.push(Play::new(ids::DUCK_WARY.0, At::Point(ducks[i].pos)).strength(level).pitch(pitch * 1.08).source(source));
                *alarmed |= heard(ducks[i].pos);
            }
            if now.calm
                && let Some((from, to)) = span
                && let Some(k) = calls_of(gseed, t.audio_duck_quack_s, t, from, to)
            {
                let who = now.members[((crate::pyro::unit(gseed ^ k.wrapping_mul(0x2545_F491), 4) * now.members.len() as f32) as usize).min(now.members.len() - 1)];
                calls.push(Call { cue: ids::DUCK_QUACK, at: ducks[who].pos, pitch: pitch * (0.97 + 0.06 * crate::pyro::unit(gseed ^ k, 6)), source });
            }
        }
    }

    /// The fireflies at night: the nearest `audio_firefly_beds` swarms in
    /// earshot each shimmer as a bed, as strong as the sky shows them
    /// (`fireflies::shown`), as lit as they glow (a blast puts them out) and
    /// as full as `audio_firefly_full` of them make it.
    #[allow(clippy::too_many_arguments)]
    fn fireflies(&mut self, life: &Fauna, game: &Game, salt: u32, ears: &[Vec2], level: f32, t: &Tuning, wishes: &mut Vec<Wish>) {
        let flies = life.fireflies().flies();
        let shown = crate::fireflies::shown(game, t);
        if shown <= 0.0 || flies.is_empty() || ears.is_empty() || t.audio_firefly_beds == 0 {
            return;
        }
        self.swarms.gather(flies, salt);
        let time = game.time;
        let reach = t.audio_class_far_px[super::Class::Small.index()];
        let near = |p: Vec2| ears.iter().map(|e| e.distance_to(p)).fold(f32::INFINITY, f32::min);
        let mut close: Vec<(f32, &Swarm)> = self.swarms.list.iter().map(|s| (near(s.at), s)).filter(|(d, _)| *d < reach).collect();
        close.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.key.cmp(&b.1.key)));
        for (_, swarm) in close.into_iter().take(t.audio_firefly_beds) {
            let lit = swarm.members.iter().map(|&i| flies[i].level(time, t)).sum::<f32>() / swarm.members.len() as f32;
            let full = (swarm.members.len() as f32 / t.audio_firefly_full.max(1.0)).min(1.0);
            let gain = shown * lit * full * level;
            if gain > 0.002 {
                wishes.push(Wish { key: swarm.key, cue: ids::FIREFLY.0, at: Some(swarm.at), velocity: Vec2::new(0.0, 0.0), gain, pitch: swarm.pitch, own: false });
            }
        }
    }
}

/// `calls` with the share every creature's calls are kept at.
fn calls_of(seed: u32, every: f32, t: &Tuning, from: f32, to: f32) -> Option<u32> {
    calls(seed, every, t.audio_critter_call_share, from, to)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::audio::Audio;
    use crate::audio::cues::Manifest;
    use crate::audio::space::View;
    use crate::crows::State as CrowState;
    use crate::simulation::{Event, Input};

    const W: f32 = 1280.0;
    const H: f32 = 720.0;
    const DT: f32 = crate::PHYSICS_FIXED_DT;

    /// A meadow with a pond, a tree and reeds by it, the seat on the
    /// grass west of it: crows on the open grass, a mallard group on the
    /// pond, fireflies round the tree and the reeds. `sky` is the map's
    /// top-level weather line, if any.
    fn pond(sky: &str) -> Game {
        let mut map = format!("{sky}version = 1\ntanks = 0\nsize = [24, 14]\ncells.\"2,2\" = {{ kind = \"bong\" }}\ncells.\"6,6\" = {{ kind = \"start\" }}\n");
        for r in 4..=9 {
            for c in 12..=17 {
                map.push_str(&format!("cells.\"{c},{r}\" = {{ kind = \"water\" }}\n"));
            }
        }
        map.push_str("cells.\"10,3\" = { kind = \"tree\" }\ncells.\"11,10\" = { kind = \"reeds\" }\ncells.\"18,10\" = { kind = \"tall_grass\" }\n");
        let mut game = Game::default();
        game.enemy_count_override = Some(0);
        game.seed_override = Some(7);
        game.player_row_override = Some(1);
        game.map = crate::map::MapFile::from_toml_str(&map).expect("the pond parses");
        game.init(W, H);
        game
    }

    fn audio() -> Audio {
        Audio::new(Arc::new(Manifest::shipped()), true, |_| None)
    }

    fn views(game: &Game) -> Vec<View> {
        let seat = crate::audio::bind::tank_in(game, 0).map(|t| t.position);
        vec![View { center: seat.unwrap_or(Vec2::new(400.0, 300.0)), half_width: 544.0, seat }]
    }

    /// One step and its frame, with `extra` events added to the step's (a
    /// blast where the test wants one); the cues the frame started.
    fn step_with(game: &mut Game, a: &mut Audio, extra: &[Event]) -> Vec<String> {
        game.update(Input::default(), DT, W, H);
        game.events.extend_from_slice(extra);
        a.observe_events(game);
        a.observe(game, &views(game), &[0], DT, true);
        a.take_commands();
        a.recent.drain(..).collect()
    }

    fn step(game: &mut Game, a: &mut Audio) -> Vec<String> {
        step_with(game, a, &[])
    }

    /// `seconds` of the round: each cue started, with the step it started
    /// on.
    fn run(game: &mut Game, a: &mut Audio, seconds: f32) -> Vec<(usize, String)> {
        let mut log = Vec::new();
        for i in 0..(seconds / DT).round() as usize {
            log.extend(step(game, a).into_iter().map(|c| (i, c)));
        }
        log
    }

    fn critter(cue: &str) -> bool {
        cue.starts_with("world.crow") || cue.starts_with("world.duck") || cue.starts_with("world.firefly")
    }

    fn of<'a>(log: &'a [(usize, String)], cue: &str) -> Vec<usize> {
        log.iter().filter(|(_, c)| c == cue).map(|&(i, _)| i).collect()
    }

    fn seat(game: &Game) -> Vec2 {
        crate::audio::bind::tank_in(game, 0).map(|t| t.position).expect("the seat")
    }

    /// The director's creatures are the field the window draws: stepped by
    /// the same `Fauna::observe` on the same steps, they stand where the
    /// window's do, a hull's flush and all.
    #[test]
    fn the_director_steps_the_field_the_window_draws() {
        let mut game = pond("");
        let mut a = audio();
        let mut window = Fauna::default();
        window.observe(&game);
        a.observe_events(&game);
        for i in 0..900 {
            if i == 300 {
                let bird = window.crows().crows().iter().find(|c| c.state == CrowState::Ground).map(|c| c.pos).expect("a crow on the grass");
                game.debug_teleport(0, bird + Vec2::new(24.0, 0.0), None).expect("beside the flock");
            }
            step(&mut game, &mut a);
            window.observe(&game);
        }
        assert!(!window.crows().crows().is_empty() && !window.ducks().ducks().is_empty() && !window.fireflies().flies().is_empty());
        assert_eq!(a.life.crows(), window.crows());
        assert_eq!(a.life.ducks(), window.ducks());
        assert_eq!(a.life.fireflies(), window.fireflies());
    }

    /// A field at rest calls now and then - the crows caw, the mallards
    /// quack and dabble - never two calls closer than the creatures' gap,
    /// and the same seed hears the same calls.
    #[test]
    fn a_quiet_field_calls_now_and_then_and_never_crowds() {
        let t = Tuning::DEFAULT;
        let listen = || {
            let mut game = pond("");
            let mut a = audio();
            run(&mut game, &mut a, 90.0).into_iter().filter(|(_, c)| critter(c)).collect::<Vec<_>>()
        };
        let log = listen();
        let caws = of(&log, "world.crow.caw");
        let quacks = of(&log, "world.duck.quack");
        assert!(caws.len() >= 2, "the crows caw: {log:?}");
        assert!(quacks.len() + of(&log, "world.duck.dabble").len() >= 2, "the ducks call: {log:?}");
        let mut calls: Vec<usize> = log.iter().filter(|(_, c)| matches!(c.as_str(), "world.crow.caw" | "world.duck.quack" | "world.duck.dabble")).map(|&(i, _)| i).collect();
        calls.sort_unstable();
        let gap = (t.audio_critter_gap_s / DT) as usize;
        for pair in calls.windows(2) {
            assert!(pair[1] - pair[0] >= gap, "two calls {} steps apart: {log:?}", pair[1] - pair[0]);
        }
        assert!(calls.len() as f32 <= 90.0 / t.audio_critter_gap_s + 1.0);
        assert_eq!(listen(), log, "hashed, never rolled");
    }

    /// A fight hushes the field: while the seat's cannon keeps the mix loud
    /// no creature calls, and once it stops they call again.
    #[test]
    fn a_fight_hushes_the_calls() {
        let mut game = pond("");
        let mut a = audio();
        run(&mut game, &mut a, 2.0);
        let fire = Input::single(crate::ai::Intent { fire: true, ..crate::ai::Intent::default() });
        let mut log = Vec::new();
        for i in 0..(40.0 / DT) as usize {
            game.update(if i % 30 == 0 { fire } else { Input::default() }, DT, W, H);
            a.observe_events(&game);
            a.observe(&game, &views(&game), &[0], DT, true);
            a.take_commands();
            log.extend(a.recent.drain(..).map(|c| (i, c)));
        }
        let call = |c: &str| matches!(c, "world.crow.caw" | "world.duck.quack" | "world.duck.dabble");
        assert!(log.iter().any(|(_, c)| c.starts_with("weapon.")), "the cannon fires: {log:?}");
        assert!(log.iter().all(|(i, c)| !call(c) || *i < 30), "no call in the fight: {log:?}");
        let after = run(&mut game, &mut a, 60.0);
        assert!(after.iter().any(|(_, c)| call(c)), "the field calls again: {after:?}");
    }

    /// A hull among a flock flushes it: wings and alarm once, a last caw as
    /// it wheels away, and the field holds its calls a while after.
    #[test]
    fn a_flushed_flock_clatters_up_and_caws_as_it_wheels_away() {
        let t = Tuning::DEFAULT;
        let mut game = pond("");
        let mut a = audio();
        run(&mut game, &mut a, 2.0);
        let here = seat(&game);
        let bird = a
            .life
            .crows()
            .crows()
            .iter()
            .filter(|c| c.state == CrowState::Ground)
            .map(|c| c.pos)
            .max_by(|a, b| a.distance_to(here).total_cmp(&b.distance_to(here)))
            .expect("a flock on the grass");
        game.debug_teleport(0, bird + Vec2::new(20.0, 0.0), None).expect("among the flock");
        let log = run(&mut game, &mut a, 6.0);
        let flush = of(&log, "world.crow.flush");
        assert!(!flush.is_empty(), "{log:?}");
        let far = of(&log, "world.crow.caw.far");
        assert!(!far.is_empty(), "a caw as it wheels away: {log:?}");
        assert!(far[0] > flush[0] + ((t.audio_crow_wheel_s - 0.1) / DT) as usize, "{flush:?} then {far:?}");
        let hush = (t.audio_critter_gap_s / DT) as usize;
        assert!(log.iter().all(|(i, c)| !(c == "world.crow.caw" && *i > flush[0] && *i < flush[0] + hush)), "no call straight after the fright: {log:?}");
    }

    /// A blast beside the mallards puts the group up - one splashing
    /// take-off - and each splashes down again; a hull on the bank sets
    /// them quacking; under ice there are none to hear.
    #[test]
    fn mallards_splash_up_in_alarm_splash_down_and_quack_at_a_hull() {
        let mut game = pond("");
        let mut a = audio();
        run(&mut game, &mut a, 1.0);
        let duck = a.life.ducks().ducks()[0].pos;
        let blast = Event::Blast { x: duck.x + 16.0, y: duck.y, chained: false, drum: crate::obstacle::Drum::Oil };
        let mut log: Vec<(usize, String)> = step_with(&mut game, &mut a, &[blast]).into_iter().map(|c| (0, c)).collect();
        log.extend(run(&mut game, &mut a, 25.0).into_iter().map(|(i, c)| (i + 1, c)));
        let up = of(&log, "world.duck.takeoff");
        assert_eq!(up.len(), 1, "one take-off for the group: {log:?}");
        let down = of(&log, "world.duck.land");
        assert!(!down.is_empty() && down[0] > up[0], "and splash-downs after: {log:?}");
        assert!(a.life.ducks().ducks().iter().all(|d| d.state == crate::ducks::State::Swim), "all down again");
        // A hull rolling up to the bank nearest them: the pond's west or
        // east shore, once a duck swims within shying reach of it.
        let shy = Tuning::DEFAULT.duck_shy_px - 8.0;
        let mut wary = Vec::new();
        for _ in 0..20 {
            let banks = a.life.ducks().ducks().iter().flat_map(|d| {
                let y = d.pos.y.clamp(4.5 * 32.0, 9.5 * 32.0);
                [Vec2::new(11.5 * 32.0, y), Vec2::new(18.5 * 32.0, y)].map(|b| (b.distance_to(d.pos), b))
            });
            if let Some((_, bank)) = banks.filter(|(d, _)| *d < shy).min_by(|x, y| x.0.total_cmp(&y.0)) {
                game.debug_teleport(0, bank, None).expect("onto the bank");
                wary = of(&run(&mut game, &mut a, 1.0), "world.duck.quack.wary");
                break;
            }
            run(&mut game, &mut a, 1.0);
        }
        assert!(!wary.is_empty(), "a hull on the bank sets them quacking");
        let mut ice = pond("weather = \"snow\"\n");
        let mut b = audio();
        assert!(run(&mut ice, &mut b, 20.0).iter().all(|(_, c)| !c.starts_with("world.duck")));
        assert!(b.life.ducks().ducks().is_empty(), "none on ice");
    }

    /// The fireflies shimmer at night where they gather, never by day; a
    /// blast over them puts the shimmer out with their glow.
    #[test]
    fn fireflies_shimmer_only_at_night_and_go_out_with_their_glow() {
        let playing = |a: &Audio| a.status().sounds.iter().any(|s| s.cue == "world.firefly");
        let mut day = pond("");
        let mut a = audio();
        run(&mut day, &mut a, 2.0);
        assert!(!playing(&a), "nothing by day");
        let mut night = pond("weather = \"night\"\n");
        let mut a = audio();
        run(&mut night, &mut a, 2.0);
        assert!(!a.life.fireflies().flies().is_empty());
        assert!(playing(&a), "a shimmer at night: {:?}", a.status().sounds);
        let blasts: Vec<Event> = a
            .life
            .fireflies()
            .flies()
            .iter()
            .map(|f| Event::Blast { x: f.home.x, y: f.home.y, chained: false, drum: crate::obstacle::Drum::Oil })
            .collect();
        step_with(&mut night, &mut a, &blasts);
        run(&mut night, &mut a, 0.5);
        assert!(!playing(&a), "out with their glow: {:?}", a.status().sounds);
        run(&mut night, &mut a, Tuning::DEFAULT.firefly_relight_seconds + 2.0);
        assert!(playing(&a), "and back as they light again");
    }

    /// The calls fall on the round's clock, not the frames': a minute cut
    /// into sixtieths or into twenty-fourths holds the same calls, at most
    /// one a window, in about the share asked for.
    #[test]
    fn calls_fall_on_the_round_clock_whatever_the_frame_rate() {
        let hear = |dt: f32| {
            let mut out = Vec::new();
            for i in 0..(600.0 / dt) as usize {
                if let Some(k) = calls(0xC0FFEE, 9.0, 0.6, i as f32 * dt, (i + 1) as f32 * dt) {
                    out.push(k);
                }
            }
            out
        };
        let fine = hear(1.0 / 60.0);
        assert_eq!(fine, hear(1.0 / 24.0));
        let mut windows = fine.clone();
        windows.dedup();
        assert_eq!(windows, fine, "one call a window");
        let share = fine.len() as f32 / (600.0 / 9.0);
        assert!((0.4..=0.8).contains(&share), "{share}");
        assert_eq!(calls(1, 9.0, 1.0, 5.0, 5.0), None);
        assert_eq!(calls(1, 9.0, 0.0, 0.0, 600.0), None);
    }
}
