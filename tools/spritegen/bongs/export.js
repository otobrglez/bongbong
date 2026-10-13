// Write the bongs' filmstrips (docs/BONG_SPEC.md): for every character,
// static/bongs/<kind>/{idle,hop,attack,hurt,explosion}.png, each a row of
// 48x48 frames, RGBA, no padding. Deterministic - no randomness anywhere in
// sprites.js - so a re-run rewrites the same bytes.
//
//   node tools/spritegen/bongs/export.js [out_dir]    (default: static/bongs)
//
// Needs nothing beyond Node itself (zlib for the PNG data).
'use strict';
const fs = require('fs');
const path = require('path');
const zlib = require('zlib');
const S = require('./sprites.js');

// The game's clip files, keyed by sprites.js's clip names.
const FILES = { idle: 'idle', hop: 'hop', attack: 'attack', hurt: 'hurt', ko: 'explosion' };

function crc32(buf) {
  let crc = 0xffffffff;
  for (let n = 0; n < buf.length; n++) {
    let c = (crc ^ buf[n]) & 0xff;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    crc = (crc >>> 8) ^ c;
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

function png(w, h, rgba) {
  const raw = Buffer.alloc((w * 4 + 1) * h);
  for (let y = 0; y < h; y++) Buffer.from(rgba.buffer, rgba.byteOffset + y * w * 4, w * 4).copy(raw, y * (w * 4 + 1) + 1);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk('IHDR', ihdr),
    chunk('IDAT', zlib.deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

const out = process.argv[2] || path.join(__dirname, '..', '..', '..', 'static', 'bongs');
for (const ch of S.CHARACTERS) {
  const dir = path.join(out, ch.id);
  fs.mkdirSync(dir, { recursive: true });
  for (const clip of S.CLIPS) {
    const w = clip.n * S.W, strip = new Uint8ClampedArray(w * S.H * 4);
    for (let f = 0; f < clip.n; f++) {
      const px = S.render(ch, clip.key, f, clip.n);
      for (let y = 0; y < S.H; y++) strip.set(px.subarray(y * S.W * 4, (y + 1) * S.W * 4), (y * w + f * S.W) * 4);
    }
    fs.writeFileSync(path.join(dir, FILES[clip.key] + '.png'), png(w, S.H, strip));
  }
  console.log(`${ch.id}: ${S.CLIPS.map(c => `${FILES[c.key]} ${c.n}`).join(', ')}`);
}
