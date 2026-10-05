# The volcano, lava and lamps

A volcano stands where a map puts its crater, wakes every half minute,
throws lava bombs round itself and at anyone near, and sends rivers of lava
across the field. Lava is a burning ford a tank can wade at a cost, and a
deep lake it cannot. Night falls on the map that asks for it, and lamps -
posts the map places and lanterns the players set down - light the way, and
show whoever stands in their light to the enemy. A heat shield keeps every
kind of heat off a tank for a while.

The level that shows the volcano and its lava is `maps/vulkan.toml`, level 2,
fought under a clear sky with no lamp posts (a dark sky and its lights
were too much on top of the eruptions); no shipped map places a lamp post
or `nightfall` now, so the lamps and the night are tried on a map of
one's own (`weather`, `nightfall`, `kind = "lamp"`).

| Where | What |
|---|---|
| `src/volcano.rs` | the cycle (`phase`), a bomb in flight (`LavaBomb`), the cone's picture and the eruption's composers |
| `src/lava.rs` | `LavaLayout`: the lava's shape, flow and heat, built from the map alone; the drawing of lava, its banks and a bomb's pools |
| `src/lamp.rs` | a lamp post's and a lantern's drawing, and `Lantern` |
| `src/simulation/volcano.rs` | the world half: `build_volcanoes`, `volcano_phase`, `tick_lava_bombs`, `lava_phase`, lanterns, nightfall, `eruption_show` |
| `src/weather.rs` | the light the lava, the craters, the bombs and the lamps throw (`lights_in`), the light sight rule (`Game::sight_on`), the dusk falling into night (`Game::look`) |
| the `volcano` tuning group | every number below |

## The map

| Key | What |
|---|---|
| `kind = "volcano"` | the crater cell. The cone covers the cells within `sqrt(5)` of it (`volcano::FOOTPRINT_RADIUS2`, 21 cells), drawn as one picture; those cells are permanent solid tiles (`Material::Volcano`) a shot stops on, nothing breaches and the nav grid walls off. A cell the map already fills keeps its object. |
| `kind = "lava"` | a lava cell, painted like water |
| `kind = "lamp"` | a lamp post: a tile one hit puts out (`lamp_max_health`), not solid to sight |
| `pickup = "heat_shield"` | the heat shield pickup, players only |
| `nightfall = 90.0` | seconds of the round when night falls (top-level, optional) |

The builder carries all four: `lava`, `volcano` and `lamp post` in the
Ground category, `heat shield` among the pickups.

## Lava

**The shape is water's.** `LavaLayout` reads the lava cells through
`ground::WaterLayout`, so a line one cell wide is a stream and the open
middle of a block three or more wide is a lake; lava never freezes and
never joins water, and a road cell in a river's line is a stone crossing
over it. The ground under it is drawn as road, so the grass tileset gives
way round it.

| Depth | What it does |
|---|---|
| `Shallow` - a stream, a lake's shore | a **burning ford**: a hull keeps `lava_speed_factor` (0.5) of its pace and `lava_grip_factor` (0.7) of its grip, burns at `lava_damage_per_second` (24) and keeps burning `lava_afterburn_seconds` (1.5) after it climbs out. The router charges `lava_ford_path_cost` (14) a step, so an enemy goes round by a crossing when one is anywhere near. |
| `Deep` - a lake's open middle | a wall to hulls (seam-closed static colliders, blocked in the nav grid) and nothing to shots, as deep water is. The pose validator refuses a client hull reported in it. |

Nothing spawns on a lava cell, a start falling back to the map's centre is
moved off it (`dry_cell_near`), and PLAY HERE never drops a tank on it.

**Flow** runs away from the volcano: every lava cell beside a cone is a
source, and a walk along the lava gives every cell its distance downstream
and its direction. A run that touches no cone - one a crossing cuts off, or
one painted on its own - flows from its end nearest a volcano, or with none
on the map from its northernmost end. The picture's bands run along it at
`lava_flow_speed`, twice that in an eruption's surge.

