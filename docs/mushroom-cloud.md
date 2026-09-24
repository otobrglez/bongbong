# Mushroom cloud

![Four mushroom clouds](mushroom-cloud.gif)

A dying tank goes up in a mushroom cloud in `wreck_mushroom_chance` (0.7)
of kills; the rest get the sheet's reference fireball (`blast::BlastFx::new`).
The pick is a salted hash of the kill position (`BlastFx::wreck`), so no
RNG is drawn and a seeded replay is unchanged. Barrels never use it.

## Composed, not baked

`mushroom.rs` builds the cloud at draw time: nothing comes from a sprite
sheet, and time is continuous, so it moves every frame.
`Cloud::compose(base, time)` is a pure function returning `Shape`s in
painting order. `mushroom::draw` paints them through a `Canvas` (the
game's `GpuCanvas`, or a `CpuCanvas` headless):

- `Puff`: a disc of whole 2 px blocks, **shaded, not outlined**. A dark
  body, a lit side up and to the left, and while it burns a fire core
  that shrinks as it cools.
- `Mark`: single blocks, for rings, arcs, debris trails and the lens streak.
- `Glow`: a translucent disc, the bloom over whatever still burns.

The look aims at test-range footage rather than a cartoon:

| Element | When | What it does |
|---|---|---|
| flash | ~0.14 s | white-hot disc, cyan halo, a horizontal lens streak |
| arcs | first ~0.6 s | jagged electric arcs off the hull, re-shaped every tick so they flicker |
| shock ring | first 0.5 s | a thin ionised ring racing out along the ground at the view's tilt, a fainter echo behind it |
| dust | after the ring | pale puffs rolling out behind the shock, split so the front half covers the stem |
| stem | throughout | narrow; puffs ride up it on a conveyor, flared at the foot, burning longest at the bottom, breaking up bottom-first |
| cap | throughout | a ring of puffs rolling over itself like a smoke ring, depth-sorted; fireball first, then heavy smoke paling from the top while its belly keeps a warm translucent glow |
| condensation | early | a dotted pale collar around the stem and a shell over the cap, gone in a moment |
| debris | first ~1.4 s | hot fragments and dark chunks streaking away on ballistic arcs |

Fire and smoke use the palette's steps (docs/PALETTE.md). The ionised
light (white, pale cyan, electric blue) is deliberately off the palette,
like the plasma sheet. Fades are stepped to eighths, and puffs dissolve
by shrinking at hashed moments.

## Variation

`Cloud::new(seed, scale)` hashes the life, height, cap size, lean, roll
speed, stem width and flow, and the puff counts for the cap, dome, skirt
and embers from the blast's seed. Every puff then hashes its own place,
size, cooling and dissolve time. Knobs: `mushroom_seconds`,
`mushroom_height_px`, `mushroom_cap_px` (each jittered per kill).

## Preview

`cargo test --lib mushroom::tests::preview -- --ignored` writes
`target/mushroom_preview.png` (six clouds over their life) and
`target/mushroom_frames/*.png` (four clouds at 30 fps, for a GIF).
