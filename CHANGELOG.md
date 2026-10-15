# Changelog

What changed in each release of bongbong, newest first. Each entry is also
that release's notes on GitHub, above the downloads. The 0.0.x builds before
0.1.0 are in the git history.

## 0.2.10 - 2026-10-15

The frog is gone: each side now fights for one of seven bongs. A battlefield
keeps burning long after the fight moves on, air support crates call in a
helicopter or a jet, and BUILD now pauses your round instead of leaving it.

### New
- Seven bongs replace the frog. Yours is Bong Spike, Bong Oggle or Bong Gloop; in a Hunt round the enemy's is Bong Blip, Puff, Capi or Fuzz. Every map picks its two, and the builder's MAP panel has YOUR BONG and FOE BONG to choose them (#163)
- A hurt bong hops to a bong health pack nearby and takes it herself, healing to full. Your own tanks parked beside her no longer keep her off it (#172)
- Air support crates, placed with the builder's PICKUP tools: after a 15 s countdown a helicopter flies in and fires seeker missiles at the enemies on your screen, or a jet makes one strafing pass and drops two bombs. Enemy guns and towers can shoot the helicopter down (#150)
- The battlefield keeps burning. Wrecks smoulder for minutes with smoke you can read a kill's age from, burnt ground glows with embers, ruins and charred stumps smoke, and a battle haze builds over a long fight and drifts with the wind. Rain, snow and sandstorms each act on the remains, and fires light the night (#149, #153)

### Changed
- BUILD opens the builder at once and pauses the round behind it; BACK (or Escape) returns to the same moment. Edits made in between are put away, and UNDO in the next BUILD brings them back. PLAY still starts a new round on the edited map (#168)
- The builder now looks like the round: the same walls, ground, grass, tanks and sky, with its controls on the HUD's corner plates, a DAY switch for editing at night, and a quick swap between modes instead of a cut. Every dialog uses one dark panel (#169)
- The builder's PICKUP list is grouped under SUPPLIES, WEAPONS, SPECIALS and AIR SUPPORT, and the eraser button is now a crisp drawn eraser (#173, #174)
- Starting over on the same map glides the camera from your wreck to the new start instead of zooming out and back in, and the end screen dissolves into the next round instead of fading through black. PROTECT and HUNT on the HUD now show the bong they mean (#170)
- A shell that hits iron now usually bursts on it; about one shot in twenty still ricochets straight back (#151)
- The gravity well reaches a little further and pulls a little harder to escape, and headlights reach further at night (#154)
- Enemies drive round crates they have no use for and leave them for you. When two want the same crate only the nearer goes for it, and one that cannot reach a crate gives up on it after a few seconds (#152, #160)
- Enemies sidestep the tank they are about to drive into instead of the one behind them, so packs pile up, grind and ram you far less (#161)

### Fixed
- In Boot Camp, a shell fired at a closed training door bursts on it instead of bouncing back at your tank (#151)
- Enemies no longer dance on the spot between chasing you and a crate, shuttle back and forth past a turning they need, or keep driving into their own pack after they stop firing (#157, #162, #164)

### Online
- Tall grass flattened by a sonic hammer or a rod strike stops hiding tanks on every player's screen for as long as it does in the room, and a player who joins meanwhile sees it lying flat (#156)
- Grass a blast sets alight shows as burnt on every screen at the same moment as in the room (#152)
- Smoke, air support and the new bongs all reach co-op rooms. Players and the room server need this version to play together (#149, #150, #163, #172)

### Platforms
- Web: bongbong.io has a page below the game with what's in it, the downloads and a newsletter form, plus new Privacy and Support pages. Typing into a field on the page no longer drives your tank (#175)

### Behind the scenes
- Pull request checks run faster: CI is split into two jobs and the room server build reuses its cache (#171)
- The online round's and the network rig's tests run on a hand-wound clock, so a busy machine no longer fails them (#158, #159)
- Builds move to Rust 1.99.0, and every download drops an unused image (#165, #167)

## 0.2.9 - 2026-10-11

Walls now break where you hit them, fire spreads on the wind, woods are
places to hide in and tanks leave tread marks that last the round. Every
special weapon is now in the campaign.

### New
- Every special weapon is now in the campaign, a new one every level or two: the sonic hammer on Vulkan, grenades on Oasis Bazaar, the EMP on Carnival, the FPV swarm on Jungle Temple, the gauss rail on Harbor Lights, the rod from god on No Man's Land and the gravity well on Black Gold. The armory now has the laser, plasma, minigun and flamethrower too (#147)
- Brick and wood walls break chunk by chunk where your shots land. A shot passes through a hole while the rest of the wall still stops tanks, so you can chip a firing slit, and the broken pieces fly, bounce and stay on the ground for the round (#130, #138)
- A shell that breaches a wall throws fragments at a tank sheltering right behind it. A falling wall brings down badly damaged neighbours one after another and leaves rubble that slows tanks, and a shattering pane takes cracked panes beside it too (#134)
- Concrete: a new, tougher wall in the builder's WALL menu, in four looks. When it gives way its rebar stays standing, letting shots through but not tanks, until heavy blasts cut it down. Wired glass keeps its mesh the same way (#135)
- A burning plank wall burns through hole by hole, and burning planks and trees throw embers that land downwind and can set other wood and trees alight (#138)
- Seven new trees (spruce, Scots pine, fir sapling, birch, willow, date palm and dead snag) and six kinds of soft cover (bush, berry bush, juniper, fern, autumn bush and reeds) that hide a tank like tall grass. They are in the builder's new PLANT group (#129, #131)
- Trails through a wood hide you like tall grass, long ones get worn ruts, and treetops over your tank fade so you can still see it. The builder's CHECK warns about trails one cell wide, which enemies never take (#136)
- A forest brush in the PLANT group paints a whole wood in one stroke and picks each tree from where it stands: willows and birches by water (palms on desert), saplings on the edge, patches of conifers and broadleaves inside, and open banks along streams (#137)
- Tread marks stay for the round. Driving the same lane again wears it into a rut and then a path, and the marks follow the ground and the weather. Tanks throw dust, mud, spray and snow as they drive (#132)
- Two experiments, off unless a tester turns them on: tanks that drive through an oil pool leave oil prints that a flame can run along, and tall grass, bushes and reeds driven over enough times stop hiding anyone for the round (#143, #144)

### Changed
- Woods look like forests on every map with trees. Treetops sit a little off their cells, with a forest floor and undergrowth round them that hides nobody. On desert maps the trees are dry and catch fire more readily (#129, #133)
- Picking up a crate of the weapon you already carry adds to your ammo, up to a limit for each weapon, and the ring under your tank shows how much more you can take. Enemies stack their own weapon's crates too (#125)
- Enemies shoot at you through holes in walls, steer round rubble, and break through walls that cut them off instead of wandering. They also get stuck less: they take the gap they used to drive past, back off and wait when hurt, and back out of jams (#141, #146)
- The rod from god hits a two-cell circle and lands after three seconds instead of four. The rod's shove, the sonic hammer and the gravity well's fling now throw tanks as far as they should (#126)

### Fixed
- Tanks no longer sink into walls. A tank that is driven, rammed, blasted, knocked or pulled into a wall stops flush against it (#126)
- An enemy winding up a sonic hammer or an EMP when the round ends no longer stays frozen mid wind-up on the end screen (#124)
- In BUILD on a touch screen, one finger paints whenever the whole map is in view, where before it did nothing. On an iPad, the builder's touch sizes now match a phone's (#122)

### Online
- A round now plays out exactly the same on every platform, so the room's server and its players' devices no longer drift apart (#126)
- Broken and burning walls, rubble, rebar, embers, oil prints and trampled grass look the same in every player's window, and a player who joins mid-round sees grass that burned before they came in as burnt (#130, #134, #135, #138, #143, #144, #145)

### Behind the scenes
- iOS builds work again with Xcode 27 and with a renewed signing certificate, and the simulator opens in Xcode 27's DeviceHub (#121, #127, #142)

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
