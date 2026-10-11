# Changelog

What changed in each release of bongbong, newest first. Each entry is also
that release's notes on GitHub, above the downloads. The 0.0.x builds before
0.1.0 are in the git history.

## 0.2.8 - 2026-10-08

Six new special weapons, and a new arena called the armory to try them in.
Your map edits are now kept between sessions, and the Mac version is a
proper app.

### New
- The sonic hammer: a cone of sound that knocks tanks off their tracks, shatters glass, flattens tall grass and throws drums. Iron, brick, wood and towers block it (#109)
- The EMP burst: a pulse that goes through walls and switches off every tank's lights, shield and special weapon for three seconds, stops enemies thinking, darkens towers and drops missiles. Your own special goes offline too (#110)
- The gauss rail: hold fire to charge it, release at full and a slug crosses the whole field through walls and every tank in line. Overcharge it to cut iron, but hold it too long and it vents (#111)
- The FPV swarm: six drones hover round your tank, and each press sends one diving at the nearest enemy over walls and water. A minigun, a tesla or a gun tower can shoot them down, and a tree hides you from them (#112)
- The rod from god: steer a reticle, let go, and after a four-second countdown a strike from orbit crushes everything in its circle, shields and teammates included, and leaves a crater (#115)
- The gravity well: fire an orb and press again to anchor it. It pulls tanks, shots, drums and grenades in for four seconds, then collapses and throws everything out. To escape, drive across the pull, not away from it (#117)
- The armory: a new free-play arena with crates of every new weapon, some for the enemies too, plus glass, grass, drums, water, lava and towers to use them on. Open it from BUILD > FILE > LOAD, or host it online (#109, #110, #111, #112, #115, #117)
- The range board: a bullseye on an easel, placed with the builder's ACTOR tools. Shells splinter it in stages and fire burns it to the ground (#108)
- A map can have several skies, and each round is played under one of them. Pick them on the MAP panel's new SKY tiles (#105)
- Your map edits are kept between sessions on desktop, web, iPhone, iPad and Android, saved automatically as you edit. A shipped map you change keeps its original, and FILE > REVERT MAP brings it back (#107)

### Changed
- The builder's MAP panel is grouped into ROUND, TANKS, FIELD and SKY, and its buttons show your choices and the tank's sprite. On a phone it shows one group at a time behind tabs, and the navigator moves out of its way (#105, #106)
- Every button on the builder's bar now looks the same: one box, one text size and one colour rule, with PLAY and PLAY HERE in amber (#116)

### Fixed
- In Boot Camp, the frog's speech bubbles stay up about twice as long, so you have time to read them (#98)
- The HUD's top corners no longer have an empty dark row on lava levels. The row appears only while it shows lanterns or a heat shield's gauge (#119)

### Online
- Every new weapon works in co-op rooms. Your own blast, pulse, charge, drone, strike or orb appears the moment you press (#109, #110, #111, #112, #115, #117)
- Burning planks and range boards catch fire, char and burn out the same way in every player's window (#108)
- A rematch with fewer players no longer keeps the larger team's wave sizes (#118)

### Platforms
- macOS: the download is now BongBong.app in a signed, notarized dmg, one app for Apple Silicon and Intel. It replaces the bare binary that macOS refused to open, and Mac testers can also get it through TestFlight (#99)
- Web: the browser tab's icon is now the game's tank (#101)

### Behind the scenes
- Testers' dev tools can now give a player or an enemy a weapon, drop crates and change one room's tuning during an online round (#118)

## 0.2.7 - 2026-10-06

Bigger maps with a camera that follows your tank, a training level, a
volcano level and a grenade launcher, and a builder made for large maps.

### New
- Maps larger than the screen: the camera follows your tank, a round opens on the whole map before zooming in, and two players on one machine get a split screen when they drift apart. The levels are bigger too (#68)
- Arrows at the screen's edge point to enemies, teammates, the frog and wave gates off screen, and tablets and desktops get a minimap (#68)
- Boot Camp, LEVEL 0: a short course for one player where the frog teaches driving, crates, firing, a chain of exploding drums and your first enemy. You can skip it and start on level 1 (#74, #79, #81, #88)
- Vulkan, the new level 2: a volcano that erupts every half minute and throws lava bombs, rivers and lakes of lava, and a heat shield crate that keeps all of it off you (#72, #84)
- A grenade launcher: lob canisters over walls, watch them roll and bounce, and get clear before the fuse runs out (#96)
- Shells, bullets, plasma and the laser now go through portals (#95)
- Every pickup is a supply crate with its symbol on the lid; crates drop in from the air and crack open when taken (#71, #73)
- Fish swim in the lakes and scatter from tanks, shots and blasts (#91)
- Lamp posts, lanterns and dusk falling into night, for maps of your own (#72)
- The builder works on large maps: zoom, pan and pinch, a navigator, select with copy, paste, flip and stamps, RECT, FILL and SCATTER brushes, PLAY HERE, and a CHECK panel that finds problems and fixes them in one tap (#68)
- A pause button next to BUILD (#83)

### Changed
- The HUD sits in the screen's two top corners on one row instead of a bar, and fades while the action is under it (#68, #81)
- You carry one special weapon at a time: a different weapon crate replaces it, and when it runs dry you are back on shells (#81)
- A round's end plays out: the deciding hit slows down, the end screen eases in, levels change through a fade, and a lost level restarts by itself after 5 s (#81)
- Enemies only fire at you from close enough to be on your screen (#68)
- The minigun crate holds 60 rounds (#94)
- Nights are a little less dark (#72)
- The builder's bar is taller with larger labels, and folds its tools into one TOOLS button where the window is narrow (#68, #85, #92)
- With Reduce Motion on, the screen does not shake or ripple (#68)

### Fixed
- The ring under your tank stays right under it at any speed instead of trailing behind (#93)
- Tall grass no longer covers walls and props; next to a wall it grows at its foot (#86, #97)

### Online
- Players without a nickname no longer take each other's seat; each gets a name of their own, like Player #8VA62T (#90)
- A map you made must be cleared, won from PLAY in the builder, before it can be hosted (#68)

### Platforms
- Web: the game fills the browser window at any shape, in full screen and through rotations, and a phone held upright is asked to turn (#68, #81)
- Testers can install a pull request's build on iOS through TestFlight or on Android as an APK before it merges (#65)

### Behind the scenes
- Faster tests and builds in CI, and preview servers no longer lose their images on a release (#80)

## 0.2.6 - 2026-10-01

The tanks are rebuilt from the ground up, and every explosion, hit and fire
is now drawn the same way.

### New
- The Vanguard tanks: every chassis redrawn with damage you can see build up, working headlights, a module on the hull for each special weapon, and colours of their own for players 3 and 4 (#58)
- A shaded ground, and wind gusts you can watch roll across the tall grass (#62)

### Changed
- Every explosion, hit, fire and damage mark redone in one blocky pixel style, with damaged tanks smoking and burning (#62, #63, #64)
- The builder's MAP panel shows the tank you picked (#61)
- A better first level

### Fixed
- Water no longer meets land with a hard tile edge (#59)

### Behind the scenes
- Pull request previews of the web build now run on workers.dev (#60)

## 0.2.5 - 2026-09-30

A campaign: fourteen levels to play in order, with defence towers and
weather that changes how a round plays.

### New
- Fourteen hand-made levels, played in order, with a result screen, a level select and progress that is saved (#53, #54)
- Defence towers: the tesla coil, the machine-gun tower and the bio slush (#55)
- Weather: night, rain, storm, fog, sand, snow, haze or a random sky, each changing how far enemies see and how tanks grip (#56)
- Every shot now has tracers, glows, flares and sparks (#52)

### Changed
- The first level's sky is picked at random
- The builder's Load list turns pages on a touch screen (#53)

### Fixed
- The Android app no longer hangs when it is opened again after being closed in the background (#51)

### Behind the scenes
- A monitoring dashboard for the room server (#57)
- Dependencies updated (#51)

## 0.2.4 - 2026-09-29

Online co-op that feels like playing locally, and the game speaks Slovenian.

### New
- The game speaks your device's language: English and Slovenian (#50)
- The builder's FILE menu has a CLEAR MAP row

### Online
- Your own tank and shots now respond the moment you press, with no network delay, and hits are judged against what you saw (#48)

### Platforms
- The Android APK is listed in the release's download table (#47)

## 0.2.3 - 2026-09-28

### Platforms
- Android: every release now comes with an installable APK (#46)

## 0.2.2 - 2026-09-27

Fixes from playing on a phone.

### Changed
- The touch stick follows your thumb, so changing direction is always a short slide (#45)
- Hits and pickups are more forgiving, so near misses count (#45)

## 0.2.1 - 2026-09-26

### Online
- Shots and the flamethrower are drawn at once and checked by the room, the delay adapts to your connection, and the room server reports how it is doing (#44)
- A server restart waits only for rounds that are being played (#43)

## 0.2.0 - 2026-09-26

Online co-op: play together over the internet.

### New
- Seeker missiles: a four-tube pod that fires homing salvos (#41)
- Seven tanks in ten go up in a mushroom cloud (#40)

### Online
- Host a room, share its code or QR, and play co-op with up to eight players, with rematches (#34)
- Your own tank moves the moment you steer, without waiting for the server (#34)

### Platforms
- iOS: the app is on TestFlight (#42)
- Web: the game stays centred and on screen in full screen on a phone (#35)

### Behind the scenes
- Every pull request gets its own room server to test against (#34)

## 0.1.0 - 2026-09-19

### New
- Rivers and lakes: streams you can ford and lakes you cannot, both changing how tanks drive (#23)
- Portals that teleport tanks across the map (#22)
- A desert theme (#21)
- Map thumbnails (#22)

### Changed
- Enemies find their way around the map faster and spread over more routes instead of queuing in one lane (#25)

### Platforms
- iPad builds (#19)
