//! A dev server with no window: a round the tools drive in-process
//! (docs/dev-server-design.md section 3.1). `bbmcp` hosts one when no game
//! from its checkout is running, so a session has a private, deterministic
//! round with no port to collide on.
//!
//! It is the same `DevServer`, `Session` and dispatch the window uses, on
//! the calls the window's loop makes around a frame: `before_frame`
//! answers what can be answered, `advance` runs a pending `step`. There is
//! no clock, so the round moves only by `step`, and nothing draws, so
//! `screenshot` is the field painted on the CPU canvas (`thumbnail`).

use std::collections::BTreeMap;
use std::sync::mpsc;

use serde_json::{Value, json};

use super::{DevServer, Request, SHOT_DIR, base64_encode};
use crate::canvas::{Pixels, Sheet};
use crate::map::MapFile;
use crate::math::Color;
use crate::mode::Session;
use crate::simulation::{Game, Input};

/// The longest side a headless screenshot may have, in pixels: a 250-cell
/// field is 8000 px across, which no reader wants as one image.
const SHOT_MAX_SIDE: usize = 1600;

/// The tools a round with no clock cannot serve, and what to use instead.
const NEEDS_A_CLOCK: &[(&str, &str)] = &[
    ("resume", "a headless round has no clock to run on - it moves only by `step`"),
    ("input", "a headless round has no real-time frames to hold input over - pass the input to `step`"),
];

/// A round with no window, and the dev server that drives it.
pub struct Headless {
    session: Session,
    server: DevServer,
    tx: mpsc::Sender<Request>,
    /// Every sheet decoded for the CPU canvas, on the first `screenshot`.
    sheets: Option<BTreeMap<Sheet, Pixels>>,
    shots: u64,
}

impl Headless {
    /// A fresh round on the shipped default map, frozen at frame 0.
    pub fn new() -> Result<Headless, String> {
        let mut game = Game::default();
        game.map = MapFile::from_toml_str(include_str!("../../maps/default.toml")).map_err(|e| format!("maps/default.toml: {e}"))?;
        game.show_intro = false;
        let (width, height) = game.map.field_size();
        game.init(width, height);
        let (mut server, tx) = DevServer::headless();
        server.windowless = true;
        server.lockstep = true;
        let session = Session::new(game);
        server.round_started(&session);
        Ok(Headless { session, server, tx, sheets: None, shots: 0 })
    }

    /// One tool call, answered before this returns.
    pub fn call(&mut self, method: &str, params: &Value) -> Result<Value, String> {
        if let Some((_, why)) = NEEDS_A_CLOCK.iter().find(|(tool, _)| *tool == method) {
            return Err(format!("{method}: {why}"));
        }
        let (reply, answer) = mpsc::channel();
        self.tx
            .send(Request { method: method.to_string(), params: params.clone(), reply })
            .map_err(|_| "the headless dev server is gone".to_string())?;
        let (width, height) = self.session.field_size();
        self.server.before_frame(&mut self.session, width, height);
        // What the window's frame would do next: run an armed `step`, and
        // take an armed `screenshot` after drawing.
        if self.server.pending_step.is_some() {
            // The field is the map's, which a `restart` in the same frame
            // may have changed.
            let (width, height) = self.session.field_size();
            self.server.advance(&mut self.session.game, Input::default(), 0, width, height, &mut |_| {});
        }
        if let Some(shot) = self.server.pending_shot.take() {
            let _ = shot.reply.send(self.screenshot(shot.scale));
        }
        answer.try_recv().map_err(|_| format!("{method} was not answered within its frame"))?
    }

    /// The field as the CPU canvas paints it, scaled, as the `screenshot`
    /// tool's reply.
    fn screenshot(&mut self, scale: f32) -> Result<Value, String> {
        if self.sheets.is_none() {
            let mut sheets = BTreeMap::new();
            for sheet in Sheet::all() {
                sheets.insert(sheet, load_sheet(&sheet.path())?);
            }
            self.sheets = Some(sheets);
        }
        let sheets = self.sheets.as_ref().expect("loaded above");
        let game = self.session.shown();
        let canvas = crate::thumbnail::render_cpu(game, sheets);
        let (width, height) = (canvas.width(), canvas.height());
        let scale = scale.min(SHOT_MAX_SIDE as f32 / width.max(height) as f32);
        let (out_w, out_h) = (((width as f32 * scale) as usize).max(1), ((height as f32 * scale) as usize).max(1));
        let png = encode_png(canvas.pixels(), width, height, out_w, out_h)?;
        self.shots += 1;
        let path = format!("{SHOT_DIR}/headless-{}-{}.png", std::process::id(), self.shots);
        std::fs::create_dir_all(SHOT_DIR).and_then(|()| std::fs::write(&path, &png)).map_err(|e| format!("{path}: {e}"))?;
        Ok(json!({
            "png_base64": base64_encode(&png),
            "path": path,
            "width": out_w,
            "height": out_h,
            "frame": game.frame(),
            "renderer": "cpu",
            "note": "a headless round: the whole field as the CPU canvas paints it - no HUD, chrome, debug overlays, particles or shaders; attach a window for those",
        }))
    }
}

