#!/usr/bin/env python3
"""Compare tuning patches on the headless combat suite."""

import argparse
from concurrent.futures import ThreadPoolExecutor
from collections import Counter
from itertools import product
import json
from pathlib import Path
from random import Random
import shlex
from statistics import mean, median
import subprocess
import sys

CASES = (
    "shell-open", "shell-shield", "shell-peek-shield", "emp-open", "emp-shield",
    "grenade-open", "grenade-shield", "grenade-cover", "protect-guard", "protect-crate",
)
PROTECT_CASES = ("protect-guard", "protect-crate")
SAMPLE_AXES = (
    "player_armor_factor", "shield_capacity", "shield_seconds",
    "shield_deflect_cost_factor", "grenade_damage_scale",
)
SCORE_FIELDS = (
    "protect_bong_damage", "protect_wrecks", "protect_win_rate", "protect_seat_peak_damage",
    "shell_deflections", "shell_peek_deflections", "shell_reflected_damage", "grenade_open_damage",
)


def seed_number(text):
    value = int(text, 16 if text.lower().startswith("0x") else 10)
    if not 0 <= value < 2**64:
        raise argparse.ArgumentTypeError("seed must fit an unsigned 64-bit integer")
    return value


def positive(text):
    value = int(text)
    if value < 1:
        raise argparse.ArgumentTypeError("must be at least 1")
    return value


def objective(text):
    field, _, direction = text.partition(":")
    if field not in SCORE_FIELDS or direction not in {"min", "max"}:
        raise argparse.ArgumentTypeError(f"use FIELD:min or FIELD:max with {', '.join(SCORE_FIELDS)}")
    return field, direction


def arguments():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--probe", type=Path, default=Path("target/release/balance_probe"))
    parser.add_argument("--out", type=Path, required=True, help="new directory for the report, trials and replay patches")
    parser.add_argument("--candidate", type=Path, action="append", default=[], help="tuning patch to compare; repeatable")
    parser.add_argument("--sample", type=positive, help="number of Latin hypercube patches around the defaults")
    parser.add_argument("--sample-seed", type=seed_number, default=2026)
    parser.add_argument("--seed", type=seed_number, default=42000)
    parser.add_argument("--rounds", type=positive, default=16, help="rounds per case and patch")
    parser.add_argument("--jobs", type=positive, default=4, help="maximum simultaneous probe processes")
    parser.add_argument("--objective", action="append", type=objective, help="shortlist trade-offs by FIELD:min or FIELD:max; repeatable")
    parser.add_argument("--protect-map", choices=("practice", "crossfire"), default="practice")
    parser.add_argument("--holdout-seed", type=seed_number, help="fresh seeds for baseline and shortlist, on the other Protect map")
    parser.add_argument("--shortlist", type=positive, default=3, help="maximum candidates sent to holdout")
    args = parser.parse_args()
    if len({field for field, _ in args.objective or []}) != len(args.objective or []):
        parser.error("each objective field can be used once")
    if args.seed + args.rounds > 2**64 or (
        args.holdout_seed is not None and args.holdout_seed + args.rounds > 2**64
    ):
        parser.error("seed ranges must fit an unsigned 64-bit integer")
    if args.holdout_seed is not None:
        if not args.objective:
            parser.error("--holdout-seed requires --objective")
        if abs(args.seed - args.holdout_seed) < args.rounds:
            parser.error("holdout seeds must be separate from the search seeds")
    return args


def checked_run(command):
    process = subprocess.run(command, capture_output=True, text=True)
    if process.returncode:
        raise RuntimeError(f"{shlex.join(map(str, command))}\n{process.stderr.strip()}")
    return process.stdout


