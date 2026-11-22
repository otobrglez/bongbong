---
name: sound-audit
description: Find every place in bongbong that needs a sound and is not covered - new entities, interactions, events, state changes - and update the sound map's catalogue, the cue manifest and Linear. Use when the user says "sound audit", "what needs sound", "update the sound map", after a feature that adds a weapon, tank, material, event, pickup, biome or sky lands, or before a sound-gen batch.
---

# Sound audit

The sound map (docs/sound-system-prd.md section 4.11) joins three sources:

- **the code**: `audio::bind` (an exhaustive match over `Event`), the exhaustive
  maps in `src/audio/source.rs`, the state watchers in `src/audio/watch.rs`
  (`WATCHERS`);
- **the manifest**: `audio/cues.toml`, every cue and sample;
- **the catalogue**: `audio/interactions.json`, every entity and interaction in
  the game and the sound each needs, each catalogue cue tied to the manifest
  cue that plays it (`manifest: {cue, proposed, fit}`).

This skill keeps the catalogue and the manifest true to the code. It never
generates audio (that is `sound-gen`) and never edits simulation code: a hook
gap becomes a Linear issue.

## 1. What the compiler and the tests already know

```
cargo run --bin soundmap --no-default-features -- --check
cargo test --lib --no-default-features --features dev-tools audio
```

A new `Event` variant or enum variant without a sound decision fails to
compile in `bind.rs`/`source.rs` - that is the change's own work, not this
skill's. Record every `--check` problem; they are the first findings.

## 2. What changed since the last audit

Read `audited_at` from `audio/interactions.json`, then
`git log --oneline <audited_at>..HEAD` and
`git diff --stat <audited_at>..HEAD -- src/simulation src/net src/*.rs`.
No `audited_at`, or the user asked for a full audit: do the full pass over
the whole tree.

## 3. The passes

Incremental: one pass over the changed files. Full: five Explore agents in
parallel, one per domain - weapons; specials and air support; the world
(tiles, props, fire, water, weather, biomes, portals, volcano); tanks and
the round (locomotion, damage, wrecks, waves, the bong, pickups, training);
presentation and online (HUD, menus, builder, `net/`). Give each this brief:

> For every entity and every interaction between entities in your domain,
> report: source and target entity; a verb; what triggers it (an `Event`
> variant with its file:line, a state edge, continuous state, player input,
> UI, or cosmetic only); whether an online client gets it (`NOT_SENT` in
> src/net/events.rs, own-shot prediction in src/net/round.rs, snapshot
> state, local only); what already draws it; how often it fires and whether
> it floods; and the sound it needs - cue name, one-line description, kind
> (one_shot, loop, modulated_loop, stinger, ui, voice), priority (critical,
> high, medium, low), what it varies by. Mark gaps: no event, not sent,
> missing payload, wrong granularity, online timing, volume risk, missing
> feedback. Cite file:line for every claim. Report only what is new or
> changed against audio/interactions.json, which you read first (with jq).

## 4. Merge

- Deduplicate against `audio/interactions.json` by (source, target, verb).
  Add new entities, interactions and gaps in the same JSON shape; update
  changed ones; mark removed ones `"retired": "<commit>"`, never delete them.
- For each catalogue cue, set `manifest`: `cue` the manifest id that plays
  it (an id more specific than a manifest cue is fine - it falls back to its
  nearest ancestor), `fit` exact / covered / missing, and `proposed` the new
  dotted id when it is missing (grammar: `hit.<shot>.<what>`,
  `destroy.<family>.<material>`, `weapon.<weapon>.<moment>`, `world.*`,
  `warning.*`, `ui.*`, `voice.*`, `music.*`).
- For a missing cue the game should have now, add `[cue."<id>"]` and a
  `[sample.<id>]` with `status = "planned"` and a prompt in the house style
  (docs/SOUND_SPEC.md: one sound, short, concrete texture and transient,
  close-mic or distant, no visual words, "no music"), and bind it in code
  where its event or watcher already exists.
- Check every cited file:line exists; drop the ones that do not.
- Set `audited_at` to `HEAD`.

## 5. Gaps to Linear

Each hook gap that needs a simulation or wire change (a new event, a
payload field, a `NOT_SENT` entry) is filed with the `linear-file` skill as
its own issue (labels: Improvement plus gameplay or server; parent BB-102),
one issue per change, naming the interactions it unblocks.

## 6. Report

Rebuild the map: `just sound-map` (writes `target/soundmap/index.html`).
Tell the user how many interactions, cues and gaps were added, changed or
retired; the new critical and high cues; the issues filed; and what
`sound-gen` would make next, priced at 3.3 credits a second of audio times
the takes. If the user wants a link, update the published map
(`https://claude.ai/artifact/M7rPUDzeCmDtk7keh34WEP`, read it first): build
it with `--audio-base audio/ --html <scratchpad>/sound-map.html`, strip the
`<!doctype>`, `<html><head>`, `</head><body>` and `</body></html>` wrapper
lines (the Artifact tool adds its own), and publish with `files` mapping each
`audio/<bus>/<file>.ogg` to `static/audio/...` (`contentType: audio/ogg`).
