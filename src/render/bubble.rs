//! The frog's speech bubble (`bubble.rs`), painted in the bitmap's pixels
//! over the world and under the HUD: a white body with a near-black rim
//! and a stepped tail, the words in the default font and every control as
//! a key cap.

use sola_raylib::prelude::*;

use crate::bubble::{Bubble, Piece, Token};

const INK: Color = Color { r: 0x25, g: 0x1D, b: 0x14, a: 255 };
const PAPER: Color = Color { r: 0xFF, g: 0xFF, b: 0xFF, a: 255 };
const RIM: Color = Color { r: 0x25, g: 0x25, b: 0x25, a: 255 };
const CAP: Color = Color { r: 0xF0, g: 0xF0, b: 0xF0, a: 255 };
const CAP_EDGE: Color = Color { r: 0x99, g: 0x65, b: 0x24, a: 255 };
const CAP_SHADE: Color = Color { r: 0xC1, g: 0xC1, b: 0xC1, a: 255 };

/// Paint `bubble` through `bitmap`, the bitmap-to-window camera.
pub fn draw_bubble(d: &mut impl RaylibDraw, bubble: &Bubble, bitmap: Camera2D) {
    d.draw_mode2D(bitmap, |mut d, _| paint(&mut d, bubble));
}

fn paint(d: &mut impl RaylibDraw, b: &Bubble) {
    let block = (2.0 * b.px).round().max(2.0) as i32;
    let r = b.rect;
    let (x, y, w, h) = (r.x as i32, r.y as i32, r.width as i32, r.height as i32);
    // The rim, a block wide, with its corners notched so the body reads
    // round at the field's grain.
    d.draw_rectangle(x - block, y, w + 2 * block, h, RIM);
    d.draw_rectangle(x, y - block, w, h + 2 * block, RIM);
    if let Some((base, tip)) = b.tail {
        tail(d, base, tip, block, RIM, block);
    }
    d.draw_rectangle(x, y, w, h, PAPER);
    if let Some((base, tip)) = b.tail {
        tail(d, base, tip, block, PAPER, 0);
    }
    let size = b.size as i32;
    for (row, line) in b.lines.iter().enumerate() {
        let top = b.text_at.y + row as f32 * b.line_height;
        for piece in line {
            match piece {
                Piece::Word { text, x } => d.draw_text(text, (b.text_at.x + x) as i32, top as i32, size, INK),
                Piece::Key { token, x } => key(d, *token, b.text_at.x + x, top, b.size, block),
            }
        }
    }
}

/// The tail: blocks narrowing from the body's edge to the tip, `grow`
/// wider on each side for the rim.
fn tail(d: &mut impl RaylibDraw, base: crate::math::Vec2, tip: crate::math::Vec2, block: i32, colour: Color, grow: i32) {
    let steps = ((tip.y - base.y).abs() / block as f32).round().max(1.0) as i32;
    let down = tip.y > base.y;
    for i in 0..steps {
        let half = ((steps - i) * block / 2).max(block / 2) + grow;
        let row = if down { base.y as i32 + i * block } else { base.y as i32 - (i + 1) * block };
        d.draw_rectangle(tip.x as i32 - half, row - if down { grow } else { 0 }, 2 * half, block + grow, colour);
    }
}

/// One control as a key cap at (`x`, `top`), text size `size` px.
fn key(d: &mut impl RaylibDraw, token: Token, x: f32, top: f32, size: f32, block: i32) {
    let h = size as i32 + block;
    let (x, top) = (x as i32, top as i32 - block / 2);
    let cap = |d: &mut dyn FnMut(i32, i32, i32, i32, Color), cx: i32, cw: i32| {
        d(cx, top + block / 2, cw, h, CAP_EDGE);
        d(cx, top, cw, h - block / 2, CAP);
        d(cx, top + h - block, cw, block / 2, CAP_SHADE);
    };
    let mut rect = |rx: i32, ry: i32, rw: i32, rh: i32, c: Color| d.draw_rectangle(rx, ry, rw, rh, c);
    match token {
        Token::Arrows => {
            let k = size as i32;
            for (i, dir) in [(-1, 0), (0, -1), (0, 1), (1, 0)].into_iter().enumerate() {
                let cx = x + i as i32 * (k + 2);
                cap(&mut rect, cx, k);
                arrow(&mut rect, cx + k / 2, top + h / 2 - block / 4, dir, block);
            }
        }
        Token::Space => {
            let w = token.width(size) as i32;
            cap(&mut rect, x, w);
            rect(x + w / 4, top + h - 2 * block, w / 2, block / 2, CAP_EDGE);
        }
        Token::Tap => {
            let w = token.width(size) as i32;
            cap(&mut rect, x, w);
            let r = (size * 0.3) as i32;
            rect(x + w / 2 - r, top + h / 2 - r, 2 * r, 2 * r, INK);
        }
        Token::Stick => {
            let w = token.width(size) as i32;
            let r = w / 2;
            rect(x, top + h / 2 - r, w, 2 * r, CAP_EDGE);
            rect(x + block / 2, top + h / 2 - r + block / 2, w - block, 2 * r - block, CAP);
            rect(x + r - r / 3 + block / 2, top + h / 2 - r / 3 - block / 2, 2 * r / 3, 2 * r / 3, INK);
        }
    }
}

/// A small arrow head of blocks pointing along `dir`, centred on (`cx`, `cy`).
fn arrow(rect: &mut impl FnMut(i32, i32, i32, i32, Color), cx: i32, cy: i32, dir: (i32, i32), block: i32) {
    let b = (block / 2).max(1);
    for i in 0..3 {
        let span = (3 - i) * 2 - 1;
        let (along, across) = (i * b - b, span * b);
        match dir {
            (1, 0) | (-1, 0) => rect(cx + dir.0 * along - b / 2, cy - across / 2, b, across, INK),
            _ => rect(cx - across / 2, cy + dir.1 * along - b / 2, across, b, INK),
        }
    }
}