def sampled_patches(count, seed, probe):
    schema = {row["name"]: row for row in json.loads(checked_run([probe, "--schema"]))}
    rng = Random(seed)
    columns = {}
    for axis in SAMPLE_AXES:
        strata = list(range(count))
        rng.shuffle(strata)
        columns[axis] = [0.8 + 0.4 * (slot + rng.random()) / count for slot in strata]
    for i in range(count):
        patch = {}
        for key in SAMPLE_AXES[:-1]:
            row = schema[key]
            value = row["default"] * columns[key][i]
            patch[key] = round(max(row["min"], min(row["max"], value)), 5)
        scale = columns["grenade_damage_scale"][i]
        for key in ("grenade_blast_damage_min", "grenade_blast_damage_max"):
            row = schema[key]
            patch[key] = round(max(row["min"], min(row["max"], row["default"] * scale)), 5)
        yield patch


def trials(args, probe, folder):
    patches = [("baseline", "defaults", {})]
    for i, path in enumerate(args.candidate, 1):
        patch = json.loads(path.read_text())
        if not isinstance(patch, dict):
            raise ValueError(f"{path}: expected a tuning JSON object")
        patches.append((f"candidate-{i:02}", path.stem, patch))
    if args.sample:
        for i, patch in enumerate(sampled_patches(args.sample, args.sample_seed, probe), 1):
            patches.append((f"sample-{i:02}", "Latin hypercube", patch))
    folder.mkdir()
    result = {}
    for trial, label, patch in patches:
        file = folder / f"{trial}.json"
        file.write_text(json.dumps(patch, indent=2, allow_nan=False) + "\n")
        result[trial] = (label, file)
    return result


def evaluate(probe, entries, seed, rounds, jobs, partition, protect_map, destination):
    def run(task):
        (trial, (_, patch)), case = task
        command = [
            probe, "--case", case, "--seed", str(seed), "--rounds", str(rounds),
            "--protect-map", protect_map, "--tuning", patch,
        ]
        rows = [json.loads(line) for line in checked_run(command).splitlines()]
        if len(rows) != rounds or any(
            row["v"] != 1 or row["case"] != case or int(row["seed"], 16) != seed + i
            or row["protect_map"] != (protect_map if case in PROTECT_CASES else None)
            for i, row in enumerate(rows)
        ):
            raise RuntimeError(f"unexpected records from {shlex.join(map(str, command))}")
        return trial, case, rows

    results = {trial: {} for trial in entries}
    with destination.open("w") as stream, ThreadPoolExecutor(max_workers=jobs) as executor:
        for trial, case, rows in executor.map(run, product(entries.items(), CASES)):
            results[trial][case] = rows
            for row in rows:
                print(json.dumps({"trial": trial, "partition": partition, **row}), file=stream)
    return results


def summed_events(rows, kind):
    result = Counter()
    for row in rows:
        result.update(row["metrics"][kind])
    return dict(result)


def timing(rows, key):
    values = [row["metrics"][key] for row in rows if row["metrics"][key] is not None]
    return {"count": len(values), "median_frame": median(values) if values else None}


def summarize_case(rows):
    target_damage = [row["target_end_damage"] for row in rows if row["target_end_damage"] is not None]
    bong_damage = [row["bong_damage"] for row in rows if row["bong_damage"] is not None]
    return {
        "outcomes": dict(Counter(row["outcome"] for row in rows)),
        "target_damage_mean": mean(target_damage) if target_damage else None,
        "bong_damage_mean": mean(bong_damage) if bong_damage else None,
        "enemy_shield_deflections_mean": mean(row["metrics"]["enemy_shield_deflections"] for row in rows),
        "target_shield_break": timing(rows, "first_target_shield_break_frame"),
        "target_hull_damage": timing(rows, "first_target_hull_damage_frame"),
        "target_wreck": timing(rows, "target_wreck_frame"),
        "target_disabled": timing(rows, "target_disabled_frame"),
        "weapon_pulls": summed_events(rows, "seat_trigger_pulls"),
        "pickups": summed_events(rows, "seat_pickups"),
    }


