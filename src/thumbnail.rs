//! The map thumbnail (docs/mapshot-prd.md): a map's field as a fresh round
//! shows it, rendered to pixels either on the CPU (no window) or on the
//! GPU (a hidden window). `src/bin/mapshot.rs` is the command line over
//! this module; the tests here pin the CPU output of the shipped maps.
//!
//! **The first frame is init plus one update.** `Tank::ring_position`
//! starts at the origin and is only snapped onto the hull by
//! `Game::update`, which the windowed game always runs before its first
//! render. With no enemies on the field that update draws no RNG, so the
//! frame is a pure function of (map, seed, options).

use crate::canvas::{CpuCanvas, GpuCanvas, Pixels, Sheet, Sheets};
use crate::game::PaintOptions;
use crate::level::{LevelOverrides, SpawnKind};
use crate::map::MapFile;
use crate::simulation::{Game, Input, PlayerCount};
use crate::PHYSICS_FIXED_DT;
use sola_raylib::prelude::*;
use std::collections::BTreeMap;

/// The round seed a thumbnail uses unless told otherwise, so a batch is
/// stable from run to run and machine to machine. It only picks cosmetics:
/// ground and grass tiles, the frog's colour, a rolled chassis.
pub const DEFAULT_SEED: u64 = 0xB0B5;

/// What the thumbnail leaves out of `paint_standing`: the locate ripple is
/// a two-second cue, not the map.
pub const PAINT: PaintOptions = PaintOptions { locate_cue: false };

/// How a map is staged for its thumbnail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThumbnailOptions {
    pub seed: u64,
    pub players: PlayerCount,
    /// `--tank` / `--tank2`: a chassis row, else the map's or a roll.
    pub tank_row: Option<i32>,
    pub tank2_row: Option<i32>,
    /// `Game::hide_players`: no player sprite or ring.
    pub hide_players: bool,
    /// `Game::plain_canvas`: flat white instead of the ground tileset.
    pub plain_canvas: bool,
    /// Drop shadows, on in the game.
    pub shadows: bool,
}

impl Default for ThumbnailOptions {
    fn default() -> Self {
        ThumbnailOptions {
            seed: DEFAULT_SEED,
            players: PlayerCount::One,
            tank_row: None,
            tank2_row: None,
            hide_players: false,
            plain_canvas: false,
            shadows: true,
        }
    }
}

/// A fresh round on `map` at its first frame: no intro banner, no
/// enemies (the spawn plan is forced to Band so the zero count is
/// honoured whatever the map says), pickups and frogs in place, the
/// player start tank(s) with their ring settled on the hull.
pub fn stage_round(map: MapFile, opts: &ThumbnailOptions) -> Game {
    let mut game = Game::default();
    game.map = map;
    game.show_intro = false;
    game.enemy_count_override = Some(0);
    game.level_overrides = LevelOverrides { spawn: Some(SpawnKind::Band), ..Default::default() };
    game.seed_override = Some(opts.seed);
    game.shadows_enabled = opts.shadows;
    game.plain_canvas = opts.plain_canvas;
    game.hide_players = opts.hide_players;
    game.players = opts.players;
    game.player_row_override = opts.tank_row;
    game.player2_row_override = opts.tank2_row;
    let (width, height) = game.map.field_size();
    game.init(width, height);
    game.update(Input::default(), PHYSICS_FIXED_DT, width, height);
    game
}

/// The field's size in whole pixels.
pub fn field_pixels(game: &Game) -> (usize, usize) {
    let (w, h) = game.map.field_size();
    (w.round().max(1.0) as usize, h.round().max(1.0) as usize)
}

/// Every sheet decoded for the CPU canvas. Load once per batch; each
/// render clones the map (a few megabytes) into its canvas.
pub fn load_cpu_sheets() -> Result<BTreeMap<Sheet, Pixels>, String> {
    let mut sheets = BTreeMap::new();
    for sheet in Sheet::all() {
        sheets.insert(sheet, Pixels::load(&sheet.path())?);
    }
    Ok(sheets)
}

/// Paint `game`'s field onto a fresh CPU canvas.
pub fn render_cpu(game: &Game, sheets: &BTreeMap<Sheet, Pixels>) -> CpuCanvas {
    let (w, h) = field_pixels(game);
    let mut canvas = CpuCanvas::new(w, h, sheets.clone());
    game.paint_field(&mut canvas, PAINT);
    canvas
}

