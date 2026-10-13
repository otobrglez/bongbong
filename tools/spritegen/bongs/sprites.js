// The bongs' sprites (docs/BONG_SPEC.md): every character drawn by code into
// a 48x48 cell, body centred on x 24 (the cell's axis, so a mirror flips it
// in place) and standing on the y 33 baseline. `export.js` writes them out as
// the game's filmstrips; this module is pure and also runs in a browser.
//
// Animation is continuous: a clip is sampled at time t in 0..1 (the middle of
// each frame), and every pose value is a curve through the clip's keyframes.
// `keys` frames per clip land exactly on the keys; the shipped `n` (three
// times as many, the same clip length) adds real in-betweens.
(function (root) {
  'use strict';
  const W = 48, H = 48, CX = 24, B = 33;
  const TAU = Math.PI * 2;

  function hex(c) {
    if (Array.isArray(c)) return c;
    const s = c.replace('#', '');
    return [parseInt(s.slice(0, 2), 16), parseInt(s.slice(2, 4), 16), parseInt(s.slice(4, 6), 16),
      s.length > 6 ? parseInt(s.slice(6, 8), 16) : 255];
  }
  function hash(a, b) {
    let h = Math.imul(a | 0, 374761393) ^ Math.imul(b | 0, 668265263) ^ 0x5bd1e995;
    h = Math.imul(h ^ (h >>> 13), 1274126177);
    return (h ^ (h >>> 16)) >>> 0;
  }
  const r = Math.round;
  const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
  const lerp = (a, b, u) => a + (b - a) * u;
  const ease = u => u * u * (3 - 2 * u);
  const frac = v => v - Math.floor(v);

  // A Catmull-Rom curve through `keys`, one key per original frame, each key
  // sitting at the middle of its frame. `t` is clip time 0..1.
  function track(keys, t, loop) {
    const n = keys.length;
    let k = t * n - 0.5;
    if (loop) k = ((k % n) + n) % n;
    else k = clamp(k, 0, n - 1);
    const i = Math.floor(k), u = k - i;
    const at = j => (loop ? keys[((j % n) + n) % n] : keys[clamp(j, 0, n - 1)]);
    const p0 = at(i - 1), p1 = at(i), p2 = at(i + 1), p3 = at(i + 2);
    return 0.5 * (2 * p1 + (-p0 + p2) * u + (2 * p0 - 5 * p1 + 4 * p2 - p3) * u * u + (-p0 + 3 * p1 - 3 * p2 + p3) * u * u * u);
  }
  // Eased piecewise-linear steps through `keys`, key j at x = j.
  function steps(keys, x) {
    x = clamp(x, 0, keys.length - 1);
    const i = Math.floor(x), u = x - i;
    return lerp(keys[i], keys[Math.min(keys.length - 1, i + 1)], ease(u));
  }
  // A part thrown from (x0, y0) at (vx, vy) per key under gravity `gr`, coming
  // to rest on `ground`.
  function fly(x0, y0, vx, vy, gr, ground, tau) {
    if (tau <= 0) return [x0, y0, false];
    const a = 0.5 * gr, c0 = y0 - ground;
    const tl = (-vy + Math.sqrt(Math.max(0, vy * vy - 4 * a * c0))) / (2 * a);
    const tt = Math.min(tau, tl);
    return [x0 + vx * tt, y0 + vy * tt + a * tt * tt, tau >= tl];
  }

  class G {
    constructor(outline) {
      this.mat = new Uint8Array(W * H);
      this.col = new Array(W * H).fill(null);
      this.fx = new Array(W * H).fill(null);
      this.under = new Array(W * H).fill(null);
      this.ramps = [null];
      this.q = [];
      this.outline = hex(outline);
    }
    ramp(o) {
      this.ramps.push({ base: hex(o.base), hi: hex(o.hi || o.base), lo: hex(o.lo || o.base), flat: !!o.flat });
      return this.ramps.length - 1;
    }
    in(x, y) { return x >= 0 && y >= 0 && x < W && y < H; }
    set(x, y, m) { x = Math.floor(x); y = Math.floor(y); if (this.in(x, y)) this.mat[y * W + x] = m; }
    at(x, y) { return this.in(x, y) ? this.mat[y * W + x] : 0; }
    rect(x, y, w, h, m) {
      x = r(x); y = r(y); w = r(w); h = r(h);
      for (let j = y; j < y + h; j++) for (let i = x; i < x + w; i++) this.set(i, j, m);
    }
    ellipse(cx, cy, rx, ry, m) {
      if (rx <= 0 || ry <= 0) return;
      for (let y = Math.floor(cy - ry) - 1; y <= Math.ceil(cy + ry); y++)
        for (let x = Math.floor(cx - rx) - 1; x <= Math.ceil(cx + rx); x++) {
          const dx = (x + 0.5 - cx) / rx, dy = (y + 0.5 - cy) / ry;
          if (dx * dx + dy * dy <= 1) this.set(x, y, m);
        }
    }
    // A flat-bottomed dome: rounded top, straight sides, bottom row on `b`.
    dome(cx, b, w, h, m, o = {}) {
      const top = o.top ?? 0.7, shear = o.shear || 0, round = o.round ?? 0;
      const y0 = b - h + 1, ry = Math.max(1, h * top), yc = y0 + ry;
      for (let y = y0; y <= b; y++) {
        let hw = w / 2;
        const yy = y + 0.5;
        if (yy < yc) { const t = (yc - yy) / ry; hw = (w / 2) * Math.sqrt(Math.max(0, 1 - t * t)); }
        if (y === b) hw -= round;
        const off = (shear * (b - y)) / Math.max(1, h);
        const x0 = r(cx - hw + off), x1 = r(cx + hw + off);
        for (let x = x0; x < x1; x++) this.set(x, y, m);
      }
    }
    poly(pts, m) {
      let ymin = Infinity, ymax = -Infinity;
      for (const p of pts) { ymin = Math.min(ymin, p[1]); ymax = Math.max(ymax, p[1]); }
      for (let y = Math.floor(ymin); y <= Math.ceil(ymax); y++) {
        const yy = y + 0.5, xs = [];
        for (let i = 0; i < pts.length; i++) {
          const a = pts[i], c = pts[(i + 1) % pts.length];
          if ((a[1] <= yy && c[1] > yy) || (c[1] <= yy && a[1] > yy))
            xs.push(a[0] + ((yy - a[1]) / (c[1] - a[1])) * (c[0] - a[0]));
        }
        xs.sort((p, q) => p - q);
        for (let k = 0; k + 1 < xs.length; k += 2)
          for (let x = r(xs[k]); x < r(xs[k + 1]); x++) this.set(x, y, m);
      }
    }
    line(x0, y0, x1, y1, m, th = 1) {
      x0 = r(x0); y0 = r(y0); x1 = r(x1); y1 = r(y1);
      const dx = Math.abs(x1 - x0), sx = x0 < x1 ? 1 : -1, dy = -Math.abs(y1 - y0), sy = y0 < y1 ? 1 : -1;
      let e = dx + dy;
      for (;;) {
        for (let a = 0; a < th; a++) for (let c = 0; c < th; c++) this.set(x0 + a, y0 + c, m);
        if (x0 === x1 && y0 === y1) break;
        const e2 = 2 * e;
        if (e2 >= dy) { e += dy; x0 += sx; }
        if (e2 <= dx) { e += dx; y0 += sy; }
      }
    }
    clipBelow(y) {
      for (let j = y + 1; j < H; j++) for (let i = 0; i < W; i++) this.mat[j * W + i] = 0;
      this.q = this.q.filter(e => e[1] <= y);
    }
    clear(x, y, w, h) { this.rect(x, y, w, h, 0); }
    // Rim light: a 1 px shadow on the lower-right edge of each material,
    // a 1 px highlight where the upper-left touches empty space.
    shade() {
      for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) {
        const i = y * W + x, m = this.mat[i];
        if (!m || m === 255) continue;
        const rp = this.ramps[m];
        if (rp.flat) { this.col[i] = rp.base; continue; }
        const lo = this.at(x + 1, y) !== m || this.at(x, y + 1) !== m || this.at(x + 1, y + 1) !== m;
        const hi = !lo && (this.at(x - 1, y - 1) === 0 || this.at(x, y - 1) === 0);
        this.col[i] = lo ? rp.lo : hi ? rp.hi : rp.base;
      }
    }
    // Details are queued and painted after the shading pass.
    d(x, y, c) { this.q.push([r(x), r(y), hex(c)]); }
    drect(x, y, w, h, c) { for (let j = 0; j < r(h); j++) for (let i = 0; i < r(w); i++) this.d(r(x) + i, r(y) + j, c); }
    dpts(pts, ox, oy, c) { for (const [px, py] of pts) this.d(ox + px, oy + py, c); }
    flush() {
      for (const [x, y, c] of this.q) if (this.in(x, y)) { this.mat[y * W + x] = 255; this.col[y * W + x] = c; }
      this.q = [];
    }
    // Effects: drawn into their own layer, optionally outlined.
    fxBegin() { this.tmp = new Array(W * H).fill(null); }
    f(x, y, c) { x = r(x); y = r(y); if (this.in(x, y)) this.tmp[y * W + x] = hex(c); }
    frect(x, y, w, h, c) { for (let j = 0; j < r(h); j++) for (let i = 0; i < r(w); i++) this.f(r(x) + i, r(y) + j, c); }
    fcircle(cx, cy, rad, c) {
      for (let y = Math.floor(cy - rad) - 1; y <= Math.ceil(cy + rad); y++)
        for (let x = Math.floor(cx - rad) - 1; x <= Math.ceil(cx + rad); x++) {
          const dx = x + 0.5 - cx, dy = y + 0.5 - cy;
          if (dx * dx + dy * dy <= rad * rad) this.f(x, y, c);
        }
    }
    fline(pts, c) {
      for (let k = 0; k + 1 < pts.length; k++) {
        let [x0, y0] = pts[k].map(r), [x1, y1] = pts[k + 1].map(r);
        const dx = Math.abs(x1 - x0), sx = x0 < x1 ? 1 : -1, dy = -Math.abs(y1 - y0), sy = y0 < y1 ? 1 : -1;
        let e = dx + dy;
        for (;;) {
          this.f(x0, y0, c);
          if (x0 === x1 && y0 === y1) break;
          const e2 = 2 * e;
          if (e2 >= dy) { e += dy; x0 += sx; }
          if (e2 <= dx) { e += dx; y0 += sy; }
        }
      }
    }
    fxEnd(outline) {
      const t = this.tmp, o = outline ? hex(outline) : null;
      for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) {
        const i = y * W + x;
        if (t[i]) { this.fx[i] = t[i]; continue; }
        if (!o || this.fx[i]) continue;
        const n = (xx, yy) => this.in(xx, yy) && t[yy * W + xx];
        if (n(x - 1, y) || n(x + 1, y) || n(x, y - 1) || n(x, y + 1)) this.fx[i] = o;
      }
      this.tmp = null;
    }
    shadow(cx, y, rx, a = 70) {
      if (rx <= 0) return;
      for (let j = y - 1; j <= y + 1; j++) for (let x = Math.floor(cx - rx); x <= Math.ceil(cx + rx); x++) {
        const dx = (x + 0.5 - cx) / rx, dy = (j - y) / 1.6;
        if (dx * dx + dy * dy <= 1 && this.in(x, j)) this.under[j * W + x] = [20, 30, 20, a];
      }
    }
    // `flash` 0..1 blends the body and its outline towards white (the hurt flash).
    compose(flash) {
      this.flush();
      const out = new Uint8ClampedArray(W * H * 4);
      const solid = (x, y) => this.in(x, y) && this.mat[y * W + x] !== 0;
      for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) {
        const i = y * W + x;
        let c = null;
        if (this.mat[i]) c = this.col[i] || this.ramps[this.mat[i]]?.base;
        else if (solid(x - 1, y) || solid(x + 1, y) || solid(x, y - 1) || solid(x, y + 1)) c = this.outline;
        if (c && flash) c = [0, 1, 2].map(k => lerp(c[k], 255, flash)).concat([255]);
        let px = [0, 0, 0, 0];
        if (this.under[i]) px = this.under[i].slice();
        if (c) px = c.slice();
        const fc = this.fx[i];
        if (fc) px = blend(px, fc);
        out.set(px, i * 4);
      }
      return out;
    }
  }
  function blend(dst, src) {
    const a = src[3] / 255, da = dst[3] / 255, oa = a + da * (1 - a);
    if (oa <= 0) return [0, 0, 0, 0];
    return [0, 1, 2].map(k => (src[k] * a + dst[k] * da * (1 - a)) / oa).concat([oa * 255]);
  }

  // ---- shared faces ------------------------------------------------------
  // (x, y) is an eye's top-left; side -1 is the left eye, +1 the right one.
  function eye(g, x, y, kind, state, side, ink, extra = {}) {
    const white = extra.white || '#FFFFFF';
    const big = kind === 'googly' || kind === 'big';
    const ew = big ? 4 : 2;
    const eh = kind === 'googly' ? 4 : kind === 'big' ? 5 : kind === 'bead' ? 3 : 2;
    const lit = extra.lit || '#4FF5E4';
    if (state === 'blink' || state === 'closed') {
      g.drect(x, y + eh - 2, ew, 1, kind === 'led' ? extra.dim || '#1E6E68' : ink);
      if (big) g.drect(x, y + eh - 1, ew, 1, extra.lid || ink);
      return;
    }
    if (state === 'half') {
      if (kind === 'googly') { g.drect(x, y + 1, 4, 1, ink); g.drect(x, y + 2, 4, 2, white); g.drect(x + 2, y + 2, 2, 1, ink); }
      else if (kind === 'big') { g.drect(x, y + 1, 4, 1, extra.lid || ink); g.drect(x, y + 2, 4, 2, white); g.d(x + 1, y + 4, white); g.d(x + 2, y + 4, white); g.drect(x + 2, y + 2, 2, 2, ink); }
      else if (kind === 'led') g.drect(x, y + 1, 2, 1, lit);
      else g.drect(x, y + eh - 2, 2, 2, ink);
      return;
    }
    if (state === 'hurt') {
      const c = kind === 'led' ? '#FF5A3A' : ink;
      const ox = x + (ew > 2 ? 1 : 0), oy = y + (eh > 3 ? 1 : 0) - (eh === 2 ? 1 : 0);
      if (side < 0) g.dpts([[0, 0], [1, 1], [0, 2]], ox, oy, c);
      else g.dpts([[1, 0], [0, 1], [1, 2]], ox, oy, c);
      return;
    }
    if (state === 'dizzy') {
      const c = kind === 'led' ? '#FF5A3A' : ink;
      const ox = x + (ew > 2 ? 0 : -1), oy = y + (eh > 3 ? 0 : -1);
      g.dpts([[0, 0], [2, 0], [1, 1], [0, 2], [2, 2]], ox, oy, c);
      return;
    }
    if (state === 'happy') {
      const c = kind === 'led' ? lit : ink;
      if (ew > 2) g.dpts([[0, 1], [1, 0], [2, 0], [3, 1]], x, y + 1, c);
      else g.dpts([[1, 0], [0, 1], [2, 1]], x - 1, y, c);
      return;
    }
    // open / focus / wide
    if (kind === 'googly') {
      g.drect(x, y, 4, 4, white);
      g.drect(x + 2, y + 1, 2, 2, ink);
    } else if (kind === 'big') {
      g.drect(x, y + 1, 4, 3, white); g.drect(x + 1, y, 2, 5, white);
      g.drect(x + 2, y + 1, 2, 3, ink);
      g.d(x + 2, y + 1, '#8A8AA0');
    } else if (kind === 'led') {
      g.drect(x, y, 2, 2, lit);
    } else {
      g.drect(x, y, 2, eh, ink);
      g.d(x, y, white);
    }
    if (state === 'focus') {
      const c = extra.brow || ink;
      if (side < 0) g.dpts([[0, 0], [1, 0], [2, 1]], x - (ew > 2 ? 0 : 1), y - 2, c);
      else g.dpts([[0, 1], [1, 0], [2, 0]], x - (ew > 2 ? -1 : 0), y - 2, c);
    }
  }
  function mouth(g, x, y, state, ink, tongue) {
    if (state === 'open') { g.drect(x, y - 1, 2, 2, ink); if (tongue) g.d(x + 1, y, tongue); }
    else if (state === 'closed') g.drect(x, y, 2, 1, ink);
    else if (state === 'wobbly') g.dpts([[-1, 0], [0, -1], [1, 0], [2, -1]], x, y, ink);
    else if (state === 'o') g.dpts([[0, -1], [1, -1], [0, 0], [1, 0]], x, y, ink);
    else g.dpts([[-1, -1], [0, 0], [1, 0], [2, -1]], x, y, ink);
  }
  function star(g, x, y, c, big) {
    g.f(x, y, c); g.f(x - 1, y, c); g.f(x + 1, y, c); g.f(x, y - 1, c); g.f(x, y + 1, c);
    if (big) { g.f(x - 2, y, c); g.f(x + 2, y, c); g.f(x, y - 2, c); g.f(x, y + 2, c); }
  }
  function zig(x0, y0, x1, y1, n, amp, seed) {
    const pts = [[x0, y0]];
    for (let k = 1; k < n; k++) {
      const t = k / n, s = (hash(seed, k) % 2 ? 1 : -1) * amp;
      const nx = -(y1 - y0), ny = x1 - x0, len = Math.hypot(nx, ny) || 1;
      pts.push([x0 + (x1 - x0) * t + (nx / len) * s, y0 + (y1 - y0) * t + (ny / len) * s]);
    }
    pts.push([x1, y1]);
    return pts;
  }
  function bolt(g, pts, core, glow, out) {
    g.fxBegin();
    g.fline(pts.map(([x, y]) => [x, y + 1]), glow);
    g.fline(pts, core);
    g.fxEnd(out);
  }
  function puffs(g, list, c, out) {
    g.fxBegin();
    for (const [x, y, rad] of list) if (rad > 0.45) g.fcircle(x, y, rad, c);
    g.fxEnd(out);
  }

  // ---- clips and poses -----------------------------------------------------
  // `keys` is a clip's keyframe count, `n` its shipped frame count (3x), `sec`
  // its length - `n / sec` is the fps (BONG_*_FRAMES and BONG_*_FPS, lib.rs).
  const CLIPS = [
    { key: 'idle', label: 'Idle', keys: 8, n: 24, sec: 8 / 6, loop: true },
    { key: 'hop', label: 'Dodge', keys: 7, n: 21, sec: 0.7 },
    { key: 'attack', label: 'Strike', keys: 6, n: 18, sec: 0.6 },
    { key: 'hurt', label: 'Hurt', keys: 4, n: 12, sec: 0.4 },
    { key: 'ko', label: 'Knock-out', keys: 9, n: 27, sec: 0.9 },
  ];
  const CLIP = Object.fromEntries(CLIPS.map(c => [c.key, c]));

  // Frame i of n: t at the frame's middle, k the same time in key units,
  // f the key whose frame it falls in, u how far through that frame.
  function clock(clip, i, n) {
    const c = CLIP[clip], t = (i + 0.5) / n, k = t * c.keys;
    const f = Math.min(c.keys - 1, Math.floor(k));
    return { clip, t, k, f, u: k - f, i, n, fl: (t * c.sec) / CLIP.idle.sec };
  }
  const flashAt = u => (u < 0.67 ? 1 : 0.55);
  const isOpen = s => s === 'open' || s === 'focus' || s === 'wide';
  const isShut = s => s === 'blink' || s === 'closed';
  // A discrete eye table with a half-closed lid on the frames either side of a blink.
  function eyes(table, c) {
    const cur = table[c.f];
    if (isOpen(cur) && isShut(table[c.f + 1]) && c.u > 0.67) return 'half';
    if (isOpen(cur) && isShut(table[c.f - 1]) && c.u < 0.33) return 'half';
    return cur;
  }
  function base(c) {
    return { cx: CX, b: B, dw: 0, dh: 0, lift: 0, lean: 0, s: 1, eye: 'open', mouth: 'smile', clip: c.clip, t: c.t, k: c.k, f: c.f, u: c.u, i: c.i, fl: c.fl, wave: 0, sway: 0, flash: 0, T: keys => track(keys, c.t, c.clip === 'idle') };
  }
  function pose(c) {
    const p = base(c), T = p.T;
    if (c.clip === 'idle') {
      const ph = TAU * c.t;
      p.dh = -r(0.5 - 0.5 * Math.cos(ph));
      p.wave = r(1.3 * Math.sin(ph));
      p.sway = r(1.3 * Math.cos(ph));
      const bt = c.t;
      p.eye = bt >= 0.75 && bt < 0.865 ? 'blink' : (bt >= 0.71 && bt < 0.75) || (bt >= 0.865 && bt < 0.91) ? 'half' : 'open';
    } else if (c.clip === 'hop') {
      p.dw = r(T([2, -2, -2, 0, -2, 4, 2]));
      p.dh = r(T([-2, 2, 1, 0, 1, -3, -1]));
      p.lift = Math.max(0, r(T([0, 2, 4, 5, 3, 0, 0])));
      p.eye = eyes(['blink', 'open', 'open', 'open', 'open', 'blink', 'open'], c);
      p.mouth = ['closed', 'o', 'o', 'smile', 'smile', 'closed', 'smile'][c.f];
      p.wave = r(T([-1, 1, 1, 1, 0, -1, 0]));
      p.sway = r(T([0, -1, -1, 0, 1, 1, 0]));
    } else if (c.clip === 'attack') {
      p.lean = r(T([0, -1, 2, 2, 2, 1]));
      p.dw = r(T([0, 2, -2, 0, 0, 0]));
      p.dh = r(T([0, -1, 1, 0, 0, 0]));
      p.eye = ['open', 'focus', 'focus', 'focus', 'focus', 'happy'][c.f];
      p.mouth = ['smile', 'closed', 'open', 'open', 'open', 'smile'][c.f];
      p.wave = r(T([0, -1, 1, 1, 1, 0]));
      p.sway = r(T([0, -1, 1, 1, 0, 0]));
    } else if (c.clip === 'hurt') {
      p.eye = ['hurt', 'hurt', 'hurt', 'open'][c.f];
      p.mouth = ['wobbly', 'wobbly', 'wobbly', 'smile'][c.f];
      p.flash = c.f === 1 ? flashAt(c.u) : 0;
      p.dw = r(T([2, 0, 0, 0]));
      p.dh = r(T([-2, 0, -1, 0]));
      p.lean = c.f === 0 || c.f === 2 ? (c.i % 2 ? 1 : -1) : 0;
    }
    p.cx += p.lean;
    return p;
  }
  // A knock-out frame's body pose: the clip's clock with the given overrides.
  const P = (c, o) => Object.assign(base(c), o);
  // A knock-out track: keys per original frame, sampled at this frame.
  const KO = (c, keys) => track(keys, c.t, false);

  // ---- the ten -------------------------------------------------------------
  const C = [];

  // 1. Spike - a potted cactus.
  C.push({
    id: 'spike', name: 'Spike', kind: 'Potted cactus',
    outline: '#1A2A20',
    setup: g => ({
      cactus: g.ramp({ base: '#2FB07E', hi: '#6AD8A4', lo: '#1E7E5C' }),
      pot: g.ramp({ base: '#C8692E', hi: '#E89058', lo: '#8D4A25' }),
    }),
    body(g, p, R, o = {}) {
      const s = p.s, cx = p.cx, b = p.b - p.lift;
      const w = 10 + p.dw, h = r(14 * s) + p.dh;
      const cb = b - 5, top = cb - h + 1;
      const shear = o.shear || 0, sh = shear;
      g.dome(cx, cb, w, h, R.cactus, { top: 0.35, shear });
      if (!o.noArms) {
        const la = p.wave, ra = -p.wave;
        g.rect(cx - 9 + shear * 0.5, top + 6, 4, 2, R.cactus); g.rect(cx - 9 + shear * 0.5, top + 2 - la, 2, 5 + la, R.cactus);
        g.rect(cx + 5 + shear * 0.4, top + 8, 4, 2, R.cactus); g.rect(cx + 7 + shear * 0.4, top + 5 - ra, 2, 4 + ra, R.cactus);
      }
      g.poly([[cx - 5.5, b - 3.5], [cx + 5.5, b - 3.5], [cx + 4.5, b + 1], [cx - 4.5, b + 1]], R.pot);
      g.rect(cx - 7, b - 5, 14, 2, R.pot);
      if (!o.noFlower) {
        const fx = cx + r(sh);
        g.d(fx, top - 1, '#FFE96A');
        g.d(fx - 1, top - 1, '#FF6FA8'); g.d(fx + 1, top - 1, '#FF6FA8'); g.d(fx, top - 2, '#FF6FA8'); g.d(fx - 1, top - 2, '#FF9FC8'); g.d(fx + 1, top - 2, '#FF9FC8');
      }
      if (h > 12) { g.d(cx - 5 + sh * 0.8, top + 9, '#E8F5D0'); g.d(cx + 4 + sh * 0.7, top + 11, '#E8F5D0'); }
      if (!o.noArms) { g.d(cx - 9 + sh * 0.5, top + 3 - p.wave, '#E8F5D0'); g.d(cx + 8 + sh * 0.4, top + 6 + p.wave, '#E8F5D0'); }
      if (s >= 0.6 && !o.noFace) {
        const ink = '#0E2A1C', ey = top + 4;
        eye(g, cx - 2 + sh * 0.7, ey, 'bead', p.eye, -1, ink);
        eye(g, cx + 2 + sh * 0.7, ey, 'bead', p.eye, 1, ink);
        mouth(g, cx + sh * 0.6, ey + 6, p.mouth, ink, '#FF7AA8');
        g.d(cx - 4 + sh * 0.7, ey + 4, '#FF9FB4'); g.d(cx + 4 + sh * 0.7, ey + 4, '#FF9FB4');
      }
    },
    strike(g, p, R) {
      const b = p.b;
      g.fxBegin();
      [b - 14, b - 10, b - 7].forEach((y, j) => {
        const u = (p.k - 2.8 - j * 0.18) / 2;
        if (u < 0) return;
        if (u > 1 && p.u > 0.66) return;
        const x = u <= 1 ? lerp(33, 44 + (j % 2), u) : 44 + (j % 2);
        const yy = y - Math.sin(clamp(u, 0, 1) * Math.PI) * 0.8;
        g.f(x, yy, '#F4FFE0'); g.f(x + 1, yy, '#F4FFE0'); g.f(x + 2, yy, '#FFFFFF');
      });
      g.fxEnd('#1A2A20');
    },
    ko(g, c, R) {
      const dh = r(KO(c, [0, 0, 0, -4, -8, -11, -11, -10, -10]));
      const shear = KO(c, [0, 0, -2, -1, 0, 0, 0, 0, 0]);
      const wave = r(KO(c, [0, 0, -2, -2, 0, 0, 0, 0, 0]));
      const wob = c.k >= 5.6 && c.k < 6.6 ? Math.sin((c.k - 5.6) * TAU) : 0;
      const eyeS = c.f <= 2 ? 'hurt' : 'closed';
      this.body(g, P(c, { eye: eyeS, mouth: 'wobbly', dh, wave, cx: CX + r(wob), lift: r(Math.max(0, wob)) }), R,
        { shear, noArms: dh <= -6, noFace: dh <= -9, noFlower: c.k >= 4.4 });
      if (c.f >= 7) {
        const st = c.f === 7 ? (c.u < 0.5 ? 'blink' : 'half') : 'open';
        eye(g, CX - 3, B - 7, 'dot', st, -1, '#0E2A1C'); eye(g, CX + 1, B - 7, 'dot', st, 1, '#0E2A1C');
      }
      const petal = (k0, x0, xEnd) => {
        const tau = c.f === 8 ? 99 : c.k - k0;
        if (tau < 0) return;
        const q = Math.min(1, tau / 4);
        g.f(lerp(x0, xEnd, q) + Math.sin(tau * 3) * 1.5 * (1 - q), Math.min(B, B - 20 + tau * 5), '#FF6FA8');
      };
      g.fxBegin(); petal(3.8, CX + 1, CX + 9); petal(4.4, CX - 1, CX - 7); g.fxEnd(null);
      return { flash: c.f === 1 ? flashAt(c.u) : 0 };
    },
  });

  // 2. Oggle - a one-eyed blob with feelers.
  C.push({
    id: 'oggle', name: 'Oggle', kind: 'One-eyed feeler blob',
    outline: '#4A1435',
    setup: g => ({
      body: g.ramp({ base: '#DB4C9E', hi: '#F585C0', lo: '#A12F6F' }),
      tip: g.ramp({ base: '#FFD447', hi: '#FFF0A0', lo: '#D99A1E' }),
      eye: g.ramp({ base: '#FFFFFF', flat: true }),
    }),
    body(g, p, R) {
      const s = p.s, cx = p.cx, b = p.b - p.lift;
      const w = r(18 * s) + p.dw, h = r(14 * s) + p.dh, footH = Math.max(1, r(2 * s));
      const bodyB = b - footH, top = bodyB - h + 1;
      if (s > 0.4) { g.rect(cx - r(6 * s), b - footH + 1, r(4 * s), footH, R.body); g.rect(cx + r(2 * s), b - footH + 1, r(4 * s), footH, R.body); }
      g.dome(cx, bodyB, w, h, R.body, { top: 0.75 });
      const ay = top + r(h * 0.55);
      if (s > 0.5) {
        g.line(cx - w / 2 + 1, ay, cx - w / 2 - 2, ay - 2 - p.wave, R.body, 2);
        g.line(cx + w / 2 - 2, ay, cx + w / 2 + 1, ay - 2 + p.wave, R.body, 2);
        const sw = p.sway;
        g.line(cx - 3, top, cx - 5 + sw, top - 3, R.body, 1);
        g.rect(cx - 6 + sw, top - 5, 2, 2, R.tip);
        g.line(cx + 2, top, cx + 4 + sw, top - 3, R.body, 1);
        g.rect(cx + 4 + sw, top - 5, 2, 2, R.tip);
      }
      const ex = cx + 1, ey = r(top + 5.5 * s);
      const ink = '#2A0A1E';
      if (s >= 0.6) {
        if (p.eye === 'blink' || p.eye === 'closed') {
          g.drect(ex - 3, ey, 7, 1, ink);
        } else if (p.eye === 'hurt') {
          g.dpts([[-3, 0], [-2, -1], [-1, 0], [0, -1], [1, 0], [2, -1], [3, 0]], ex, ey, ink);
        } else if (p.eye === 'happy') {
          g.dpts([[-3, 1], [-2, 0], [-1, -1], [0, -1], [1, -1], [2, 0], [3, 1]], ex, ey, ink);
        } else {
          g.ellipse(ex + 0.5, ey + 0.5, 4.2 * s, 3.8 * s, R.eye);
          if (p.eye === 'half') {
            for (let y = ey - 4; y < ey; y++) for (let x = ex - 4; x <= ex + 4; x++) if (g.at(x, y) === R.eye) g.d(x, y, '#C23A86');
            g.drect(ex - 3, ey, 7, 1, ink);
            g.drect(ex, ey + 1, 3, 1, ink);
          } else if (p.eye === 'dizzy') {
            g.dpts([[-1, -1], [0, -1], [1, -1], [-1, 0], [1, 0], [-1, 1], [0, 1], [1, 1]], ex, ey, ink);
          } else if (p.eye === 'wide') {
            g.d(ex + 1, ey, ink);
          } else {
            g.drect(ex, ey - 1, 3, 3, ink); g.d(ex, ey - 1, '#FFFFFF');
          }
          if (p.eye === 'focus') g.dpts([[-3, -4], [-2, -4], [-1, -4], [0, -3], [1, -3]], ex, ey, '#7A1E50');
        }
        mouth(g, cx, top + r(10 * s) + 1, p.mouth, ink, '#FF8FB0');
      } else if (s > 0.25) {
        g.ellipse(cx + 0.5, top + h / 2, 1.6, 1.6, R.eye);
        g.d(cx, r(top + h / 2), ink);
      }
    },
    strike(g, p, R) {
      const top = p.b - 2 - (14 + p.dh) + 1;
      const tx = p.cx + 5 + p.sway, ty = top - 4;
      if (p.f === 2) { g.fxBegin(); star(g, tx, ty - 1, '#FFFFFF', p.i % 2 === 0); g.fxEnd(null); }
      if (p.f === 3 || p.f === 4) {
        const u = clamp((p.k - 3) / 1.5, 0, 1);
        const ex = lerp(40, 46, u), ey = lerp(p.b - 8, p.b - 5, u);
        bolt(g, zig(tx + 1, ty, ex, ey, 5, 2, 7 + p.i), '#FFFFFF', '#FFD447', '#6B3A00');
        if (p.k > 3.6) { g.fxBegin(); star(g, r(ex), r(ey), '#FFF0A0', p.i % 2 === 0); g.fxEnd('#6B3A00'); }
      }
      if (p.f === 5) { g.fxBegin(); g.f(44 + p.u * 2, p.b - 7 - p.u * 2, '#FFD447'); g.f(41, p.b - 4 + p.u * 2, '#FFD447'); g.fxEnd(null); }
    },
    ko(g, c, R) {
      const sKeys = [1, 1, 1, 0.8, 0.6, 0.45, 0.3, 0.15, 0];
      const s = KO(c, sKeys);
      const liftAt = t => Math.max(0, track([0, 0, 0, 3, 10, 17, 24, 30, 34], t, false));
      const dxAt = t => { const k = t * 9; return track([0, 0, 0, 2, 6, 11, 16, 20, 22], t, false) + (k >= 3 ? Math.sin(k * 9) * 1.3 : 0); };
      const dw = r(KO(c, [0, 0, 4, 1, 0, 0, 0, 0, 0])), dh = r(KO(c, [0, 0, 3, 1, 0, 0, 0, 0, 0]));
      const eyeS = c.f <= 1 ? 'hurt' : c.f === 2 ? 'wide' : 'dizzy';
      if (c.f < 7 && s > 0.2) this.body(g, P(c, { eye: eyeS, mouth: c.f <= 1 ? 'wobbly' : 'o', s, lift: r(liftAt(c.t)), cx: CX + r(dxAt(c.t)), dw, dh }), R);
      if (c.f >= 3 && c.f < 8) {
        const list = [];
        for (let j = 1; j <= 4; j++) {
          const kk = c.k - j * 0.32;
          if (kk < 3) continue;
          const tt = kk / 9, ss = track(sKeys, tt, false);
          list.push([CX + dxAt(tt) - 7 * ss, B - liftAt(tt) - 2, Math.max(0.6, 1.7 - j * 0.28)]);
        }
        puffs(g, list, '#E8EEF8', '#8090A8');
      }
      if (c.f === 8) puffs(g, [[CX + 15, B - 27, 0.8]], '#E8EEF8', '#8090A8');
      return { flash: c.f === 1 ? flashAt(c.u) : 0 };
    },
  });

  // 3. Gloop - a bouncy slime.
  C.push({
    id: 'gloop', name: 'Gloop', kind: 'Bouncy slime',
    outline: '#0D4553',
    setup: g => ({ body: g.ramp({ base: '#3ACBD0', hi: '#A6F2EC', lo: '#1D8EA6' }) }),
    body(g, p, R) {
      const s = p.s, cx = p.cx, b = p.b - p.lift;
      const w = r(20 * s) + p.dw, h = r(13 * s) + p.dh;
      const shear = p.clip === 'idle' ? 1.6 * Math.cos(TAU * p.t) : p.lean * 0.5;
      g.dome(cx, b, w, h, R.body, { top: 0.9, shear });
      if (p.lift === 0 && s > 0.5) g.rect(cx - w / 2 - 1, b, w + 2, 1, R.body);
      const top = b - h + 1, sh = shear * 0.6;
      if (s >= 0.6) {
        g.drect(cx - w / 2 + 4 + sh, top + 2, 2, 1, '#FFFFFF');
        g.d(cx - w / 2 + 3 + sh, top + 3, '#FFFFFF');
        g.d(cx - w / 2 + 3 + sh, top + 4, '#DFFCF8');
        const ink = '#0A2E38', ey = top + Math.min(r(5 * s), Math.max(2, r(h * 0.38)));
        eye(g, cx - 3 + sh, ey, 'bead', p.eye, -1, ink);
        eye(g, cx + 3 + sh, ey, 'bead', p.eye, 1, ink);
        mouth(g, cx + 1 + sh, ey + 5, p.mouth, ink, '#FF7AA8');
        g.drect(cx - 6 + sh, ey + 4, 2, 1, '#FF9FC4');
        g.drect(cx + 6 + sh, ey + 4, 2, 1, '#FF9FC4');
      } else if (s > 0.25) {
        const ey = top + Math.max(1, r(h * 0.4));
        g.d(cx - 2, ey, '#0A2E38'); g.d(cx + 1, ey, '#0A2E38');
      }
    },
    strike(g, p, R) {
      const b = p.b;
      g.fxBegin();
      if (p.k >= 2.7 && p.k < 5) {
        const u = clamp((p.k - 2.7) / 2.2, 0, 1);
        const x = lerp(35, 43.5, u), y = b - 7 - Math.sin(u * Math.PI) * 1.5, rad = lerp(2, 2.9, u);
        g.fcircle(x, y, rad, '#3ACBD0'); g.f(x - 1, y - 1, '#FFFFFF');
        g.f(x - rad - 2, y + 1, '#3ACBD0'); if (u > 0.3) g.f(x - rad - 4, y + 2, '#3ACBD0');
      }
      if (p.f === 5) {
        const gw = p.u < 0.33 ? 0 : 1;
        g.frect(41, b - 7 - gw, 2, 5 + gw, '#3ACBD0'); g.frect(43, b - 8 - gw, 2, 7 + gw * 2, '#3ACBD0');
        g.f(40 - gw, b - 9 - gw, '#3ACBD0'); g.f(46, b - 4 + gw, '#3ACBD0'); g.f(43, b - 7, '#A6F2EC');
      }
      g.fxEnd('#0D4553');
    },
    ko(g, c, R) {
      if (c.f <= 1) this.body(g, P(c, { eye: 'hurt', mouth: 'wobbly' }), R);
      else if (c.f === 2) this.body(g, P(c, { eye: 'hurt', mouth: 'wobbly', dw: r(lerp(3, 8, c.u)), dh: -r(lerp(3, 6, c.u)) }), R);
      else if (c.f < 8) {
        const kk = c.k - 3, side = 5.5 + 4.4 * kk;
        const hopL = r(3 * Math.abs(Math.sin((Math.PI * kk) / 1.3)));
        const hopR = r(3 * Math.abs(Math.sin((Math.PI * (kk + 0.4)) / 1.3)));
        const s = lerp(0.42, 0.36, clamp(kk / 4, 0, 1));
        this.body(g, P(c, { cx: CX - r(side), lift: hopL, s: 0.4 }), R);
        this.body(g, P(c, { cx: CX + r(side), lift: hopR, s: 0.4 }), R);
        const up = 3 * Math.pow(kk, 1.5);
        if (up < 32) this.body(g, P(c, { cx: CX + r(kk * 0.8), lift: r(up), s }), R);
      } else { g.fxBegin(); g.frect(CX - 2, B, 4, 1, '#3ACBD0'); g.f(CX + 3, B - 1, '#3ACBD0'); g.fxEnd('#0D4553'); }
      return { flash: c.f === 1 ? flashAt(c.u) : 0 };
    },
  });

  // 4. Puff - a pocket storm cloud.
  C.push({
    id: 'puff', name: 'Puff', kind: 'Pocket storm cloud',
    outline: '#4B5470',
    setup: g => ({
      cloud: g.ramp({ base: '#F2F5FA', hi: '#FFFFFF', lo: '#B5C0D6' }),
      grey: g.ramp({ base: '#A9B1C4', hi: '#C9D0DE', lo: '#7D869C' }),
    }),
    body(g, p, R, o = {}) {
      const s = p.s, cx = p.cx;
      const float = p.clip === 'idle' ? r(1.6 * (0.5 - 0.5 * Math.cos(TAU * p.t))) : 0;
      const by = p.b - 3 - p.lift - float;
      const m = o.grey ? R.grey : R.cloud;
      const kx = s * (1 + p.dw / 20), ky = s * (1 + p.dh / 12);
      g.shadow(cx, B, Math.max(2, 8 * s - (p.lift + float) * 0.4));
      g.ellipse(cx, by - 5 * ky, 7.5 * kx, 6 * ky, m);
      g.ellipse(cx - 6.5 * kx, by - 2.5 * ky, 5 * kx, 3.5 * ky, m);
      g.ellipse(cx + 6.5 * kx, by - 2.5 * ky, 5 * kx, 3.5 * ky, m);
      g.ellipse(cx - 3.5 * kx, by - 9 * ky, 4.5 * kx, 3.5 * ky, m);
      g.ellipse(cx + 3 * kx, by - 8.5 * ky, 3.5 * kx, 3 * ky, m);
      g.clear(0, by + 1, W, 4);
      const ink = '#2A3046', ey = r(by - 7 * ky);
      if (s >= 0.6) {
        eye(g, cx - 3, ey, 'dot', p.eye, -1, ink);
        eye(g, cx + 3, ey, 'dot', p.eye, 1, ink);
        g.drect(cx - 6, ey + 3, 2, 1, '#FFA6C0'); g.drect(cx + 6, ey + 3, 2, 1, '#FFA6C0');
        mouth(g, cx + 1, ey + 4, p.mouth, ink, '#FF7AA8');
      } else if (s > 0.25) { g.d(cx - 1, r(by - 3 * ky), ink); g.d(cx + 1, r(by - 3 * ky), ink); }
    },
    strike(g, p, R) {
      const by = p.b - 3 - p.lift;
      if (p.f === 3 || p.f === 4) {
        const u = clamp((p.k - 3) / 1.4, 0, 1);
        const x0 = p.cx + 10, y0 = by - 6, x1 = lerp(42, 46, u), y1 = by - 1;
        const j = p.i % 2;
        const pts = [[x0, y0], [x0 + 4, y0 - 2 + j], [x0 + 6, y0 + 1], [x0 + 10, y0 - 1 - j], [x0 + 11, y0 + 3], [x1, y1]];
        g.fxBegin();
        g.fline(pts, '#FFD84A'); g.fline(pts.map(([x, y]) => [x, y + 1]), '#FFD84A'); g.fline(pts.map(([x, y]) => [x, y - 1]), '#FFFFFF');
        g.fxEnd('#6B4A00');
        if (p.k > 3.6) { g.fxBegin(); star(g, r(x1), by - 2, '#FFF3A0', j === 0); g.fxEnd('#6B4A00'); }
      }
      if (p.f === 5) { g.fxBegin(); g.f(43, B - 3 - p.u * 2, '#FFD84A'); g.f(46, B - 1 - p.u, '#FFD84A'); g.fxEnd(null); }
    },
    preStrike(p) { return { grey: p.f >= 1 && p.f <= 4 }; },
    ko(g, c, R) {
      const liftKeys = [0, 0, 4, 7, 10, 13, 16, 18, 18], sKeys = [1, 1, 1, 0.85, 0.7, 0.5, 0.3, 0.12, 0];
      const s = KO(c, sKeys), lift = r(Math.max(0, KO(c, liftKeys)));
      const eyeS = c.f <= 1 ? 'hurt' : 'closed';
      if (c.f < 7 && s > 0.2) this.body(g, P(c, { eye: eyeS, mouth: c.f <= 4 ? 'wobbly' : 'smile', lift, s }), R, { grey: c.k >= 2 });
      if (c.f < 8 && c.k >= 2) {
        g.fxBegin();
        for (let j = 0; j < 14; j++) {
          const k0 = 2 + j * 0.33;
          if (k0 > 6.4 || c.k < k0) continue;
          const t0 = k0 / 9, s0 = track(sKeys, t0, false), y0 = B - 3 - track(liftKeys, t0, false);
          const x = CX + ((hash(j, 5) % 15) - 7) * s0, y = y0 + (c.k - k0) * 10;
          if (y < B - 1) { g.f(x, y, '#5AB8F0'); g.f(x, y + 1, '#5AB8F0'); }
        }
        g.fxEnd(null);
      }
      const pw = r(Math.max(0, KO(c, [0, 0, 0, 4, 8, 10, 12, 12, 10])));
      if (pw >= 2) { g.fxBegin(); g.frect(CX - pw / 2, B, pw, 1, '#5AB8F0'); g.frect(CX - pw / 2 + 1, B, 2, 1, '#B8E8FF'); g.fxEnd('#1E4E72'); }
      return { flash: c.f === 1 ? flashAt(c.u) : 0 };
    },
  });

  // 5. Capi - a mushroom kid.
  C.push({
    id: 'capi', name: 'Capi', kind: 'Mushroom kid',
    outline: '#3E1A14',
    setup: g => ({
      cap: g.ramp({ base: '#E4421A', hi: '#FF7A4A', lo: '#9C3527' }),
      stem: g.ramp({ base: '#F0E2C4', hi: '#FFF6E2', lo: '#C9B48C' }),
      foot: g.ramp({ base: '#8D4A25', flat: true }),
      dirt: g.ramp({ base: '#8D6A3A', hi: '#B08A50', lo: '#5E4424' }),
      leaf: g.ramp({ base: '#7FCB4A', hi: '#B6E87A', lo: '#4F8A2E' }),
    }),
    body(g, p, R) {
      const s = p.s, cx = p.cx, b = p.b - p.lift;
      const capUp = p.clip === 'idle' ? r(Math.max(0, Math.sin(TAU * (p.t - 0.25)))) : p.clip === 'attack' ? r(p.T([0, -1, 1, 1, 1, 0])) : 0;
      const stemH = r(9 * s) + Math.min(0, p.dh), capB = b - stemH - capUp;
      const cw = r(22 * s) + p.dw, ch = r(8 * s) + Math.max(0, p.dh) + (p.clip === 'attack' && p.f === 1 ? -1 : 0);
      if (s > 0.5) {
        const tap = p.clip === 'idle' && frac(p.t * 2) >= 0.5 ? 1 : 0;
        g.rect(cx - 5, b, 3, 1, R.foot); g.rect(cx + 2, b - tap, 3, 1, R.foot);
      }
      g.dome(cx, b - 1, r(10 * s) + (p.dw > 0 ? 2 : 0), stemH + 1, R.stem, { top: 0.25, round: 1 });
      g.dome(cx, capB, cw, ch, R.cap, { top: 0.9 });
      g.rect(cx - cw / 2 + 1, capB + 1, cw - 2, 1, R.cap);
      const capTop = capB - ch + 1;
      if (s >= 0.6) {
        g.drect(cx - 6, capTop + 2, 3, 2, '#FFF6E2'); g.d(cx - 7, capTop + 3, '#FFF6E2');
        g.drect(cx + 3, capTop + 1, 2, 2, '#FFF6E2');
        g.drect(cx + 7, capTop + 4, 2, 1, '#FFF6E2');
        g.drect(cx - 1, capTop + 4, 2, 1, '#FFF6E2');
        const ink = '#2A140E', ey = capB + 3;
        eye(g, cx - 3, ey, 'dot', p.eye, -1, ink);
        eye(g, cx + 2, ey, 'dot', p.eye, 1, ink);
        g.d(cx - 4, ey + 2, '#FF9FB4'); g.d(cx + 4, ey + 2, '#FF9FB4');
        mouth(g, cx, ey + 4, p.mouth, ink, '#FF7AA8');
      }
    },
    strike(g, p, R) {
      const b = p.b, sp = ['#E8EC8A', '#C8D85A', '#FFF6C0'];
      if (p.k >= 2 && p.k < 5) {
        const u = clamp((p.k - 2) / 3, 0, 1);
        g.fxBegin();
        for (let j = 0; j < 4; j++) {
          const uj = clamp((u - j * 0.1) / 0.7, 0, 1);
          if (uj <= 0) continue;
          const x = lerp(p.cx + 10, 39 + (j % 2) * 4 + j, uj), y = lerp(b - 13, b - 15 + j * 2.6, uj);
          const rad = 0.8 + 2 * Math.sin(Math.min(1, uj) * Math.PI * 0.8);
          g.fcircle(x, y, rad, sp[j % 3]);
        }
        g.fxEnd('#5E6A1E');
      }
      if (p.f === 5) {
        g.fxBegin();
        [[41, b - 13], [45, b - 10], [43, b - 6], [39, b - 9], [46, b - 14]].forEach(([x, y], k) => { if (hash(k, p.i) % 4) g.f(x + p.u * 1.5, y - p.u * 2, sp[k % 3]); });
        g.fxEnd(null);
      }
    },
    ko(g, c, R) {
      const sink = KO(c, [0, 0, 0, 3, 8, 13, 18, 18, 18]);
      const mw = r(Math.max(0, KO(c, [0, 0, 0, 16, 16, 16, 14, 12, 12])));
      if (c.f <= 1) this.body(g, P(c, { eye: 'hurt', mouth: 'wobbly' }), R);
      else if (sink < 0.8) this.body(g, P(c, { eye: 'closed', mouth: 'wobbly', dw: r(lerp(0, 2, c.u)), dh: -r(lerp(0, 2, c.u)) }), R);
      else if (sink < 17) { this.body(g, P(c, { eye: 'closed', b: B + r(sink) }), R); g.clipBelow(B - 2); }
      if (c.f >= 2 && mw >= 4) g.dome(CX, B, mw, 3, R.dirt, { top: 1 });
      if (c.k >= 3 && c.f < 7) {
        const a = fly(CX - 7, B - 3, -2.5, -4, 6, B, (c.k - 3) * 1.1), b2 = fly(CX + 7, B - 3, 2.8, -4.5, 6, B, (c.k - 3.3) * 1.1);
        g.fxBegin(); g.f(a[0], a[1], '#8D6A3A'); if (c.k > 3.3) g.f(b2[0], b2[1], '#8D6A3A'); g.fxEnd(null);
      }
      if (c.k >= 6.4) {
        const h = c.f === 8 ? 1 : clamp((c.k - 6.4) / 1.5, 0, 1);
        const len = r(1 + 3 * h);
        g.line(CX, B - 3, CX, B - 2 - len, R.leaf);
        if (h > 0.35) { g.rect(CX - 2, B - 2 - len, 2, 1, R.leaf); g.rect(CX + 1, B - 1 - len, 2, 1, R.leaf); }
        if (h > 0.8) { g.rect(CX - 3, B - 3 - len, 2, 1, R.leaf); g.rect(CX + 2, B - 2 - len, 2, 1, R.leaf); }
      }
      return { flash: c.f === 1 ? flashAt(c.u) : 0 };
    },
  });

  // 6. Blip - a squid-shaped space invader.
  C.push({
    id: 'blip', name: 'Blip', kind: 'Space-invader squid',
    outline: '#33204F',
    setup: g => ({ body: g.ramp({ base: '#B48CDC', hi: '#DCC6F4', lo: '#8462B0' }) }),
    body(g, p, R) {
      const s = p.s, cx = p.cx, b = p.b - p.lift;
      const w = r(18 * s) + p.dw, hh = r(11 * s) + p.dh, legH = Math.max(1, r(4 * s));
      const headB = b - legH, top = headB - hh + 1;
      g.dome(cx, headB, w, hh, R.body, { top: 0.8 });
      const k = w / 18;
      [-9, -5, -1, 3, 7].forEach((o, i) => {
        const lx = r(cx + o * k);
        const wig = p.clip === 'idle' ? r(Math.sin(TAU * (2 * p.t + i / 4)))
          : p.clip === 'hop' && p.lift > 0 ? (i < 2 ? -1 : i > 2 ? 1 : 0) : 0;
        if (legH > 1) g.rect(lx, headB + 1, Math.max(1, r(2 * s)), legH - 1, R.body);
        const fx = lx + (i === 0 ? -1 : i === 4 ? 1 : wig);
        g.rect(fx, b, Math.max(1, r(2 * s)), 1, R.body);
      });
      if (s >= 0.7) {
        const ink = '#2A1640';
        eye(g, cx - 6, top + 3, 'googly', p.eye, -1, ink);
        eye(g, cx + 2, top + 3, 'googly', p.eye, 1, ink);
        mouth(g, cx, top + 9, p.mouth, ink, '#FF7AA8');
        g.drect(cx - 7, top + 8, 2, 1, '#E9A3D6');
        g.drect(cx + 6, top + 8, 2, 1, '#E9A3D6');
      } else if (s > 0.3) {
        g.d(cx - 2, top + 2, '#2A1640'); g.d(cx + 1, top + 2, '#2A1640');
      }
    },
    strike(g, p, R) {
      const b = p.b, x0 = p.cx + 8;
      const reach = Math.max(0, r(p.T([0, 0, 3, 11, 15, 8])));
      if (reach > 0) {
        g.line(x0, b - 3, x0 + reach, b - 4, R.body, 2);
        if (reach >= 9) { g.rect(x0 + reach, b - 6, 2, 2, R.body); g.rect(x0 + reach - 1, b - 7, 2, 1, R.body); }
      }
      if (p.f === 4) { g.fxBegin(); star(g, 45, b - 9, '#FFF1A8', p.u < 0.67); g.fxEnd('#6B4A1A'); }
    },
    ko(g, c, R) {
      const half = r(Math.max(0, KO(c, [0, 0, 11, 11, 10, 8, 6, 4, 0])));
      const a = Math.max(0, KO(c, [0, 0, 64, 72, 72, 64, 48, 32, 0]));
      if (c.f < 8 && half > 0 && a > 6) {
        for (let y = 0; y <= B; y++) for (let x = CX - half; x < CX + half; x++) {
          const edge = x === CX - half || x === CX + half - 1;
          g.under[y * W + x] = [255, 243, 160, Math.min(255, r(edge ? a * 2 : a))];
        }
      }
      const s = KO(c, [1, 1, 1, 0.85, 0.7, 0.5, 0.3, 0.1, 0]);
      const lift = r(Math.max(0, KO(c, [0, 0, 1, 5, 11, 18, 25, 30, 30])));
      if (c.f < 7 && s > 0.2) this.body(g, P(c, { eye: c.f <= 2 ? 'dizzy' : 'happy', mouth: c.f <= 1 ? 'wobbly' : 'o', lift, s }), R);
      if (c.f === 0) {
        g.fxBegin();
        for (let j = 0; j < 3; j++) {
          const a2 = TAU * (c.k * 1.2 + j / 3);
          const x = CX + 8 * Math.cos(a2), y = 15 + 2 * Math.sin(a2);
          g.f(x, y, '#FFE96A'); g.f(x - 1, y, '#FFE96A'); g.f(x + 1, y, '#FFE96A'); g.f(x, y - 1, '#FFE96A'); g.f(x, y + 1, '#FFE96A');
        }
        g.fxEnd('#6B4A1A');
      }
      if (c.f === 7) { g.fxBegin(); star(g, CX, 4, '#FFF7C8', c.u < 0.5); g.fxEnd(null); }
      if (c.f === 8) { g.fxBegin(); g.f(CX - 4, B - 1, '#FFF3A0'); g.f(CX + 5, B - 3, '#FFF3A0'); g.f(CX, B, '#FFF3A088'); g.fxEnd(null); }
      return { flash: c.f === 1 ? flashAt(c.u) : 0 };
    },
  });

  // 7. Fuzz - a soot puff from the tank exhaust.
  C.push({
    id: 'fuzz', name: 'Fuzz', kind: 'Soot puff',
    outline: '#121218',
    setup: g => ({ fuzz: g.ramp({ base: '#3B3B4A', hi: '#5E5E76', lo: '#26262F' }) }),
    body(g, p, R, o = {}) {
      const s = p.s, cx = p.cx, b = p.b - p.lift;
      const rx = 8 * s * (1 + p.dw / 18), ry = 7.5 * s * (1 + p.dh / 16);
      const cy = b - 2 - ry;
      if (s > 0.5) {
        g.line(cx - 3, cy + ry - 1, cx - 3, b - 1, R.fuzz); g.rect(cx - 5, b, 3, 1, R.fuzz);
        g.line(cx + 3, cy + ry - 1, cx + 3, b - 1, R.fuzz); g.rect(cx + 3, b, 3, 1, R.fuzz);
        g.line(cx - rx, cy + 1, cx - rx - 3, cy - 1 - p.wave, R.fuzz);
        g.line(cx + rx - 1, cy + 1, cx + rx + 2, cy - 1 + p.wave, R.fuzz);
      }
      g.ellipse(cx, cy + 0.5, rx, ry, R.fuzz);
      const spiky = o.spiky || 0, n = 18, seed = p.clip === 'idle' ? Math.floor(p.t * 8) : Math.floor(p.k * 2);
      for (let k = 0; k < n; k++) {
        const a = (k / n) * TAU;
        if (Math.sin(a) > 0.55 && s > 0.5) continue;
        const on = hash(k, seed + 7) % 3 !== 0;
        if (!on && !spiky) continue;
        const len = 1 + spiky + (hash(k, 3) % 2);
        for (let l = 0; l < len; l++) g.set(cx + Math.cos(a) * (rx + 0.5 + l), cy + 0.5 + Math.sin(a) * (ry + 0.5 + l), R.fuzz);
      }
      if (s >= 0.6) {
        const ink = '#0A0A10', ey = r(cy - 3 * s);
        eye(g, cx - 4, ey, 'big', p.eye, -1, ink, { lid: '#5E5E76' });
        eye(g, cx + 2, ey, 'big', p.eye, 1, ink, { lid: '#5E5E76' });
        if (p.eye === 'hurt' || p.eye === 'dizzy' || p.eye === 'happy') {
          g.drect(cx - 4, ey, 4, 5, '#3B3B4A'); g.drect(cx + 2, ey, 4, 5, '#3B3B4A');
          eye(g, cx - 4, ey, 'googly', p.eye, -1, '#FFFFFF');
          eye(g, cx + 2, ey, 'googly', p.eye, 1, '#FFFFFF');
        }
        if (p.mouth === 'open' || p.mouth === 'o') g.drect(cx, ey + 6, 2, 1, '#FF7AA8');
      }
    },
    attackPose(p) {
      p.eye = ['open', 'open', 'wide', 'blink', 'blink', 'happy'][p.f];
      p.mouth = ['smile', 'closed', 'o', 'open', 'open', 'smile'][p.f];
      p.dw = r(p.T([0, 2, 3, -2, 0, 0])); p.dh = r(p.T([0, 1, 2, -2, 0, 0]));
    },
    strikeOpts(p) { return { spiky: p.f === 1 || p.f === 2 ? 1 : 0 }; },
    strike(g, p, R) {
      const b = p.b, cy = b - 9.5;
      const motes = (pts) => { g.fxBegin(); for (const [x, y] of pts) g.f(x, y, '#4E4E62'); g.fxEnd(null); };
      if (p.k >= 2.8 && p.k < 5) {
        const u = clamp((p.k - 2.8) / 2.2, 0, 1), x = lerp(p.cx + 12, 42, u);
        puffs(g, [[x, cy + 2 - u, 2.4 + 1.2 * Math.sin(u * Math.PI)], [x - 4 - u * 2, cy + 3, 1.3 * (1 - u * 0.5)]], '#4E4E62', '#121218');
        motes([[x - 3, cy - 2 - u * 2], [x - 2, cy + 6 + u]]);
      }
      if (p.f === 5) {
        puffs(g, [[44 + p.u, cy - 1 - p.u, 2.4 - p.u], [46, cy + 4, 1.6 - p.u * 0.6], [41 - p.u, cy + 4, 1.2 - p.u * 0.5]], '#4E4E62', '#121218');
        motes([[40 - p.u, cy - 4 - p.u], [47, cy - 3]]);
      }
    },
    ko(g, c, R) {
      if (c.f <= 1) this.body(g, P(c, { eye: 'hurt' }), R);
      else if (c.f === 2) this.body(g, P(c, { eye: 'wide', dw: r(lerp(1, 4, c.u)), dh: r(lerp(1, 3, c.u)) }), R, { spiky: c.u > 0.4 ? 2 : 1 });
      else {
        const x = c.f === 8 ? 6 : c.k - 3;
        const d = steps([4.5, 8, 11.5, 14, 16, 17, 17], x), rad = steps([5.4, 3.8, 2.5, 1.5, 0.8, 0, 0], x);
        if (c.f < 8) {
          const list = [];
          for (let j = 0; j < 6; j++) {
            const a = (j / 6) * TAU + 0.3;
            list.push([CX + Math.cos(a) * d, Math.min(B - 1, B - 10 + Math.sin(a) * d * 0.8), rad * (j % 2 ? 0.85 : 1)]);
          }
          if (x < 1.4) list.push([CX, B - 10, 8 - 5.5 * x]);
          puffs(g, list, '#4A4A5C', '#121218');
        }
        if (x < 4) {
          const dy = -r(x), blink = x >= 2 && x < 2.5;
          g.fxBegin();
          for (const ex of [CX - 4, CX + 2]) {
            if (blink) { g.frect(ex, B - 11 + dy, 4, 1, '#0A0A10'); continue; }
            g.frect(ex, B - 12 + dy, 4, 3, '#FFFFFF'); g.frect(ex + 1, B - 13 + dy, 2, 5, '#FFFFFF');
            g.frect(ex + 2, B - 12 + dy, 2, 3, '#0A0A10');
          }
          g.fxEnd('#121218');
        }
        if (x >= 2) {
          g.fxBegin();
          [[CX - 17, B - 3], [CX + 17, B - 6], [CX + 10, B], [CX - 12, B]].forEach(([mx, my], j) => {
            if (c.f === 8 && j < 2) return;
            g.f(mx, lerp(my - 6, my, clamp((x - 2) / 2, 0, 1)), '#4A4A5C');
          });
          g.fxEnd(null);
        }
      }
      return { flash: c.f === 1 ? flashAt(c.u) : 0 };
    },
  });

  // Frame `i` of `n` (n defaults to the smooth count) of one clip.
  function render(ch, clip, i, n) {
    n = n || CLIP[clip].n;
    const c = clock(clip, i, n);
    const g = new G(ch.outline);
    const R = ch.setup(g);
    let flash = 0;
    if (clip === 'ko') {
      const res = ch.ko(g, c, R);
      flash = (res && res.flash) || 0;
    } else {
      const p = pose(c);
      if (clip === 'attack' && ch.attackPose) ch.attackPose(p);
      flash = p.flash;
      if (p.lift > 0 && ch.id !== 'puff') g.shadow(p.cx, B, 7 - p.lift * 0.5);
      const opts = clip === 'attack' ? Object.assign({}, ch.preStrike ? ch.preStrike(p) : {}, ch.strikeOpts ? ch.strikeOpts(p) : {}) : {};
      ch.body(g, p, R, opts);
      if (clip === 'attack' && ch.strike) ch.strike(g, p, R);
    }
    g.shade();
    return g.compose(flash);
  }

  const api = { W, H, CX, B, CLIPS, CHARACTERS: C, render };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  else root.BongSprites = api;
})(typeof window !== 'undefined' ? window : globalThis);
