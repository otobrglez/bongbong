# Changelog

What changed in each release of bongbong, newest first. Each entry is also
that release's notes on GitHub, above the downloads. The 0.0.x builds before
0.1.0 are in the git history.

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