def summarize(results, baseline):
    cases = {case: summarize_case(rows) for case, rows in results.items()}
    protect = [row for case in PROTECT_CASES for row in results[case]]
    scores = {
        "protect_bong_damage": mean(row["bong_damage"] for row in protect),
        "protect_wrecks": mean(row["wrecks"] for row in protect),
        "protect_win_rate": mean(row["outcome"] == "won" for row in protect),
        "protect_seat_peak_damage": mean(row["metrics"]["seat_peak_damage"] for row in protect),
        "shell_deflections": cases["shell-shield"]["enemy_shield_deflections_mean"],
        "shell_peek_deflections": cases["shell-peek-shield"]["enemy_shield_deflections_mean"],
        "shell_reflected_damage": mean(
            dict(row["metrics"]["hit_events"].get("seat", ())).get("shell", {}).get("reported_damage", 0)
            for row in results["shell-shield"]
        ),
        "grenade_open_damage": cases["grenade-open"]["target_damage_mean"],
    }
    constraints = {
        "emp_always_breaks_shield": all(
            row["metrics"]["target_disabled_frame"] is not None
            and row["metrics"]["target_disabled_frame"] == row["metrics"]["first_target_shield_break_frame"]
            for row in results["emp-shield"]
        ),
        "grenade_cover_reduces_damage": cases["grenade-cover"]["target_damage_mean"] < scores["grenade_open_damage"],
        "protect_crate_exercises_minigun": cases["protect-crate"]["weapon_pulls"].get("minigun", 0) > 0,
    }
    return {
        "scores": scores, "constraints": constraints, "cases": cases,
        "paired_bong_damage_delta": {
            case: mean(row["bong_damage"] - old["bong_damage"] for row, old in zip(results[case], baseline[case], strict=True))
            for case in PROTECT_CASES
        },
    }


def pareto_front(summaries, objectives):
    if not objectives:
        return []
    points = {
        name: tuple(summary["scores"][field] * (-1 if direction == "max" else 1) for field, direction in objectives)
        for name, summary in summaries.items() if all(summary["constraints"].values())
    }

    def dominates(a, b):
        return all(x <= y for x, y in zip(a, b)) and any(x < y for x, y in zip(a, b))

    return [
        name for name, point in sorted(points.items(), key=lambda entry: entry[1])
        if not any(dominates(other, point) for other in points.values())
    ]


def probe_command(probe, patch, case, seed, protect_map):
    return shlex.join([
        str(probe), "--case", case, "--seed", str(seed), "--protect-map", protect_map, "--tuning", str(patch),
    ])


