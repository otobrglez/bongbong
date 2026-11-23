---
name: sound-gen
description: Make, replace or re-pick bongbong's sounds with ElevenLabs through the MCP - planned samples in audio/cues.toml become generated, measured, pre-picked, processed takes in static/audio, with an audition page for the owner. Use when the user says "generate sounds", "make the sounds", "sound-gen", "regenerate <sound>", "apply my picks", or after sound-audit planned new samples.
---

# Sound generation

The pipeline of docs/sound-system-prd.md section 4.12. The manifest
(`audio/cues.toml`) is the source of truth: a `[sample.<id>]` holds the
recipe (prompt, `duration_seconds`, `prompt_influence`, `loop`) and, once
made, its takes with their provenance. You cannot listen: you pick by
measurement and the owner reviews on the audition page.

## 1. Choose the batch

Samples with `status = "planned"` or `"rejected"`, or the ones the user
names; for a rejected one, fold the owner's note (step 6) into its prompt.
Order by the needs in the sound map (critical and high first). A batch is at
most 20 samples. Prompt = the `[style].prefix` + ": " + the sample's prompt;
keep it one sound, concrete, no visual words, "no music" (the model guide:
`creative_get_model_guide eleven_text_to_sound_v3`).

## 2. Price, then run

1. `creative_create_flow {name: "bongbong sfx <date> <batch>"}`.
2. For each sample, `creative_add_flow_node {flow_id, node_type: "sfx",
   model_id: "eleven_text_to_sound_v3", label: "<sample id>", prompt,
   model_parameters: {duration_seconds, prompt_influence, loop}}`.
3. `creative_run_flow_nodes {flow_id, node_ids, estimate_only: true}`. The
   estimate runs about three times the real charge (real: 3.3 credits a
   second of audio per take). Above 2,000 estimated credits, or past the
   budget the owner set, ask first.
4. `creative_run_flow_nodes {flow_id, node_ids}` - 4 takes each by default.
   Never call it again to retry: every call is a new charge. The account's
   queue holds about 60 generations: run at most 14 nodes a call, and the
   next call only once the last one's sessions are all done, or the takes
   past 60 fail with a 429 (lost, not charged).
   The status replies are large (about 25k characters per 20 takes): for a
   batch of many samples, hand the run, the polling and the fetch to a
   subagent with these rules and keep only its report.
5. Write `tools/audio/batches/<date>-<batch>.json`: `flow`, `name`, `model`,
   `estimate_credits`, `nodes` (node id -> sample id) and the run's
   `session_ids` (see `2026-11-22-core-1.json`).

## 3. Fetch

Poll `creative_get_flow_run_status {flow_id, session_ids}` (at most 20
sessions a call; its reply is large - ask in chunks) until `all_completed`.
Write each take as a line `<sample>\t<generation id>\t<content_url>` to a TSV
in the scratchpad - the URLs are signed and expire - then:

```
python3 -I tools/audio/fetch.py <takes.tsv> ...
```

Takes land in `tools/audio/_takes/<sample>/<generation>.mp3` (gitignored;
the flow keeps them).

## 4. Measure, pre-pick, process

```
python3 -I tools/audio/process.py --batch tools/audio/batches/<batch>.json [sample ...]
```

It measures every take (onset, crest, clipping, a loop's seam and
steadiness), keeps the best (2 per sample, 1 for UI and warnings), trims and
fades one-shots, makes loop seams seamless, levels each to its bus's target
RMS under a -1 dB peak ceiling, writes 48 kHz Ogg Vorbis to
`static/audio/<bus>/<sample>_<n>.ogg`, and writes the takes and their
measurements into the manifest with the sample `approved`.

## 5. Check

```
cargo test --lib --no-default-features --features dev-tools audio
cargo run --bin soundmap --no-default-features -- --check
just audio-render 42 advance
```

The render (`probe --audio-out`) is a seeded round's sound, for a listen
without a window.

## 6. Audition

```
python3 -I tools/audio/audition.py --batch tools/audio/batches/<batch>.json \
  --urls <urls.json> --out <scratchpad>/sound-audition.html \
  --credits "<spent>" --round-url round.mp3 --round-caption "<what the render is>"
```

`urls.json` maps each generation id to `takes/<generation>.mp3`. Publish the
page with the Artifact tool, its `files` mapping each `takes/<id>.mp3` (and
`round.mp3`, the render as MP3) to its file with `contentType: audio/mpeg`,
`capabilities: {db: {}, user: {}}` - to the existing audition artifact
(`https://claude.ai/artifact/EPzvTHYuPMvt2sZwfvmdDr`) by its URL, read
first. The owner's choices land in its database, collection `verdicts`, one
document per sample: `ship` (the generation ids to ship), `verdict`
(`keep` or `again`), `note`.

## 7. Apply the owner's choices

`ArtifactData list` of `verdicts`. For each sample:

- `ship` differs from the manifest's takes: run `process.py` with those
  takes first (edit the metrics order or pass the sample alone after moving
  the chosen takes up) so exactly the chosen generations ship.
- `verdict: again`: set the sample `rejected`, fold the `note` into its
  prompt, and it goes into the next batch.

Then commit the manifest, the Ogg files and the batch file together.

## Music and voice

Music: `node_type: "music"`, `model_id: "eleven_music_v2_5"`,
`model_parameters: {duration_seconds: <bars x 4 x 60 / bpm>, instrumental:
true}` - only once the owner has heard the effects (budget order, PRD
section 8). The bong's babble is sound effects (`bong_babble`); the
announcer is `tts` with a designed voice (`creative_design_voice`, then
`creative_save_designed_voice`), coloured by a free `audio-effects` node
(`filter_preset_id: "old_radio"`).