/// The GPU side's `Sheet` lookup: one texture per sheet, loaded by path.
pub struct GpuSheets(pub BTreeMap<Sheet, Texture2D>);

impl GpuSheets {
    /// Load every sheet as a texture. Needs a window (hidden is fine).
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Result<Self, String> {
        let mut map = BTreeMap::new();
        for sheet in Sheet::all() {
            let path = sheet.path();
            let texture = rl.load_texture(thread, &path).map_err(|e| format!("{path}: {e}"))?;
            map.insert(sheet, texture);
        }
        Ok(GpuSheets(map))
    }
}

impl Sheets for GpuSheets {
    fn texture(&self, sheet: Sheet) -> &Texture2D {
        self.0.get(&sheet).unwrap_or_else(|| panic!("{sheet:?} was not loaded"))
    }
}

/// Paint `game`'s field into a render texture through the same `Canvas`
/// stages the game's own pass 1 runs, and read it back as an `Image`
/// (top row first, like the CPU canvas).
pub fn render_gpu(rl: &mut RaylibHandle, thread: &RaylibThread, sheets: &GpuSheets, game: &Game) -> Result<Image, String> {
    let (w, h) = field_pixels(game);
    let mut scene = rl.load_render_texture(thread, w as u32, h as u32).map_err(|e| e.to_string())?;
    rl.draw_texture_mode(thread, &mut scene, |mut d| {
        d.clear_background(Color::WHITE);
        game.paint_field(&mut GpuCanvas::new(&mut d, sheets), PAINT);
    });
    let mut image = scene.load_image().map_err(|e| e.to_string())?;
    image.flip_vertical();
    Ok(image)
}

/// How far two renders of one field are apart: the mean absolute
/// per-channel difference and the share of pixels whose summed RGB
/// difference exceeds `OFF_THRESHOLD`. `mapshot --check` reports it for
/// the GPU render against the CPU one; rings are a 48-gon on the GPU and
/// a true annulus on the CPU, and GL's fill rules differ at edges, so the
/// two are close, not equal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Comparison {
    pub mean_channel_diff: f64,
    pub off_fraction: f64,
}

/// Summed RGB difference above which a pixel counts as off.
pub const OFF_THRESHOLD: i32 = 24;
/// The mean per-channel difference `within_tolerance` allows.
pub const MEAN_TOLERANCE: f64 = 2.0;
/// The share of off pixels `within_tolerance` allows.
pub const OFF_TOLERANCE: f64 = 0.02;

impl Comparison {
    pub fn within_tolerance(&self) -> bool {
        self.mean_channel_diff < MEAN_TOLERANCE && self.off_fraction < OFF_TOLERANCE
    }
}

/// Compare two RGBA buffers of the same size (alpha ignored: both canvases
/// are opaque). Sizes that differ compare as entirely off.
pub fn compare_pixels(a: &[Color], b: &[Color]) -> Comparison {
    if a.len() != b.len() || a.is_empty() {
        return Comparison { mean_channel_diff: 255.0, off_fraction: 1.0 };
    }
    let mut sum = 0u64;
    let mut off = 0usize;
    for (p, q) in a.iter().zip(b) {
        let d = (p.r as i32 - q.r as i32).abs() + (p.g as i32 - q.g as i32).abs() + (p.b as i32 - q.b as i32).abs();
        sum += d as u64;
        if d > OFF_THRESHOLD {
            off += 1;
        }
    }
    let n = a.len() as f64;
    Comparison { mean_channel_diff: sum as f64 / (3.0 * n), off_fraction: off as f64 / n }
}

