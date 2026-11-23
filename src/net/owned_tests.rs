//! A client-owned hull against its room over a link `delay` ticks each
//! way (docs/online-coop-prd.md §4.14). The client's `Predictor` drives its
//! own hull and puts every tick's pose on the wire; the room takes it the
//! room server's way - `authority::take_pose` before the tick, `moved_since`
//! after it - and sends a snapshot a tick, a refusal's `Placed` ahead of
//! its events; the client obeys a `Placed`, takes the room's world and
//! applies a `Shoved` on arrival, in `net::round`'s order. Every message
//! goes through the codec. A local round on the same drive is the
//! yardstick: an owned hull is to meet the room's faces, props and loose
//! drums as the local one meets them, on any link.

use std::collections::VecDeque;

use crate::PHYSICS_FIXED_DT;
use crate::ai::Intent;
use crate::level::Mission;
use crate::map::MapFile;
use crate::math::Vec2;
use crate::net::authority::{self, PoseOutcome};
use crate::net::codec::{self, Msg};
use crate::net::events::WireEvent;
use crate::net::predict::Predictor;
use crate::net::wire::{IntentMsg, Seat, Snapshot, dequantise_pos, dequantise_seconds, dequantise_velocity, dir_from_index};
use crate::net::{MAX_SEATS, apply, encode};
use crate::obstacle::{Material, Surface};
use crate::simulation::{Event, Game, Input};
use crate::tank::Dir;

/// A one-seat room with no enemy and no banner, the seat at cell (12, 4)
/// facing down, and `cells` on the field.
fn round(cells: &str) -> Game {
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.player_row_override = Some(1);
    game.show_intro = false;
    game.level_overrides.mission = Some(Mission::Destroy);
    game.map = MapFile::from_toml_str(&format!("version = 1\ntanks = 0\ncells.\"12,4\" = {{ kind = \"start\" }}\n{cells}")).expect("the map parses");
    let (width, height) = game.map.field_size();
    game.init(width, height);
    game
}

/// A row of `kind` across the seat's way down, on row 9.
fn row(kind: &str) -> String {
    (8..=16)
        .map(|c| match kind {
            "iron" | "glass" => format!("cells.\"{c},9\" = {{ kind = \"wall\", material = \"{kind}\" }}\n"),
            other => format!("cells.\"{c},9\" = {{ kind = \"{other}\" }}\n"),
        })
        .collect()
}

fn drive(dir: Dir) -> Intent {
    Intent { move_dir: Some(dir), ..Intent::default() }
}

/// A message there and back through the codec.
fn wired(msg: Msg) -> Msg {
    codec::decode(&codec::encode(&msg)).expect("a message decodes")
}

/// The room and the client at the two ends of the link.
struct Link {
    room: Game,
    client: Predictor,
    delay: usize,
    /// Link ticks run so far.
    now: usize,
    up: VecDeque<(usize, IntentMsg)>,
    down: VecDeque<(usize, Snapshot)>,
    /// The poses the room refused, by the link tick it read them.
    refused: Vec<(usize, &'static str)>,
    /// How many `Placed` the room sent.
    placed: usize,
    /// The room's events for seat 0's hull, by the link tick.
    struck: Vec<(usize, Surface, bool)>,
    shoved: usize,
    /// The own hull after each link tick.
    hull: Vec<Vec2>,
}

impl Link {
    /// The link over `room`, the client welcomed into it.
    fn new(room: Game, delay: usize) -> Link {
        let welcome = encode::welcome(&room, 0, vec![Seat { seat: 0, nick: "a".into(), chassis: 1 }], "{}".into(), [0; MAX_SEATS]).expect("the map serialises");
        let Msg::Welcome(welcome) = wired(Msg::Welcome(welcome)) else { panic!("not a welcome") };
        let mut client = Predictor::new(apply::welcome(&welcome).expect("the sandbox builds"), 0, 1);
        client.set_owned(true);
        Link { room, client, delay, now: 0, up: VecDeque::new(), down: VecDeque::new(), refused: Vec::new(), placed: 0, struck: Vec::new(), shoved: 0, hull: Vec::new() }
    }