**Heat** radiates from every lava cell: 1 in the lava, `lava_heat_falloff`
(0.55) a cell out - by the larger of the two offsets -, its square two out,
and so on to four. Heat over `heat_hurt_from` (0.5) burns a hull by how far
it stands over it, so a bank burns at a tenth of the lava's rate and the
cells past it not at all. Heat scorches the banks (baked once into a
`BlockImage`, as the floor shade is), lights the ground at night, and shows
a tank standing on it to the enemy (below).

None of this draws RNG, and a map without lava runs none of it.

## The volcano

**The cycle** is a pure function of the round clock (`volcano::phase`):
asleep, then a rumble of `volcano_rumble_seconds` (4), the eruption for
`volcano_erupt_seconds` (5) and the cooling for `volcano_cool_seconds` (7),
every `volcano_period_seconds` (30), the first rumble at
`volcano_first_rumble_seconds` (14) - each volcano on a map offset from it by
a hash of its cell. A replica's clock stands on the room's tick, so it
rumbles and erupts on the same tick with nothing on the wire.

- **The rumble** brightens the crater, darkens the plume, trembles the
  ground (a small shock) and pulses the volcano's off-screen arrow and its
  frame on the minimap, amber.
- **The eruption** opens with a flash, a shock ring and a screen flash
  (`volcano_shock_scale` of a kill's), throws a fountain of fire and
  lightning from the crater, surges the rivers - and sets alight the
  flammable tiles on their banks, a hash of each cell picking half of them -
  and throws `volcano_bombs_per_eruption` (9) lava bombs over its length.
  The arrow and the minimap frame turn red.
- **A lava bomb** flies `volcano_bomb_flight_seconds` (1.9) from the crater
  in an arc, a ring of warning marks on the ground where it will land the
  whole way. `volcano_bomb_aimed_share` of them are thrown at a seat within
  `volcano_bomb_range_px` - landing a hashed step off it, always inside that
  seat's sight box -, the rest at a spot round the crater no nearer than
  `volcano_bomb_min_range_px`. Where it lands it bursts through the drums'
  blast path as `Drum::Lava` (`volcano_bomb_radius_px`, the damage between
  `volcano_bomb_damage_min` and `_max`, `volcano_bomb_knockback`; both sides
  and every tile), splashes a pool of burning lava over the cells within
  `volcano_pool_radius_cells` for `volcano_pool_seconds`, and breaks any
  lantern near its middle.

Every target is a hash of the eruption and the bomb's number, never a roll:
the only RNG the volcano draws is a blast's damage roll, as a drum's.

## Lamps and the night

`weather = "dusk"` with `nightfall = 90.0` is dusk, the light easing into
night's over the `nightfall_seconds` (25) before ninety, and the rules
turning to the night's at ninety itself (`Game::
tick_nightfall`, which keeps the map's own key, so the round still counts as
played as authored).

**The light sight rule.** Night shortens how far an enemy sees
(`night_sight_factor`). Whoever stands within `lamp_reveal_px` (110) of a
standing lamp post or a lantern, or on ground the lava makes at least
`lava_reveal_heat` (0.3) hot, an enemy sees at the full `enemy_view_range`:
light cuts both ways. Under a clear sky, or with nothing that shines, sight
is what it always was.

**Lanterns.** In a round whose sky is dark (night, a storm, fog, dusk) or
whose night will fall, each seat has `lamps_per_seat` (3) lanterns: Enter
(or keypad Enter) for player 1, E for player 2, the LAMP button on a touch
screen. A lantern stands where it was set for the rest of the round, lights
the ground round it at night and shows whoever is near it to the enemy; a
lava bomb landing on it breaks it. The HUD's vitals carry the count and,
while a heat shield is up, its gauge.

## The heat shield

`heat_shield_seconds` (10) of no heat at all: no lava or bank damage, no
afterburn, no burning ground, no flamethrower cone - and a burning tank
that collects one is put out. A red-over-black ring round the hull shows it.
Its supply crate carries a shield painted molten red over black basalt
(docs/CRATES_SPEC.md).
Enemies never collect it.

## Drawing it cheaply

Lava is drawn in 2 px blocks, a stream's cell alone a hundred and more of
them with its flow bands, crust and glow, and a dark sky over it adds a
light map full of lamps. Drawn block by block every frame, Vulkan (then
at dusk falling into night) cost a release build twice the frame of any
other level. Nothing on screen changes faster than a few frames, so the
picture is kept between frames and a frame draws a quad a cell:

- **The lava** (`lava::LavaPictures`, brought up by `Game::refresh_pictures`
  before the frame's textures are synced): each cell's blocks keep what is
  fixed about them (`BlockGeo`: which block, its crust and plate, its
  place along the flow), and the colour is worked out from the clock. A
  cell is baked into a tile of a `canvas::TileAtlas` (16 x 16 texels, a
  texel a block) and its bright blocks into a second atlas the night draws
  over the dark field; a quarter of the tiles on screen are baked each
  frame (`STAGGER`), so a tile is never more than three frames old, and a
  clock that jumped (a dev-server step, a replica catching up) bakes them
  all. Tiles off every view wait their turn.
- **The bombs' pools** keep their metaball shape until a pool comes or
  goes, and are recoloured as they cool.
- **The light on the banks** by day is baked once at a surge's strength
  and drawn fainter when calm, added to the colour alone
  (`Game::draw_lava_ground_light`, a separate colour and alpha blend):
  an additive blend would add its alpha to the scene target's too.
- **The cones** are baked once per set of outlets, outline and shadow
  included (`volcano::ConeImages`), and each crater's molten pixels in
  turn.
- **The night's light map** (docs/weather.md, "Lights") is drawn a texel a
  2 px block, an unblocked light as one soft disc and a lamp post's or a
  lantern's shadowed pool kept in an atlas until a wall near it falls.

A painter draws from the pictures only on the frame they were brought up
to (`LavaPictures::fresh`) and with their texture uploaded
(`Canvas::has_blocks`); anything else - a thumbnail, the builder, a test -
draws block by block, the same picture, and the tests hold the two to
each other texel for texel. On the GLES builds (the web, iOS, Android)
raylib's draw batch is raised to the desktop's 8192 quads
(`render::batch`), since ES 2's default flushes four times as often.

In a release build at 1920 x 1080 (llvmpipe, uncapped), a frame of Vulkan
under that sky went from 16.8 to 9.0 ms at dusk, 16.3 to 7.6 at night and 27.6 to 12.9 at
the worst of an eruption; another level's frame is about 7.5 ms. A
dev-tools build times each stage of the frame (`frame_stages.rs`: the
steps, the particles, the pictures, the lights, the world, the post pass,
the chrome and the swap) - `status.frame_stages`, the line under the left
cluster with `ui_frame_stages` on (a PR preview's tuning panel, on a phone
too) and a phone's `FrameStats` log line.

## Online

The cycle needs nothing on the wire. What does travel:

- `IntentMsg::lamp` - a held key like the trigger, merged press for press by
  the room's mailbox.
- `LampState` - every lantern, a keyed family with deltas, so a late join
  sees them.
- `FireState::lava` - a burning cell that is a bomb's pool.
- the `HEAT_SHIELD` tank flag.
- `WireEvent::LavaBombLaunched` (the bomb in the air on the replica),
  `LanternSet`, `LanternBroken`.

`net::PROTOCOL_VERSION` is 12.

## Not done

The heat shimmer over the lava (a haze bending rows of the field, the way
`weather_sky.fs`'s heat haze does, but only over hot cells) is not drawn:
the sky pass has no cell mask and runs only under a sky that asks for it.
The MAP panel has no `nightfall` row; the key is set in the TOML.
