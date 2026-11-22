# PRD: bongbong sound

Status: agreed with the owner 2026-11-22 (section 8) and being built in one
PR for BB-102: desktop and the web first, mobile as a follow-up. This
document supersedes the October research write-up
([Bongbong Sound Plan](https://claude.ai/artifact/2nMF6XMiVHtujdjhhyNX6B)).
Its input data is BB-199's interaction map
([Bongbong Sound Map](https://claude.ai/artifact/MstM43AFoX2ibaQKAwbhEZ)).

Contents

1. The decision, in one paragraph
2. Goals and non-goals
3. What we have today (verified 2026-11-22, master `fd5fd56`)
4. Design, by area
   - 4.1 Architecture
   - 4.2 Where sound hooks into the frame
   - 4.3 Space: scaled realism
   - 4.4 Sources: tanks, weapons, impacts, explosions, the world
   - 4.5 The mix
   - 4.6 Adaptive music
   - 4.7 Voice: the bong and the announcer
   - 4.8 Data: the cue manifest
   - 4.9 Coverage that compiles
   - 4.10 Online
   - 4.11 The sound map: a living graph
   - 4.12 Generation: ElevenLabs through the MCP
   - 4.13 Platforms
   - 4.14 Dev tooling and tests
5. Numbers
6. Phased plan
7. Risks, mitigations, stop conditions
8. Decisions
9. References

- Appendix A: the `sound-audit` skill
- Appendix B: the `sound-gen` runbook
- Appendix C: the first slice and the direction audition

## 1. The decision, in one paragraph

Sound is a presentation layer, the way `fx.rs` is.

- **It only reads.** An `Audio` owned by `app.rs` reads the round each step and each frame (the replica online), and never writes it. It draws no round RNG, so seeds, replays, the probe and the room server are untouched.
- **A director decides what plays.** It is headless pure Rust and turns events and watched state into commands for a small, fixed set of voice parameters: which sample, gain, pan, pitch, high-cut, start delay and reverb send. It is the place for the physics:
  - where a sound is, how far, what stands between and how fast it moves;
  - how heavy the chassis behind it is and what it hits or rolls on.
- **A pure-Rust DSP engine plays it.** The same code runs on every platform and headless, so a seeded round can be rendered offline to a WAV and pinned in a test. One raylib `AudioStream` carries the engine's output on desktop, the web, Android and iOS.
- **Every sound file is generated with ElevenLabs through the MCP.** Like the sprite sheets, the picked files are committed, together with their recipe and provenance in one manifest, `audio/cues.toml`.
- **The game cannot drift silent.** An exhaustive `match` over `Event` and over every enumeration (chassis, weapons, materials, skies, biomes) fails the build until each has a sound decision. A `soundmap` tool turns the code, the manifest and a curated catalogue of interactions into one graph that shows what is needed, what is hooked and what exists.
- **Two skills keep it current:** `sound-audit` after a feature lands, and `sound-gen` to make or replace sounds.
- **The rest:**
  - Adaptive music and a voice for the bong and an announcer are in scope.
  - The space model is scaled realism: real behaviour, with every exaggeration a tuning knob.

## 2. Goals and non-goals

Goals

- **G1. Physical.** Every sound is placed by:
  - where it is (pan) and how far away (gain, high-cut, a short delay);
  - what stands between (occlusion) and how fast it moves (doppler);
  - how heavy its source is (chassis mass, calibre, blast size);
  - what it hits or rolls on (material, surface).
- **G2. Readable.**
  - The player's own tank and every threat cut through the mix.
  - A warning is never culled.
  - A drum chain or a minigun burst never swamps the mixer.
- **G3. Presentation only.**
  - Sound never mutates `Game` and never draws `RoundRng` (or `rand::rng()`; §4.1 D6), so replays and determinism are untouched.
  - Online play hears the replica, plus the player's own press at once.
- **G4. One sound everywhere.**
  - The same engine and mix run on desktop, the web, Android and iOS.
  - The probe and the dev server can render a seeded round's audio headless.
- **G5. Living.**
  - A new `Event`, chassis, weapon, material, sky or biome fails the build or a test until it has a sound decision.
  - The manifest, the sound map and the generation status stay current with the code.
  - Making, replacing or retuning a sound is one skill run or one manifest edit, and hot-reloads in a dev build.
- **G6. Generated and traceable.**
  - Every shipped file comes from ElevenLabs through the MCP.
  - It records its prompt, model, parameters, flow and generation id, the date and the plan it was made on.
  - The plan must grant commercial rights.

Non-goals

- HRTF or binaural 3D.
- Voice chat between players.
- Procedural synthesis as the main source; it is used only for the dev-build placeholders (§4.8).
- A different mix per platform.
- Sound on the room server.
- Captions and haptics: considered and left out of this PRD; each can be its own issue later, fed from the same director.

## 3. What we have today (verified 2026-11-22, master `fd5fd56`)

- **No sound at all.** There is no audio code, no audio crate in `Cargo.lock` and no audio file under `static/`. `docs/air-support.md:404` says so.
- **The hook already exists.** After every simulation step, five readers take the step's events before the next step clears them (`src/app.rs:2528-2534`):
  ```rust
  let mut after_step = |game: &Game| {
      self.fx.observe_events(game);
      self.follow.observe_events(game);
      self.awareness.observe_events(game, &seats);
      self.feedback.observe_events(game, &seats, &tuning(), hold);
      self.voice.observe(game);
  };
  ```
  - Each reader dedupes on `game.frame()` and resets on a lower frame or a new seed (`src/fx.rs:902-912`).
  - Per frame, `fx.set_views` and `fx.observe` (`src/app.rs:2772-2776`) sample the world: `Fx::sample_world`, `src/fx.rs:1396`, and its 20 state watchers.
  - Online, the replica is read once per frame (`src/app.rs:2787-2788`), and `voice` is not read there.
  - `fx_runs` (`src/app.rs:127`) is the one switch that freezes presentation under a dialog, a pause or the builder.
- **The `Event` enum** (`src/simulation/mod.rs:480`) has 87 variants.
  - `WireEvent::from_event` (`src/net/events.rs:349`) is an exhaustive `match` with no wildcard, so a new variant fails to compile until it is classified. Sound copies this pattern.
  - 20 variants are on `NOT_SENT` (`src/net/events.rs:37`). `MissileLocked` is dropped too but missing from the list (BB-201).
- **BB-199's map:** 150 entities, 444 interactions, 319 cues (82 loops) and 156 gaps.
  - **220 interactions have no event:** engine and treads, slams and scrapes, charges, fuses, fires catching.
  - `Fired { slot, weapon }` (`src/simulation/mod.rs:485`) is one per trigger pull and carries no position.
  - `Hit` has no shooter, `Wreck` no cause, and `Ricochet` no surface.
- **Physics, read-only.** Physics is the input for G1.
  - **`Game::contacts()`** (`src/simulation/mod.rs:6040`) gives each hull's hardest contact per step: slot, point, `max_impulse` and whether it is touching a tank. A replica has none.
  - **Slams.** `Fx::watch_slams` (`src/fx.rs:851`) and `slam_strength` (`src/fx.rs:2133`) already turn impulses into slams. The thresholds are the `ram_fx_*` rows (`src/tuning.rs:3436`); a light chassis hitting a wall at full speed is about 650.
  - **Mass.** `tank_mass_factor` (`src/tuning.rs:3473`) runs 0.77 (scout, wraith) to 1.89 (titan); `Tank::mass` is `scale² × factor`, so 4.0 for a standard tank and about 7.5 for a titan.
  - **Calibre.** `tank_damage_factor` (`src/tuning.rs:3481`) runs 0.75 to 1.60. `Tank::twin_gun()` is at `src/tank.rs:1604`.
  - **Tread state.** `Game::driving()` (`src/simulation/wear.rs:86`) gives each hull's tread state: `Surface` (Grass, Road, Sand, Ford, Deep, Ice; `src/wear.rs:157`), `Roll` (Rolling, Sliding, Pivoting; `src/wear.rs:247`), measured speed, `speeding_up` and `wet`. `tank.throttle` is at `src/tank.rs:1069`.
  - **Blasts.** Radii run 28 px (jet cannon) to 208 px (the rod's shove), through `Game::blast` (`src/simulation/combat.rs:394`). The fx shock ladder (`src/tuning.rs:4564-4601`; rod 1.5, kill 1.0, grenade and fuel 0.9, barrel 0.7 ... EMP 0.25) is a ready loudness ladder.
  - **Speeds.** Shell 522 px/s, minigun round 596, missile 345, helicopter up to 320, jet 264 (`Game::aircraft()`, `src/simulation/air_support.rs:328`).
- **Materials.** `Material` (`src/obstacle.rs:74`) has 24 variants and no `ALL`. `Material::blocks_sound` (`src/obstacle.rs:505`) already exists for the sonic hammer: walls, towers, the volcano and doors, glass included.
  - `weather::Occluders::reach` (`src/weather.rs:560`) is an Amanatides-Woo cell walk over 32 px cells, the base for occlusion.
  - `Game::sound_blocks` (`src/simulation/sonic.rs:229`) is the hammer's own blocking grid.
- **Weather.** `Game::look()` (`src/weather.rs:218`) gives the rain, snow, fog, sand and lightning intensities per frame. `weather::gust_at` (`src/weather.rs:348`) and `weather::lightning` (`src/weather.rs:471`) are pure functions of the round clock, so a client can compute them too.
- **Places.** `Theme` (Grass, Desert; `src/map.rs:322`), `Weather` (`src/map.rs:397`) and `Biome` (Valley, Floodplain, Karst; `src/biome.rs:40`) each have `ALL`.
- **The listener.** `follow::Shot` gives the camera centre and the seat per view, `ShotKind` (`src/follow.rs:471`) is Seat, Shared, Split, Spectating or Nobody, and `seat_anchor` is at `src/follow.rs:354`.
  - Map sizes run 8 to 250 cells a side (256 to 8000 px). A followed field shows about 34 × 17 cells.
  - Units are pixels throughout. A hull is 28 to 52 px long, so about 5 px is a metre.
- **sola-raylib 6.4.1** (from crates.io, not `../sola-raylib`) wraps raylib's audio. Its gaps and bugs are listed under D2.
  - `AudioStream::update` with `is_processed()` is safe and usable.
  - raylib's decoders are WAV, OGG, MP3 and QOA; FLAC is off.
- **Web.** No ASYNCIFY, and none should be needed: see D4. The page's audio-unlock hook resumes a `Module.audioContext` that nothing sets (`site/src/scripts/runtime.ts:27, 58-69`).
- **iOS.** raylib is built with `-DMA_NO_COREAUDIO` (`tools/setup_ios.sh:104-114`), so it has no audio backend.
- **Android.** AAudio and OpenSL ES are available. raylib leaves audio playing when the app loses focus (`rcore_android.c:1137-1142`).
- **ElevenLabs connector.** It is read in this session; no credits were spent.
  - **Sound Effects:** the `sfx` node type. The default model is `eleven_text_to_sound_v3`; `eleven_text_to_sound_v2` remains. Its parameters are `duration_seconds` 0.5–30, `prompt_influence` 0–1 (default 0.3) and `loop`.
  - **Runs:** up to 4 takes a run and 20 nodes a batch. `estimate_only` prices a run without spending.
  - **Other nodes:** `music` (`eleven_music_v2_5`, 3–600 s, instrumental), `tts`, voice design, and a free `audio-effects` node (radio and phone filters, rooms, distance).
  - **Results** come back as signed URLs through `creative_get_generations`.

## 4. Design, by area

### 4.1 Architecture

```
Game, read-only: events() per step, state per frame
  (session.shown() is the replica online)
   │
   ▼
audio::Director (headless, pure Rust, no raylib)
   bind.rs     Event -> Binding, an exhaustive match with no wildcard
   watch.rs    state watchers (edges, continuous drivers), each a WatcherSpec
   emitters.rs persistent loops keyed by source (engine, flame, fire cluster, rain)
   space.rs    listeners; distance, pan, high-cut, delay, doppler, occlusion
   source.rs   chassis engine model, impact strength, blast size class
   mix.rs      priority, HDR window, voice budget, polyphony, cooldown, aggregation, ducking
   music.rs    intensity and the music state machine
   speech.rs   the announcer and bong voice queue
   │  Vec<Command>: Start, Update, Stop, Bus, Duck, Stinger
   ▼
audio::dsp::Engine (pure Rust, Send, deterministic f32, libm for transcendentals)
   voices: cubic resampler (pitch x doppler x rate), SVF high-cut and low-cut, sample-accurate
   delayed start, equal-power pan, reverb send, gain ramps
   bus tree; sidechain ducking; 4-line FDN reverb; lookahead limiter; DC blocker
   │  f32 stereo frames
   ▼
Drivers
   render/audio.rs   one raylib AudioStream (desktop, web, Android, iOS)
   null              room server, probe without --audio-out
   wav               probe --audio-out, dev tool audio_render, tests
```

**Modules.** Headless code goes under `src/audio/` (`mod.rs`, `bind.rs`, `watch.rs`, `emitters.rs`, `space.rs`, `source.rs`, `mix.rs`, `music.rs`, `speech.rs`, `cues.rs`, `dsp/{voice,filter,resample,reverb,limiter,bus,decode,wav}.rs`). The raylib half is `src/render/audio.rs`, under `render` as CLAUDE.md requires. The `soundmap` binary is headless, in `src/bin/soundmap.rs`.

**D1. An engine of our own, not raylib `Sound` voices.**
- raylib's `Sound` has volume, pitch and pan only. It has no per-voice filter, no scheduled start and no send, and so no occlusion, distance high-cut, propagation delay, reverb or ducking.
- A pure-Rust engine compiles headless. That makes the offline render and the pinned render hash possible (the mapshot pattern, §4.14), and gives the same output on every platform.
- It is about 2,000 lines of well-known DSP: a Hermite resampler, a Chamberlin SVF, an FDN, and a lookahead limiter.

**D2. The driver: one raylib `AudioStream`.**
- **P1: pushed.** The engine renders on the main thread into a raylib `AudioStream` through the safe `AudioStream::update` whenever `is_processed()`. The stream uses two sub-buffers of 1,024 frames at the device rate, about 40 ms ahead. A main-thread hitch longer than the buffer underruns; raylib plays silence.
- **The alternative: pulled.** The P0 spike also measures this path. The engine lives on miniaudio's audio thread behind the raw `ffi::SetAudioStreamCallback`, and the director sends `Command`s over a lock-free SPSC queue. On native, a main-thread hitch then cannot underrun it.
  - The engine is `Send` and command-driven, so either driver works without a change.
  - The web has no threads, and its callback runs on the main thread either way.
- **sola-raylib 6.4.1 traps to avoid,** each filed upstream as its own issue:
  - `set_audio_stream_callback` takes the stream by value and drops it, and hands the callback a read-only buffer (`callbacks.rs:308-316`).
  - Closure processors never call raylib's detach, so the next callback panics on the audio thread.
  - `RaylibAudio` is `Clone`, but its `Drop` closes the device. Keep exactly one handle alive for the program. On the web `game_loop::run` returns at once, so the handle must not be a local of `app::run`.
  - `Wave::load_samples` returns half the samples of a stereo wave. We never use it (D3).

**D3. Decoding: Ogg Vorbis, decoded in Rust with `lewton`.**
- `lewton` is pure Rust and safe on wasm.
- Windowed and headless builds decode the same bytes the same way. raylib's decoders are not used.
- Assets are 48 kHz Ogg Vorbis: mono for point sources, stereo for beds, music and UI.
- One-shots and short loops are decoded at load into `i16` PCM. Music is streamed.

**D4. The web: the same raylib path, no ASYNCIFY.**
- **Why no ASYNCIFY.** The vendored miniaudio (0.11.25) uses a ScriptProcessorNode unless `MA_ENABLE_AUDIO_WORKLETS` is defined. Nothing defines it: not `raudio.c`, not raylib's cmake and not the sys crate. Its only `emscripten_sleep` is inside the worklet branch. So `InitAudioDevice` should not need ASYNCIFY.
  - That contradicts `.cargo/config.toml:31-35` and `docs/platforms.md:321`.
  - The P0 spike verifies it in a release web build under load, and both are corrected.
- **Unlock.** miniaudio itself resumes on `touchend` and `click`, but not `keydown`. The page's hook (`runtime.ts:58-69`) is fixed to resume every `window.miniaudio.devices[i].webaudio` context on the first `keydown` as well.
- **A hidden tab.** The page sets `bbHidden` and calls `bbChanged()` on `visibilitychange`, and the game fades its audio out. This is the existing page-to-game direction (`src/app/web.rs:106-145`), so no CLAUDE.md invariant changes.
- **Cost.** The ScriptProcessorNode runs on the main thread between frames. Our engine's cost there is the CPU budget in §5.
- **The fallback.** Only if the spike shows crackle under load: a page-side Web Audio executor that takes the same `Command`s through a batched game-to-page call. That changes a CLAUDE.md invariant, so it is the owner's call (§8, decision 5).

**D5. A watchdog.** The engine fades to silence over 100 ms when the director has sent nothing for 250 ms. That one rule covers:
- an Android app losing focus (raylib's own pause is commented out, and the main loop blocks in `ALooper_pollOnce`);
- a hidden web tab whose page has not yet signalled;
- a debugger break.

**D6. Variation without randomness.**
- Which take plays and how much pitch and gain jitter it gets come from a hash of the cue id, the frame, the source id and the event's index. They never come from `RoundRng` or `rand::rng()`.
- A seeded round therefore renders to the same audio every time, which is what makes a pinned render hash possible.
- Takes are round-robin without immediate repeats.

### 4.2 Where sound hooks into the frame

- **Owner.** `Audio` is owned by `app.rs`, beside `Fx`, and reads `session.shown()` (`src/mode.rs:1203`).
- **Per step.** `audio.observe_events(game)` goes in the `after_step` closure (`src/app.rs:2528`), deduped on `game.frame()`, which resets on a lower frame or a new seed. This is exactly the `Fx` rule (`src/fx.rs:905`), so a frame that runs four steps neither drops nor doubles a sound.
- **Per frame.** `audio.observe(game, &listeners, dt)` runs beside `fx.observe` (`src/app.rs:2776`). It runs the watchers, updates the emitters, spatialises, mixes, and sends the frame's commands to the driver.
- **Online.** Both run on the replica once per frame. `BongVoice::observe` joins them there too, so the bong speaks online.
- **Pause and dialogs** follow `fx_runs` (`src/app.rs:127`):
  - the world buses fade out over 150 ms and their loops pause, keeping their place;
  - UI and music carry on, the music under a high-cut while paused.
- **Build mode** hears the UI and the map's ambience bed only.
- **Level select and the menus** hear the menu music and the UI.

### 4.3 Space: scaled realism

Every number here is a `tunables!` row in a new `group audio` (§4.5), readable and settable live on the web tuning panel.

**Listeners (2.5D).**
- **Height.** Each view has a listener floating above the field at a height `H`, defaulting to 0.6 of half the view's width.
- **Pan.** Equal-power, from `dx / sqrt(dx² + H²)`. A sound straight below is centred, and pan widens smoothly towards the edges.
- **Distance** is `sqrt(dx² + dy² + H²)`.
- **Ears.** On a followed field the ears sit between the camera centre and the seat's tank (`audio_ear_bias` 0.6, toward the tank), so your own tank stays near the centre of the stereo image when the camera leads.
- **Arena.** A map shown whole gets one listener at the field centre with a larger `H`, so everything on screen is audible and the stereo stays gentle.
- **Couch** (`follow::ShotKind`):
  - `Shared` puts one listener at the pair's midpoint.
  - `Split` gives two listeners. Each voice is spatialised for both and takes the louder one's gain and pan.
- **Online:** the seat's own view.

**Distance.**
- **Audibility classes.** Each cue names one:

  | Class | Far (px) | Typical |
  |---|---|---|
  | tiny | 300 | a casing, a footstep in grass, a pickup hum |
  | small | 600 | bullet hits, ricochets, tread squeal |
  | medium | 1200 | engines, cannon fire, small blasts |
  | large | 2500 | wrecks, drum and fuel blasts, the jet |
  | huge | whole map | the rod, the volcano, thunder |

- **Gain.** An inverse-distance rolloff, clamped to the cue's `near` and `far`.
- **Past `far`:**
  - a one-shot never starts;
  - a loop goes **virtual**: its time runs on but it is not rendered.
- **Exempt.** Warnings and the player's own seat are never distance-culled.

**Air absorption.**
- A per-voice high-cut falls from 20 kHz at `near` to `audio_far_cutoff_hz` (1,500) at `far`.
- **Two layers by distance.** Cannons and explosions carry a near and a far sample (a crack against a rumble), crossfaded over the cue's distance bands. A distant battle sounds like distant battle, not like a quiet copy of a close one.

**Occlusion.**
- **The walk.** From the listener to the source, over 32 px cells, at most 64 steps, the Amanatides-Woo walk `weather::Occluders::reach` already uses (`src/weather.rs:560`).
- **What it sums.** A new `Material::sound_transmission() -> f32` (dB lost per cell crossed):
  - brick, concrete, iron, the towers, the volcano and doors are heavy (−12 dB);
  - glass is medium (−6);
  - wood, fences, sandbags, drums, trees, lamps and range boards are light (−1.5);
  - grass and water lose nothing.
- **Consistency.** It agrees with `Material::blocks_sound` (`src/obstacle.rs:505`); a test holds the two together.
- **The result:**
  - a gain loss, capped at `audio_occlusion_max_db` (−24), plus a high-cut down to 800 Hz;
  - re-evaluated at 10 Hz per audible voice and smoothed over 80 ms, so a tank rolling behind a wall ducks rather than clicks.
- **The grid** is one byte per cell, rebuilt when a tile dies (`obstacle_died`), not per frame.
- **Stretch, P6: path-based propagation.**
  - A BFS from each listener over the same grid, within the audible radius, at 4 Hz.
  - A voice whose straight line is blocked but whose path is open hears the path length with a small diffraction loss. Sound then wraps round the end of a wall.

**Propagation delay.**
- `delay = min(d / audio_sound_speed_px, audio_delay_max_s)`. The defaults are 3,000 px/s (about 1.75× the real 1,700 at 5 px/m) and a 0.25 s cap.
- On screen a delay stays under about 0.12 s, so a blast across the field booms just after its flash.
- **Zero** for the own seat, UI, warnings and voice.
- **Speed of sound.** Setting `audio_sound_speed_px` to 1,700 with no cap gives full realism.

**Doppler.**
- Pitch is multiplied by `c / (c − v_radial)` with `c = audio_doppler_c_px` (3,000), clamped to 0.8..1.25.
- `v_radial` is the source's velocity toward the listener minus the listener's own:
  - aircraft from `Game::aircraft()`;
  - missiles, drones and shells from their states;
  - hulls from `Physics::velocity` (`TankSnapshot::velocity` on a replica);
  - the listener from the camera's motion.
- **Fly-by.** A one-shot fires when a projectile's closest approach to a listener's seat tank passes within `audio_flyby_px` (48), triggered once per projectile id (the `Fx::watch_casings` pattern). Shells crack past, rounds snap, missiles whoosh.

### 4.4 Sources: tanks, weapons, impacts, explosions, the world

**Engine and treads, per chassis.**
- **The family.** `source::engine_profile(TankKind) -> EngineProfile` is an exhaustive `match` over `TankKind::ALL` (`src/tank.rs:189`). It picks one of four families, each its own sample set:
  - light petrol: scout, wraith, flak, glacier;
  - medium diesel: assault, warden, longbow, obelisk;
  - heavy diesel: breaker, ravager;
  - twin-deck heavy: titan, leviathan.
- **Per chassis** it adds a pitch and body offset from `tank_mass_factor`: heavier is lower and slower to rev.
- **RPM.**
  - The target is `idle + throttle·k1 + (speed / top_speed)·k2`, plus a load term while `speeding_up`.
  - The engine climbs toward the target with a time constant proportional to the mass factor, so a titan spools slowly and a scout snaps.
  - The inputs are `tank.throttle` and `Tread::speed` from `Game::driving()`.
- **The engine layers.** Each family has three loops, idle, low and high, crossfaded and pitched by RPM. A load layer (a grittier loop) adds in under acceleration.
- **The tread layers:**
  - **Clank.** A rattling loop whose gain and rate follow the measured speed.
  - **Turns.** `Roll::Pivoting` adds a squeal; `Roll::Sliding` adds a skid scrub.
  - **Surface,** an exhaustive `source::surface_layer(Surface)`: grass swish, road grit, sand hiss, ford splash, deep water (no hull enters it), ice crunch, weighted by `Tread::wet`.
- **Damage.** From `damage_stage::smoke()` (`src/damage_stage.rs:59`) the engine stutters and misfires. From `damage_stage::fire()` (`:70`) a fire crackle sits on the hull.
- **A wreck.** The engine dies with a cough and a burning crackle runs until `wreck_burn_seconds` ends.
- **The own seat** gets its engine a little drier and louder, so you feel your tank under you.

**Weapons.**
- **One set per weapon.** `source::weapon_set(ActiveWeapon)` is exhaustive over all 13 weapons. It names the fire, the in-flight sound and the end of each.
- **The cannon** is tiered by `tank_damage_factor`: light below 0.85, medium to 1.1, heavy to 1.4, super above. Each tier has its own sample set plus a pitch offset by mass.
- **Twin guns.** A twin-gun chassis plays a second crack `tank_twin_shot_delay_seconds` (0.05 s, `src/tuning.rs:715`) later. The new-projectile watcher drives it, not `Fired`.
- **Minigun.** It is a spin-up, a firing loop and a spin-down tail driven by `Tank::minigun_spin`. It is never one sample per round.
- **Charges and tells** come from state edges:
  - the gauss charge hum rising to full, and the overcharge whine (`Tank::charge`);
  - the sonic and EMP wind-up tells (`Tank::tell`);
  - the tesla tower's charge (`TowerView::charge`).
- **Lock-ons** are warnings: the missile lock (from replicated missile state), the drone lock and the rod's designator, then the incoming whistle during the rod's three-second countdown.

**Impacts.**
- **A matrix** of projectile class × surface family:
  - the classes are shell, bullet, plasma, laser, slug, grenade, missile and drone;
  - the families come from `source::surface_family(Material)`, exhaustive over a new `Material::ALL`: metal, masonry, wood, earth, glass, foliage, water, energy shield, and the bong.
- **The fallback chain** resolves a missing combination: `hit.<proj>.<material>` → `hit.<proj>.<family>` → `hit.<proj>.generic`. A new material plays its family's sound at once, and the sound map shows the specific one as wanted.
- **Debris.** `ChunksBroken { broken, collapsed }` scales the debris layer. A collapse adds a rumble.
- **Glass** (`breaks_by_sound`) shatters.

**Collisions.**
- **Slams.** Strength comes from `Game::contacts()`'s `max_impulse` through the existing `slam_strength` and the `ram_fx_*` thresholds, mapped to a light knock, a heavy slam or a crash. A ram plays steel on steel.
- **Scrapes.** While contact holds above `contact_fx_min_impulse` (`src/tuning.rs:3419`), a scrape loop plays, steel on steel or steel on wall, with gain proportional to the impulse.
- **Online** there is no physics, so this needs the `Slam` event of §4.10.

**Explosions.**
- **Layers:** a transient crack, a body boom, a debris tail and a sub-bass sweetener for the big ones.
- **The size class** comes from the blast radius and the fx shock ladder (`src/tuning.rs:4564-4601`). Each class sets the layers, the audibility class and the duck depth.
- **Chains.** Each link of a drum chain plays 3 dB under the one before and shares the chain's voices.
- **The ear-ring.** A huge blast within 150 px of a listener adds a short high whine, a 0.4 s world high-cut and a duck of every other bus. It is `audio_ear_ring` and can be turned off.

**The world.**
- **Weather beds** come from `Game::look()`: rain, snow and sand intensity, and fog muffling the world bus a little.
  - Sandstorm gusts (`weather::gust_at`) swell the wind bed and pan it with the gust's direction.
  - **Thunder.** `weather::lightning` gives the flash from the round clock. The director finds the strike times the same way and plays thunder after each flash with a hashed 0.5–3 s delay; the flash has no position. No event is needed, and a client computes the same strikes.
- **Places.** An ambience bed per place is an exhaustive `source::place_bed(Theme, Option<Biome>)` over `Theme::ALL` and `Biome::ALL`: the valley's mill race and birds, the floodplain's frogs and water, the karst's wind over stone, the desert's dry wind.
- **Water.** Emitters are built once per map from clusters of water cells near the listener, and fade with distance.
- **Fire** is one loop per fire cluster, with gain proportional to the burning cells (`Game::burning_cells`, `src/simulation/props.rs:1311`; `Game::burning_tiles`, `src/simulation/mod.rs:6013`), never one per cell. A fuse sizzles (`fused_barrels`).
- **The volcano** follows `volcano::Phase`: the rumble swells and the eruption roars. A lava bomb whistles on the way down.
- **The rest:** portals hum (`Game::portals()`); pickups land, open and are collected; the bong hops, bites and is hurt.

### 4.5 The mix

**The bus tree:**

```
master ─┬─ music
        ├─ sfx ─┬─ weapons
        │       ├─ impacts
        │       ├─ explosions
        │       ├─ vehicles
        │       ├─ world
        │       └─ warnings
        ├─ voice
        └─ ui
```

**Tuning rows.** Every bus gain, duck depth and time, range class and space constant is a row in `group audio`:

```
group audio {
    /// Master gain in dB.
    audio_master_db: f32 = 0.0 in -60.0 ..= 6.0;
    /// Per-bus gains in dB: music, weapons, impacts, explosions, vehicles, world, warnings, voice, ui.
    audio_bus_db: [f32; 9] = [-6.0, 0.0, -2.0, 0.0, -4.0, -8.0, 0.0, 0.0, -6.0] in -60.0 ..= 6.0 labels BUS_NAMES;
    /// How far the HDR window reaches below the loudest sound, in dB.
    audio_hdr_window_db: f32 = 45.0 in 10.0 ..= 90.0;
    audio_sound_speed_px: f32 = 3000.0 in 500.0 ..= 20000.0;
    audio_delay_max_s: f32 = 0.25 in 0.0 ..= 5.0;
    audio_doppler_c_px: f32 = 3000.0 in 500.0 ..= 20000.0;
    audio_far_cutoff_hz: f32 = 1500.0 in 200.0 ..= 20000.0;
    audio_occlusion_max_db: f32 = -24.0 in -60.0 ..= 0.0;
    audio_ear_bias: f32 = 0.6 in 0.0 ..= 1.0;
    audio_voices: usize = 48 in 8 ..= 128 @ Restart;
    ...
}
```

These are read through `tuning()` and bound once a frame, as CLAUDE.md asks of hot loops.

**The HDR window.**
- **Ranking.** Every live voice has a loudness at the listener: the sample's measured LUFS + the cue's gain + distance + occlusion, weighted by priority. The loudest sets the top of the window.
- **What falls out.** Voices more than `audio_hdr_window_db` under the top are faded out (loops go virtual, one-shots stop). A close blast briefly hides the ricochets.
- **Relaxing.** The top falls back at 20 dB/s, so the quiet world returns as the blast decays.
- **Pinned.** Warnings and the own seat stay in whatever the window says.

**The voice budget.**
- `audio_voices` real voices (48). Beyond that a voice is virtual.
- **Per cue:**
  - `polyphony`;
  - `cooldown_ms`, the minimum gap between starts of the same cue from the same source;
  - `steal`: the oldest, the quietest, or none (refuse).
- **Priority classes:** warning > own seat > critical > high > medium > low.

**Aggregation:**
- A burst is a loop with a tail: the minigun, the gun tower's five rounds, the flamethrower stream.
- A cluster is one emitter: fires, embers, burning wrecks close together, rain on the field.
- A chain shares its voices and steps its level down: drum chains, a rod's tick of tile deaths, the EMP ring.

**Ducking**, through a sidechain envelope per rule:
- explosions duck vehicles and world (−6 dB, 300 ms release);
- voice ducks music and world (−6 dB);
- warnings duck music (−4 dB);
- the ear-ring ducks everything but itself.

**The master.** A 3 ms lookahead limiter at −1 dBFS after a DC blocker. Assets are loudness-normalised per category (§4.12), so the limiter rarely works.

**Settings.**
- **The controls:**
  - sliders for master, music, effects and voice;
  - mute;
  - mono, for a single speaker or one ear.
- **Saved** with the progress store (`save_progress`, `src/app.rs:724`; `localStorage` on the web).
- **The HUD mute button:**
  - at least 44 pt;
  - in a corner cluster, through `hud::corners()` (`src/hud.rs:1233`), so the painter and the hit test share one geometry;
  - labelled with `text::keys` strings.
  - M toggles it on a keyboard.

### 4.6 Adaptive music

**States.**
- Menu, level select and the builder (calm).
- In a round, by intensity:
  - **Explore:** 0 to 0.3;
  - **Tension:** 0.3 to 0.6;
  - **Combat:** 0.6 to 0.85;
  - **Climax:** above 0.85.
- **Stingers:** wave start, last enemy, victory and defeat.
- **Pause** keeps the music playing under a high-cut.

**Intensity** is a 0..1 value with a 2 s attack and an 8 s release. It is fed by:
- enemies alerted within a seat's sight box;
- shots fired near a seat in the last few seconds;
- damage taken;
- the wave number and `Tier`;
- how few enemies are left.

It is computed in `music.rs` from the round's public state, never from RNG.

**Structure.**
- Each `Theme` has three intensity tracks: calm, tense and fight. Each `Biome` may override them.
- All three share a BPM, key and bar length. The track switches on a bar boundary with a one-bar crossfade (horizontal re-sequencing).
- Vertical layering (stems) is used only if the P0 music spike shows that separately generated ElevenLabs stems stay coherent.
- The map from theme to tracks is exhaustive, `music::theme_tracks(Theme, Option<Biome>)`.

**Generation.**
- `eleven_music_v2_5`, instrumental. The prompt names the BPM, key, instrumentation and mood.
- The duration is a whole number of bars (32 bars at 120 BPM = 64 s), trimmed to the bar grid with a crossfaded loop point.
- The tempo, bars and loop point go in the manifest.

**Delivery.**
- Streamed with `lewton`, never decoded whole.
- Lazy-loaded on the web after the first input (§4.8).

### 4.7 Voice: the bong and the announcer

**Keys.**
- A spoken line is keyed by its `text::keys` constant, so a line and its bubble or HUD text share one id. Files are `static/audio/voice/<lang>/<key>.ogg` for each shipped language (en, sl).
- **Stale lines.** The manifest stores a hash of the source string a line was made from. A test reports the stale lines when a translation changes, and `sound-gen` regenerates them.
- **Coverage.** A test checks that every voiced key has a file per shipped language or is marked text-only for that language.

**The bong.**
- **Babble.** It babbles whenever `BongVoice` (`src/bubble.rs:59`) puts up a bubble: `BONG_*` keys on hits, heals, bites, pickups and wrecks, and the training beats.
  - The babble is language-free creature chatter with no words, so it never needs translating.
  - Its length follows the bubble's text length.
  - Its mood follows the line's family: happy, hurt, angry, alarmed.
- **Samples.** A small set of babble syllable and phrase samples per mood, generated as sound effects. The director strings them together, hashed from the line so a bubble always sounds the same.
- **The kinds.** A per-`BongKind` pitch and speed offset (exhaustive over `BongKind::ALL`) tells the seven apart.

**The announcer.**
- **The callouts** are short and localised:
  - wave incoming, last enemy, air support inbound, mission complete, tank lost;
  - spoken warnings: an incoming rod, a missile lock.
- **Radio colour** comes from the free `audio-effects` node (`old_radio` or `cheap_microphone`), applied before processing.

**The queue.**
- One announcer line at a time, the highest priority first. Each line expires after `max_wait_s`.
- The voice bus ducks music and world.
- The bong's lines and the announcer never overlap; the announcer wins.

### 4.8 Data: the cue manifest

**`audio/cues.toml`.**
- A top-level data file like `lang/*.ftl` and `levels.toml`, embedded with `include_str!` so the headless build, the tests and the web all have it.
- **Two levels:**
  - **a sample** is a set of sound files and how they were made;
  - **a cue** is a behaviour that plays samples.
- Many cues share samples (the calibre tiers, the material families), so generation works on samples and mixing works on cues.

```toml
[style]
model = "eleven_text_to_sound_v3"
prefix = "close-mic, dry, punchy, no music, no reverb"   # set by the P0 audition

[sample.cannon_medium_near]
status = "approved"          # planned | review | approved | placeholder | rejected
bus = "weapons"
channels = 1
prompt = "tank main gun firing, sharp concussive crack, deep body, metallic breech clank"
duration_seconds = 1.2
prompt_influence = 0.45
loop = false
takes_wanted = 3
  [[sample.cannon_medium_near.take]]
  file = "weapons/cannon_medium_near_1.ogg"
  flow = "<elevenlabs flow id>"
  generation = "<elevenlabs generation id>"
  made = 2026-12-01
  plan = "creator"
  lufs = -14.1
  peak_dbtp = -1.0
  seconds = 0.94

[cue."weapon.cannon.fire"]
kind = "one_shot"            # one_shot | loop | modulated_loop | stinger | ui | voice
bus = "weapons"
priority = "high"
class = "medium"             # audibility class (§4.3)
polyphony = 6
cooldown_ms = 30
steal = "quietest"
gain_db = -2.0
pitch_jitter = 0.03
varies_by = ["calibre", "mass"]
layers = [
  { sample = "cannon_medium_near", until_px = 700 },
  { sample = "cannon_medium_far",  from_px  = 400 },
]
```

**Typed ids.**
- A `cues!` macro mirrors `text::keys` (`src/text.rs:62`), with `pub const CANNON_FIRE: Cue = Cue("weapon.cannon.fire");` and `ALL`.
- **Data-name families** are resolved with `cues::named(family, name)`, as `text::named` does:
  - `engine.<tank>`
  - `hit.<proj>.<material>`
  - `bed.<theme>.<biome>`
  - `sky.<weather>`
  - `pickup.<kind>`
- A test mirrors `the_keys_and_the_english_file_name_the_same_messages` (`src/text.rs:833`): every constant is in the manifest, and every manifest cue is a constant or a dynamic id.

**Files.**
- **Paths.** `static/audio/<bus>/<sample>_<n>.ogg`.
- **The web's core set:** UI, the own weapons, the common impacts and the engines, at most 1.5 MB. It goes through the existing `--preload-file static@/static` (`.cargo/config.toml:29`).
- **Lazy files.** Music, voice, the beds and the rest are kept out of the preload with `--exclude-file`. After the first input they are fetched with `emscripten_async_wget`, so first load does not grow by more than the core set.

**Status, derived per cue for the sound map:**
- **silent**: by design, with a reason in the binding;
- **unhooked**: in the catalogue, with no binding yet (a gap);
- **missing**: hooked, with no file;
- **placeholder**: hooked, with a temporary file;
- **done**: hooked, with approved files.

**Dev placeholders.** In a `dev-tools` build a hooked cue with no file plays a short synthesized earcon, coded by bus: a tick for impacts, a thump for explosions, a hum for loops. Missing sounds are then audible while play-testing. A release build plays nothing for them.

**Hot reload.** A `dev-tools` build watches `audio/cues.toml` and `static/audio/`, as `TuningWatch` does (`src/app.rs:1149`), and reloads what changed between frames. A gain, a range or a new take is heard without a restart.

### 4.9 Coverage that compiles

**The bindings.**
- `audio::bind(&Event, &Ctx) -> Binding` is an exhaustive `match` with no wildcard. It returns one of:
  - `Cue(CueRequest)`: cue, position, source id, drivers;
  - `Watched(WatcherId)`: the sound comes from a state watcher;
  - `Silent(&'static str)`: the reason.
- **A new `Event` fails to compile** until it is given one, the same guard as `WireEvent::from_event`.

**Exhaustive maps over every enumeration** a sound depends on:

| Function | Over |
|---|---|
| `engine_profile` | `TankKind::ALL` |
| `weapon_set` | `ActiveWeapon` (13; gets an `ALL`) |
| `surface_family` | `Material::ALL` (new, 24) |
| `surface_layer` | `Surface` |
| `pickup_cue` | `PickupKind::ALL` |
| `bong_voice` | `BongKind::ALL` |
| `tower_set` | `TowerKind::ALL` |
| `strike_cue` | `AirStrike::ALL` |
| `swallow_cue` | `Swallow::ALL` |
| `sky_bed` | `Weather::ALL` |
| `place_bed` | `Theme::ALL` × `Biome::ALL` |
| `theme_tracks` | `Theme::ALL` × `Biome::ALL` |

**Watchers.** `WatcherSpec { id, entity, reads, cues, online }` is a registry of the state watchers. The same table drives `observe` and the sound map, the way `devserver::tools::TOOLS` is one table for dispatch and advertisement (`src/devserver/tools.rs:953`).

**Tests** (`audio_tests`, headless):
- every `ALL` maps to a cue in the manifest or to `Silent`;
- every manifest cue is reachable from a binding, a watcher, the UI or the music;
- every `approved` sample has its files, and each is within its loudness, length and size budget;
- `sound_transmission` agrees with `blocks_sound`;
- the voiced keys match the text;
- nothing under `simulation/` or `net/` names `crate::audio`, a source-grep test like `the_simulation_and_the_wire_never_read_the_catalogue`.

**Scenario tests**, mechanics-style on tiny inline maps, assert on the recorded `Command`s:
- a shot and a wreck each start their cue once, at the right frame and side;
- a five-drum chain stays within its polyphony and steps down;
- a minigun burst is one loop with a tail;
- a wall between the listener and a blast lowers its gain and high-cut;
- a titan's engine sits lower than a scout's at the same speed;
- online, through `rig::Lockstep`: the own press sounds once, at the press.

### 4.10 Online

- **Own fire at the press.**
  - The own seat's fire, press shows and impacts play when the predictor makes them. `OnlineRound` exposes what `fly_own_shots` (`src/net/round.rs:1301`) drains: `take_muzzles`, `take_press_shows` and `take_impacts` (`src/net/predict.rs:1559-1571`).
  - **Skips.** The audio skips the room's `Fired` for its own seat, which still arrives an interpolation delay late (`src/net/apply.rs:783`). It also skips the second copy of `WellAnchored` and `ChargeEnded`.
- **Never-sent events** are heard from replicated state edges instead: `TankState`'s tell, charge and disabled flags, and the tower state on the wire. These are the 20 on `NOT_SENT` plus `MissileLocked` (BB-201).
- **Hook gaps.** Each is its own Linear issue. Where the wire changes it bumps `PROTOCOL_VERSION`. None draws RNG.

  | Change | What it is for |
  |---|---|
  | `Hit` gains the shooter's slot | the hit-marker tick, a distinct friendly-fire cue |
  | `Wreck` gains its cause | a different death for a burn-out, a ram or a blast |
  | `Ricochet` gains the surface and the shot kind | a bullet glancing off a hull against a shell off iron |
  | `Ram` gains the closing speed | the crash tier online |
  | `Slam { slot, x, y, strength }` is added, and sent | slams and crashes online, where a client has no physics |
  | `BongBite` gains its position | a bite placed in space |
  | `RoundEnded` gains its reason | the right ending stinger |
  | `ObstacleDestroyed` gains its cause | a hammer throw against a destruction |

- **No new event for launches.** A watcher on new projectile ids covers a local round and a replica alike: twin barrels, each minigun round, each missile tube.
- **Thunder, gusts and the volcano's stages** are pure functions of the round clock and state, so a client hears them with no wire change.

### 4.11 The sound map: a living graph

The sound map is where you see what needs sound, what is hooked, and what has been generated, approved or is still to make. It has three inputs:

1. **Facts from the code.** `cargo run --bin soundmap` is headless, with no raylib. It walks:
   - the bindings (each `bind` arm has a `describe()` row: event, cue or reason, online behaviour);
   - the `WatcherSpec` registry;
   - every enumeration's `ALL` through the exhaustive maps;
   - `audio/cues.toml`, with each file's presence, length and measured loudness, read with `lewton`.
2. **The curated catalogue,** `audio/interactions.toml`. It holds what code cannot know:
   - each interaction's description;
   - interactions planned but not hooked;
   - gaps and their suggested fixes, and priorities.
   - It is seeded once from BB-199's `graph.json` (444 interactions, 156 gaps, the same schema) and then kept by `sound-audit`. It records `audited_at = "<commit>"`.
3. **The generation state:** the takes waiting in `tools/audio/_takes/` and the open audition picks.

**The output** is `target/soundmap/index.html`, one file with the data inlined, so it opens from disk.
- It carries BB-199's six views (overview, graph, interactions, sounds, gaps, events) and a seventh, **Assets**:
  - every node and cue is coloured by status: silent, unhooked, missing, placeholder, in review, done;
  - each sample has players for its committed takes and its waiting candidates;
  - coverage bars per domain and priority;
  - a credits-spent tally from the manifest's provenance.
- `just sound-map` builds it and opens it.

**Checks.**
- `soundmap --check` runs in CI. It fails when:
  - the catalogue names a cue the manifest does not have;
  - a bound cue has no catalogue entry;
  - a sample is marked approved without its files.
- **Sharing.** For a link to share, `sound-audit` republishes the page as an artifact snapshot. The artifact is a view; the repository is the source.

### 4.12 Generation: ElevenLabs through the MCP

**The style guide.** `docs/SOUND_SPEC.md` is the sound counterpart of `docs/PALETTE.md` and the `*_SPEC.md` sheets. It is written from the P0 audition and holds:
- the direction;
- the prompt grammar: a style prefix, then the sample's prompt, then a category suffix;
- the dos and don'ts from the model guide:
  - one sound per generation;
  - short and concrete;
  - name the texture and the transient;
  - "close-mic" or "distant";
  - no visual or cinematic words;
  - "no music";
- the loudness targets, the processing chain, and the review checklist.

**The `sound-gen` skill** (`.claude/skills/sound-gen/SKILL.md`, the runbook in Appendix B):
1. **Select** samples with status `planned` or `rejected`, or the ones named.
2. **Compose** each prompt from the style guide.
3. **Price** the batch with `creative_run_flow_nodes {estimate_only: true}`. Above 2,000 credits, ask the owner before spending.
4. **Make the nodes.** `creative_create_flow` creates one flow per batch, named `bongbong sfx <date> <batch>`. Then `creative_add_flow_node` adds up to 20 `sfx` nodes: model `eleven_text_to_sound_v3`, `model_parameters` `{duration_seconds, prompt_influence, loop}`, and the sample id as the node's label.
5. **Run** them with `creative_run_flow_nodes` (4 takes each), then poll `creative_get_flow_run_status` until every run is done.
6. **Fetch.** `creative_get_generations` returns signed URLs. `tools/audio/fetch.py` downloads each to `tools/audio/_takes/<sample>/<generation>.mp3`, a gitignored directory; the flow keeps them, so they can be fetched again. Each take's flow and generation id are written into the manifest, and the sample goes to `review`.
7. **Audition.** The skill publishes a private audition artifact: a page per batch with every take as an asset, a waveform, the prompt, and pick and reject buttons that save to the artifact's database. The skill reads the picks back. The ElevenLabs flow canvas is a second place to listen.
8. **Process** the picks with `tools/audio/process.py` (ffmpeg, added to `devenv.nix`):
   - trim silence at −60 dB, high-pass at 30 Hz, remove DC;
   - normalise loudness (EBU R128) to the category's target: weapons −14, impacts −16, explosions −12, vehicles −18, world beds −24, warnings −14, UI −20, voice −18 LUFS;
   - true peak at −1 dBTP;
   - a 2 ms fade in and a 10–50 ms fade out;
   - a loop gets an equal-power seam crossfade and a seam discontinuity check;
   - 48 kHz Ogg Vorbis at q4.
   - The measurements are written back to the take, and the sample goes to `approved`.
9. **Check** with `just check-sounds`, a Rust test, so CI needs no ffmpeg:
   - the manifest and the files agree;
   - lengths, peaks and loudness are within tolerance;
   - loops are seamless;
   - the web size budget holds.

**Music and voice** use the same pipeline: `music` nodes; `creative_design_voice`, `creative_save_designed_voice` and `tts` nodes; and the free `audio-effects` node for the announcer's radio.

**Provenance and licence.**
- Every take records its model, parameters, flow, generation, date and plan.
- Shipped sounds are generated only on a plan with commercial rights. Before a plan is cancelled, its terms are checked for whether those rights outlive it.

### 4.13 Platforms

| Platform | State | Work |
|---|---|---|
| Linux, Windows, macOS | ready | raylib `AudioStream`. `libasound2-dev` is already in `dist-workspace.toml`. A lost device (headphones unplugged) re-inits on the next frame. |
| Web | spike | D4: no ASYNCIFY (verify), the unlock fix in `runtime.ts`, `bbHidden`, the lazy fetch, buffer size against latency. |
| Android | follow-up | AAudio through miniaudio (OpenSL ES is linked, `android/build.rs:39`). The watchdog (D5) silences focus loss. Not in the first PR: the build stays silent until its follow-up issue. |
| iOS | follow-up | Drop `-DMA_NO_COREAUDIO` and compile `raudio.c` as Objective-C (`tools/setup_ios.sh:104-114`). A raylib patch beside `tools/ios/raylib-sdl-highdpi.patch` sets the session category to Ambient with mix-with-others; miniaudio's default is PlayAndRecord. Handle interruptions (calls, Siri) by pausing and resuming the stream. Not in the first PR. |
| Room server, probe | none | The null driver. The director still runs in the probe for tests and offline renders. |

### 4.14 Dev tooling and tests

**Dev-server tools.** Each is one `ToolSpec` row and a `dispatch` arm (`src/devserver/tools.rs:29`):
- `audio_status`: live voices (cue, gain, pan, occlusion, virtual or real), bus levels, the HDR window, the last 32 cues;
- `audio_play {cue, x, y}`: audition a cue in the round at a place;
- `audio_mute`, `audio_solo {bus | cue}`;
- `audio_render {seed, frames, map, scenario}`: renders that replay headless and returns a WAV path.

**The `audio` overlay** (`simulation::Overlays` gains a flag) draws:
- each emitter, with its cue id and gain;
- the occlusion ray, coloured by loss;
- each listener and its `H` circle.

**The probe.** `--audio-out round.wav` runs the director and the engine on the probe's seeded round (`run_round`, `src/bin/probe.rs:2249`) and writes the mix. Listening to any seed needs no window.

**The pinned render** (`just audio-render-compare`):
- a short scripted scenario's render is hashed and pinned, as the CPU thumbnails are;
- an unintended change to the mix, a binding or a sample fails it;
- a deliberate change re-pins it consciously, never to go green.

**The tuning panel.** The web preview's panel gets the `audio` group, so the mix can be tuned live on a PR preview.

## 5. Numbers

| What | Target |
|---|---|
| Engine CPU | ≤ 0.5 ms a frame for 48 voices in the wasm release build (measured in P0) |
| Decoded PCM resident | ≤ 24 MB (i16, 48 kHz); music is streamed |
| Web, first load | core ≤ 1.5 MB preloaded |
| Web, lazy | the rest of the effects ≤ 3 MB, music ≤ 5 MB, voice ≤ 1.5 MB per language |
| Latency, press to sound | ≤ 50 ms on desktop, ≤ 100 ms on the web |
| Voices | 48 real, unlimited virtual |
| Cues | about 320 (BB-199), on about 180 sample sets, 80 of them loops |

**Credits, measured with `estimate_only` on 2026-11-22.**
- **Effects:** 11 credits a second (a 1.2 s sound with 4 takes is 53 credits). 180 sets × 1.5 s × 4 takes × 1.5 rounds, plus 80 loops × 4 s × 4 takes × 1.5, is about 3,500 s, so about 40k credits.
- **Music:** 27.5 credits a second (a 64 s track with 4 takes is 7,040 credits). The planned set comes to about 25k: a menu track, three intensity tracks per theme and four stingers, with 2 takes each.
- **Voice** (the announcer's TTS and the babble's effects) is about 2k.
- **Total:** about 67k, inside a Creator month.
- The connector's account tool lacks the scope to read the balance from a session.

## 6. Phased plan

Each phase is a sub-issue of BB-102, all built in one PR. Android and iOS audio (marked below) move to a follow-up issue. The direction audition in P0 is replaced by the first-batch listening checkpoint (§8, decision 3).

- **P0. Spikes and direction.** About a week.
  - **The output spike.** A raylib `AudioStream` fed from Rust on desktop and in a release web build:
    - confirm no ASYNCIFY;
    - measure underruns under load and the engine's CPU;
    - compare the push and pull drivers.
  - **The direction audition.** Appendix C's six sounds in two directions, plus a third made by processing rather than generating, published as an audition artifact for the owner to pick. The sound counterpart of "visual choices need a preview".
  - **The music spike.** One theme's three intensity tracks: do they stay in key and tempo, and do the loops seam?
  - **Out:** `docs/SOUND_SPEC.md`, the corrected `docs/platforms.md` and `.cargo/config.toml` comment, and the measured credit rate.
- **P1. The core.**
  - The engine, the director skeleton, the manifest and `cues!`.
  - The exhaustive bindings and maps with their tests.
  - The WAV driver and `probe --audio-out`.
  - Twelve cues on desktop: cannon fire, minigun, hull hit, wall hit, wreck, barrel blast, three engine families, treads, a UI click, the mute.
- **P2. Space and mix.**
  - Listeners, distance, pan, high-cut, delay, doppler and occlusion.
  - The chassis engine model and impacts by impulse.
  - The HDR window, buses, ducking, the `audio` tuning group, the settings and the mute button.
- **P3. Tooling and the first content wave.**
  - `soundmap`, the `interactions.toml` import and the viewer.
  - The `sound-audit` and `sound-gen` skills.
  - All 114 critical and high-priority cues.
  - The `audio` overlay and the dev-server tools.
- **P4. Web, Android, online.**
  - The web driver, unlock and lazy fetch; Android.
  - The predicted press, the `NOT_SENT` watchers, and the hook-gap issues of §4.10.
- **P5. The rest of the content.**
  - Medium and low cues, the beds, weather and places.
  - The music system and its tracks.
  - The bong and announcer voices in en and sl.
- **P6. iOS and polish.**
  - The iOS backend; reverb presets per class.
  - The ear-ring, path-based propagation and a performance pass.

**Documentation that lands with the code:**
- `docs/codebase.md`: one entry per audio module;
- `docs/tooling.md`: sound-map, check-sounds, audio-render;
- `docs/platforms.md`: audio per platform;
- the `.cargo/config.toml` comment;
- one CLAUDE.md line, added to the existing "Drawing and effects" bullet: `audio/` reads `Game`, never writes it, draws no round RNG, and `audio::bind` is exhaustive over `Event`.

## 7. Risks, mitigations, stop conditions

- **The web crackles on the main thread.**
  - Mitigations: a larger buffer, a cheaper mix (fewer voices, the reverb off on the web), then the page executor (decision 5).
  - Stop: if the P0 spike shows underruns at 60 fps in the release build with 48 voices.
- **The style drifts across 300 generated sounds.**
  - The style guide, one processing chain and loudness targets hold it together, plus an audition per batch.
  - Stop and re-audition if a batch's picks rate below half.
- **Credits overrun.** Price every batch before it runs, ask above 2,000 credits, and generate the critical and high cues first.
- **Gameplay readability suffers** from delay or occlusion. Each is a knob; play-test on PR previews with the tuning panel. A warning is never delayed, occluded or culled.
- **Online timing doubles or delays own sounds.** The `rig::Lockstep` scenario test, and a netlab run (`just netlab run`) against local play.
- **The download grows.** Core and lazy tiers, budgets enforced by `check-sounds`, and q4 Vorbis.
- **Licence.** Generate only on a paid plan, and record the plan per take.

## 8. Decisions

Settled with the owner, 2026-11-22:

1. **Scope.** Adaptive music and the bong and announcer voices are in. Captions and haptics are out.
2. **Space** is scaled realism (§4.3). Full realism is two knob settings away.
3. **Direction:** real, heavy sources in an arcade mix (short, dry, punchy, larger than life). Appendix C's direction audition is replaced by a listening checkpoint on the first batch of twelve core sounds, before the rest is generated.
4. **ElevenLabs plan:** Creator. Measured rates through the connector: sound effects 11 credits a second, music 27.5 credits a second.
5. **Budget order.** Effects and voice are generated first. Music waits until the owner has heard the effects.
6. **Picking takes.** The session cannot listen. It pre-picks each take by measurement (length, loudness, clipping, a clean start and end, the loop seam) and ships it. The owner reviews on the audition page and swaps or rejects; rejected takes are regenerated.
7. **The bong's voice** is language-free babble timed to its bubbles: per-kind pitch, nothing to translate. The announcer speaks localised lines.
8. **Platforms.** Desktop and the web first. iOS and Android audio are a follow-up issue, and those builds stay silent until it lands.
9. **Online:** the event and wire changes of §4.10 are in, with a `PROTOCOL_VERSION` bump.
10. **Delivery.** Everything is built in one PR with this document, tracked as one issue (BB-102) with sub-issues per phase. It is pushed at phase ends only, to spare CI.

Open:

1. **The web fallback executor.** It is decided only if the P0 spike shows crackle, because it changes the CLAUDE.md rule that the page talks to the game only one way.

## 8b. As built (2026-11-22)

Where the build departs from the text:

- **The catalogue** is `audio/interactions.json`, not TOML: BB-199's map in its own schema, each cue
  given `manifest: {cue, proposed, fit}`. That is 108 exact, 55 covered and 156 missing at seeding.
- **The engine** resamples linearly, not cubically: cheaper on wasm and inaudible at these pitches.
  The FDN reverb is not built yet, and neither are the voice's reverb send or the path-based
  propagation of §4.3; occlusion is the straight-line cell walk.
- **The driver** is the pull path from the start: the engine lives behind a lock the raylib stream's
  callback takes, so a main-thread hitch never starves the device. The web takes the same path
  (D4), verified in the release build: no ASYNCIFY, no `emscripten_sleep`, a ScriptProcessorNode.
- **Hidden tab:** the page itself suspends miniaudio's contexts (`runtime.ts`), so no `bbHidden`
  global was needed.
- **Settings:** mute is the HUD's sound button and the M key, kept beside the progress. There are
  no volume sliders yet: the levels are the `audio` tuning group's.
- **Slams online** are a new `Event::Slam`, sent (`PROTOCOL_VERSION` 36). The other payload
  changes of §4.10 are still to do: `Hit`'s shooter, `Wreck`'s cause, `Ricochet`'s surface,
  `Ram`'s speed, `BongBite`'s position, `RoundEnded`'s reason, `ObstacleDestroyed`'s cause.
- **Takes are pre-picked by measurement** (`tools/audio/process.py`) and reviewed by the owner on
  the audition page. The real charge is about 3.3 credits a second of audio per take, a third of
  `estimate_only`'s figure.
- **Still to make:** every planned sample after the first batch, the bong's babble, the announcer,
  and music.

## 9. References

- **Issues:**
  - BB-102 (this);
  - BB-199 (the map);
  - BB-201 (`MissileLocked` not on `NOT_SENT`);
  - BB-36 (the special weapons);
  - BB-177 (air support);
  - BB-187 (the minigun);
  - BB-188 (friendly fire).
- **Pages:**
  - [Bongbong Sound Plan](https://claude.ai/artifact/2nMF6XMiVHtujdjhhyNX6B) (October);
  - [Bongbong Sound Map](https://claude.ai/artifact/MstM43AFoX2ibaQKAwbhEZ) (BB-199, with `graph.json`).
- **Code:**
  - the hooks: `src/app.rs`, `src/fx.rs`, `src/mode.rs`, `src/pacing.rs`;
  - the simulation: `src/simulation/mod.rs`, `src/simulation/wear.rs`, `src/wear.rs`, `src/obstacle.rs`, `src/weather.rs`, `src/biome.rs`, `src/follow.rs`;
  - online: `src/net/events.rs`, `src/net/apply.rs`, `src/net/round.rs`, `src/net/predict.rs`;
  - the patterns: `src/tuning.rs`, `src/text.rs`, `src/devserver/tools.rs`, `src/bin/probe.rs`;
  - the platforms: `.cargo/config.toml`, `site/src/scripts/runtime.ts`, `src/app/web.rs`, `tools/setup_ios.sh`.
- **sola-raylib 6.4.1:** `src/core/audio.rs`, `src/core/callbacks.rs`. In raylib: `raudio.c`, `config.h`, and miniaudio 0.11.25's `miniaudio.h`, its Web Audio backend.
- **Docs:**
  - `docs/effects.md`, `docs/online-coop-prd.md` (decision 19), `docs/platforms.md`;
  - `docs/ios-native-port-prd.md` (§ audio), `docs/android-port-prd.md`;
  - `docs/localization-prd.md` (the keys pattern), `docs/runtime-tuning-design.md`, `docs/mapshot-prd.md` (pinned hashes).
- **ElevenLabs** (from the connector): the `sfx`, `music`, `tts` and `audio-effects` node types and the `eleven_text_to_sound_v3` model guide.

---

## Appendix A: the `sound-audit` skill

The text of `.claude/skills/sound-audit/SKILL.md`, written with P3:

```markdown
---
name: sound-audit
description: Find every place in bongbong that needs a sound and is not covered - new entities, interactions, events, state changes - and update the sound map, the cue manifest and Linear. Use when the user says "sound audit", "what needs sound", "update the sound map", after a feature that adds a weapon, tank, material, event, pickup, biome or sky lands, or before a sound-gen batch.
---

# Sound audit

The sound map has three inputs: the code (bindings, watchers, enum maps), the
curated catalogue `audio/interactions.toml`, and the manifest `audio/cues.toml`.
This skill keeps the catalogue and the manifest true to the code. It never
generates audio (that is `sound-gen`) and never edits simulation code (hook
gaps become Linear issues).

## 1. What the compiler and the tests already know

Run `cargo run --bin soundmap --no-default-features -- --check` and
`cargo test --lib --no-default-features audio_`. A missing `bind` arm or enum
map fails to compile - that is a code change for the issue in hand, not for
this skill. Record every check failure; they are the first findings.

## 2. What changed since the last audit

Read `audited_at` from `audio/interactions.toml`. Run
`git log --oneline <audited_at>..HEAD` and
`git diff --stat <audited_at>..HEAD -- src/simulation src/net src/*.rs`.
No `audited_at`, or the user asked for a full audit: do the full pass (§3)
over the whole tree.

## 3. The passes

Incremental: one pass over the changed files. Full: five Explore agents in
parallel, one per domain - weapons; specials and air support; the world
(tiles, props, fire, water, weather, biomes, portals, volcano); tanks and the
round (locomotion, damage, wrecks, waves, the bong, pickups, training); and
presentation and online (HUD, menus, builder, `net/`). Give each this brief:

> For every entity and every interaction between entities in your domain,
> report: source and target entity; a verb; what triggers it (an `Event`
> variant with its file:line, a state edge, continuous state, player input,
> UI, or cosmetic only); whether an online client gets it (`NOT_SENT` in
> src/net/events.rs, own-shot prediction in src/net/round.rs, snapshot
> state, local only); what already draws it; how often it fires and whether
> it floods; and the sound it needs - cue id, one-line description, kind
> (one_shot, loop, modulated_loop, stinger, ui, voice), priority (critical,
> high, medium, low), what it varies by. Mark gaps: no event, not sent,
> missing payload, wrong granularity, online timing, volume risk, missing
> feedback. Cite file:line for every claim. Report only what is new or
> changed against audio/interactions.toml, which you read first.

## 4. Merge

- Deduplicate against `audio/interactions.toml` by (source, target, verb).
- Add new interactions with their gaps; update changed ones; mark removed
  ones `retired = true` with the commit, never delete them.
- For each cue an interaction names that the manifest lacks, add a
  `[cue.<id>]` with its behaviour and a `[sample.<id>]` with
  `status = "planned"` and a draft prompt that follows docs/SOUND_SPEC.md.
- Check every cited file:line exists (a scratchpad script); fix or drop the
  ones that do not.
- Set `audited_at` to `HEAD`.

## 5. Gaps to Linear

Each hook gap that needs a simulation or wire change (a new event, a payload
field, a `NOT_SENT` entry) is filed with the `linear-file` skill as its own
issue (labels: Improvement, gameplay or server; parent BB-102), one issue per
change, naming the interactions it unblocks.

## 6. Report

Rebuild the map (`just sound-map`). Tell the user: how many interactions,
cues and gaps were added, changed or retired; the new critical and high cues;
the issues filed; and what `sound-gen` would now make, with an estimate. If
the user wants a link, publish `target/soundmap/index.html` as an artifact
(updating the existing Sound Map artifact by its URL).
```

## Appendix B: the `sound-gen` runbook

`.claude/skills/sound-gen/SKILL.md`, written with P3. Its steps, with the exact connector calls:

1. **Read** `audio/cues.toml` and `docs/SOUND_SPEC.md`. Select the samples: those with `status` `planned` or `rejected`, or the ones the user named. Group them into batches of at most 20, by bus.
2. **For each batch:**
   1. `creative_create_flow {name: "bongbong sfx 2026-12-01 weapons-1"}` returns the `flow_id`.
   2. For each sample, `creative_add_flow_node`:
      ```
      {flow_id, node_type: "sfx", model_id: "eleven_text_to_sound_v3",
       label: "<sample id>", prompt: "<prefix>, <prompt>, <suffix>",
       model_parameters: {duration_seconds, prompt_influence, loop}}
      ```
      It returns the `node_id`.
   3. `creative_run_flow_nodes {flow_id, node_ids, estimate_only: true}` returns the credits. Report them, and ask the user if the batch is over 2,000 or the session's total passes the budget they gave.
   4. `creative_run_flow_nodes {flow_id, node_ids}`. Leave `generations_count` at its default of 4, or the sample's `takes_wanted`. **Never call it again to retry:** every call is a new charge.
   5. Poll `creative_get_flow_run_status {flow_id, session_ids}`, waiting `poll_after_seconds` between calls, until `all_completed` or `has_failures`.
   6. `creative_get_flow_node {flow_id, node_id}` lists each node's generation ids. `creative_get_generations` (up to 4 ids per call) returns their signed `full_res_url`s, which expire, so fetch them at once.
   7. Run `python3 -I tools/audio/fetch.py --sample <id> --url <url> --generation <gen>` once per take. Each take is saved to `tools/audio/_takes/<id>/<gen>.mp3`.
   8. Write each take's `flow`, `generation` and `made` into the manifest, and set the sample's `status = "review"`.
3. **Audition.**
   - Publish or update the audition artifact. Load the `artifact-capabilities` skill first; the page declares the `assets` and `db` capabilities.
   - Upload the takes as assets.
   - Each row shows the sample, its prompt and its takes, with pick and reject buttons that write `{sample, generation, verdict, note}` to the database.
   - Tell the user the link, and also the ElevenLabs flow URLs.
4. **Picks.** Read the picks with `ArtifactData` `query`.
   - A rejected sample goes back to `planned`, with the note folded into its prompt.
   - For each pick, run `tools/audio/process.py --sample <id> --take <gen>`, which runs the §4.12 chain and writes `static/audio/<bus>/<id>_<n>.ogg` and the measurements. Then set `status = "approved"`.
5. **Verify.**
   - `just check-sounds` and `cargo run --bin soundmap -- --check`.
   - Play the cues in the game with `audio_play` through the dev server, or render a scenario with `audio_render`.
   - Rebuild the sound map.
6. **Report:** what was made and approved, the credits spent, and what is still planned.

**Music** follows the same steps with `node_type: "music"`, `model_id: "eleven_music_v2_5"` and `model_parameters: {duration_seconds: <bars × 4 × 60 / bpm>, instrumental: true}`. `process.py --music` trims to the bar grid and seams the loop.

**Voice:**
- The voice is made once: `creative_design_voice` with a description and a sample line, then `creative_save_designed_voice`. The `voice_id` is stored in `[style.voice]`.
- Each line: a `tts` node with that `voice_id`, the text from `lang/<tag>.ftl` and `language_code`.
- For the announcer, add an `audio-effects` node connected from it with `filter_preset_id: "old_radio"`. It is free.

## Appendix C: the first slice and the direction audition

**The six sounds:**

| Sample | Seconds | Loop | Takes | Draft prompt (before the style prefix) |
|---|---|---|---|---|
| `cannon_medium_near` | 1.2 | no | 4 | tank main gun firing, sharp concussive crack, deep body, metallic breech clank |
| `hit_shell_metal` | 0.7 | no | 4 | heavy shell striking thick steel armour, hard metallic clang, short rattle |
| `hit_shell_masonry` | 0.7 | no | 4 | shell smashing into a brick wall, crunchy thud, falling grit and chips |
| `wreck_tank` | 2.5 | no | 4 | small armoured vehicle exploding, deep punchy boom, metal debris, crackling fire tail |
| `blast_barrel_oil` | 2.0 | no | 4 | oil drum exploding, quick fiery whoomp, steel barrel clatter |
| `engine_diesel_medium_idle` | 4.0 | yes | 4 | medium diesel tank engine idling, steady low rumble, faint track squeak |

**The directions:**

- **A. Real, heavy:**
  - the style prefix is "close-mic, dry, realistic military foley, no music, no reverb";
  - `prompt_influence` 0.5.
- **B. Arcade punch:**
  - the style prefix is "punchy, exaggerated, larger than life, tight transient, video game sound effect, dry, no music";
  - `prompt_influence` 0.4.
- **C. Retro crunch:**
  - no generation. A's picks go through a 12-bit, 22 kHz bit-crush and a gentle low-pass in `process.py`, to hear how the pixel art's era sounds on real sources.

**The cost.** The six sounds are 11.1 s, × 4 takes × 2 generated directions = 89 s. That is about 1,000 to 3,600 credits, priced with `estimate_only` before the run.

**The audition page** plays each sound in A, B and C side by side, then each set in sequence over a recorded engine bed. The owner picks a direction, and `docs/SOUND_SPEC.md` is written from it.
