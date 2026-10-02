# Patterns from shipped games

Status: research, the companion catalogue to
docs/large-maps-follow-camera.md (section 13 there summarises what it
confirms and what it changed). Written 2026-10. What shipped games do
about the problems that document works on - following a tank over a map
larger than the screen, showing what is off screen, laying out a HUD for
thumbs and mice alike, keeping a cross-play room fair, pacing a large
map, and editing one - and what bongbong should take from each:
**Adopt**, **Adapt**, **Already does** or **Skip**. 72 patterns: 40 to
adopt, 20 to adapt, 4 bongbong already follows, 8 to skip. The Camera
Lab page carries the same catalogue with filters, and a button on each
pattern it can demonstrate.

How sure each source is: **[read]** the page or source code was opened
and read; **[summary]** only a search engine's summary of the page was
seen, because most game wikis, press sites and forums could not be
opened from the research sandbox; **[forum]** a community post. Treat
[summary] and [forum] items as leads to confirm before quoting them as
fact. The source code read directly (Celeste, DDNet, OpenRA, Teeworlds,
Suroi, Tiled, Wesnoth, Ultimate Doom Builder, the Godot demos, raylib)
is the most reliable evidence here.

Contents

1. Camera and framing
2. Off-screen awareness and telegraphs
3. HUD and controls
4. Fairness across devices and inputs
5. Large maps: pacing, spawning and orientation
6. The builder

## 1. Camera and framing

