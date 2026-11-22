//! The sound map (docs/sound-system-prd.md section 4.11): where the game
//! needs sound, what plays there, and what has been made - one graph from
//! three sources:
//!
//! - the code: the state watchers (`audio::watch::WATCHERS`) and every
//!   enumeration's exhaustive sound map (`audio::source`), walked over each
//!   `ALL`;
//! - the manifest, `audio/cues.toml`: every cue and sample, with each
//!   take's file looked for on disk;
//! - the curated catalogue, `audio/interactions.json`: every entity and
//!   interaction the game has and the sound each needs (BB-199's map, kept
//!   by the `sound-audit` skill), each cue tied to the manifest cue that
//!   plays it.
//!
//!     cargo run --bin soundmap --no-default-features -- [--json out.json] [--html out.html] [--check]
//!
//! `--check` fails (exit 1) on a cue the code, the catalogue or a watcher
//! names that the manifest cannot resolve, an approved sample with a
//! missing file, or a catalogue cue tied to nothing that is not marked
//! missing. Headless: no raylib.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use bongbong::audio::cues::{Manifest, ROOT, Status};
use bongbong::audio::source::{self, FireSound};
use bongbong::audio::watch::WATCHERS;
use bongbong::map::{Theme, Weather};
use bongbong::obstacle::Material;
use bongbong::pickup::PickupKind;
use bongbong::tank::{ActiveWeapon, TankKind};
use bongbong::tower::TowerKind;

