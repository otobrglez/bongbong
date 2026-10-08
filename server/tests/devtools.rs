//! The room server's dev tools (`bongbong_server::devserver`, feature
//! `dev-tools`): a co-op scenario driven with no client, no browser and
//! no second machine - a room opened, its seats' intents posted, the
//! round stepped deterministically and the server's own answer read
//! back.
//!
//! This is the lane `bbmcp rooms` drives, so a green run here is the
//! promise that the tools work; and it is the shape of the reproduction
//! any future co-op bug wants.

#![cfg(feature = "dev-tools")]
//
// Two worker threads each, not four: a room is its own task and these
// tests only need it to run beside the caller. Four apiece, times the
// tests cargo runs at once, oversubscribes a laptop that is also
// compiling - and a starved room task shows up as a dev call that never
// returns, which is the one failure mode hardest to read.


use std::sync::Arc;

use bongbong_server::devserver::dispatch;
use bongbong_server::hub::Hub;
use bongbong_server::metrics::Metrics;
use serde_json::{Value, json};

fn hub() -> Arc<Hub> {
    Hub::new(8, Arc::new(Metrics::new()))
}

/// Open a two-seat room on a pinned seed and hand back its code.
async fn open(hub: &Arc<Hub>, seats: u64) -> String {
    let opened = dispatch(hub, "room_open", &json!({ "map": "default", "seed": 0xB0B5, "seats": seats }))
        .await
        .expect("a room opens");
    opened["code"].as_str().expect("a code").to_string()
}

/// **The scenario the whole thing exists for.** Two seats, no clients,
/// the trigger tapped on one of them, and the authoritative round's own
/// events read back - what took two real browsers and a guess before.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_co_op_round_is_opened_driven_and_read_with_no_client() {
    let hub = hub();
    let code = open(&hub, 2).await;

    // The room is real and playing, with a seat per bot.
    let room = dispatch(&hub, "room", &json!({ "code": code })).await.unwrap();
    assert_eq!(room["phase"], "playing", "{room}");
    assert_eq!(room["players"], 2, "{room}");
    assert_eq!(room["seats"].as_array().unwrap().len(), 2);
    assert_eq!(room["seed"], 0xB0B5_u64, "the round runs on the seed that was pinned");
    // A room of two is fought under a scaled wave plan; a room of one is
    // the empty patch. Reading it is how a scenario knows which.
    assert!(room["tuning_patch"].is_object(), "{}", room["tuning_patch"]);

    // Tap seat 1's trigger for 120 ticks: down one tick in twelve. A
    // shell is edge-triggered, so a held intent would fire exactly once.
    let posted = dispatch(
        &hub,
        "seat_intent",
        &json!({ "code": code, "seat": 1, "ticks": 120, "fire": true, "fire_every": 12 }),
    )
    .await
    .unwrap();
    assert_eq!(posted["driving"], 120, "{posted}");

    // Step the round deterministically - no wall clock, no sleeping.
    let stepped = dispatch(&hub, "room_step", &json!({ "code": code, "ticks": 120 })).await.unwrap();
    assert_eq!(stepped["frozen"], true);
    assert!(stepped["tick"].as_u64().unwrap() >= 120, "{}", stepped["tick"]);
    assert!(stepped["snapshot"]["tanks"].is_array(), "a step carries the world back");

    // And the server's own account of what the seat did.
    let events = dispatch(
        &hub,
        "room_events",
        &json!({ "code": code, "kinds": ["fired"], "limit": 1000 }),
    )
    .await
    .unwrap();
    let fired: Vec<&Value> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["slot"] == 1)
        .collect();
    assert!(
        fired.len() >= 8,
        "seat 1 tapped the trigger ten times in 120 ticks and the room recorded {} shells: {events}",
        fired.len()
    );

    // The mailbox is the reading that says whether input was lost on the
    // way in: everything posted was applied, and nothing starved.
    let room = dispatch(&hub, "room", &json!({ "code": code })).await.unwrap();
    let seat1 = &room["seats"][1];
    assert_eq!(seat1["bot"], true);
    assert_eq!(seat1["mailbox"]["depth"], 0, "every posted intent was applied: {seat1}");
    assert_eq!(seat1["mailbox"]["starvations"], 0, "the buffer never ran dry: {seat1}");
    assert!(seat1["mailbox"]["acked"].as_u64().unwrap() >= 119, "{seat1}");
}

