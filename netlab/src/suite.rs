//! The suite: maps x profiles x scenarios x client-hull modes, each run in
//! its own process (the tuning table and the room server's base table are
//! process-wide, and a run must not inherit the last one's), gathered into
//! one markdown table with the local twin as the reference row.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::report::{self, Report};
use crate::script::Scenario;

/// The tiled map the shooting scenarios play again on: a shipped campaign
/// field whose seats start in a fenced garden with sandbags and trees in
/// their line of fire, where an own shot's provisional and the room's copy
/// have tile edges and pass-overs to agree on - the arena has none.
pub const TILED_MAP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../maps/wildwood.toml");

/// One stretch of the suite: a map, and the profiles, scenarios and hull
/// modes played on it.
#[derive(Clone, Debug)]
pub struct Leg {
    /// `None` is the map the suite was given (`--map`, the arena unless
    /// another is named).
    pub map: Option<PathBuf>,
    pub profiles: Vec<&'static str>,
    pub scenarios: Vec<Scenario>,
    pub hulls: Vec<bool>,
}

/// What the suite sweeps.
#[derive(Clone, Debug)]
pub struct Plan {
    pub legs: Vec<Leg>,
    pub seconds: f64,
}

impl Plan {
    /// Every profile, every scenario and both hull modes on the given map,
    /// then shoot and duel on every profile on the tiled map with the
    /// client's hull on (the shipped default); twenty seconds a run.
    pub fn full() -> Plan {
        let profiles: Vec<&'static str> = crate::run::PROFILES.iter().map(|p| p.0).collect();
        Plan {
            legs: vec![
                Leg { map: None, profiles: profiles.clone(), scenarios: vec![Scenario::Drive, Scenario::Shoot, Scenario::Duel], hulls: vec![true, false] },
                Leg { map: Some(TILED_MAP.into()), profiles, scenarios: vec![Scenario::Shoot, Scenario::Duel], hulls: vec![true] },
            ],
            seconds: 20.0,
        }
    }

    /// The three profiles that bracket the range, every scenario, both
    /// modes, then shoot and duel on the tiled map on a typical and a bad
    /// link with the hull on; eight seconds a run: a few minutes of wall
    /// clock.
    pub fn quick() -> Plan {
        Plan {
            legs: vec![
                Leg { map: None, profiles: vec!["lan", "typical", "bad"], scenarios: vec![Scenario::Drive, Scenario::Shoot, Scenario::Duel], hulls: vec![true, false] },
                Leg { map: Some(TILED_MAP.into()), profiles: vec!["typical", "bad"], scenarios: vec![Scenario::Shoot, Scenario::Duel], hulls: vec![true] },
            ],
            seconds: 8.0,
        }
    }

    /// How many runs the plan makes.
    pub fn runs(&self) -> usize {
        self.legs.iter().map(|l| l.profiles.len() * l.scenarios.len() * l.hulls.len()).sum()
    }
}

/// Run every cell of `plan` as `netlab run` in a child process - a leg with
/// no map of its own on `map` - passing `extra` through, and return the
/// reports in plan order.
pub fn run(plan: &Plan, map: &Path, extra: &[String], scratch: &Path) -> Vec<Result<Report, String>> {
    let exe = std::env::current_exe().expect("the running binary");
    std::fs::create_dir_all(scratch).expect("a scratch directory");
    let mut out = Vec::new();
    let mut i = 0;
    for leg in &plan.legs {
        let map = leg.map.as_deref().unwrap_or(map);
        let name = map.file_stem().map_or_else(|| map.display().to_string(), |s| s.to_string_lossy().into_owned());
        for &scenario in &leg.scenarios {
            for &profile in &leg.profiles {
                for &hull in &leg.hulls {
                    i += 1;
                    let json: PathBuf = scratch.join(format!("run-{i}.json"));
                    let _ = std::fs::remove_file(&json);
                    eprintln!("[suite] {i}/{}: {name} {profile} {} hull {}", plan.runs(), scenario.name(), if hull { "on" } else { "off" });
                    let status = Command::new(&exe)
                        .arg("run")
                        .arg("--map")
                        .arg(map)
                        .args(["--profile", profile, "--scenario", scenario.name()])
                        .args(["--client-hull", if hull { "on" } else { "off" }])
                        .args(["--seconds", &plan.seconds.to_string()])
                        .args(["--label", &format!("suite-{i}")])
                        .arg("--json-out")
                        .arg(&json)
                        .arg("--quiet")
                        .args(extra)
                        .stdout(Stdio::null())
                        .status();
                    let report = match status {
                        Ok(s) if s.success() => std::fs::read_to_string(&json)
                            .map_err(|e| format!("{}: {e}", json.display()))
                            .and_then(|text| serde_json::from_str::<Report>(&text).map_err(|e| format!("{}: {e}", json.display()))),
                        Ok(s) => Err(format!("{name} {profile} {} hull {hull}: exited with {s}", scenario.name())),
                        Err(e) => Err(format!("{name} {profile} {} hull {hull}: {e}", scenario.name())),
                    };
                    out.push(report);
                }
            }
        }
    }
    out
}

/// The markdown table: per map and scenario, the local twin's row, then
/// every online row.
pub fn table(reports: &[Result<Report, String>]) -> String {
    let mut s = String::from(report::TABLE_HEADER);
    s.push('\n');
    let ok: Vec<&Report> = reports.iter().filter_map(|r| r.as_ref().ok()).collect();
    let mut groups: Vec<(&str, Scenario)> = Vec::new();
    for r in &ok {
        if !groups.contains(&(r.map.as_str(), r.scenario)) {
            groups.push((r.map.as_str(), r.scenario));
        }
    }
    for (map, scenario) in groups {
        let rows: Vec<&&Report> = ok.iter().filter(|r| r.map == map && r.scenario == scenario).collect();
        if let Some(first) = rows.first() {
            s.push_str(&report::table_row(first, true));
            s.push('\n');
        }
        for r in rows {
            s.push_str(&report::table_row(r, false));
            s.push('\n');
        }
    }
    for r in reports.iter().filter_map(|r| r.as_ref().err()) {
        s.push_str(&format!("\nfailed: {r}\n"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both plans play the tiled map, and only the shooting scenarios on
    /// it: the drive's pacing is the link's alone on the open arena.
    #[test]
    fn both_plans_shoot_on_a_tiled_map_that_exists() {
        assert!(Path::new(TILED_MAP).is_file(), "{TILED_MAP}");
        for plan in [Plan::quick(), Plan::full()] {
            let tiled: Vec<&Leg> = plan.legs.iter().filter(|l| l.map.is_some()).collect();
            assert_eq!(tiled.len(), 1, "{plan:?}");
            assert_eq!(tiled[0].scenarios, vec![Scenario::Shoot, Scenario::Duel]);
        }
        assert_eq!(Plan::quick().runs(), 18 + 4);
        assert_eq!(Plan::full().runs(), 30 + 10);
    }
}