fn main() {
    let mut json_out: Option<PathBuf> = None;
    let mut html_out: Option<PathBuf> = None;
    let mut check = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--json" => json_out = args.next().map(PathBuf::from),
            "--html" => html_out = args.next().map(PathBuf::from),
            "--check" => check = true,
            other => {
                eprintln!("soundmap: unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    let manifest_text = std::fs::read_to_string("audio/cues.toml").unwrap_or_else(|_| bongbong::audio::cues::SHIPPED.to_string());
    let manifest = Manifest::parse(&manifest_text).unwrap_or_else(|e| {
        eprintln!("soundmap: audio/cues.toml: {e}");
        std::process::exit(1);
    });
    let catalogue: Value = std::fs::read_to_string("audio/interactions.json")
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);

    let mut problems: Vec<String> = Vec::new();
    let resolve = |id: &str| manifest.resolve(id).map(|(k, _)| k.to_string());

    // The manifest: every sample with its files, every cue with its status.
    let mut samples = BTreeMap::new();
    for (id, s) in &manifest.sample {
        let takes: Vec<Value> = s
            .takes
            .iter()
            .map(|t| {
                let exists = Path::new(ROOT).join(&t.file).is_file();
                if s.status == Status::Approved && !exists {
                    problems.push(format!("sample {id}: approved, but {}/{} is missing", ROOT, t.file));
                }
                json!({"file": t.file, "exists": exists, "generation": t.generation, "flow": t.flow, "made": t.made, "seconds": t.seconds, "rms_db": t.rms_db})
            })
            .collect();
        let has_files = takes.iter().any(|t| t["exists"].as_bool() == Some(true));
        samples.insert(id.clone(), json!({"id": id, "status": status_name(s.status), "has_files": has_files, "bus": s.bus, "prompt": s.prompt, "looped": s.looped, "takes": takes}));
    }
    let sample_done = |id: &str| samples.get(id).is_some_and(|s| s["status"] == "approved" && s["has_files"] == true);
    let mut cues = BTreeMap::new();
    for (id, c) in &manifest.cue {
        let layers: Vec<&str> = c.layers.iter().map(|l| l.sample.as_str()).collect();
        let done = layers.iter().filter(|s| sample_done(s)).count();
        let status = if done == layers.len() { "done" } else if done > 0 { "partial" } else { "planned" };
        cues.insert(id.clone(), json!({"id": id, "kind": c.kind, "bus": c.bus, "priority": c.priority, "class": c.class, "note": c.note, "samples": layers, "status": status, "requested_by": []}));
    }

    // What the code asks for: each enumeration's map, walked over `ALL`.
    let mut requests: Vec<(String, String, String)> = Vec::new();
    for k in TankKind::ALL {
        requests.push(("TankKind".into(), k.name().into(), source::engine_family(k).cue().into()));
    }
    for w in ActiveWeapon::ALL {
        match source::weapon_sound(w) {
            FireSound::Cue(c) => requests.push(("ActiveWeapon".into(), w.name().into(), c.into())),
            FireSound::Cannon => {
                for f in [0.75, 1.0, 1.3, 1.6] {
                    requests.push(("ActiveWeapon".into(), format!("{} x{f}", w.name()), source::cannon_cue(f).into()));
                }
            }
            FireSound::Watched(watcher) => requests.push(("ActiveWeapon".into(), w.name().into(), format!("watcher:{watcher}"))),
            FireSound::Elsewhere(event) => requests.push(("ActiveWeapon".into(), w.name().into(), format!("event:{event}"))),
        }
    }
    for m in Material::ALL {
        let (family, name) = (source::surface_family(m), source::material_name(m));
        requests.push(("Material".into(), name.into(), format!("hit.shell.{family}.{name}")));
        requests.push(("Material".into(), name.into(), format!("destroy.{family}.{name}")));
    }
    for k in PickupKind::ALL {
        requests.push(("PickupKind".into(), format!("{k:?}"), source::pickup_cue(k).into()));
    }
    for k in TowerKind::ALL {
        requests.push(("TowerKind".into(), format!("{k:?}"), source::tower_cue(k).into()));
    }
    for w in Weather::ALL {
        if let Some(c) = source::sky_bed(w) {
            requests.push(("Weather".into(), format!("{w:?}"), c.into()));
        }
    }
    for t in Theme::ALL {
        requests.push(("Theme".into(), t.name().into(), source::place_bed(t, None).into()));
        for b in bongbong::biome::Biome::ALL {
            requests.push(("Biome".into(), format!("{}/{b:?}", t.name()), source::place_bed(t, Some(b)).into()));
        }
    }
    for w in WATCHERS {
        for c in w.cues {
            requests.push(("watcher".into(), w.id.into(), (*c).into()));
        }
    }
    let mut code = Vec::new();
    for (from, variant, wants) in &requests {
        let resolved = if wants.contains(':') { None } else { resolve(wants) };
        if !wants.contains(':') && resolved.is_none() {
            problems.push(format!("{from}::{variant} asks for cue {wants}, which the manifest cannot resolve"));
        }
        if let Some(r) = &resolved
            && let Some(cue) = cues.get_mut(r)
        {
            cue["requested_by"].as_array_mut().expect("an array").push(json!(format!("{from}::{variant}")));
        }
        code.push(json!({"from": from, "variant": variant, "wants": wants, "resolved": resolved}));
    }

    // The catalogue's cues, each tied to the manifest cue that plays it.
    let mut needed = Vec::new();
    if let Some(list) = catalogue["cues"].as_array() {
        for c in list {
            let name = c["cue"].as_str().unwrap_or("?");
            let link = &c["manifest"];
            let cue = link["cue"].as_str();
            let resolved = cue.and_then(resolve);
            if cue.is_some() && resolved.is_none() {
                problems.push(format!("catalogue cue {name} names {} , which the manifest cannot resolve", cue.unwrap_or("")));
            }
            if cue.is_none() && link["fit"].as_str() != Some("missing") {
                problems.push(format!("catalogue cue {name} is tied to nothing and not marked missing"));
            }
            let status = match &resolved {
                Some(r) => cues.get(r).and_then(|c| c["status"].as_str()).unwrap_or("planned").to_string(),
                None => "not_in_manifest".to_string(),
            };
            needed.push(json!({
                "cue": name, "priority": c["priority"], "kind": c["kind"], "descriptions": c["descriptions"],
                "interactions": c["interactions"], "domains": c["domains"], "hangs_on": c["status"],
                "manifest": resolved, "proposed": link["proposed"], "fit": link["fit"], "status": status,
            }));
        }
    }

    let count = |v: &[Value], k: &str, s: &str| v.iter().filter(|x| x[k] == s).count();
    let needed_stats = json!({
        "total": needed.len(),
        "done": count(&needed, "status", "done"),
        "partial": count(&needed, "status", "partial"),
        "planned": count(&needed, "status", "planned"),
        "not_in_manifest": count(&needed, "status", "not_in_manifest"),
    });
    let cue_list: Vec<Value> = cues.into_values().collect();
    let sample_list: Vec<Value> = samples.into_values().collect();
    let out = json!({
        "commit": git_head(),
        "stats": {
            "manifest_cues": cue_list.len(),
            "cues_done": count(&cue_list, "status", "done"),
            "cues_partial": count(&cue_list, "status", "partial"),
            "cues_planned": count(&cue_list, "status", "planned"),
            "samples": sample_list.len(),
            "samples_approved": count(&sample_list, "status", "approved"),
            "samples_planned": count(&sample_list, "status", "planned"),
            "needed": needed_stats,
            "problems": problems.len(),
        },
        "cues": cue_list,
        "samples": sample_list,
        "code": code,
        "watchers": WATCHERS.iter().map(|w| json!({"id": w.id, "entity": w.entity, "reads": w.reads, "cues": w.cues, "online": w.online})).collect::<Vec<_>>(),
        "needed": needed,
        "catalogue": {
            "entities": catalogue["entities"],
            "interactions": catalogue["interactions"],
            "gaps": catalogue["gaps"],
            "domains": catalogue["domains"],
            "audited_at": catalogue["audited_at"],
        },
        "problems": problems,
    });
    let text = serde_json::to_string(&out).expect("the map serializes");
    if let Some(path) = &json_out {
        write(path, &text);
    }
    if let Some(path) = &html_out {
        let template = std::fs::read_to_string("tools/audio/soundmap/template.html").unwrap_or_else(|e| {
            eprintln!("soundmap: tools/audio/soundmap/template.html: {e}");
            std::process::exit(1);
        });
        write(path, &template.replace("__DATA__", &text.replace("</", "<\\/")));
    }
    println!(
        "soundmap: {} cues ({} done, {} partial, {} planned), {} samples ({} approved); needed {}: {} done, {} partial, {} planned, {} not in the manifest; {} problems",
        out["stats"]["manifest_cues"], out["stats"]["cues_done"], out["stats"]["cues_partial"], out["stats"]["cues_planned"],
        out["stats"]["samples"], out["stats"]["samples_approved"],
        needed_stats["total"], needed_stats["done"], needed_stats["partial"], needed_stats["planned"], needed_stats["not_in_manifest"],
        problems.len()
    );
    for p in &problems {
        println!("soundmap: problem: {p}");
    }
    if check && !problems.is_empty() {
        std::process::exit(1);
    }
}

fn status_name(s: Status) -> &'static str {
    match s {
        Status::Planned => "planned",
        Status::Review => "review",
        Status::Approved => "approved",
        Status::Placeholder => "placeholder",
        Status::Rejected => "rejected",
    }
}

fn git_head() -> Option<String> {
    let out = std::process::Command::new("git").args(["rev-parse", "--short", "HEAD"]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn write(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(path, text) {
        eprintln!("soundmap: {}: {e}", path.display());
        std::process::exit(1);
    }
    println!("soundmap: wrote {}", path.display());
}
