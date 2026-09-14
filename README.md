# BongBong

BongBong is a simplistic and entertaining modern tank shooter game. 

The main goal of this project is to build a fun, entertaining game with retro graphics and an illusion of modern physics, while paying respect to old-school tank games.

Play it live @ [bongbong.io](https://bongbong.io) or [subscribe and follow the progress via the blog](https://blog.bongbong.io).

## Build and run

Everything runs from inside the devenv (Nix) shell. `just --list` has the rest.

| Where | One-time setup | Build and run |
| --- | --- | --- |
| Desktop | - | `just run` (or `just watch`, `just run-dev` with the dev server) |
| Web | `just setup-web` | `just build-web` then `just serve-web` |
| iOS simulator | `just ios-setup` | `just run-ios-sim` |
| iPhone | `just ios-setup-device` (plus an Apple ID in Xcode) | `just run-ios-device` |
| Android emulator / phone | `just android-setup` | `just run-android` |

```bash
# Options work the same on every desktop run
cargo run -- -e 12 --map=maps/default.toml   # 12 enemies on a map
cargo run -- --tank titan                    # pick the player's chassis
cargo run -- --editor                        # start in the map builder (BUILD/PLAY switch anytime)
```

The phone lanes are documented in `CLAUDE.md` (the "iOS simulator build" and
"Android build" sections) and in `docs/ios-native-port-prd.md` /
`docs/android-port-prd.md`. `just ios-smoke` and `just android-smoke` are the
graphics checks to run after an Xcode or SDK update; `just ios-screenshot`,
`just android-screenshot` and `just android-tap` / `android-swipe` drive a
running build from the shell.

## Dependencies

- The game is written from scratch with [raylib] via [sola-raylib] bindings for Rust.
- The [rapier](https://github.com/dimforge/rapier) physics engine alleviates some of the physics challenges.
- The game uses the [hecs](https://docs.rs/hecs/latest/hecs/) entity-component-system (ECS).
- This game is confined to libraries and tools that compile to WASM, as one of the main distribution channels is the web.

## Collaboration

Please feel free to reach out or interact with me if you have any ideas or anything else. ;\)

\- Oto Brglez


[sola-raylib]: https://github.com/brettchalupa/sola-raylib
[raylib]: https://www.raylib.com/