    /// One tick of the link: the client steps and sends; the room ticks on
    /// whatever has arrived; the client takes whatever has come back.
    fn step(&mut self, intent: Intent) {
        let t = self.now;
        let tick = self.client.step(intent);
        let msg = IntentMsg::new(tick, &intent).with_pose(self.client.pose().expect("the own hull"));
        let Msg::Intent(msg) = wired(Msg::Intent(msg)) else { panic!("not an intent") };
        self.up.push_back((t + self.delay, msg));
        while self.up.front().is_some_and(|&(due, _)| due <= t) {
            let (_, msg) = self.up.pop_front().expect("one is due");
            let mut events = Vec::new();
            let outcome = authority::take_pose(&mut self.room, 0, msg.pose(), 1);
            if let PoseOutcome::Refused(why, answer) = &outcome {
                self.refused.push((t, why));
                events.extend(answer.clone());
            }
            let (width, height) = self.room.map.field_size();
            self.room.update(Input::single(msg.intent()), PHYSICS_FIXED_DT, width, height);
            if let PoseOutcome::Applied(at) = outcome {
                events.extend(authority::moved_since(&mut self.room, 0, at));
            }
            for event in self.room.events() {
                match event {
                    Event::HullStruck { slot: 0, surface, bounced, .. } => self.struck.push((t, *surface, *bounced)),
                    Event::Shoved { seat: 0, .. } => self.shoved += 1,
                    _ => {}
                }
            }
            let mut acked = [0; MAX_SEATS];
            acked[0] = msg.tick;
            let mut snapshot = encode::snapshot(&self.room, acked);
            events.append(&mut snapshot.events);
            snapshot.events = events;
            self.placed += snapshot.events.iter().filter(|e| matches!(e, WireEvent::Placed { seat: 0, .. })).count();
            let Msg::Snapshot(snapshot) = wired(Msg::Snapshot(snapshot)) else { panic!("not a snapshot") };
            self.down.push_back((t + self.delay, snapshot));
        }
        while self.down.front().is_some_and(|&(due, _)| due <= t) {
            let (_, snapshot) = self.down.pop_front().expect("one is due");
            for event in &snapshot.events {
                if let WireEvent::Placed { seat: 0, x, y, dir } = *event {
                    let rotation = dir_from_index(dir).unwrap_or(Dir::Up).rotation();
                    self.client.place_own(Vec2::new(dequantise_pos(x), dequantise_pos(y)), rotation);
                }
            }
            self.client.reconcile(&snapshot, snapshot.acked[0]);
            for event in &snapshot.events {
                if let WireEvent::Shoved { seat: 0, vx, vy, skid } = *event {
                    self.client.shove(Vec2::new(dequantise_velocity(vx), dequantise_velocity(vy)), dequantise_seconds(skid));
                }
            }
        }
        self.hull.push(self.client.pose().expect("the own hull").position);
        self.now += 1;
    }