/// **A weapon in a seat's hands** (BB-58): `room_set_tank` arms the
/// authoritative round the way the game's `set_tank` arms a local one, a
/// press fires what it was given, and `room_spawn_pickup` puts a crate on
/// the field for a tank to drive over.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_seat_is_armed_on_the_authority_and_fires_what_it_was_given() {
    let hub = hub();
    let code = open(&hub, 1).await;
    let tank = dispatch(&hub, "room_set_tank", &json!({ "code": code, "slot": 0, "sonic_ammo": 2 })).await.unwrap();
    assert_eq!((tank["weapon"].as_str(), tank["sonic"].as_i64()), (Some("sonic_hammer"), Some(2)), "{tank}");
    dispatch(&hub, "seat_intent", &json!({ "code": code, "seat": 0, "ticks": 10, "fire": true })).await.unwrap();
    dispatch(&hub, "room_step", &json!({ "code": code, "ticks": 10 })).await.unwrap();
    let events = dispatch(&hub, "room_events", &json!({ "code": code, "kinds": ["sonic_blast"] })).await.unwrap();
    assert!(
        events["events"].as_array().unwrap().iter().any(|e| e["slot"] == 0),
        "the armed seat's press set off no hammer: {events}"
    );
    let snapshot = dispatch(&hub, "room_snapshot", &json!({ "code": code })).await.unwrap();
    let seat = snapshot["tanks"].as_array().unwrap().iter().find(|t| t["slot"] == 0).cloned().unwrap();
    assert_eq!(seat["sonic"], 1, "one blast spent: {seat}");

    // A crate down at the first open cell along a row.
    let mut placed = None;
    for col in 2..30 {
        let ask = json!({ "code": code, "kind": "emp_burst", "x": col as f64 * 32.0 + 16.0, "y": 3.0 * 32.0 + 16.0 });
        if let Ok(at) = dispatch(&hub, "room_spawn_pickup", &ask).await {
            placed = Some((at, ask));
            break;
        }
    }
    let (at, kind) = placed.expect("an open cell");
    assert_eq!(at["kind"], "emp_burst", "{at}");
    let snapshot = dispatch(&hub, "room_snapshot", &json!({ "code": code })).await.unwrap();
    assert!(
        snapshot["pickups"].as_array().unwrap().iter().any(|p| p["x"] == at["x"] && p["y"] == at["y"]),
        "the crate is not on the field: {}",
        snapshot["pickups"]
    );
    let again = dispatch(&hub, "room_spawn_pickup", &kind).await;
    assert!(again.is_err(), "a second crate on the same cell: {again:?}");
    let bad = dispatch(&hub, "room_set_tank", &json!({ "code": code, "slot": 0, "shield_timer": 3 })).await;
    assert!(bad.unwrap_err().contains("shield_hp"));
}

/// **A room's own rows are its alone** (BB-58): the process holds one
/// tuning table and every room on it, yet a room opened with rows of its
/// own is set up and fought under them - every enemy carrying the EMP, a
/// seat driving at the speed it was given - while a room beside it on
/// the same seed, and the process's own table, never see them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rooms_own_tuning_is_its_alone() {
    // An open field whose enemies are all down at the start.
    const BAND: &str = r#"