/// One sprite sheet decoded to RGBA.
fn load_sheet(path: &str) -> Result<Pixels, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{path}: {e} (the adapter runs from the repository root, where static/ is)"))?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| format!("{path}: {e}"))?;
    let mut bytes = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut bytes).map_err(|e| format!("{path}: {e}"))?;
    let bytes = &bytes[..info.buffer_size()];
    let data: Vec<Color> = match info.color_type {
        png::ColorType::Rgba => bytes.chunks_exact(4).map(|p| Color::new(p[0], p[1], p[2], p[3])).collect(),
        png::ColorType::Rgb => bytes.chunks_exact(3).map(|p| Color::new(p[0], p[1], p[2], 255)).collect(),
        png::ColorType::GrayscaleAlpha => bytes.chunks_exact(2).map(|p| Color::new(p[0], p[0], p[0], p[1])).collect(),
        png::ColorType::Grayscale => bytes.iter().map(|&v| Color::new(v, v, v, 255)).collect(),
        png::ColorType::Indexed => return Err(format!("{path}: an indexed sheet came back unexpanded")),
    };
    Ok(Pixels { width: info.width as usize, height: info.height as usize, data })
}

/// `pixels` (`width` x `height`) as a PNG of `out_w` x `out_h`, each
/// output pixel the nearest source one.
fn encode_png(pixels: &[Color], width: usize, height: usize, out_w: usize, out_h: usize) -> Result<Vec<u8>, String> {
    let mut rgba = Vec::with_capacity(out_w * out_h * 4);
    for y in 0..out_h {
        let row = (y * height / out_h).min(height - 1) * width;
        for x in 0..out_w {
            let c = pixels[row + (x * width / out_w).min(width - 1)];
            rgba.extend_from_slice(&[c.r, c.g, c.b, c.a]);
        }
    }
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, out_w as u32, out_h as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(png)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_headless_round_restarts_steps_and_replays() {
        let run = || {
            let mut host = Headless::new().unwrap();
            let status = host.call("restart", &json!({ "seed": 7, "enemies": 3 })).unwrap();
            assert_eq!(status["frame"], 0, "{status}");
            assert_eq!(status["host"]["kind"], "headless", "{status}");
            let stepped = host.call("step", &json!({ "frames": 90, "move_dir": "up", "snapshot": true })).unwrap();
            assert_eq!(stepped["frame"], 90, "{stepped}");
            stepped["snapshot"].to_string()
        };
        assert_eq!(run(), run(), "a seed replays bit for bit with no window");
    }

    #[test]
    fn a_step_stops_after_the_frame_its_event_happens_on() {
        let mut host = Headless::new().unwrap();
        host.call("restart", &json!({ "seed": 7, "enemies": 3 })).unwrap();
        let stepped = host.call("step", &json!({ "frames": 300, "fire": true, "until": ["fired"] })).unwrap();
        assert_eq!(stepped["stopped"], "event", "{stepped}");
        assert_eq!(stepped["frames_run"], 1, "{stepped}");
        assert_eq!(stepped["frame"], 1, "{stepped}");
        // Nothing asked for happens: the step runs its frames out.
        let stepped = host.call("step", &json!({ "frames": 5, "until": ["round_won"], "until_outcome": true })).unwrap();
        assert_eq!(stepped["stopped"], "frames", "{stepped}");
        assert_eq!(stepped["frames_run"], 5, "{stepped}");
        // The round ending stops it: the seat's hull goes in one blow.
        host.call("set_tank", &json!({ "slot": 0, "damage": 1.0 })).unwrap();
        let stepped = host.call("step", &json!({ "frames": 600, "until_outcome": true })).unwrap();
        assert_eq!(stepped["stopped"], "outcome", "{stepped}");
        assert!(stepped["frames_run"].as_u64().unwrap() < 600, "{stepped}");
    }

    #[test]
    fn lint_reads_a_map_that_is_loaded_nowhere() {
        let mut host = Headless::new().unwrap();
        let toml = include_str!("../../maps/test/maze.toml");
        let report = host.call("lint", &json!({ "map_toml": toml })).unwrap();
        assert_eq!(report["source"], "text", "{report}");
        assert!(report["findings"].is_array());
        assert_ne!(host.call("status", &json!({})).unwrap()["map"]["cells"], report["map"]["cells"], "the round's map is untouched");
        assert!(host.call("lint", &json!({ "map_toml": toml, "source": "round" })).unwrap_err().contains("not both"));
    }

    #[test]
    fn the_clock_tools_say_what_to_use_instead() {
        let mut host = Headless::new().unwrap();
        assert!(host.call("resume", &json!({})).unwrap_err().contains("step"));
        assert!(host.call("input", &json!({ "fire": true })).unwrap_err().contains("step"));
        // And the builder works with no window, as it does in the tests.
        assert_eq!(host.call("build", &json!({})).unwrap()["mode"], "build");
        let painted = host.call("builder_paint", &json!({ "cells": [[3, 3]], "tool": "brick" })).unwrap();
        assert_eq!(painted["changes"].as_array().unwrap().len(), 1, "{painted}");
    }

    #[test]
    fn a_headless_screenshot_is_the_field_as_a_png() {
        let mut host = Headless::new().unwrap();
        let shot = host.call("screenshot", &json!({ "scale": 0.25 })).unwrap();
        assert_eq!(shot["renderer"], "cpu", "{}", shot["note"]);
        let (width, height) = host.session.field_size();
        assert_eq!(shot["width"], (width * 0.25) as usize);
        assert_eq!(shot["height"], (height * 0.25) as usize);
        let path = shot["path"].as_str().unwrap();
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(&bytes[1..4], b"PNG");
        let _ = std::fs::remove_file(path);
    }
}