    fn run(&mut self, intent: impl Fn(usize) -> Intent, ticks: usize) {
        for _ in 0..ticks {
            let t = self.now;
            self.step(intent(t));
        }
    }
}

/// The same drive in a local round: the hull after each tick, the slams.
fn local(cells: &str, intent: Intent, ticks: usize) -> (Game, Vec<Vec2>, Vec<(usize, Surface, bool)>) {
    let mut game = round(cells);
    let (width, height) = game.map.field_size();
    let mut hull = Vec::new();
    let mut struck = Vec::new();
    for t in 0..ticks {
        game.update(Input::single(intent), PHYSICS_FIXED_DT, width, height);
        hull.push(game.seat_pose(0).expect("the seat").position);
        for event in game.events() {
            if let Event::HullStruck { slot: 0, surface, bounced, .. } = event {
                struck.push((t, *surface, *bounced));
            }
        }
    }
    (game, hull, struck)
}

/// How many tiles of `material` stand, and their health in all.
fn standing(game: &Game, material: Material) -> (usize, u32) {
    let tiles = game.drawable_state().tiles;
    let of: Vec<_> = tiles.iter().filter(|t| t.material == material).collect();
    (of.len(), of.iter().map(|t| t.hp as u32).sum())
}

/// The first tick the hull is below `y`.
fn past(hull: &[Vec2], y: f32) -> Option<usize> {
    hull.iter().position(|p| p.y > y)
}

/// The first tick the hull went back up.
fn thrown_back(hull: &[Vec2]) -> Option<usize> {
    hull.windows(2).position(|w| w[1].y < w[0].y - 0.05).map(|i| i + 1)
}

/// The furthest the hull ever went back up in one tick, px.
fn worst_yank(hull: &[Vec2]) -> f32 {
    hull.windows(2).map(|w| w[0].y - w[1].y).fold(0.0, f32::max)
}

/// The link delays this file plays every case over, each way, in ticks:
/// none, a 100 ms and a 200 ms round trip.
const DELAYS: [usize; 3] = [0, 3, 6];

/// The bottom of row 9, which a hull that went through the row is below.
const ROW_PAST_Y: f32 = 300.0;

/// **An owned hull rams through soft props as a local one does.** A
/// sandbag row and a fence collapse in the room under the client's hull,
/// the room reports the slam, and the hull is through no later than the
/// local one plus the round trip the collapse takes to come back.
#[test]
fn an_owned_hull_rams_through_a_sandbag_row_and_a_fence_as_a_local_one_does() {
    for (kind, material) in [("sandbag", Material::Sandbag), ("fence", Material::Fence)] {
        let cells = row(kind);
        let (game, hull, struck) = local(&cells, drive(Dir::Down), 240);
        let local_through = past(&hull, ROW_PAST_Y).expect("the local hull rams through");
        assert!(!struck.is_empty(), "the local slam");
        for delay in DELAYS {
            let mut link = Link::new(round(&cells), delay);
            link.run(|_| drive(Dir::Down), 240);
            assert_eq!(standing(&link.room, material), standing(&game, material), "{kind}, delay {delay}: the room's row is as the local one is left");
            assert!(!link.struck.is_empty(), "{kind}, delay {delay}: the room reports the slam");
            assert_eq!(link.refused, vec![], "{kind}, delay {delay}: no pose refused");
            let through = past(&link.hull, ROW_PAST_Y).unwrap_or_else(|| panic!("{kind}, delay {delay}: the own hull never got through"));
            assert!(through <= local_through + 2 * delay + 1, "{kind}, delay {delay}: through at tick {through}, the local hull at {local_through}");
        }
    }
}

/// **An owned hull is thrown off iron on the tick a local one is.** The
/// client throws its own hull in its sandbox step, so the bounce is
/// there at once on any link; the room reports the slam as bounced and
/// sends no `Shoved` for it, which would throw the hull a second time.
#[test]
fn an_owned_hull_is_thrown_off_iron_on_the_tick_a_local_one_is() {
    let cells = row("iron");
    let (_, hull, struck) = local(&cells, drive(Dir::Down), 90);
    let local_throw = thrown_back(&hull).expect("the local hull is thrown back");
    assert!(struck.iter().any(|&(_, surface, bounced)| surface == Surface::Hard && bounced), "the local slam: {struck:?}");
    for delay in DELAYS {
        let mut link = Link::new(round(&cells), delay);
        link.run(|_| drive(Dir::Down), 90);
        assert_eq!(thrown_back(&link.hull), Some(local_throw), "delay {delay}: thrown back on the local tick");
        let lowest = |h: &[Vec2]| h.iter().map(|p| p.y).fold(f32::MAX, f32::min);
        assert!((lowest(&link.hull[local_throw..]) - lowest(&hull[local_throw..])).abs() < 0.5, "delay {delay}: as far back as the local hull");
        assert!(link.struck.iter().any(|&(_, surface, bounced)| surface == Surface::Hard && bounced), "delay {delay}: the room's slam: {:?}", link.struck);
        assert_eq!(link.shoved, 0, "delay {delay}: the room sends no throw the client has already put on its hull");
        assert_eq!(link.refused, vec![], "delay {delay}: no pose refused");
    }
}

/// **An owned hull cracks a pane** as a local one does: the room's glass
/// loses what the local glass loses, and the slam is reported.
#[test]
fn an_owned_hull_cracks_a_glass_row_as_a_local_one_does() {
    let cells = row("glass");
    let (game, _, struck) = local(&cells, drive(Dir::Down), 120);
    let before = standing(&round(&cells), Material::Glass);
    let after = standing(&game, Material::Glass);
    assert!(after.1 < before.1, "the local hull cracks a pane: {before:?} -> {after:?}");
    assert!(!struck.is_empty(), "the local slam");
    for delay in DELAYS {
        let mut link = Link::new(round(&cells), delay);
        link.run(|_| drive(Dir::Down), 120);
        assert_eq!(standing(&link.room, Material::Glass), after, "delay {delay}: the room's glass is as the local glass is left");
        assert!(link.struck.iter().any(|&(_, surface, _)| surface == Surface::Brittle), "delay {delay}: the room's slam: {:?}", link.struck);
    }
}

/// **The cell a loose drum was shoved off is open ground.** An owned hull
/// pushing a drum ahead of it drives over the drum's home cell with no
/// pose refused, no `Placed` and no yank back, on any link.
#[test]
fn an_owned_hull_pushes_a_drum_over_its_home_cell_unrefused() {
    let cells = "cells.\"12,7\" = { kind = \"barrel\", drum = \"oil\" }\n";
    for delay in DELAYS {
        let mut link = Link::new(round(cells), delay);
        link.run(|_| drive(Dir::Down), 120);
        assert_eq!(link.refused, vec![], "delay {delay}: no pose refused");
        assert_eq!(link.placed, 0, "delay {delay}: no Placed");
        assert!(worst_yank(&link.hull) < 2.0, "delay {delay}: yanked back {} px", worst_yank(&link.hull));
        assert!(past(&link.hull, 8.0 * 32.0).is_some(), "delay {delay}: the hull went on past the drum's home cell");
    }
}

/// **The cell of a wall that has died is open ground.** The seat shoots a
/// pane out of a glass wall from where it stands, then drives through the
/// gap: the room takes every pose over the dead pane's cell, on any link.
#[test]
fn an_owned_hull_drives_through_a_wall_it_shot_down_unrefused() {
    let cells = row("glass");
    for delay in DELAYS {
        let mut link = Link::new(round(&cells), delay);
        // Fire a shell every quarter second standing, then drive down.
        let shoot = |t: usize| Intent { face: Some(Dir::Down), fire: t % 15 < 2, ..Intent::default() };
        link.run(shoot, 300);
        let (left, _) = standing(&link.room, Material::Glass);
        assert!(left < 9, "delay {delay}: the fixture's shells broke a pane: {left} of 9 stand");
        let from = link.now;
        link.run(|_| drive(Dir::Down), 150);
        assert!(past(&link.hull[from..], ROW_PAST_Y).is_some(), "delay {delay}: the hull drove through the gap");
        assert_eq!(link.refused, vec![], "delay {delay}: no pose refused");
        assert_eq!(link.placed, 0, "delay {delay}: no Placed");
        assert!(worst_yank(&link.hull[from..]) < 2.0, "delay {delay}: yanked back {} px", worst_yank(&link.hull[from..]));
    }
}

/// **A run of refusals ends on the one `Placed` the client obeys.** The
/// room's copy of a hull driving down open ground is moved a cell aside
/// once, as a disagreement would move it: the room answers the first
/// refusal with a `Placed`, refuses the poses that were already on their
/// way, and takes the client's again from the one it snapped to - one
/// `Placed`, refusals for no longer than the round trip, on any link.
#[test]
fn a_run_of_refusals_ends_on_the_one_placed_the_client_obeys() {
    for delay in [0, 3, 6, 9] {
        let mut link = Link::new(round(""), delay);
        let intent = |t: usize| drive(if t < 90 { Dir::Down } else { Dir::Right });
        link.run(intent, 40);
        let at = link.room.seat_pose(0).expect("the seat").position;
        link.room.debug_teleport(0, Vec2::new(at.x + 32.0, at.y), None).expect("the seat");
        link.run(intent, 200);
        assert_eq!(link.placed, 1, "delay {delay}: one Placed");
        let last = link.refused.last().map(|&(t, _)| t);
        assert!(last.is_some_and(|t| t <= 40 + 2 * delay + 2), "delay {delay}: refusals went on to tick {last:?}: {:?}", link.refused);
        let jumps = link.hull[39..].windows(2).filter(|w| w[1].distance_to(w[0]) > 8.0).count();
        assert_eq!(jumps, 1, "delay {delay}: the hull jumped once, to the Placed");
    }
}

/// An owned hull's shell leaves where the room's copy of it leaves, even
/// with the hull drifting across its line of fire - turned up on the press
/// with its speed still carrying it right, which a shot grazing a tile's
/// edge meets or misses by: the room fires before its tick's step, from
/// where the pose puts its copy, and the provisional is drawn from there
/// (`Predictor::launch_shift`).
#[test]
fn an_owned_hulls_shot_leaves_where_the_rooms_copy_does_while_it_drifts() {
    let mut link = Link::new(round(""), 0);
    link.run(|_| drive(Dir::Right), 40);
    link.step(Intent { move_dir: Some(Dir::Up), fire: true, ..Intent::default() });
    let drift = link.hull[link.hull.len() - 1].x - link.hull[link.hull.len() - 2].x;
    assert!(drift > 1.0, "the hull still drifts right on the press: {drift}");
    let (_, drawn) = link.client.shots().next().expect("the provisional");
    let room = link.room.world.query::<&crate::shell::Shell>().iter().map(|s| s.position).next().expect("the room's shell");
    assert!(drawn.position.distance_to(room) < 0.3, "the provisional leaves at {:?}, the room's copy at {room:?}", drawn.position);
}