version = 1
tanks = 4
spawn.kind = "band"
cells."2,2" = { kind = "frog" }
cells."4,12" = { kind = "start" }
"#;
    let hub = hub();
    let rows = json!({ "enemy_special_weapon_chance": 1, "enemy_special_weapon_emp_share": 1, "tank_speed": 400 });
    let mut codes = Vec::new();
    for tuning in [rows.clone(), json!({})] {
        let opened = dispatch(&hub, "room_open", &json!({ "map_toml": BAND, "seed": 0xB0B5, "seats": 1, "tuning": tuning }))
            .await
            .expect("a room opens");
        codes.push(opened["code"].as_str().unwrap().to_string());
    }
    let (own, plain) = (codes[0].clone(), codes[1].clone());
    let room = dispatch(&hub, "room", &json!({ "code": own })).await.unwrap();
    assert_eq!(room["dev_tuning"], rows, "{room}");
    assert_eq!(room["tuning_patch"], rows, "a room of one is the empty seat patch plus its rows: {room}");

    // Kitted at the start under the room's rows, and only that room.
    let enemies = |snapshot: &Value| -> Vec<String> {
        snapshot["tanks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["slot"].as_u64().unwrap() >= 1)
            .map(|t| t["weapon"].as_str().unwrap().to_string())
            .collect()
    };
    let armed = enemies(&dispatch(&hub, "room_snapshot", &json!({ "code": own })).await.unwrap());
    let kitted = enemies(&dispatch(&hub, "room_snapshot", &json!({ "code": plain })).await.unwrap());
    assert!(!armed.is_empty() && armed.iter().all(|w| w == "emp_burst"), "{armed:?}");
    assert!(!kitted.iter().any(|w| w == "emp_burst"), "the rows reached the room beside it: {kitted:?}");

    // A live row in force on every tick: the same drive goes further.
    let mut moved = Vec::new();
    for code in [&own, &plain] {
        let y = |snapshot: &Value| {
            snapshot["tanks"].as_array().unwrap().iter().find(|t| t["slot"] == 0).unwrap()["y"].as_f64().unwrap()
        };
        let from = y(&dispatch(&hub, "room_snapshot", &json!({ "code": code })).await.unwrap());
        dispatch(&hub, "seat_intent", &json!({ "code": code, "seat": 0, "ticks": 60, "move_dir": "up" })).await.unwrap();
        let stepped = dispatch(&hub, "room_step", &json!({ "code": code, "ticks": 60 })).await.unwrap();
        moved.push(from - y(&stepped["snapshot"]));
    }
    assert!(moved[0] > moved[1] * 1.25, "the room's tank_speed did not drive its seat: {moved:?}");
    assert_eq!(bongbong::tuning::tuning().tank_speed, bongbong::tuning::Tuning::DEFAULT.tank_speed, "the rows reached the process's table");

    // A bad row refuses the whole call and opens no room.
    let refused = dispatch(&hub, "room_open", &json!({ "seats": 1, "tuning": { "tank_speed": 99999 } })).await;
    assert!(refused.unwrap_err().contains("tank_speed"));
    let listed = dispatch(&hub, "rooms", &json!({})).await.unwrap();
    assert_eq!(listed["rooms"].as_array().unwrap().len(), 2, "{listed}");
}

/// A stepped room is off the wall clock and stays where it was left, so
/// a scenario can look between two ticks without the round running on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stepped_room_holds_still_until_it_is_resumed() {
    let hub = hub();
    let code = open(&hub, 1).await;
    let first = dispatch(&hub, "room_step", &json!({ "code": code, "ticks": 30 })).await.unwrap();
    let at = first["tick"].as_u64().unwrap();
    // Real time passes; a frozen room does not.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let room = dispatch(&hub, "room", &json!({ "code": code })).await.unwrap();
    assert_eq!(room["tick"], at, "a frozen room ticked on its own");
    assert_eq!(room["frozen"], true);
    let resumed = dispatch(&hub, "room_resume", &json!({ "code": code })).await.unwrap();
    assert_eq!(resumed["frozen"], false);
}

/// The same seed, the same inputs, the same round - twice. Without this
/// a scenario is a story rather than a reproduction.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_same_seed_and_the_same_inputs_replay_the_same_round() {
    async fn play() -> Value {
        let hub = hub();
        let code = open(&hub, 1).await;
        dispatch(
            &hub,
            "seat_intent",
            &json!({ "code": code, "seat": 0, "ticks": 90, "move_dir": "right", "fire": true, "fire_every": 10 }),
        )
        .await
        .unwrap();
        dispatch(&hub, "room_step", &json!({ "code": code, "ticks": 90 })).await.unwrap();
        dispatch(&hub, "room_snapshot", &json!({ "code": code })).await.unwrap()
    }
    let (a, b) = (play().await, play().await);
    assert_eq!(a["tanks"], b["tanks"], "the same seed and inputs came out differently");
}

/// A room opened by a tool is a room like any other, and closing it
/// takes it out of the listing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rooms_are_listed_and_closed() {
    let hub = hub();
    let code = open(&hub, 1).await;
    let listed = dispatch(&hub, "rooms", &json!({})).await.unwrap();
    let rows = listed["rooms"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{listed}");
    assert_eq!(rows[0]["code"], code);
    assert_eq!(rows[0]["phase"], "playing");

    let status = dispatch(&hub, "server_status", &json!({})).await.unwrap();
    assert_eq!(status["rooms"], 1, "{status}");

    dispatch(&hub, "room_close", &json!({ "code": code })).await.unwrap();
    let room = dispatch(&hub, "room", &json!({ "code": code })).await.unwrap();
    assert_eq!(room["phase"], "ended", "{room}");
}

