//! The raylib side of the game, behind the `render` feature (Cargo.toml):
//! everything that needs a draw handle, a texture, a shader, an image
//! decoder or the window. Each file here is the raylib half of the module
//! it is named after - `render::shell` holds `draw_shell` for `shell.rs`,
//! `render::game` holds `Game::render` and `Textures` for `game.rs` - and
//! keeps the item names, so a reader who finds a `draw_*` missing from an
//! entity file looks in one place. The entity files keep their state and
//! the drawing that is generic over `canvas::Canvas` (plain arithmetic
//! over `math`'s types, which the CPU canvas runs headless); `editor/render.rs`
//! is the builder's chrome, a child of `editor` because it draws the
//! builder's private state.
//!
//! Nothing under `simulation/` names this module. With
//! `--no-default-features` the crate builds without it, with no C
//! compiler and no raylib: the probe, the simulation tests and the room
//! server (docs/online-coop-prd.md section 4.5).

pub mod blast;
pub mod bullet;
pub mod canvas;
pub mod decal;
pub mod frog;
pub mod fx;
pub mod game;
pub mod hud;
pub mod laser;
pub mod plasma;
pub mod portal;
pub mod shell;
pub mod shockwave;
pub mod tank;
pub mod thumbnail;
pub mod view;
