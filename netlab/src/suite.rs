//! The suite: profiles x scenarios x client-hull modes, each run in its
//! own process (the tuning table and the room server's base table are
//! process-wide, and a run must not inherit the last one's), gathered into
//! one markdown table with the local twin as the reference row.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::report::{self, Report};
use crate::script::Scenario;

/// What the suite sweeps.
#[derive(Clone, Debug)]
pub struct Plan {
    pub profiles: Vec<&'static str>,
    pub scenarios: Vec<Scenario>,
    pub hulls: Vec<bool>,
    pub seconds: f64,
}

impl Plan {
    /// Every profile, every scenario, both hull modes, twenty seconds.
    pub fn full() -> Plan {
        Plan {
            profiles: crate::run::PROFILES.iter().map(|p| p.0).collect(),
            scenarios: vec![Scenario::Drive, Scenario::Shoot, Scenario::Duel],
            hulls: vec![true, false],
            seconds: 20.0,
        }
    }

    /// The three profiles that bracket the range, every scenario, both
    /// modes, eight seconds: a few minutes of wall clock.
    pub fn quick() -> Plan {
        Plan { profiles: vec!["lan", "typical", "bad"], seconds: 8.0, ..Plan::full() }
    }
}

/// Run every cell of `plan` as `netlab run` in a child process, passing
/// `extra` through, and return the reports in plan order.
pub fn run(plan: &Plan, extra: &[String], scratch: &Path) -> Vec<Result<Report, String>> {
    let exe = std::env::current_exe().expect("the running binary");
    std::fs::create_dir_all(scratch).expect("a scratch directory");
    let mut out = Vec::new();
    let mut i = 0;
    for &scenario in &plan.scenarios {
        for &profile in &plan.profiles {
            for &hull in &plan.hulls {
                i += 1;
                let json: PathBuf = scratch.join(format!("run-{i}.json"));
                let _ = std::fs::remove_file(&json);
                eprintln!("[suite] {i}: {profile} {} hull {}", scenario.name(), if hull { "on" } else { "off" });
                let status = Command::new(&exe)
                    .arg("run")
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
                    Ok(s) => Err(format!("{profile} {} hull {hull}: exited with {s}", scenario.name())),
                    Err(e) => Err(format!("{profile} {} hull {hull}: {e}", scenario.name())),
                };
                out.push(report);
            }
        }
    }
    out
}

/// The markdown table: per scenario, the local twin's row, then every
/// online row.
pub fn table(reports: &[Result<Report, String>]) -> String {
    let mut s = String::from(report::TABLE_HEADER);
    s.push('\n');
    let ok: Vec<&Report> = reports.iter().filter_map(|r| r.as_ref().ok()).collect();
    let mut scenarios: Vec<Scenario> = Vec::new();
    for r in &ok {
        if !scenarios.contains(&r.scenario) {
            scenarios.push(r.scenario);
        }
    }
    for scenario in scenarios {
        let rows: Vec<&&Report> = ok.iter().filter(|r| r.scenario == scenario).collect();
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