/// **A due tick goes ahead of the commands waiting** (docs/online-coop-prd.md
/// §4.16): a room flooded with dev calls from eight callers at once - a
/// burst of lobby or tool traffic, on the real clock - still starts its
/// ticks on time, because the task serves a due tick before the next
/// command.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_flood_of_commands_does_not_hold_the_tick_up() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    let hub = hub();
    let code = open(&hub, 1).await;
    let tick = |v: Value| v["tick"].as_u64().expect("a tick");
    let from = tick(dispatch(&hub, "room", &json!({ "code": code })).await.unwrap());
    let started = Instant::now();
    let stop = Arc::new(AtomicBool::new(false));
    let mut flood = Vec::new();
    for _ in 0..8 {
        let (hub, code, stop) = (hub.clone(), code.clone(), stop.clone());
        flood.push(tokio::spawn(async move {
            let mut calls = 0u64;
            while !stop.load(Ordering::Relaxed) {
                dispatch(&hub, "room", &json!({ "code": code })).await.expect("an answer");
                calls += 1;
            }
            calls
        }));
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    stop.store(true, Ordering::Relaxed);
    let mut calls = 0;
    for caller in flood {
        calls += caller.await.expect("a caller");
    }
    let elapsed = started.elapsed().as_secs_f64();
    let ticked = tick(dispatch(&hub, "room", &json!({ "code": code })).await.unwrap()) - from;
    let expected = elapsed * 60.0;
    let metrics = &dispatch(&hub, "server_status", &json!({})).await.unwrap()["metrics"];
    eprintln!("{calls} calls in {elapsed:.3} s, {ticked} ticks of an expected {expected:.0}; {metrics}");
    assert!(calls > 200, "the flood was a trickle: {calls} calls");
    assert!(ticked as f64 >= expected * 0.8, "the flood held the tick up: {ticked} ticks in {elapsed:.3} s");
    // The count alone does not tell: the schedule catches a late tick up.
    // How late the ticks started does - served after the commands, the
    // median tick starts tens of milliseconds late under this flood.
    let late_p50 = metrics["tick_lateness_p50_us"].as_u64().expect("a reading");
    assert!(late_p50 < 5_000, "the median tick started {late_p50} µs late under the flood: {metrics}");
}

/// **A room keeps its clients' pace** (docs/online-coop-prd.md §4.16),
/// on tokio's paused clock so the seconds are exact. Tick n is due at a
/// fixed instant, so two seconds are 120 ticks however the timer rounds
/// each wake-up (a schedule that restarted from every tick's own start
/// drifts a tick or two behind in that time); a room frozen by a dev tool
/// and resumed ticks on from the resume rather than paying the frozen
/// second back in a burst; and `/metrics` reads how late every tick
/// started - under a millisecond here, the timer's own resolution.
#[tokio::test(start_paused = true)]
async fn the_room_ticks_on_a_wall_clock_schedule() {
    use std::time::Duration;

    let hub = hub();
    let code = open(&hub, 1).await;
    let tick = |v: Value| v["tick"].as_u64().expect("a tick");
    let by_code = json!({ "code": code });
    let room = || dispatch(&hub, "room", &by_code);

    let from = tick(room().await.unwrap());
    tokio::time::sleep(Duration::from_secs(2)).await;
    let ticked = tick(room().await.unwrap()) - from;
    assert!((119..=121).contains(&ticked), "two seconds at 60 Hz ran {ticked} ticks");

    // Frozen for a second, then resumed: the frozen second is gone, not
    // owed.
    let frozen_at = tick(dispatch(&hub, "room_step", &json!({ "code": code, "ticks": 1, "snapshot": false })).await.unwrap());
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(tick(room().await.unwrap()), frozen_at, "a frozen room ticked");
    dispatch(&hub, "room_resume", &json!({ "code": code })).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let resumed = tick(room().await.unwrap()) - frozen_at;
    assert!((29..=31).contains(&resumed), "half a second after the resume ran {resumed} ticks");

    let metrics = &dispatch(&hub, "server_status", &json!({})).await.unwrap()["metrics"];
    eprintln!("{metrics}");
    assert_eq!(metrics["ticks_late_total"], 0, "{metrics}");
    assert_eq!(metrics["ticks_dropped_total"], 0, "{metrics}");
    assert!(metrics["tick_lateness_p99_us"].as_u64().unwrap() <= 1_000, "{metrics}");
}