| Pattern | Who does it | Verdict | For bongbong |
|---|---|---|---|
| **Smoothed follow with a leash.** The camera eases toward its target, independent of frame rate, and never trails more than a set distance. Apple's motion guidance: whole-scene motion causes discomfort, and people are very sensitive to oscillation around 0.2 Hz. | Celeste: each frame closes the gap by 1 - 0.01^dt (99% a second) toward a target clamped to the room; Super Meat Boy (course exercise on Keren's taxonomy): lerp follow with separate follow and catch-up speeds and a maximum leash; Mindustry: a smooth-camera setting, on by default | Adopt | A critically damped spring (no overshoot, no sway) plus a hard leash, so a boosted tank never outruns its own sight box. |
| **Camera window and snapping.** The target moves freely inside a box; the camera moves only when it is pushed, or snaps on a stable event. | Rastan: a window as tall as a jump, so jumping does not scroll; Super Mario World: snaps to the platform's height on landing; Super Mario Bros.: a push box with a speed-up zone | Adopt | A window of about half a cell to a cell. It also swallows the hull settling after a stop, ram knockback and small online Placed corrections, which would otherwise shake the whole screen. |
| **Forward focus and aim offset.** The view leads in the direction the player moves or aims. | Cave Story: dual forward focus: the focal point slides slowly to the side the player walks toward; DDNet: offset along the aim, past a dead zone, capped at 350 x zoom, with smoothing settings; Enter the Gungeon: drifts toward the aim; an Aim Look slider down to 0, which the developers point motion-sick players to; Hotline Miami: hold Shift to look far ahead, at the risk of losing sight of yourself | Adapt | Lead along the commanded direction with a short hold before a U-turn flips it, capped so the sight box stays on screen behind the tank, with a strength slider. No manual peek: touch and the couch layouts have no spare input. |
| **Bounds, lock areas, pan or cut.** The camera is clamped to the level; authored areas lock it; adjacent spaces pan, distant ones cut. | Celeste: target clamped to the room, per-level offsets, anchors placed by triggers, one state that snaps; Hollow Knight: CameraLockArea volumes with min and max bounds and look-up/down locks; The Legend of Zelda (NES): moving to the next room pans the whole screen | Adopt | Clamp to the field and centre any axis where the map is smaller than the view. Cut on a portal or a large Placed correction. Lock areas can come later as a builder tool (the frog's yard). |
| **View range as a game rule.** The amount of world a player sees is fixed by the game, not by the screen. | surviv.io: buildings force the base zoom so no one can outspot you; Shovel Knight, Celeste, Hyper Light Drifter: fixed internal resolutions (400x240, 320x180, 480x270), bars on other shapes; diep.io: field of view is a class stat; one upgrade adds only view | Adopt | The view area is a difficulty dial beside enemy_view_range, set by the room for every seat, not by each screen. Decided: 578 cells in a room; a round with no room lets a big screen zoom out to about 40 x 22.5 cells, since no other screen shares it. |
| **Dynamic zoom.** The view zooms with speed, enemy count or dramatic moments. | GTA 2: the top-down camera rises with speed; Gauntlet (2014): zoom changes with spawners and enemy counts; players report being pinned at the edge; Super Smash Bros. Ultimate: Special Zoom freezes and zooms on decisive hits | Skip | Continuous zoom resamples the 2 px blocks so they shimmer, and it moves the difficulty dial mid-round. Speed shows through the look-ahead instead. |
| **Shared screen with zoom-to-fit and a leash.** Local co-op frames everyone on one screen, zooms out as they spread, then holds them at the edge. | Super Smash Bros.: averages the fighters and zooms out, too far on huge stages; Diablo III (console): zooms out to a limit, then holds players on screen; New Super Mario Bros. Wii: the screen edge drags anyone who lags behind | Skip | A leash pins a tank against the edge while it is under fire, and zoom breaks the block grid. Couch play gets the Voronoi split instead. |
| **Dynamic (Voronoi) split screen.** One view while players are close; when they separate, a split line at any angle gives each a half that points at the other. | LEGO games: one view while close, a split at any angle when apart; Godot demo project: the split line follows the players' positions; credits the LEGO games | Adopt | For couch play (desktop only): one view while both sight boxes fit, otherwise split along the bisector with each half shifted toward the partner. No zoom, a 2 px divider. |
| **One camera per online client.** Online co-op gives each player their own view rather than one shared screen. | Diablo III, Vampire Survivors: online players roam freely; local players share a screen; Helldivers (2015): kept one shared screen online by design; long threads ask for it to change | Adopt | Each window follows its own seat; teammates off screen get edge arrows in their ring colour. |
| **Spectate, then cut back on respawn.** A dead player watches a living teammate and cuts to their own respawn. | DDNet: the spectator camera reuses the follow code on the watched player; Apex Legends: respawned squadmates arrive on a visible dropship; Super Smash Bros.: a revival platform with a short invincibility | Adopt | Hold on your wreck for a second or two, then follow a living seat, cutting between seats; cut to the gate when the next wave brings you back, with the locate ripple. |
| **Death recap.** After a death, the game shows what killed you and from where. | Call of Duty 4: a killcam replays your death from the killer's view; League of Legends: Death Recap shows damage by source and time without control | Adapt | One line of text and an arrow along the row or column the killing shot came from (all fire is cardinal, so the direction is exact). No replays. |
| **Establishing shot.** A round opens on the whole arena, then cuts or pans to the player. Apple: let people cancel motion, especially motion they see more than once. Xbox guideline 117: let players disable automatic camera movement. | Splatoon: the camera pans across the stage, then cuts to the players at spawn; Mario Kart World: a Grand Prix opens with a cinematic of its courses | Adapt | On a level's first play only, skippable by any input, not after an R restart; a dissolve instead of the zoom when reduced motion is on. |
| **Where-am-I cue on every cut.** After any jump of the view, the player's own unit is marked. | Brawl Stars: your name and circle are green, allies blue, enemies red | Adopt | Fire the existing locate ripple and P label on every cut: round start, respawn, portal, split or merge, spectate switch. With eight seats, the local one needs a cue beyond its colour. |
| **Side normalization.** Team games rotate the map so every player starts from the same corner. | Wild Rift: rotates the red side's map so both teams start bottom-left; lanes renamed Baron and Dragon; Brawl Stars: both teams see themselves as blue at the bottom | Skip | Co-op puts everyone on one side, and the fixed shadow direction, north-south currents and edge gates would break. The lesson that survives: keep the tank's surroundings out from under the thumbs. |
| **Shake attenuated, capped and scalable.** Shake falls off with distance from the viewer, is capped, and players can scale or turn it off. | OpenRA: intensity is the sum of shakes over distance² from the viewport centre, capped; Nuclear Throne: screenshake 0 to 200%; Celeste: added a shake option, later defaulting to 50%; Xbox Accessibility Guideline 117: avoid camera shake or provide an option to turn it off | Adopt | Off, 50% and 100% per client scaling screen_fx_intensity, and attenuation by distance from that client's camera, so a blast across the map does not shake your screen. |
| **One reduce-motion switch, seeded by the OS.** A single setting tames all camera motion, defaulting to the system's preference. | Apple: detect the system Reduce Motion setting; replace meaningful motion with a dissolve; make motion optional; Game Accessibility Guidelines: an adjustable field of view (3D) among motion settings | Adopt | Seed it from iOS Reduce Motion and prefers-reduced-motion on the web. On: look-ahead and shake off, the intro, spectate and split transitions become cuts or dissolves. |
| **Crisp or fill, and a separate UI scale.** Pixel-art games offer pixel-perfect and filled scaling, and size the UI apart from the world. | Shovel Knight: an official pixel-perfect option; other resolutions get slight smoothing; Enter the Gungeon: Pixel Perfect, Uniform and Fast scaling modes; Hyper Light Drifter: Pixel Perfect plus a scaled mode for resolutions with no even multiple; Mindustry: UI scale 25 to 300% and a pixelate option | Adopt | Because the art sits on 2 px blocks, display scales in 0.5 steps are already block-perfect; offer crisp and fill, and draw the HUD at its own scale. |
| **Snap the camera, slide the picture.** The camera moves in whole pixels; the finished frame is shifted by the fraction thrown away. | Godot pixel camera: renders 320x180 into 322x182, rounds the camera each physics tick, and a short shader slides the picture back by the fraction; voithos demo: the same technique; sprites still move in whole pixels, so the followed one can jitter; raylib: core_smooth_pixelperfect does it with a second Camera2D | Adopt | Snap to the 2 px block, keep one block of slack, apply the remainder when presenting; run the camera on the clock its target moves on, and draw the followed tank in the camera's block phase so it does not wobble by a pixel. |

Sources:

- Celeste: <https://github.com/NoelFB/Celeste/blob/master/Source/Player/Player.cs> [read]
- Super Meat Boy (course exercise on Keren's taxonomy): <https://github.com/dr-jam/CameraControlExercise> [read]
- Mindustry: <https://github.com/Anuken/Mindustry/blob/master/core/src/mindustry/ui/dialogs/SettingsMenuDialog.java> [read]
- Rastan: <https://www.gamedeveloper.com/design/scroll-back-the-theory-and-practice-of-cameras-in-side-scrollers> [summary]
- DDNet: <https://github.com/ddnet/ddnet/blob/master/src/game/client/components/camera.cpp> [read]
- Enter the Gungeon: <https://steamcommunity.com/app/311690/discussions/0/364040166670065237/> [forum]
- Hotline Miami: <https://steamcommunity.com/app/219150/discussions/0/1620599015875791416/> [forum]
- Hollow Knight: <https://github.com/PaleCourt/PaleCourt-Unity/blob/master/Assets/Scripts/Assembly-CSharp/CameraLockArea.cs> [read]
- The Legend of Zelda (NES): <https://www.gridbugs.org/zelda-screen-transitions-are-undefined-behaviour/> [summary]
- surviv.io: <https://survivio.fandom.com/wiki/Buildings> [summary]
- Shovel Knight, Celeste, Hyper Light Drifter: <https://www.pcgamingwiki.com/wiki/Shovel_Knight:_Treasure_Trove> [summary]
- diep.io: <https://diepio.fandom.com/wiki/Ranger> [summary]
- GTA 2: <https://thekingofgrabs.com/2022/04/25/grand-theft-auto-2-playstation/> [summary]
- Gauntlet (2014): <https://steamcommunity.com/app/258970/discussions/0/520518053439668399/> [forum]
- Super Smash Bros. Ultimate: <https://www.ssbwiki.com/Special_Zoom> [summary]
- Super Smash Bros.: <https://www.ssbwiki.com/Camera> [summary]
- Diablo III (console): <https://www.co-optimus.com/review/1289/diablo-3-console-co-op-review.html> [summary]
- New Super Mario Bros. Wii: <https://www.mariowiki.com/New_Super_Mario_Bros._Wii> [summary]
- LEGO games: <https://gamefaqs.gamespot.com/boards/119316-lego-jurassic-world/71981052> [forum]
- Godot demo project: <https://github.com/godotengine/godot-demo-projects/tree/master/viewport/dynamic_split_screen> [read]
- Diablo III, Vampire Survivors: <https://vampire.survivors.wiki/w/Co-op_mode> [summary]
- Helldivers (2015): <https://steamcommunity.com/app/394510/discussions/0/494631967652452527/> [forum]
- Apex Legends: <https://apexlegends.fandom.com/wiki/Respawn_Beacon> [summary]
- Super Smash Bros.: <https://www.ssbwiki.com/Revival_platform> [summary]
- Call of Duty 4: <https://callofduty.fandom.com/wiki/KillCam> [summary]
- League of Legends: <https://www.riftherald.com/lol-gameplay/2019/7/17/20697931/new-league-death-recap> [summary]
- Splatoon: <https://splatoonwiki.org/wiki/Opening> [summary]
- Mario Kart World: <https://www.mariowiki.com/Mario_Kart_World> [summary]
- Brawl Stars: <https://brawlstars.fandom.com/wiki/User_blog:Speedmaster6670/Brawl_Basics:_How_to_Play> [summary]
- Wild Rift: <https://leagueoflegends.fandom.com/wiki/Map_(Wild_Rift)> [summary]
- Brawl Stars: <https://en.namu.wiki/w/%EB%B8%8C%EB%A1%A4%EC%8A%A4%ED%83%80%EC%A6%88/%EB%B2%84%EA%B7%B8> [summary]
- OpenRA: <https://github.com/OpenRA/OpenRA/blob/bleed/OpenRA.Game/Traits/World/ScreenShaker.cs> [read]
- Nuclear Throne: <https://steamcommunity.com/app/242680/discussions/0/535152511377555483> [forum]
- Celeste: <https://steamcommunity.com/app/504230/discussions/0/1692659769951781925/> [summary]
- Xbox Accessibility Guideline 117: <https://learn.microsoft.com/en-us/gaming/accessibility/xbox-accessibility-guidelines/117> [summary]
- Apple: <https://developer.apple.com/help/app-store-connect/manage-app-accessibility/reduced-motion-evaluation-criteria> [read]
- Game Accessibility Guidelines: <https://gameaccessibilityguidelines.com/if-the-game-uses-field-of-view-3d-engine-only-allow-a-means-for-it-to-be-adjusted/> [summary]
- Enter the Gungeon: <https://steamcommunity.com/app/311690/discussions/1/1736588252417556241> [forum]
- Hyper Light Drifter: <https://steamcommunity.com/app/257850/discussions/0/365163686063144831> [forum]
- Godot pixel camera: <https://github.com/mjasnikovs/godot-pixel-perfect-camera> [read]
- voithos demo: <https://github.com/voithos/godot-smooth-pixel-camera-demo> [read]
- raylib: <https://github.com/raysan5/raylib/blob/master/examples/core/core_smooth_pixelperfect.c> [read]

## 2. Off-screen awareness and telegraphs

| Pattern | Who does it | Verdict | For bongbong |
|---|---|---|---|
| **Edge pointers for a chosen few.** Arrows at the screen edge go to selected targets, not to every entity. | Among Us: yellow arrows to your next task, flashing ones to a sabotage; a comms sabotage removes them; Vampire Survivors: the Milky Way Map relic adds edge arrows with the icon of every item and secret; Everspace 2: off-screen arrows only for tagged enemies, a double arrow for the locked target; Halo 3 (HUD spec): waypoints scale with distance but never under 50% or over 100% of their size | Adapt | Keep the distance encoding (the 60% minimum matches Halo 3) and the cap of eight, but fill it by priority: lane threats, teammates, the frog, then the nearest enemies; fold the rest into one count per edge. |
| **Allies can always be found.** Teammates are never lost: outlines through walls, arrows on the radar edge. | Overwatch: an option to see friendly outlines through walls; Halo 3: a waypoint beyond the motion tracker rides its edge as an arrow | Adopt | Teammates are never merged into a count and keep their ring colour and P label on the arrow; a teammate waiting at a gate shows there. |
| **Radar blips clamped to the edge.** Out-of-range markers sit on the radar border, pointing the way. | GTA (Vice City on): out-of-range blips sit on the radar border; triangles for above or below; vhud-rs (GTA V radar port): documents circular and rectangular clamping | Adapt | A 96 x 54 field fits a whole-map minimap; past that, the minimap becomes a radar window whose border holds the frog, gates and objectives, using the same projection as the screen-edge arrows. |
| **Firing reveals you, briefly.** A hidden unit that shoots shows up for a moment. | Call of Duty: unsuppressed fire puts a red dot on enemy minimaps; MW3 restored it after players objected to its removal; League of Legends: attacking from brush reveals a small radius for 2 s; Brawl Stars: a brawler in a bush is hidden only from enemies two or more tiles away; Halo Infinite: a preview that showed only sprinting and shooting on the tracker confused players and was reverted | Adopt | A tank in tall grass gets its arrow for 1.5 s after it fires (League uses 2 s), and always within about two cells. The rule applies only to concealed tanks; Halo Infinite shows that hiding every quiet enemy confuses players. |
| **Last-seen ghosts that never update.** A unit that disappears leaves a marker frozen where it was last seen. | StarCraft, Warcraft II: buildings under fog keep their last known state; OpenRA: FrozenUnderFog: stays visible but not updated once discovered | Adopt | A tank that slips into grass leaves a hollow, fading arrow and a hollow minimap square where it was last seen, and the marker never moves while the tank is hidden, or it would leak the position. |
| **Pings instead of voice.** Players mark enemies and places for their team with one input. | Apex Legends: the ping system was built after a month of playtests with voice banned; pings last about 15 s; Helldivers 2: a ping highlights an enemy or place for about 6 s with its distance | Adapt | Later: a room of up to eight phones has no voice. On tablets and desktops a ping comes from a tap on the minimap (the field halves are steer and fire); phones, which have no minimap, would need a ping button. A ping shows to teammates as an edge arrow for 6 to 10 s. |
| **Minimap where the hands are not.** On touch screens the minimap goes in a top corner. | Mobile Legends, Call of Duty: Mobile: minimap top-left; Wild Rift: top-left by default, movable; PUBG Mobile: the upper corners | Adopt | A top corner, under the right-hand cluster, on tablets and desktops. Decided: none on phones, where the edge arrows carry the field (Vampire Survivors keeps its map in the pause menu). |
| **An aim telegraph only the target sees.** Being aimed at shows on the victim’s screen before the shot. | Battlefield 4: scopes glint only within about 10 degrees of the sniper’s aim; Fortnite: a sniper glare visible while scoped | Adopt | The lane warning pulses only on the seat the enemy is lined up on (inside enemy_fire_align_px), from the moment its 0.25 s aim starts, not at the shot. |
| **Two-stage lock warning.** A warning while a weapon locks on, a stronger one once it fires or locks. | Battlefield (vehicles): a beeping tone while being locked, a solid one when locked; flares answer it; Everspace 2: incoming missiles bracketed, with an edge marker when off screen; World of Tanks: Sixth Sense lights 3 s after you are spotted | Adopt | Pulse while the enemy settles its aim, flash when it fires; no delay (unlike Sixth Sense) and cleared the moment the alignment breaks. |
| **Show where danger comes from.** Hits and nearby threats are drawn as a direction. | Call of Duty 4: a grenade icon with an arrow, and the direction of incoming damage; Fortnite: Visualize Sound Effects: a ring pointing at footsteps, gunfire and chests | Adapt | bongbong has no audio, the channel most shooters use for off-screen danger. Stand-ins: an arc on your tank pointing at whoever hit you (in the lab), and a short edge tick for blasts just off screen. |
| **One meaning per colour; telegraphs that read.** Danger keeps one colour and shape everywhere; enemy fire draws above everything. | Combat design (anatomy of an attack): anticipation, attack, recovery, each anticipation distinct; Hades II: criticised for yellow telegraphs in one fight when red means danger everywhere else; Shoot ’em ups: enemy bullets draw above everything with a clear origin | Adopt | One red for "lined up on you" across the arrow, the lane warning, the hit arc and the minimap; shots stay above the HUD fade. |

Sources:

- Among Us: <https://among-us.fandom.com/wiki/Tasks> [summary]
- Vampire Survivors: <https://vampire-survivors.fandom.com/wiki/Milky_Way_Map> [summary]
- Everspace 2: <https://everspace.fandom.com/wiki/Heads-up_display_(ES2)> [summary]
- Halo 3 (HUD spec): <http://www.cand.land/halohud> [summary]
- Overwatch: <https://us.forums.blizzard.com/en/overwatch/t/psa-turn-this-setting-on/715741> [forum]
- GTA (Vice City on): <https://www.grandtheftwiki.com/Radar> [summary]
- vhud-rs (GTA V radar port): <https://github.com/bhubbard/vhud-rs> [read]
- Call of Duty: <https://www.charlieintel.com/call-of-duty/modern-warfare-3-brings-back-classic-red-dot-minimap-266546/> [summary]
- League of Legends: <https://wiki.leagueoflegends.com/en-us/Basic_attack> [summary]
- Brawl Stars: <https://pro-brawl-stars-tipsntricks.fandom.com/wiki/Bushes> [summary]
- Halo Infinite: <https://gameinformer.com/2021/09/09/343-makes-halo-infinite-gameplay-change-to-radar-following-criticism> [summary]
- StarCraft, Warcraft II: <https://liquipedia.net/starcraft2/Fog_of_War> [summary]
- OpenRA: <https://github.com/OpenRA/OpenRA/blob/bleed/OpenRA.Mods.Common/Traits/Modifiers/FrozenUnderFog.cs> [read]
- Apex Legends: <https://www.gamerevolution.com/news/493241-apex-legends-ping-system> [summary]
- Helldivers 2: <https://gamerant.com/helldivers-2-how-ping-mark-map-locations/> [summary]
- Mobile Legends, Call of Duty: Mobile: <https://callofduty.fandom.com/wiki/Minimap> [summary]
- Wild Rift: <https://playerassist.com/league-legends-wild-rift-change-hud-layout/> [summary]
- PUBG Mobile: <https://www.wesplays.com/wes-plays/pubg-mobile-hud> [summary]
- Battlefield 4: <https://battlefield.fandom.com/wiki/Scope_Glint> [summary]
- Fortnite: <https://fortnite.fandom.com/wiki/Sniper_Glare> [summary]
- Battlefield (vehicles): <https://battlefield.fandom.com/wiki/Weapon_lock> [summary]
- World of Tanks: <https://wotlytics.com/mechanics/spotting/> [summary]
- Call of Duty 4: <https://strategywiki.org/wiki/Call_of_Duty_4:_Modern_Warfare/HUD> [summary]
- Fortnite: <https://gamingaccessibility.org/how-fortnites-visualize-sound-effects-shows-the-direction-of-selected-sounds/> [summary]
- Combat design (anatomy of an attack): <https://gdkeys.com/keys-to-combat-design-1-anatomy-of-an-attack/> [summary]
- Hades II: <https://bloomedwings.com/2025/11/23/not-just-the-writing-hades-iis-issues-are-entrenched-in-mechanics/> [summary]
- Shoot ’em ups: <https://shmups.wiki/library/Boghog's_bullet_hell_shmup_101> [summary]

## 3. HUD and controls

| Pattern | Who does it | Verdict | For bongbong |
|---|---|---|---|
| **Four-corner thumb-zone layout.** Information sits in the top corners, the thumbs own the bottom corners and the middle belongs to the world. | Honor of Kings: minimap top-left, dashboard top-right, stick bottom-left, skills bottom-right; Pokémon Unite: minimap top-left with a position setting, scoreboard top-right; PUBG Mobile: moved the minimap and team info to the upper corners so the hands stop covering them; Apple HIG: secondary controls such as menus at the top, frequent ones by the thumbs, clear of where the stick goes | Adopt | Two clusters in the top corners, nothing in the bottom ones where the stick and fire thumb rest. The minimap only on field maps and not on phones, under the top-right cluster; an arena needs none. |
| **Vitals drawn on the unit.** Health and ammo sit on the character, where the player is already looking. | Brawl Stars: health bar with the number under your name, a segmented ammo bar that refills between attacks; League of Legends: champion bars with a tick every 100 HP and a heavier one every 1000; Vampire Survivors: health bar on the character, XP across the top | Adopt | The health ring already exists. Add shell pips as a short arc and ticks at the 25/50/75 damage tiers; the corner readout becomes the precise copy. |
| **Contextual vitals.** A readout appears when it changes or matters and fades otherwise. | Assassin's Creed Odyssey: health shows in combat and on damage, twenty HUD toggles and presets; Dead Space: health on the suit's spine, blinking under 25%; Apple HIG: show and hide virtual controls to reflect gameplay | Already does | Enemy rings already appear after a hit or under a threshold. Keep your own and teammates' state always visible (it is the co-op information); let the weapon queue dim when it has not changed for a while. |
| **HUD that gets out of the way.** An element that would hide play moves or turns translucent. | Stardew Valley: the toolbar jumps to the top when the player nears the bottom edge, with a lock option; PUBG Mobile, Call of Duty: Mobile: per-element transparency sliders | Adopt | Fade a corner cluster to about 35% while a tank, shell, pickup or frog is under it: a rectangle test on positions the game already has. Fading suits four corners better than jumping, since there is no free corner to jump to. |
| **Layout editor with presets.** Players drag, resize and fade each control and save layouts. | Brawl Stars: Edit Controls: size per button, drag to move, lock the stick, swap sides; Call of Duty: Mobile: a mock HUD where everything can be dragged, resized and faded; Wild Rift: size and edge offset per button, three saved layouts | Adapt | Two touch zones and two clusters do not need an editor. Ship HUD scale, HUD opacity and the stick's leash (already the touch_follow_radius_pt knob) as settings. |
| **Mirrored controls as a setting.** Left-handed players swap the stick and fire sides at runtime. | Brawl Stars: swap attack and movement sides in Edit Controls; Call of Duty: Mobile: stick on the right, fire on the left for southpaws | Skip | Decided: the stick side stays a build-time choice (the touch-steer-right feature) for now. The corner clusters leave both bottom corners to the thumbs, so both builds use the same HUD; the touch scheme already takes the side as an argument, so a setting later would change only where app.rs reads it from. |
| **Floating stick with feedback around the finger.** The stick appears where the thumb lands, and presses glow beyond the finger. | Apple HIG: show a thumbstick wherever the thumb lands; a press state the player can see under the finger; Wild Rift: Fixed, Floating and Follow stick modes; Vampire Survivors (mobile): a Visible joystick toggle, because the stick hides the hero's surroundings | Already does | The stick already floats and trails the thumb. Add a hide-stick toggle and make the fire press ring large enough to show around the thumb. |
| **One control, two intents.** A tap does the assisted version of an action, a drag or hold the manual one. | Brawl Stars: tap attacks the nearest target, drag aims and fires on release; Apple HIG: combine functionality into a single control; use double tap and touch and hold for variations | Adapt | Aim is the hull's facing, equally coarse on keys and touch, so no aim assist is needed. A double tap or long press can carry a second action instead of a new button. |
| **Opt-in automation.** The game takes over the hardest touch task if the player asks it to. | Archero: the hero fires at the nearest enemy automatically while standing still; Call of Duty: Mobile: Simple mode fires when the crosshair is on an enemy; Fortnite (mobile): Auto Fire, Tap Anywhere or a dedicated button | Adapt | In co-op against the AI an opt-in 'fire when an enemy is in my lane' costs no fairness; it can use the same line-of-sight gate the AI fires through. player_shot_hit_pad_px is already an assist that works the same on every device. |
| **One HUD, input-aware hints.** The same HUD everywhere, sized per device, with touch hints only for touch and glyphs for the input last used. | Diablo Immortal (PC): kept the mobile interface but shrank the portrait, minimap and buttons and added hotkeys; Google Play Games on PC: replace touch controls with mouse and hotkeys, ship keyboard tutorials; Apple HIG: use glyphs that match the controller the player is using | Adopt | One pair of clusters everywhere, sized in points; stick and fire hints only on touch, switching on the input last used (an iPad with a keyboard, a laptop with a touchscreen). |
| **UI scale separate from world zoom.** Players set how big the HUD is independently of how much world they see. | Stardew Valley: UI scale 75 to 150% separate from zoom since 1.5; Terraria: separate Zoom and UI Scale sliders; console and mobile layouts; Fortnite: a HUD scale percentage and a safe-zone slider | Adopt | Draw the clusters in screen space after the field is presented, at their own scale in whole steps so the pixel font stays crisp. The world zoom is the room's rule; the HUD scale is the player's. |
| **Minimum sizes in physical units.** Text and touch targets have floors in points or dp, inside the safe area. | Apple HIG: text 11 pt minimum, controls 44 x 44 pt, respect the safe area; Android: touch targets of at least 48 x 48 dp; Xbox Accessibility Guideline 101: 26 px text at 1080p, resizable to 200%; Game Accessibility Guidelines: 28 px at 1080p as a floor, 46 px for brief text | Adopt | Today the bar's 10 px labels render at 6.8 pt on an iPhone 15 and the 44 px lobby buttons at 27 to 32 pt. State minimums in pt and dp and convert through the view scale. |
| **Never colour alone.** Identity and state use shape, numbers or motion as well as hue. | Apple HIG: convey information with more than colour; offer shapes or icons; League of Legends: its colour-blind mode recolours bars and was criticised as not enough on its own | Adapt | Eight team colours cannot all stay distinct: print the seat number on every chip and arrow (P1 to P8), and show low health with a blink or fewer segments, not only a colour step. |

Sources:

- Honor of Kings: <https://www.researchgate.net/figure/Game-UI-of-Honor-of-Kings-1v1-In-the-main-screen-there-are-four-sub-parts-mini-map-A_fig2_338115939> [summary]
- Pokémon Unite: <https://game8.co/games/Pokemon-UNITE/archives/338353> [summary]
- PUBG Mobile: <https://www.wesplays.com/wes-plays/pubg-mobile-hud> [summary]
- Apple HIG: <https://developer.apple.com/design/human-interface-guidelines/game-controls> [read]
- Brawl Stars: <https://brawlstars.fandom.com/wiki/Beginner's_Guide> [summary]
- League of Legends: <https://wiki.leagueoflegends.com/en-us/Health> [summary]
- Vampire Survivors: <https://jboger.substack.com/p/the-secret-sauce-of-vampire-survivors> [summary]
- Assassin's Creed Odyssey: <https://www.gamerevolution.com/guides/437757-assassins-creed-odyssey-disable-ui-elements-customize-hud> [summary]
- Dead Space: <https://www.gamedeveloper.com/design/video-designing-i-dead-space-i-s-immersive-user-interface> [summary]
- Stardew Valley: <https://stardewvalleywiki.com/Options> [summary]
- PUBG Mobile, Call of Duty: Mobile: <https://www.playbite.com/q/how-to-change-your-hud-pubg> [summary]
- Brawl Stars: <https://www.techy.how/tutorials/brawl-stars-change-controls> [summary]
- Call of Duty: Mobile: <https://blog.activision.com/call-of-duty/2019-10/Getting-a-Grip-on-the-Call-of-Duty-Mobile-Controls> [summary]
- Wild Rift: <https://mobi.gg/en/tips/move-buttons-wild-rift/> [summary]
- Wild Rift: <https://www.thegamer.com/league-of-legends-lol-wild-rift-best-settings-guide/> [summary]
- Vampire Survivors (mobile): <https://www.androidpolice.com/vampire-survivors-guide/> [summary]
- Brawl Stars: <https://brawlstars.fandom.com/wiki/User_blog:Speedmaster6670/Brawl_Basics:_How_to_Play> [summary]
- Archero: <https://www.deconstructoroffun.com/blog/2019/8/9/why-archero-banked-25m-but-leaves-25m-hanging-hlx9n> [summary]
- Fortnite (mobile): <https://www.thespike.gg/fortnite/beginner-guides/how-to-turn-on-auto-fire> [summary]
- Diablo Immortal (PC): <https://news.blizzard.com/en-us/article/23797159/making-diablo-immortal-for-pc> [summary]
- Google Play Games on PC: <https://developer.android.com/games/playgames/input> [read]
- Stardew Valley: <https://stardewvalleywiki.com/Modding:Migrate_to_Stardew_Valley_1.5> [summary]
- Terraria: <https://terraria.wiki.gg/wiki/Settings> [summary]
- Fortnite: <https://www.videogamer.com/guides/how-to-adjust-screen-size-in-fortnite/> [summary]
- Apple HIG: <https://developer.apple.com/design/human-interface-guidelines/accessibility> [read]
- Android: <https://developer.android.com/guide/topics/ui/accessibility/apps> [read]
- Xbox Accessibility Guideline 101: <https://learn.microsoft.com/en-us/xbox/accessibility/xbox-accessibility-guidelines/101> [summary]
- Game Accessibility Guidelines: <https://gameaccessibilityguidelines.com/use-an-easily-readable-default-font-size/> [summary]
- League of Legends: <https://www.gamepressure.com/newsroom/lol-colorblind-mode-is-useles-for-players-suffering-from-actual-c/za2e03> [summary]

## 4. Fairness across devices and inputs

| Pattern | Who does it | Verdict | For bongbong |
|---|---|---|---|
| **Input-based matchmaking (PvP only).** Competitive games pool players by input and tune aim assist per pool. | Fortnite: keyboard players meet keyboard players; touch players meet controllers; Call of Duty: Mobile: touch only meets touch, controllers only controllers; Apex Legends: refused input pools and tunes aim assist per lobby instead | Skip | Rooms are invite-only co-op against the AI. If an assist ever ships, make it a tuning row so a room sets it for everyone. |
| **Co-op never splits by device.** PvE matchmaking keeps one cross-platform pool. | Destiny 2: all PvE in one cross-platform pool; only PvP splits PC from console; Helldivers 2: everyone matched together by default, with an opt-out; Diablo Immortal: PC and mobile players share one world | Already does | Rooms already mix devices and scale difficulty by seat count. Add a touch or keyboard glyph to each roster chip so teammates know who is on a phone. |
| **The same view for everyone.** No device sees more of the battlefield than another. | Teeworlds: a constant view area with per-axis caps; Wild Rift: 4:3 tablets saw less; Riot split the difference: more vertical view than phones, less horizontal; StarCraft II: caps the aspect ratio; a mod that lifts it calls the wider view an unfair advantage; Brawl Stars: black side bars on wider phones | Adopt | Same area is Teeworlds’ rule and Wild Rift’s compromise, and with it a phone and a tablet see the same amount of battlefield in different outlines. Decided for rooms at 578 cells; local play on a big monitor zooms out, since nobody shares that round. |
| **One simulation, frame rate as a fairness variable.** Gameplay runs at a fixed rate everywhere; only presentation follows the device. | Fortnite: 120 fps on 120 Hz iPads only at medium settings; Google Play 'Level Up': 60 fps by default: average 55 or more, P90 50, P99 30 | Already does | Fixed PHYSICS_FIXED_DT steps and the room's 60 Hz authority already make frame rate a presentation matter. What remains is holding 60 fps on phones with a camera that draws less. |
| **Off-screen enemies do not shoot.** Enemies may only attack from where the player can see them. | Shoot ’em up design: "off-screen enemies should not be able to shoot"; ceasefire zones near the edge; Diablo II: Resurrected: capped ultrawide at 19:9 because monster AI only reacts inside the original view, so wide screens hit monsters that never answered | Adopt | The sight box: enemies fire only from inside it, and every device shows it. Add a probe count of seat hits on enemies outside the seat’s box, since those cannot fire back until they close in (the Diablo II problem in reverse). |
| **Graphics settings never change what you see.** Low settings and weaker devices must not reveal more. | VALORANT: shaders run on a 2012 integrated GPU with no visibility differences between quality settings; Counter-Strike 2: low settings dropped player shadows; Valve added an option to keep them; PUBG: players on low foliage settings see prone enemies | Adopt | Concealment is already a cell rule. Check two things: phones halve fx_density, which thins smoke that may hide tanks, and tall grass must draw the same everywhere. |
| **Darkness no fallback can clear.** Night stays dark whatever the brightness setting or failed shader. | Rust: a small light around the player, distant pixels pure black so gamma cannot recover them; DayZ: anti-gamma grain at night; Escape from Tarkov: disallowed NVIDIA Freestyle filters | Adapt | Adapted: a device whose weather shaders fail drew a clear sky, so its player saw through the night. It now draws the sky without them (`weather::plain`): the light map multiplied onto the field by a blend mode, so the night stays as dark, with every headlight and shadow, and the fog, sand, rain and snow as plain blocks; `status.weather.without_shaders` reports it. |
| **Do not send what cannot be seen.** Competitive servers withhold hidden enemies, with a margin so nothing pops in. | VALORANT: withholds enemy positions until line of sight is possible, with a ping-sized lead; DDNet: clips to the client’s view plus 10 blocks so nothing appears late; Suroi: sends each player a square around its view, recomputed every 8 ticks | Skip | Co-op against the AI has no wallhack to stop, and the arrows need every position. If bandwidth ever demands it, cull to the union of the seats’ sight boxes plus a DDNet-style margin. |
| **Fill the screen, do not letterbox.** Platform guidance asks games to use every pixel at every common aspect. | Google Play “Level Up”: landscape games should fill 4:3, 16:10 and 21:9 without letterboxing and keep UI clear of cutouts; Apple (WWDC24): take advantage of every pixel; adjust the camera first, letterbox only if you must, and fill bars with artwork | Adapt | Field maps fill every screen. Arenas still letterbox; draw the margin as out-of-bounds ground beyond the boundary walls rather than flat bars. |

Sources:

- Fortnite: <https://toucharcade.com/2019/01/29/fortnite-controller-mapping-matchmaking-explained-cross-platform-keyboard/> [summary]
- Call of Duty: Mobile: <https://activision.helpshift.com/hc/en/3-cod-mobile/faq/88-how-does-matchmaking-work/> [summary]
- Apex Legends: <https://www.pcgamesn.com/apex-legends/aim-assist-nerf-season-22> [summary]
- Destiny 2: <https://www.dexerto.com/destiny/what-we-know-destiny-2-crossplay-1522167/> [summary]
- Helldivers 2: <https://www.windowscentral.com/gaming/helldivers-2-faq-crossplay-price-multiplayer-platforms-and-other-questions-answered> [summary]
- Diablo Immortal: <https://news.blizzard.com/en-us/article/23797159/making-diablo-immortal-for-pc> [summary]
- Teeworlds: <https://github.com/teeworlds/teeworlds/blob/master/src/game/client/render.cpp> [read]
- Wild Rift: <https://spookyfairy.com/playing-wilf-rift-on-a-tablet-screen-aspect-ratio-affects-visible-area-dev-response/> [summary]
- StarCraft II: <https://github.com/PKDT-93/sc2-ultrawide> [read]
- Brawl Stars: <https://brawlstars.fandom.com/f/p/4400000000000124328> [forum]
- Fortnite: <https://www.cultofmac.com/how-to/play-fortnite-120hz-ipad-pro> [summary]
- Google Play 'Level Up': <https://developer.android.com/games/guidelines> [read]
- Shoot ’em up design: <https://shmups.wiki/library/Boghog's_bullet_hell_shmup_101> [summary]
- Diablo II: Resurrected: <https://www.techspot.com/news/91167-blizzard-removed-ultrawide-monitor-support-diablo-2-because.html> [summary]
- VALORANT: <https://www.riotgames.com/en/news/valorant-shaders-and-gameplay-clarity> [summary]
- Counter-Strike 2: <https://ggscore.com/en/csgo/news/59316> [summary]
- PUBG: <https://forums.pubg.com/topic/34295-idea-for-dealing-with-players-using-very-lowlow-foliage-settings/> [forum]
- Rust: <https://rust.facepunch.com/news/lighting-the-way> [summary]
- DayZ: <https://feedback.bistudio.com/T87202> [forum]
- Escape from Tarkov: <https://forum.escapefromtarkov.com/topic/131377-use-nvidia-freestyle-will-be-ban/> [forum]
- VALORANT: <https://technology.riotgames.com/news/demolishing-wallhacks-valorants-fog-war> [summary]
- DDNet: <https://github.com/ddnet/ddnet/blob/master/src/game/server/player.cpp> [read]
- Suroi: <https://github.com/HasangerGames/suroi/blob/master/server/src/objects/player.ts> [read]
- Apple (WWDC24): <https://developer.apple.com/videos/play/wwdc2024/10085/> [read]

## 5. Large maps: pacing, spawning and orientation

| Pattern | Who does it | Verdict | For bongbong |
|---|---|---|---|
| **Pacing director.** A director measures how hard the players are pushed and cycles build-up, peak and relax. | Left 4 Dead: per-survivor intensity drives Build Up, Sustain Peak, Peak Fade and Relax; Vermintide 2 and Darktide: players praise Vermintide 2’s highs and lows and fault Darktide’s constant pressure | Adapt | Drive the wave breather from a per-seat intensity (damage taken, enemies in the sight box, frog health). It reads only simulation state, so it stays deterministic. |
| **Spawn budget.** Spawners earn credits over time and buy enemies by cost. | Risk of Rain 2: combat directors earn credits and buy weighted cards with spawn distances; Deep Rock Galactic: a swarm spends difficulty points across spawn points near the players | Adapt | On field maps only: a budget per minute times wave_size_scale spent on chassis tiers under wave_max_alive. Arena levels keep their authored wave plans. |
| **Spawn out of sight, within reach.** Enemies appear just past what any player sees, close enough to arrive soon; stragglers are recycled. | Vampire Survivors: spawns just off screen, despawns far enemies, teleports bosses back; Vampire Survivors (ultrawide mod): has to expand the spawn zone, because it follows the screen size; Terraria: spawns between an inner safe range and an outer spawn range around the player; Left 4 Dead: spawns only in nav areas the survivors cannot see, near the team | Adopt | Spawns and gate choice: outside every seat’s sight box, within N cells of path distance (one flow-field read). Give wave_gate_min_player_dist a maximum, and re-roll a wave tank still far away after a while through a nearer gate. This fixes the study map’s 52 s walk. |
| **Pressure follows groups and objectives.** Spawning is worked out per cluster of players and raised near objectives. | Helldivers 2: patrols spawn per player group (75 m); objectives give off heat out to 150 m; Helldivers 2 (a bug): solo missions shipped with four-player patrol timers | Adapt | Cluster the seats, pick gates per cluster, treat the frog as heat; keep the per-seat tuning patch and test one-seat rooms explicitly. |
| **Bounded awareness: radius, chain, leash.** What one enemy knows spreads locally, and an enemy pulled too far goes home. | World of Warcraft: past the leash a mob drops aggro and runs home; Diablo II: aggro range is usually just under half a screen; Helldivers 2: a spotter calls a breach or dropship; kill it in time and nothing comes | Adopt | Replace the map-wide alert with a chain (an alerted tank alerts others within its own sight) or a radius of a few screens; give guards a home cell and a leash; optionally a visible "calling" tank players can cut short. |
| **Simulation bubble.** Only the area near the players is fully simulated; the rest sleeps. | Minecraft: simulation distance ticks entities near players, separate from render distance; Cataclysm: DDA: a reality bubble about 60 tiles each way; things outside stop, and seams show; Factorio: idle entities sleep until an event wakes them | Adapt | An enemy with no seat or frog within a few screens of path distance, and not rolling in, sleeps: no thinking, no routing; a hit, an alert or a seat nearby wakes it. A pure function of positions, so replays hold; the probe needs a re-baseline. |
| **AI level of detail.** Distant or unimportant agents think less often and more cheaply. | Unreal Engine: the Significance Manager lets objects run complex AI less often; Game AI Pro (LOD Trader): picks detail levels under a budget rather than by distance alone | Adopt | Far enemies think every k-th tick, staggered by owner slot, and flow fields refresh in turn and stay inside the bubble. The target is the 0.61 ms a tick measured on 96 x 54. |
| **Interest management.** The server sends each client what is relevant to it, and far things less often. | Unreal Engine: net relevancy, cull distance and the Replication Graph’s spatial grid | Skip | For now: one delta encoded for every seat, a replica that is a whole Game, and arrows that need everything. If the bytes grow, send far entities less often before culling any. |
| **Convergence by structure.** The map itself pulls players and enemies together. | PUBG: the circle forces contact and scales the play area to the survivors; League of Legends, Wild Rift: lanes carry the waves; Wild Rift shrank its map for shorter mobile matches | Adapt | No shrinking zone in co-op. Author field maps as road lanes from gate clusters to the frog or a fort, sized so a walk from a gate to the fight stays around 15 s. |
| **Re-entry near the team.** Respawns land near living teammates, scored against danger. | Battlefield: spawn on a squadmate, but not while they are in combat; Halo: Reach: respawn zones scored up near teammates and down near enemies; Helldivers 2: a teammate throws a reinforce beacon | Adopt | A wrecked seat comes back through the gate with the shortest path to the living seats, with a penalty for enemies near that lane. |
| **Readable large maps.** Landmarks, districts, edges and paths tell a player who sees one screen where they are. | Kevin Lynch’s five elements in level design: paths, edges, districts, nodes and landmarks; Disney “weenies”: a visible destination that pulls people toward it; Zelda: Breath of the Wild (CEDEC 2017): triangles at three scales and “gravity” structures; an evenly spread tower layout felt over-guided and was redone | Adapt | A top-down view hides little, so landmarks must read within one screen and on the minimap: a material palette per region, roads and rivers as paths and edges, a fort as a node, and a lint warning for a sight-box-sized window with nothing distinctive in it. |

Sources:

- Left 4 Dead: <https://left4dead.fandom.com/wiki/The_Director> [summary]
- Vermintide 2 and Darktide: <https://forums.fatsharkgames.com/t/darktide-ai-director-needs-much-better-pacing/114343> [forum]
- Risk of Rain 2: <https://riskofrain2.wiki.gg/wiki/Directors> [summary]
- Deep Rock Galactic: <https://deeprockgalactic.wiki.gg/wiki/Swarm> [summary]
- Vampire Survivors: <https://vampire-survivors.fandom.com/wiki/Enemies> [summary]
- Vampire Survivors (ultrawide mod): <https://github.com/p1xel8ted/UltrawideFixes/blob/main/src/VampireSurvivors/VampireSurvivors-BepInEx/Plugin.cs> [read]
- Terraria: <https://terraria.wiki.gg/wiki/NPC_spawning> [summary]
- Helldivers 2: <https://helldivers-2.game-vault.net/wiki/Patrol_Spawn_Mechanics> [summary]
- Helldivers 2 (a bug): <https://www.pcgamer.com/games/third-person-shooter/solo-players-its-not-just-you-1-player-missions-in-helldivers-2-currently-have-4-player-patrol-spawn-times/> [summary]
- World of Warcraft: <https://warcraft.wiki.gg/wiki/Leash> [summary]
- Diablo II: <https://www.purediablo.com/forums/threads/how-does-aggro-work-in-diablo-2.180517/> [forum]
- Helldivers 2: <https://screenrant.com/helldivers-2-stealth-patrols-breach-dropship-reinforcements/> [summary]
- Minecraft: <https://minecraft.wiki/w/Simulation_distance> [summary]
- Cataclysm: DDA: <https://github.com/CleverRaven/Cataclysm-DDA/issues/30209> [read]
- Factorio: <https://factorio.com/blog/post/fff-204> [summary]
- Unreal Engine: <https://dev.epicgames.com/documentation/en-us/unreal-engine/significance-manager-in-unreal-engine> [summary]
- Game AI Pro (LOD Trader): <http://www.gameaipro.com/GameAIPro/GameAIPro_Chapter14_Phenomenal_AI_Level-of-Detail_Control_with_the_LOD_Trader.pdf> [summary]
- Unreal Engine: <https://dev.epicgames.com/documentation/en-us/unreal-engine/actor-relevancy-in-unreal-engine> [summary]
- PUBG: <https://gamesbeat.com/brendan-greene-and-rami-ismail/> [summary]
- League of Legends, Wild Rift: <https://www.dexerto.com/league-of-legends/league-of-legends-wild-rift-gameplay-reveal-champions-map-more-1372889/> [summary]
- Battlefield: <https://gamefaqs.gamespot.com/boards/190144-battlefield-1/75706660> [forum]
- Halo: Reach: <https://archive.forgehub.com/wiki/halo-reach-forge-spawning/> [forum]
- Helldivers 2: <https://www.dexerto.com/helldivers/how-to-reinforce-teammates-in-helldivers-2-code-explained-2543371/> [summary]
- Kevin Lynch’s five elements in level design: <https://www.gamedeveloper.com/design/the-image-of-a-game-space> [summary]
- Disney “weenies”: <https://www.gamedeveloper.com/design/what-mario-learned-from-mickey-mouse---part-3-decision-making-and-weenies> [summary]
- Zelda: Breath of the Wild (CEDEC 2017): <https://gist.github.com/idbrii/e39fe96279aa1670319bfa521d907399> [read]

## 6. The builder

| Pattern | Who does it | Verdict | For bongbong |
|---|---|---|---|
| **Play from here.** A test starts at the view or the cursor instead of the map’s start. | Unreal Engine: Play From Here; starting at the camera can break navmesh pathing; Ultimate Doom Builder: test from the camera in visual mode, from the cursor in classic modes; Super Mario Maker 2: tap Play to start where Mario stands, hold it to start from the beginning; Geometry Dash: Start Pos objects set a test start | Adopt | PLAY starts from the map’s start; PLAY HERE (or a long press) puts seat 1 on the drivable, spawn-legal cell nearest the view’s centre without changing the map, which avoids Unreal’s pathing trap. |
| **Clear check before sharing.** A map cannot be shared until its author beats it, and that run sets the par. | Super Mario Maker 2: clear the course from the start before uploading; Geometry Dash: a level must be verified with every Start Pos removed; Trackmania: the validation drive sets the author time and the medal times | Adapt | A map revision (a hash of its TOML) counts as cleared once its author wins it from plain PLAY with no edits since; require that before a custom map can be hosted in a room, and show the clear time as a par. |
| **Lint panel with jump-to and fixes.** Map problems are listed; selecting one zooms to it; some have a fix button. | Ultimate Doom Builder: Map Analysis: pick checks, click a problem to zoom to it, fix one or all of a kind | Adopt | maplint findings carry only a message today. Add the cells, list them in the builder with pan-and-zoom to each, and add quick fixes (move a penned-in start, delete a lone portal). |
| **Selection, copy and stamps.** Rectangles of cells are selected, moved, copied, flipped and saved as stamps. | Tiled: named stamps with variations, random placement from a set, flips and rotation; Super Mario Maker 2: Multi-Grab moves a rectangle; Copy duplicates pieces or groups | Adopt | A rectangle-select tool with move, copy and flip, and a few saved stamps (fort, bunker, river bend), each paste one undo step. On a 96 x 54 map this replaces hundreds of taps. |
| **Area brushes: fill, scatter, auto-tile.** Large areas are painted with fills, random scatter and brushes that pick transitions. | Tiled: bucket and shape fills, random mode, a terrain brush that picks transition tiles, automapping while drawing | Adopt | Rectangle and flood fill for water, road, grass and walls; a scatter brush for trees, grass and props hashed by cell rather than drawn from the RNG. Water and roads already auto-tile in the ground layer. |
| **Resize with an anchor and a preview.** Map size changes with a choice of where the old map sits. | Tiled: the Resize Map dialog previews the result with a frame you drag to set the offset; The Battle for Wesnoth: nine anchor directions and an option to copy edge terrain | Adopt | A nine-way anchor, copy-edge for water that runs off the map, a flag on starts and gates that fall outside, and the resize as one undo step. |
| **Overview navigator.** A small map of the whole level that jumps the view. | Tiled: a mini-map dock, redrawn only when needed for speed; Super Mario Maker (Wii U), Super Mario Maker 2: a scroll bar on the GamePad; a one-step zoom-out view (pinch in handheld mode) | Adopt | A cached minimap redrawn per painted cell, with tap or drag to jump, plus a FIT button. |
| **Touch editing without clashes, and a loupe.** Gestures are kept apart, and the cell under the finger is shown magnified. | Super Mario Maker 2: handheld building is touch-only and a stylus is recommended; Apple HIG: standard gestures first; custom gestures only when necessary, as in a game or drawing app; Apple UITextLoupeSession: the system magnifier at a given point, following a pan; Android: touch slop: how far a touch wanders before it counts as a scroll | Adopt | Hold a stroke back until it moves past the slop or 100 ms pass without a second finger; a second finger rolls the stroke back into pan and pinch; keep visible undo buttons; draw a loupe above the finger when cells are small. |
| **Automatic thumbnails.** Saving a map renders its preview. | OpenRA: writes map.png from the minimap on save unless the author locks a preview | Adopt | Render the whole map with mapshot’s CPU renderer on save, for the Load list and the lobby’s map stepper. *Landed for the Load list as OpenRA does it, from the minimap and as the list shows each page: mapshot’s renderer paints a large map in a quarter of a second (docs/large-maps-follow-camera.md section 9).* |
| **Layers and symmetry.** Editors split content into layers and mirror strokes. | Tiled: tile, object, image and group layers with show, hide and lock | Skip | One object per cell keeps the toggle-erase rule and the touch UI simple, and co-op against the AI needs no mirrored fairness. A view filter that hides grass and trees covers the real need. |

Sources:

- Unreal Engine: <https://dev.epicgames.com/documentation/unreal-engine/ineditor-testing-play-and-simulate-in-unreal-engine> [summary]
- Ultimate Doom Builder: <https://github.com/UltimateDoomBuilder/UltimateDoomBuilder/blob/master/Help/gzdb/features/features.html> [read]
- Super Mario Maker 2: <https://supermariomaker2.fandom.com/wiki/Course_Maker> [summary]
- Geometry Dash: <https://www.gdcreatorschool.com/docs/guides/triggers-1/start-pos-end/> [summary]
- Super Mario Maker 2: <https://supermariomaker2.fandom.com/wiki/Play_Guide> [summary]
- Geometry Dash: <https://www.gdcreatorschool.com/docs/guides/the-editor/testing-levels/> [summary]
- Trackmania: <https://www.trackmania.wiki/w/index.php?title=Track_Editor> [summary]
- Ultimate Doom Builder: <https://github.com/UltimateDoomBuilder/UltimateDoomBuilder/blob/master/Help/e_mapanalysis.html> [read]
- Tiled: <https://raw.githubusercontent.com/mapeditor/tiled/master/docs/manual/editing-tile-layers.rst> [read]
- Super Mario Maker 2: <https://www.shacknews.com/article/112653/how-to-grab-copy-undo-delete-in-super-mario-maker-2-course-maker> [summary]
- Tiled: <https://raw.githubusercontent.com/mapeditor/tiled/master/docs/manual/terrain.rst> [read]
- Tiled: <https://github.com/mapeditor/tiled/pull/1516> [read]
- The Battle for Wesnoth: <https://github.com/wesnoth/wesnoth/blob/master/src/gui/dialogs/editor/resize_map.hpp> [read]
- Tiled: <https://github.com/mapeditor/tiled/blob/master/src/tiled/minimapdock.cpp> [read]
- Super Mario Maker (Wii U), Super Mario Maker 2: <https://attackofthefanboy.com/guides/super-mario-maker-2-how-to-zoom-out/> [summary]
- Super Mario Maker 2: <https://nintendoeverything.com/super-mario-maker-2-doesnt-support-button-controls-when-making-levels-in-portable-mode/> [summary]
- Apple HIG: <https://developer.apple.com/design/human-interface-guidelines/gestures> [read]
- Apple UITextLoupeSession: <https://developer.apple.com/documentation/uikit/uitextloupesession> [read]
- Android: <https://developer.android.com/develop/ui/views/touch-and-input/gestures/viewgroup> [read]
- OpenRA: <https://github.com/OpenRA/OpenRA/blob/bleed/OpenRA.Game/Map/Map.cs> [read]
- Tiled: <https://raw.githubusercontent.com/mapeditor/tiled/master/docs/manual/layers.rst> [read]