/// PNG bytes of a read-back `Image`, scaled up `scale` times with whole
/// pixels.
pub fn image_png_bytes(mut image: Image, scale: u32) -> Result<Vec<u8>, String> {
    if scale > 1 {
        image.resize_nn(image.width() * scale as i32, image.height() * scale as i32);
    }
    image.export_image_to_memory(".png").map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Canvas;
    use crate::map::SHIPPED_MAPS;

    /// raylib logs every decoded file at INFO straight to stdout, past the
    /// test harness's capture; the tests only want its warnings.
    fn quiet_raylib() {
        unsafe { sola_raylib::ffi::SetTraceLogLevel(sola_raylib::consts::TraceLogLevel::LOG_WARNING as i32) }
    }

    fn shipped(name: &str) -> MapFile {
        let (_, toml) = SHIPPED_MAPS.iter().find(|(n, _)| *n == name).expect("shipped map");
        MapFile::from_toml_str(toml).expect("shipped map parses")
    }

    /// The CPU render of every shipped map at the default options, pinned.
    /// Deterministic, so a change here is a real change to what a thumbnail
    /// shows - re-baseline consciously after a deliberate art, map or
    /// tuning change, never to go green.
    const PINNED: [(&str, u64); 3] = [
        ("default", 0x4322_3fa2_935b_7511),
        ("hunt-basic", 0xe569_2a10_47fe_b1a8),
        ("waves-basic", 0x6e02_7f87_d416_2e96),
    ];

    #[test]
    fn shipped_maps_render_on_the_cpu() {
        quiet_raylib();
        let sheets = load_cpu_sheets().expect("sheets under static/");
        let mut report = Vec::new();
        for (name, pinned) in PINNED {
            let map = shipped(name);
            let (fw, fh) = map.field_size();
            let game = stage_round(map, &ThumbnailOptions::default());
            let canvas = render_cpu(&game, &sheets);
            assert_eq!((canvas.width(), canvas.height()), (fw.round() as usize, fh.round() as usize), "{name}: field size");
            // Not blank: a real field has many colours, and the ground is
            // never plain white.
            let mut distinct: Vec<u32> = canvas.pixels().iter().map(|c| u32::from_le_bytes([c.r, c.g, c.b, c.a])).collect();
            distinct.sort_unstable();
            distinct.dedup();
            assert!(distinct.len() > 64, "{name}: only {} distinct colours", distinct.len());
            assert!(canvas.pixels().iter().all(|c| c.a == 255), "{name}: the canvas stays opaque");
            let again = render_cpu(&stage_round(shipped(name), &ThumbnailOptions::default()), &sheets);
            assert_eq!(canvas.hash(), again.hash(), "{name}: two renders of one map differ");
            report.push((name, canvas.hash(), pinned));
        }
        let changed: Vec<String> = report.iter().filter(|(_, got, want)| got != want).map(|(n, got, _)| format!("(\"{n}\", 0x{got:016x})")).collect();
        assert!(changed.is_empty(), "pinned CPU renders changed; new rows for PINNED: {}", changed.join(", "));
    }

    #[test]
    fn options_change_the_picture() {
        quiet_raylib();
        let sheets = load_cpu_sheets().expect("sheets under static/");
        let base = render_cpu(&stage_round(shipped("default"), &ThumbnailOptions::default()), &sheets).hash();
        let hidden = ThumbnailOptions { hide_players: true, ..Default::default() };
        assert_ne!(base, render_cpu(&stage_round(shipped("default"), &hidden), &sheets).hash());
        let plain = ThumbnailOptions { plain_canvas: true, ..Default::default() };
        assert_ne!(base, render_cpu(&stage_round(shipped("default"), &plain), &sheets).hash());
        let seeded = ThumbnailOptions { seed: 7, ..Default::default() };
        assert_ne!(base, render_cpu(&stage_round(shipped("default"), &seeded), &sheets).hash());
    }

    #[test]
    fn no_enemies_and_no_cue_on_the_first_frame() {
        let game = stage_round(shipped("waves-basic"), &ThumbnailOptions::default());
        assert_eq!(game.tank_snapshots().iter().filter(|t| !t.is_player).count(), 0, "a Waves map stages no enemies");
        let game = stage_round(shipped("default"), &ThumbnailOptions::default());
        assert_eq!(game.tank_snapshots().iter().filter(|t| !t.is_player).count(), 0, "a Band map stages no enemies");
        assert!(!PAINT.locate_cue);
    }

    #[test]
    fn compare_pixels_measures_distance() {
        let a = CpuCanvas::blank(4, 4);
        let mut b = CpuCanvas::blank(4, 4);
        let same = compare_pixels(a.pixels(), b.pixels());
        assert_eq!((same.mean_channel_diff, same.off_fraction), (0.0, 0.0));
        assert!(same.within_tolerance());
        // One pixel of sixteen turned black: 1/16 off, mean 255/16 per channel.
        b.fill_rect(0, 0, 1, 1, Color::new(0, 0, 0, 255));
        let far = compare_pixels(a.pixels(), b.pixels());
        assert!((far.off_fraction - 1.0 / 16.0).abs() < 1e-9);
        assert!((far.mean_channel_diff - 255.0 / 16.0).abs() < 1e-6);
        assert!(!far.within_tolerance());
    }

}
