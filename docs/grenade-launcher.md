# Grenade launcher

BB-21. A special weapon from its own crate (`pickup = "grenades"`), one at a
time like every other (`Tank::take_weapon`): a crate loads
`grenade_ammo_per_pickup` (6) grenades into the launcher's drum, a re-pick
stacks six more up to `grenade_ammo_max` (12; the drum shows one crate's
worth, full while spares are carried), another weapon's crate replaces it. Player-only - an enemy
drives over the crate and leaves it where it is (`Tank::wants_pickup`).

## How it plays

- **One per press.** The trigger lobs a grenade on the press edge, like a
  shell; `grenade_reload_seconds` between launches. It leaves just clear of
  the hull along the gun line at `grenade_launch_speed`, plus
  `grenade_launch_carry` of the hull's own velocity, so a tank driving
  forward throws further.
- **It is lobbed.** Up at `grenade_launch_climb` from
  `grenade_launch_height`, brought down by `grenade_gravity` - about 105 px
  out and 36 px up at the defaults. In the air it flies over walls, props,
  trees, towers and tanks; only the field's edge stops it. It comes down,
  hops (`grenade_ground_bounce` of its fall while that is faster than
  `grenade_hop_min_speed`), keeps `grenade_landing_keep` of its speed each
  landing, and rolls. It is drawn lifted by its height and a little bigger
  near the top, its shadow on the ground, tumbling end over end in the air,
  and trails a short, thick plume of shaded white smoke
  (`grenade_trail_*`) while it flies or rolls fast.
- **It rolls.** A steel canister - black cap, red band, brass lever, the
  tank modules' materials - `grenade_radius` (12) px, lying across the way
  it rolls with the lever turning round it, slowing by `grenade_roll_drag`
  a second until it rests under `grenade_stop_speed`.
  Water drags it `grenade_water_drag_factor` times as hard; ice hardly at
  all (`grenade_ice_drag_factor`).
- **It bounces** off every tile, tree, tower and the field's edge, keeping
  `grenade_wall_restitution` of its speed into the face it struck.
- **Tanks push it.** A hull - anyone's, wrecks too - that overlaps a
  grenade puts it out on its shallower side and hands it the hull's own
  motion, keeping `grenade_tank_restitution` of the speed into it. A tank
  driving into a resting grenade knocks it away faster than it drives; a
  parked one is a wall. A grenade never moves a tank.
- **It flashes, then goes off.** Its band burns white-hot, from
  `grenade_blink_hz_start` to
  `grenade_blink_hz_end` across `grenade_fuse_seconds` (6), so how fast it
  flashes says how soon it goes. Then a blast: `grenade_blast_radius`,
  falloff damage to the side opposing whoever launched it, a shove for
  everyone in range, the opposing frog hurt, tiles cracked, drums set off,
  crates broken - and a shockwave (`grenade_shock`, with the screen flash).
  Kicking a grenade back at its launcher only shoves them.

## Where it lives

| | |
|---|---|
| `src/grenade.rs` | the ball: `Grenade::launch`, `roll` (drag, hulls, the sweep), the blink, `draw_grenade` |
| `src/simulation/grenades.rs` | `roll_grenades`, `resolve_grenades`, `grenade_show`; the blast shares `side_blast` with the missiles |
| `src/simulation/weapons.rs` | `fire_grenade`, the dispatch arm |
| `src/tank.rs` | `grenade_ammo`, `ActiveWeapon::Grenades`, the drum module's cell |
| `net/wire.rs` | `GrenadeState` (protocol 14), `WireEvent::GrenadeBlast` |

No physics body: rapier is never told about a grenade, so a round with none
steps, draws RNG and replays exactly as before. No RNG of its own; the
blast's damage rolls are drawn only for what is inside its radius.

## Online

The room rolls every grenade; a client draws it. `GrenadeState` carries the
position and the fuse in hundredths of a second (a timer's tenths would jerk
a blink that quickens to ten a second); the replica interpolates the
position, runs the fuse down between snapshots (`tick_presentation`), turns
the lamp by how far its copy moved, and puts on the blast's show off
`GrenadeBlast`. The height travels too (quarter pixels), so the arc is the
room's. Nothing is predicted - like the missile pod, a grenade is the
room's. The launch kick travels as `Shoved`, since a client that owns its
hull draws no grenade to kick it by.

## Art

The launcher is a drum of four grenades with a short wide barrel, on the
missile pod's roof hardpoint (a tank carries one special at a time); its
five cells (`TANK_MODULE_GRENADE_COL` + empty chambers, the six rounds
shown over four chambers rounded up) come out of
`tools/spritegen/tankdesign` with `tank_art::GRENADE_MUZZLE`. The crate is row
12 of `gen_crates.py`'s sheets: four canisters standing in two rows, each
with its band lit, in the orchid ink (`punypalette.PICKUP_INK['grenades']`),
which the HUD and the ammo pips share. The design was picked from six
studies (fuse bomb, frag, hazard ball, brass round, ember core, canister).