def markdown(probe, catalog, summaries, frontier, shortlist, holdout, holdout_map, args):
    table_header = [
        "| Patch | Bong damage | Protect wrecks | Protect win rate | Peak hull damage | Shield deflections (rapid / peek) | Reflected shell damage | Grenade damage (open / shield / cover) | Checks |",
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |",
    ]
    lines = [
        "# BongBong balance report",
        "",
        f"Search seeds {args.seed}–{args.seed + args.rounds - 1}, {args.rounds} rounds per case. Protect map: {args.protect_map}.",
        "A Protect round still playing at its cap is unfinished. Controlled cases report target wrecks separately.",
        "Protect averages weight the guard and crate scripts equally. These are scripted measurements.",
        "",
        *table_header,
    ]

    def trial_row(name, summary):
        s = summary["scores"]
        c = summary["cases"]
        checks = ", ".join(key for key, passed in summary["constraints"].items() if not passed) or "pass"
        grenade = " / ".join(f"{c[case]['target_damage_mean']:.1f}" for case in ("grenade-open", "grenade-shield", "grenade-cover"))
        return (
            f"| {name} | {s['protect_bong_damage']:.1f} | {s['protect_wrecks']:.2f} | {s['protect_win_rate']:.0%} | "
            f"{s['protect_seat_peak_damage']:.1f} | {s['shell_deflections']:.1f} / {s['shell_peek_deflections']:.1f} | "
            f"{s['shell_reflected_damage']:.1f} | {grenade} | {checks} |"
        )

    lines.extend(trial_row(name, summary) for name, summary in summaries.items())
    if args.objective:
        goals = ", ".join(f"{field} ({direction})" for field, direction in args.objective)
        lines += ["", f"Objectives: {goals}. Pareto front: {', '.join(frontier) or 'no eligible patch'}.",
                  f"Playtest shortlist: {', '.join(shortlist) or 'no candidate beyond baseline'}."]
    if holdout:
        lines += ["", f"## Held out seeds {args.holdout_seed}–{args.holdout_seed + args.rounds - 1}", "",
                  f"Protect map: {holdout_map}.", "", *table_header]
        lines.extend(trial_row(name, summary) for name, summary in holdout.items())
    lines += [
        "", "## Baseline by case", "",
        "| Case | Mission W / L / Unfinished | Shield breaks (rounds; median s) | Rounds with target hull damage | Target wrecks (rounds; median s) | Seat triggers | Seat pickups |",
        "| --- | ---: | ---: | ---: | ---: | --- | --- |",
    ]
    def format_timing(timing):
        seconds = timing["median_frame"]
        return "—" if seconds is None else f"{timing['count']}; {seconds / 60:.2f}"

    for name, case in summaries["baseline"]["cases"].items():
        outcomes = "—"
        if name in PROTECT_CASES:
            outcomes = " / ".join(str(case["outcomes"].get(outcome, 0)) for outcome in ("won", "lost", "playing"))
        presses = ", ".join(f"{kind} ×{n}" for kind, n in case["weapon_pulls"].items()) or "—"
        pickups = ", ".join(f"{kind} ×{n}" for kind, n in case["pickups"].items()) or "—"
        lines.append(
            f"| {name} | {outcomes} | {format_timing(case['target_shield_break'])} | {case['target_hull_damage']['count']} | "
            f"{format_timing(case['target_wreck'])} | {presses} | {pickups} |"
        )
    lines += ["", "## Replay examples", "", "Each round record carries its case and seed.", "", "```sh"]
    for name in ("baseline", *shortlist):
        lines.append(probe_command(probe, catalog[name][1], "protect-crate", args.seed, args.protect_map))
        if holdout:
            lines.append(probe_command(probe, catalog[name][1], "protect-crate", args.holdout_seed, holdout_map))
    lines += ["```", ""]
    return "\n".join(lines)


def main():
    args = arguments()
    try:
        probe = args.probe.resolve(strict=True)
        args.out.mkdir(parents=True)
        catalog = trials(args, probe, args.out / "patches")
        results = evaluate(probe, catalog, args.seed, args.rounds, args.jobs, "search", args.protect_map, args.out / "rounds.jsonl")
        summaries = {name: summarize(batch, results["baseline"]) for name, batch in results.items()}
        frontier = pareto_front(summaries, args.objective)
        shortlist = []
        fields = [field for field, _ in args.objective or []]
        seen = {tuple(summaries["baseline"]["scores"][field] for field in fields)}
        for name in frontier:
            point = tuple(summaries[name]["scores"][field] for field in fields)
            if point not in seen and len(shortlist) < args.shortlist:
                shortlist.append(name)
                seen.add(point)
        holdout = {}
        holdout_map = None
        if args.holdout_seed is not None:
            entries = {name: catalog[name] for name in ("baseline", *shortlist)}
            holdout_map = "crossfire" if args.protect_map == "practice" else "practice"
            batches = evaluate(
                probe, entries, args.holdout_seed, args.rounds, args.jobs, "holdout", holdout_map, args.out / "holdout.jsonl",
            )
            holdout = {name: summarize(batch, batches["baseline"]) for name, batch in batches.items()}
        summary = {
            "rounds_per_case": args.rounds, "seed": args.seed, "sample_seed": args.sample_seed,
            "protect_map": args.protect_map, "holdout_map": holdout_map,
            "holdout_seed": args.holdout_seed, "objectives": args.objective or [],
            "trial_labels": {name: label for name, (label, _) in catalog.items()},
            "search": summaries, "pareto_front": frontier, "shortlist": shortlist, "holdout": holdout,
        }
        (args.out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        (args.out / "report.md").write_text(markdown(probe, catalog, summaries, frontier, shortlist, holdout, holdout_map, args))
        print(args.out / "report.md")
    except (OSError, ValueError, RuntimeError) as error:
        print(f"balance: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
