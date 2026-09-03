# BongBong

BongBong is a simplistic and entertaining modern tank shooter game. 

The main goal of this project is to build a fun, entertaining game with retro graphics and an illusion of modern physics, while paying respect to old-school tank games.

Play it live @ [bongbong.io](https://bongbong.io) or [subscribe and follow the progress via the blog](https://blog.bongbong.io).

## Development

```bash
# Use devenv (Nix)
cargo watch -x "run -- -e 12 --map=maps/default.toml"

# Map editor
cargo run --features map-editor -- --editor --map=maps/default.toml

# Preview web
just build-web-dev && just preview-web
```

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
