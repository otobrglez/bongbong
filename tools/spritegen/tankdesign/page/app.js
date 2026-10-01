(() => {
'use strict';

// ---------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------
const DATA = JSON.parse(document.getElementById('bb-data').textContent);
const S = DATA.S || 40;
const TS = 32;
const WEAPONS = ['minigun', 'missiles', 'plasma', 'laser', 'flame'];
const WNAME = { minigun: 'Minigun', missiles: 'Seeker missiles', plasma: 'Plasma', laser: 'Laser', flame: 'Flamethrower' };
const WSTATES = { minigun: 4, missiles: 5, plasma: 3, laser: 3, flame: 4 };
const MOD0 = {};
{ let c = 0; for (const w of WEAPONS) { MOD0[w] = c; c += WSTATES[w]; } }
const TEAMS = ['enemy', 'p1', 'p2', 'green', 'white', 'orange'];
const TEAM_NAME = { enemy: 'Enemy', p1: 'P1 blue', p2: 'P2 pink', green: 'Green', white: 'White', orange: 'Orange' };
const P34 = ['green', 'white', 'orange'];
const WRECKS = ['blown', 'gutted', 'husk', 'cookoff'];
const WRECK_NAME = { blown: 'Blown', gutted: 'Gutted', husk: 'Husk', cookoff: 'Cook-off' };
const TIER_NAME = ['Pristine', 'Scuffed', 'Damaged', 'Critical'];
const TIER_RANGE = ['0–24 %', '25–49 %', '50–74 %', '75–99 %'];
const LINES = DATA.lines;
const LINE_KEYS = LINES.map(l => l.key);
const COLS = [{ key: 'today', title: 'Today', tagline: 'The sprites in the game now, for comparison.' }].concat(LINES);
const COLOR = { today: '#9e9e96', vanguard: '#de9943', skimmer: '#27d8c5', foundry: '#e44219', prototype: '#00d097' };
const CHASSIS = DATA.chassis;
const CH = Object.fromEntries(CHASSIS.map(c => [c.key, c]));
const DESIGNS = Object.fromEntries(DATA.designs.map(d => [d.id, d]));
const TEAM_RGB = { p1: [0.302, 0.608, 0.902], p2: [0.941, 0.31, 0.471], green: [0.118, 0.737, 0.451], white: [0.827, 0.855, 0.89], orange: [0.984, 0.42, 0.114] };
const TEAM_LIGHT_RGB = { p1: [0.561, 0.827, 1.0], p2: [0.929, 0.502, 0.6], green: [0.42, 0.9, 0.635], white: [0.957, 0.973, 1.0], orange: [1.0, 0.659, 0.227] };
const TEAM_HEX = { enemy: '#9e9e96', p1: '#4d9be6', p2: '#f04f78', green: '#1ebc73', white: '#d3dae3', orange: '#fb6b1d' };
const dpr = () => Math.max(1, Math.min(3, Math.round((window.devicePixelRatio || 1) * 2) / 2));

function des(col, chassis) {
  return col === 'today' ? null : DESIGNS[col + '.' + chassis] || null;
}

function colTitle(col) {
  const c = COLS.find(x => x.key === col);
  return c ? c.title : col;
}

function hexRGB(h) {
  h = h.replace('#', '');
  return [parseInt(h.slice(0, 2), 16) / 255, parseInt(h.slice(2, 4), 16) / 255, parseInt(h.slice(4, 6), 16) / 255];
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------
const st = {
  team: 'enemy', light: 'day', ground: 'grass', scale: 3, motion: true,
  col: LINE_KEYS[0], chassis: 'assault',
  tier: 0, wreck: null, mods: new Set(), heading: 0, turretMode: 'sweep', zoom: 6,
  box: false, fixtures: false, aim: null, fireAt: -99, drive: true,
};
try {
  const saved = JSON.parse(localStorage.getItem('bb-motorpool-view') || 'null');
  if (saved) {
    for (const k of ['team', 'light', 'ground', 'scale', 'motion']) if (saved[k] !== undefined) st[k] = saved[k];
  }
} catch (e) { /* storage may be unavailable */ }

function saveView() {
  try {
    localStorage.setItem('bb-motorpool-view', JSON.stringify({ team: st.team, light: st.light, ground: st.ground, scale: st.scale, motion: st.motion }));
  } catch (e) { /* ignore */ }
}

// ---------------------------------------------------------------------------
// Assets
// ---------------------------------------------------------------------------
const A = { design: {}, today: null, ground: {}, field: {}, density: {} };

function loadImg(src) {
  return new Promise((res, rej) => {
    const i = new Image();
    i.onload = () => res(i);
    i.onerror = rej;
    i.src = src;
  });
}

function silhouette(img) {
  const c = document.createElement('canvas');
  c.width = img.width;
  c.height = img.height;
  const g = c.getContext('2d');
  g.drawImage(img, 0, 0);
  g.globalCompositeOperation = 'source-in';
  g.fillStyle = '#000';
  g.fillRect(0, 0, c.width, c.height);
  return c;
}

async function loadAssets() {
  const jobs = [];
  for (const d of DATA.designs) jobs.push(loadImg(d.atlas).then(img => { A.design[d.id] = { img, sh: silhouette(img) }; }));
  jobs.push(loadImg(DATA.today).then(img => { A.today = { img, sh: silhouette(img) }; }));
  for (const k of Object.keys(DATA.grounds)) jobs.push(loadImg(DATA.grounds[k]).then(img => { A.ground[k] = img; }));
  for (const k of Object.keys(DATA.field.img)) jobs.push(loadImg(DATA.field.img[k]).then(img => { A.field[k] = img; }));
  if (DATA.density) {
    for (const k of Object.keys(DATA.density.img)) jobs.push(loadImg(DATA.density.img[k]).then(img => { A.density[k] = { img, sh: silhouette(img) }; }));
  }
  await Promise.all(jobs);
}

// ---------------------------------------------------------------------------
// Tank layers
// ---------------------------------------------------------------------------
// A tank to draw: {col, chassis, team, tier 0-3, wreck|null, frame, pose,
// hull deg, turret deg, mods {weapon: state}, flash, alpha}
function layers(t) {
  const L = [];
  if (!t.design && (t.col === 'today' || !des(t.col, t.chassis))) {
    const a = A.today;
    const block = t.team === 'enemy' ? 0 : t.team === 'p2' ? 2 : 1;
    const sy = (block * 12 + CH[t.chassis].row) * TS;
    let hc;
    if (t.wreck) hc = 8 + WRECKS.indexOf(t.wreck);
    else if (t.tier >= 3) hc = 7;
    else if (t.tier >= 1) hc = 6;
    else hc = [0, 2, 3, 4][t.frame % 4];
    L.push({ img: a.img, sh: a.sh, sx: hc * TS, sy, esy: null, cell: TS, rot: t.hull, which: 'hull' });
    if (t.wreck === 'blown') L.push({ img: a.img, sh: a.sh, sx: 5 * TS, sy, esy: null, cell: TS, rot: t.hull + 150, which: 'turret', dx: 12, dy: 9 });
    else L.push({ img: a.img, sh: a.sh, sx: (t.wreck ? 5 : 1) * TS, sy, esy: null, cell: TS, rot: t.turret, which: 'turret' });
    return L;
  }
  const d = t.design || des(t.col, t.chassis);
  const a = A.design[d.id];
  const teams = d.teams || TEAMS;
  let ti = teams.indexOf(t.team);
  if (ti < 0) ti = teams.indexOf('p1');
  const sy = ti * 2 * S;
  const esy = (ti * 2 + 1) * S;
  const mrow = teams.length * 2;
  if (t.wreck) {
    const wi = WRECKS.indexOf(t.wreck);
    L.push({ img: a.img, sh: a.sh, sx: (16 + wi) * S, sy, esy, cell: S, rot: t.hull, which: 'hull' });
    if (t.wreck === 'blown') L.push({ img: a.img, sh: a.sh, sx: 32 * S, sy, esy, cell: S, rot: t.hull + 150, which: 'turret', dx: 12, dy: 9 });
    else L.push({ img: a.img, sh: a.sh, sx: 32 * S, sy, esy, cell: S, rot: t.turret, which: 'turret' });
    return L;
  }
  L.push({ img: a.img, sh: a.sh, sx: (t.tier * 4 + (t.frame % 4)) * S, sy, esy, cell: S, rot: t.hull, which: 'hull' });
  L.push({ img: a.img, sh: a.sh, sx: (20 + t.tier * 3 + (t.pose || 0)) * S, sy, esy, cell: S, rot: t.turret, which: 'turret' });
  if (t.mods) {
    for (const w of WEAPONS) {
      if (t.mods[w] == null) continue;
      L.push({ img: a.img, sh: a.sh, sx: (MOD0[w] + t.mods[w]) * S, sy: mrow * S, esy: (mrow + 1) * S, cell: S, rot: t.turret, which: 'mod' });
    }
  }
  return L;
}

function blit(g, img, sx, sy, cell, x, y, rot, k) {
  g.save();
  g.translate(Math.round(x), Math.round(y));
  if (rot) g.rotate(rot * Math.PI / 180);
  const h = Math.round(cell * k / 2);
  g.drawImage(img, sx, sy, cell, cell, -h, -h, h * 2, h * 2);
  g.restore();
}

function rot2(x, y, deg) {
  const r = deg * Math.PI / 180;
  const c = Math.cos(r), s = Math.sin(r);
  return [x * c - y * s, x * s + y * c];
}

// Draw one tank into a view at (x, y) device px, k device px per design px.
function drawTank(v, t, x, y, k) {
  const g = v.g;
  if (t.team && t.team !== 'enemy' && !TEAM_RGB[t.team]) t.team = 'p1';
  if (!t.design && t.col !== 'today' && !des(t.col, t.chassis)) {
    g.save();
    g.strokeStyle = 'rgba(236,232,223,0.35)';
    g.setLineDash([4 * dpr(), 4 * dpr()]);
    g.lineWidth = dpr();
    const fp = CH[t.chassis].fp;
    g.strokeRect(Math.round(x + fp[0] * k) + 0.5, Math.round(y + fp[1] * k) + 0.5, (fp[2] - fp[0] + 1) * k, (fp[3] - fp[1] + 1) * k);
    g.restore();
    return;
  }
  const L = layers(t);
  const d = t.design || des(t.col, t.chassis);
  const hov = d ? (d.hover || 0) : 0;
  const so = (1.5 + hov) * k;
  const alpha = t.alpha == null ? 1 : t.alpha;
  g.save();
  g.globalAlpha = 0.486 * alpha;
  for (const l of L) blit(g, l.sh, l.sx, l.sy, l.cell, x + (l.dx || 0) * k + 0.595 * so, y + (l.dy || 0) * k + 0.48 * so, l.rot, k);
  g.restore();
  g.save();
  g.globalAlpha = alpha;
  for (const l of L) blit(g, l.img, l.sx, l.sy, l.cell, x + (l.dx || 0) * k, y + (l.dy || 0) * k, l.rot, k);
  g.restore();
  if (v.lit) {
    // Each layer hides the light beneath it (the turret covers the hull's
    // glow), then adds its own.
    for (const l of L) {
      const lx = x + (l.dx || 0) * k, ly = y + (l.dy || 0) * k;
      v.eg.globalCompositeOperation = 'destination-out';
      blit(v.eg, l.sh, l.sx, l.sy, l.cell, lx, ly, l.rot, k);
      v.eg.globalCompositeOperation = 'source-over';
      if (l.esy != null) blit(v.eg, l.img, l.sx, l.esy, l.cell, lx, ly, l.rot, k);
    }
    tankLights(v, t, x, y, k);
  }
  if (t.flash) drawFlash(v, t, x, y, k);
  if (t.fx) drawWeaponFx(v, t, x, y, k);
}

function muzzles(t) {
  const d = t.design || des(t.col, t.chassis);
  if (d && d.muzzles && d.muzzles.length) return d.muzzles;
  const c = CH[t.chassis];
  const y = -c.muzzle;
  return c.lat ? [[-c.lat, y], [c.lat, y]] : [[0, y]];
}

// A muzzle flash: a small hot star at the barrel that fired.
const FLASH = [[0, -1, 'w'], [0, -2, 'g'], [-1, -1, 'g'], [1, -1, 'g'], [0, -3, 'r'], [-1, -2, 'r'], [1, -2, 'r'], [-2, 0, 'r'], [1, 0, 'r']];
function drawFlash(v, t, x, y, k) {
  const ms = muzzles(t);
  const which = t.flash === 2 && ms.length > 1 ? [ms[1]] : t.flash === 1 && ms.length > 1 ? [ms[0]] : ms;
  const col = { w: '#ffffff', g: '#eea343', r: '#ff421a' };
  for (const ctx of v.lit ? [v.g, v.eg] : [v.g]) {
    ctx.save();
    ctx.translate(Math.round(x), Math.round(y));
    ctx.rotate(t.turret * Math.PI / 180);
    for (const [mx, my] of which) {
      for (const [px, py, c] of FLASH) {
        ctx.fillStyle = col[c];
        ctx.fillRect(Math.round((mx - 0.5 + px) * k), Math.round((my + py) * k), Math.ceil(k), Math.ceil(k));
      }
    }
    ctx.restore();
  }
  if (v.lit) {
    for (const [mx, my] of which) {
      const p = rot2(mx, my - 2, t.turret);
      v.lights.push({ x: x + p[0] * k, y: y + p[1] * k, r: 26 * k, c: [1.0, 0.7, 0.35], a: 1.0 });
    }
  }
}

// Laser beam and flame jet while those modules fire.
function drawWeaponFx(v, t, x, y, k) {
  const d = des(t.col, t.chassis);
  if (!d) return;
  const hp = d.hardpoints || {};
  for (const ctx of v.lit ? [v.g, v.eg] : [v.g]) {
    ctx.save();
    ctx.translate(Math.round(x), Math.round(y));
    ctx.rotate(t.turret * Math.PI / 180);
    if (t.fx.laser && hp.laser) {
      const [lx, ly] = hp.laser;
      const x0 = Math.round((lx - 0.5) * k);
      const top = Math.round((ly - 5) * k);
      const len = Math.round(120 * k);
      ctx.fillStyle = '#ff421a';
      ctx.fillRect(x0 - Math.round(k), top - len, Math.round(3 * k), len);
      ctx.fillStyle = '#ffffff';
      ctx.fillRect(x0, top - len, Math.max(1, Math.round(k)), len);
    }
    if (t.fx.flame && hp.flame) {
      const [fx, fy] = hp.flame;
      const seed = Math.floor(t.fx.flame * 30);
      for (let i = 0; i < 70; i++) {
        const h = hash(i, seed);
        const dist = 3 + h * 26;
        const spread = (hash(i, seed + 7) - 0.5) * (1 + dist * 0.45);
        const c = dist < 8 ? '#ffffff' : dist < 16 ? '#eea343' : '#ff421a';
        ctx.fillStyle = c;
        ctx.fillRect(Math.round((fx + spread) * k), Math.round((fy - 5 - dist) * k), Math.ceil(k), Math.ceil(k));
      }
    }
    ctx.restore();
  }
  if (v.lit && t.fx.flame && hp.flame) {
    const p = rot2(hp.flame[0], hp.flame[1] - 16, t.turret);
    v.lights.push({ x: x + p[0] * k, y: y + p[1] * k, r: 55 * k, c: [1.0, 0.55, 0.2], a: 0.9 });
  }
  if (v.lit && t.fx.laser && hp.laser) {
    for (let s = 0; s < 6; s++) {
      const p = rot2(hp.laser[0], hp.laser[1] - 10 - s * 20, t.turret);
      v.lights.push({ x: x + p[0] * k, y: y + p[1] * k, r: 16 * k, c: [1.0, 0.3, 0.25], a: 0.7 });
    }
  }
}

function hash(a, b) {
  let h = (a * 374761393 + b * 668265263) | 0;
  h = (h ^ (h >>> 13)) * 1274126177;
  h = h ^ (h >>> 16);
  return (h >>> 0) / 4294967296;
}

// ---------------------------------------------------------------------------
// Lights (night and dusk): the game's model - the field multiplied by a
// light map cleared to the ambient, every light added into it - plus the
// sprites' emissive layer and a bloom.
// ---------------------------------------------------------------------------
const LIGHTING = {
  day: null,
  dusk: { amb: [0.8, 0.62, 0.54], k: 0.6, bloom: 0.35 },
  night: { amb: [0.15, 0.18, 0.29], k: 1.0, bloom: 0.85 },
};

function roleRGB(role, t) {
  const player = t.team !== 'enemy';
  const c = CH[t.chassis];
  const accent = c && c.glow ? hexRGB(c.glow) : [1, 0.7, 0.4];
  switch (role) {
    case 'tail': case 'warn': return [1.0, 0.22, 0.12];
    case 'marker': case 'core': return player ? TEAM_LIGHT_RGB[t.team] : accent;
    case 'sensor': return player ? TEAM_LIGHT_RGB[t.team] : [1.0, 0.22, 0.12];
    case 'beacon': return [1.0, 0.64, 0.26];
    case 'engine': case 'hot': case 'fire': return [1.0, 0.58, 0.24];
    case 'ion': case 'plasma': return [0.58, 0.93, 0.89];
    case 'laser': return [1.0, 0.3, 0.25];
    default: return [1.0, 0.9, 0.72];
  }
}

function tankLights(v, t, x, y, k) {
  const d = t.design || des(t.col, t.chassis);
  const player = t.team !== 'enemy';
  if (t.wreck) {
    if (t.wreck !== 'husk') {
      const f = 0.75 + 0.25 * Math.sin(v.now * 17 + x) * Math.sin(v.now * 5.3 + y);
      v.lights.push({ x, y, r: 36 * k, c: [1.0, 0.5, 0.18], a: 0.8 * f });
    }
    return;
  }
  const glow = player ? TEAM_RGB[t.team].map(c => c * 0.55 + 0.45) : [0.9, 0.55, 0.38];
  v.lights.push({ x, y, r: 23 * k, c: glow, a: 0.5 * (player ? 1 : 0.7) });
  const beam = player ? [1.0, 0.95, 0.82] : [1.0, 0.68, 0.42];
  const reach = 95 * (player ? 1 : 0.8) * k;
  let heads = [];
  let points = [];
  let spots = [];
  if (d) {
    for (const l of d.hull_lights || []) {
      if (l.kind === 'head') heads.push(l);
      else points.push([l, 'hull']);
    }
    for (const l of d.turret_lights || []) {
      if (l.kind === 'spot') spots.push(l);
      else points.push([l, 'turret']);
    }
  } else {
    heads = [{ x: 0, y: -9, dir: 0 }];
  }
  // Damage takes the lamps out: half at tier 2, all at tier 3.
  if (t.tier >= 3) { heads = []; spots = []; }
  else if (t.tier === 2) { heads = heads.slice(0, 1); }
  const each = heads.length > 1 ? 0.7 : 1.0;
  for (const l of heads) {
    const p = rot2(l.x, l.y, t.hull);
    const dir = (t.hull + (l.dir || 0) - 90) * Math.PI / 180;
    v.lights.push({ x: x + p[0] * k, y: y + p[1] * k, r: reach, c: beam, a: each, cone: { dir, half: 24 * Math.PI / 180 } });
    v.lights.push({ x: x + p[0] * k, y: y + p[1] * k, r: 6 * k, c: beam, a: 0.8 });
  }
  for (const l of spots) {
    const p = rot2(l.x, l.y, t.turret);
    const dir = (t.turret + (l.dir || 0) - 90) * Math.PI / 180;
    v.lights.push({ x: x + p[0] * k, y: y + p[1] * k, r: 75 * k, c: [1.0, 0.93, 0.8], a: 0.75, cone: { dir, half: 13 * Math.PI / 180 } });
  }
  if (t.tier < 3) {
    for (const [l, layer] of points) {
      const p = rot2(l.x, l.y, layer === 'hull' ? t.hull : t.turret);
      v.lights.push({ x: x + p[0] * k, y: y + p[1] * k, r: 7 * k, c: roleRGB(l.role, t), a: 0.55 });
    }
  } else if (Math.floor(v.now * 2.5) % 2 === 0) {
    v.lights.push({ x, y, r: 12 * k, c: [1.0, 0.2, 0.1], a: 0.8 });
  }
}

function drawLight(lg, l, gain) {
  const s = Math.max(0, l.a * gain);
  const c = l.c.map(v => Math.round(Math.min(1, v * s) * 255));
  const mid = l.c.map(v => Math.round(Math.min(1, v * s * 0.42) * 255));
  const grad = lg.createRadialGradient(l.x, l.y, 0, l.x, l.y, l.r);
  grad.addColorStop(0, `rgb(${c[0]},${c[1]},${c[2]})`);
  grad.addColorStop(0.35, `rgb(${mid[0]},${mid[1]},${mid[2]})`);
  grad.addColorStop(1, 'rgb(0,0,0)');
  lg.fillStyle = grad;
  if (l.cone) {
    for (const [w, a] of [[1.0, 0.75], [1.3, 0.35]]) {
      lg.save();
      lg.globalAlpha = a;
      lg.beginPath();
      lg.moveTo(l.x, l.y);
      lg.arc(l.x, l.y, l.r, l.cone.dir - l.cone.half * w, l.cone.dir + l.cone.half * w);
      lg.closePath();
      lg.clip();
      lg.fillRect(l.x - l.r, l.y - l.r, l.r * 2, l.r * 2);
      lg.restore();
    }
  } else {
    lg.fillRect(l.x - l.r, l.y - l.r, l.r * 2, l.r * 2);
  }
}

// ---------------------------------------------------------------------------
// Views: a canvas plus its offscreen light, emissive and bloom buffers.
// ---------------------------------------------------------------------------
const VIEWS = new Set();

function makeView(canvas, cssW, cssH, draw, opts) {
  const v = {
    canvas, draw, fps: (opts && opts.fps) || 12, last: 0, visible: false, dirty: true,
    lightMode: (opts && opts.lightMode) || null, now: 0,
  };
  v.g = canvas.getContext('2d');
  v.lc = document.createElement('canvas');
  v.lg = v.lc.getContext('2d');
  v.ec = document.createElement('canvas');
  v.eg = v.ec.getContext('2d');
  v.bc = document.createElement('canvas');
  v.bg = v.bc.getContext('2d');
  sizeView(v, cssW, cssH);
  VIEWS.add(v);
  io.observe(canvas);
  canvas._view = v;
  return v;
}

function sizeView(v, cssW, cssH) {
  const r = dpr();
  v.cssW = cssW;
  v.cssH = cssH;
  v.W = Math.round(cssW * r);
  v.H = Math.round(cssH * r);
  v.canvas.width = v.W;
  v.canvas.height = v.H;
  v.canvas.style.width = cssW + 'px';
  v.canvas.style.height = cssH + 'px';
  v.lc.width = v.W; v.lc.height = v.H;
  v.ec.width = v.W; v.ec.height = v.H;
  v.bc.width = Math.max(1, Math.round(v.W / 4));
  v.bc.height = Math.max(1, Math.round(v.H / 4));
  v.dirty = true;
}

function begin(v, now) {
  v.now = now;
  const g = v.g;
  g.setTransform(1, 0, 0, 1, 0, 0);
  g.globalCompositeOperation = 'source-over';
  g.globalAlpha = 1;
  g.imageSmoothingEnabled = false;
  const mode = v.lightMode || st.light;
  v.L = LIGHTING[mode];
  v.lit = !!v.L;
  v.lights = [];
  if (v.lit) {
    v.eg.setTransform(1, 0, 0, 1, 0, 0);
    v.eg.clearRect(0, 0, v.W, v.H);
    v.eg.imageSmoothingEnabled = false;
  }
}

function finish(v) {
  if (!v.lit) return;
  const L = v.L, g = v.g, lg = v.lg;
  lg.setTransform(1, 0, 0, 1, 0, 0);
  lg.globalCompositeOperation = 'source-over';
  lg.globalAlpha = 1;
  lg.fillStyle = `rgb(${L.amb.map(c => Math.round(c * 255)).join(',')})`;
  lg.fillRect(0, 0, v.W, v.H);
  lg.globalCompositeOperation = 'lighter';
  for (const l of v.lights) drawLight(lg, l, L.k);
  lg.globalCompositeOperation = 'source-over';
  g.globalCompositeOperation = 'multiply';
  g.drawImage(v.lc, 0, 0);
  // Bloom: the emissive layer downsampled and drawn back soft, then sharp.
  const bg = v.bg;
  bg.setTransform(1, 0, 0, 1, 0, 0);
  bg.clearRect(0, 0, v.bc.width, v.bc.height);
  bg.imageSmoothingEnabled = true;
  bg.drawImage(v.ec, 0, 0, v.bc.width, v.bc.height);
  g.globalCompositeOperation = 'lighter';
  g.imageSmoothingEnabled = true;
  g.globalAlpha = L.bloom;
  g.drawImage(v.bc, 0, 0, v.W, v.H);
  g.globalAlpha = L.bloom * 0.6;
  g.drawImage(v.bc, -v.W * 0.01, -v.H * 0.01, v.W * 1.02, v.H * 1.02);
  g.imageSmoothingEnabled = false;
  g.globalAlpha = 1;
  g.drawImage(v.ec, 0, 0);
  g.globalCompositeOperation = 'source-over';
}

const io = new IntersectionObserver(entries => {
  for (const e of entries) {
    const v = e.target._view;
    if (v) {
      v.visible = e.isIntersecting;
      if (v.visible) v.dirty = true;
    }
  }
}, { rootMargin: '120px' });

function dirtyAll() {
  for (const v of VIEWS) v.dirty = true;
}

function loop(ts) {
  const now = ts / 1000;
  for (const v of VIEWS) {
    if (!v.visible) continue;
    const due = st.motion && (ts - v.last) >= 1000 / v.fps;
    if (v.dirty || due || v.always) {
      v.last = ts;
      v.dirty = false;
      try {
        v.draw(v, now);
      } catch (err) {
        console.error(err);
      }
    }
  }
  requestAnimationFrame(loop);
}

// Ground: the real field render (1 field px = half a design px).
function ground(v, k, seed, theme) {
  const img = A.ground[theme || st.ground] || A.ground.grass;
  const f = k / 2;
  const sw = v.W / f, sh = v.H / f;
  const m = 80;
  const maxX = Math.max(0, img.width - 2 * m - sw);
  const maxY = Math.max(0, img.height - 2 * m - sh);
  const sx = m + Math.floor(hash(seed, 3) * maxX);
  const sy = m + Math.floor(hash(seed, 5) * maxY);
  v.g.drawImage(img, sx, sy, sw, sh, 0, 0, v.W, v.H);
}

// Idle animation for a tank in a card: rolling tracks, a turret that scans.
function idle(t, now, seed, opts) {
  const motion = st.motion;
  t.frame = motion ? Math.floor(now * 7 + seed) % 4 : 0;
  t.turret = (t.hull || 0) + (motion ? Math.sin(now * 0.55 + seed * 1.7) * 22 : 0);
  if (opts && opts.fire && motion) {
    const period = 3.4;
    const ph = (now + seed * 0.37) % period;
    t.pose = 0;
    t.flash = 0;
    const twin = muzzles(t).length > 1;
    if (ph < 0.09) { t.pose = 1; t.flash = twin ? 1 : 3; }
    else if (ph < 0.18) { t.pose = twin ? 2 : 2; if (twin) t.flash = 2; }
  }
  return t;
}

// ---------------------------------------------------------------------------
// Header legend
// ---------------------------------------------------------------------------
function buildLegend() {
  const el = document.getElementById('legend-lines');
  el.innerHTML = '';
  for (const c of COLS) {
    const d = document.createElement('div');
    d.className = 'll';
    d.innerHTML = `<span class="swatch" style="background:${COLOR[c.key] || '#888'}"></span><b></b><span></span>`;
    d.querySelector('b').textContent = c.title;
    d.querySelectorAll('span')[1].textContent = c.tagline;
    el.appendChild(d);
  }
}

// ---------------------------------------------------------------------------
// Matrix
// ---------------------------------------------------------------------------
const CHECK_SVG = '<svg viewBox="0 0 12 12" aria-hidden="true"><path d="M2 6.5 L5 9.5 L10 2.5" fill="none" stroke="currentColor" stroke-width="2"/></svg>';
let matrixCells = [];

function cellSize() {
  return (S + 4) * st.scale;
}

function buildMatrix() {
  const m = document.getElementById('matrix');
  for (const c of matrixCells) VIEWS.delete(c.view);
  matrixCells = [];
  m.innerHTML = '';
  const cs = cellSize();
  m.style.gridTemplateColumns = `132px repeat(5, ${cs}px)`;
  m.style.minWidth = (132 + cs * 5) + 'px';
  const corner = document.createElement('div');
  corner.className = 'mh';
  corner.innerHTML = '<div class="tag">Chassis</div>';
  m.appendChild(corner);
  for (const c of COLS) {
    const h = document.createElement('div');
    h.className = 'mh';
    h.innerHTML = `<div class="name"><span class="swatch" style="background:${COLOR[c.key]}"></span><span></span></div><div class="tag"></div>`;
    h.querySelector('.name span:last-child').textContent = c.title;
    h.querySelector('.tag').textContent = c.tagline;
    if (c.key !== 'today') {
      const b = document.createElement('button');
      b.className = 'lineall';
      b.textContent = 'Pick for all';
      b.title = `Pick ${c.title} for every chassis`;
      b.addEventListener('click', () => pickAll(c.key));
      h.appendChild(b);
    }
    m.appendChild(h);
  }
  for (const ch of CHASSIS) {
    const rl = document.createElement('div');
    rl.className = 'rl';
    rl.innerHTML = '<div class="cn"></div><div class="role"></div><div class="dims"></div>';
    rl.querySelector('.cn').textContent = ch.name;
    rl.querySelector('.role').textContent = ch.role;
    rl.querySelector('.dims').textContent = `${ch.fp[2] - ch.fp[0] + 1}×${ch.fp[3] - ch.fp[1] + 1} · ${ch.tier}`;
    m.appendChild(rl);
    for (const c of COLS) {
      const cell = document.createElement('div');
      cell.className = 'cell';
      cell.tabIndex = 0;
      cell.setAttribute('role', 'button');
      const d = des(c.key, ch.key);
      const code = c.key === 'today' ? 'Today' : d ? d.codename : 'Not drawn';
      cell.setAttribute('aria-label', `${ch.name}, ${c.title}${d ? ' ' + d.codename : ''}`);
      cell.innerHTML = `<span class="picked">PICK</span><canvas></canvas><div class="foot"><span class="code"></span></div>`;
      cell.querySelector('.code').textContent = code;
      const pb = document.createElement('button');
      pb.className = 'pickbtn';
      pb.innerHTML = CHECK_SVG;
      pb.title = `Pick ${c.title} for ${ch.name}`;
      pb.setAttribute('aria-pressed', 'false');
      pb.addEventListener('click', e => { e.stopPropagation(); togglePick(ch.key, c.key); });
      cell.querySelector('.foot').appendChild(pb);
      cell.addEventListener('click', () => select(c.key, ch.key, true));
      cell.addEventListener('keydown', e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); select(c.key, ch.key, true); } });
      m.appendChild(cell);
      const seed = CHASSIS.indexOf(ch) * 7 + COLS.indexOf(c) * 3;
      const view = makeView(cell.querySelector('canvas'), cs, cs, (v, now) => {
        begin(v, now);
        const k = st.scale * dpr();
        ground(v, k, seed);
        const t = idle({ col: c.key, chassis: ch.key, team: st.team, tier: 0, wreck: null, hull: 0 }, now, seed);
        drawTank(v, t, v.W / 2, v.H / 2 + k, k);
        finish(v);
      }, { fps: 10 });
      matrixCells.push({ el: cell, pb, col: c.key, chassis: ch.key, view });
    }
  }
  refreshMatrixMarks();
}

function refreshMatrixMarks() {
  for (const c of matrixCells) {
    c.el.classList.toggle('sel', c.col === st.col && c.chassis === st.chassis);
    const p = picks[c.chassis];
    const on = !!(p && p.pick === c.col);
    c.el.classList.toggle('is-picked', on);
    c.pb.setAttribute('aria-pressed', on ? 'true' : 'false');
  }
}

// ---------------------------------------------------------------------------
// Inspector
// ---------------------------------------------------------------------------
let insView = null;

function select(col, chassis, scroll) {
  st.col = col;
  st.chassis = chassis;
  refreshMatrixMarks();
  refreshInspector();
  rebuildDependent();
  if (scroll && window.matchMedia('(max-width: 1180px)').matches) {
    document.getElementById('inspector').scrollIntoView({ behavior: 'smooth', block: 'start' });
  }
}

function chipRow(el, items, isOn, onClick, cls) {
  el.innerHTML = '';
  for (const it of items) {
    const b = document.createElement('button');
    b.className = 'chip' + (cls ? ' ' + cls : '');
    b.textContent = it.label;
    if (it.title) b.title = it.title;
    b.setAttribute('aria-pressed', isOn(it) ? 'true' : 'false');
    b.addEventListener('click', () => onClick(it));
    el.appendChild(b);
  }
}

function refreshInspector() {
  const ch = CH[st.chassis];
  const d = des(st.col, st.chassis);
  document.getElementById('ins-name').textContent = `${ch.name} · ${colTitle(st.col)}`;
  document.getElementById('ins-code').textContent = d ? `“${d.codename}”` : (st.col === 'today' ? '' : 'not drawn yet');
  document.getElementById('ins-blurb').textContent = d ? d.blurb : st.col === 'today' ? `Today's ${ch.name}: ${ch.role.toLowerCase()}. The sprite in the game now.` : '';
  chipRow(document.getElementById('ins-tier'), TIER_NAME.map((n, i) => ({ label: n, i, title: TIER_RANGE[i] + ' damage' })),
    it => !st.wreck && st.tier === it.i, it => { st.tier = it.i; st.wreck = null; refreshInspector(); dirtyAll(); });
  chipRow(document.getElementById('ins-wreck'), WRECKS.map(w => ({ label: WRECK_NAME[w], w })),
    it => st.wreck === it.w, it => { st.wreck = st.wreck === it.w ? null : it.w; refreshInspector(); dirtyAll(); }, 'wreck');
  chipRow(document.getElementById('ins-mods'), WEAPONS.map(w => ({ label: WNAME[w], w })),
    it => st.mods.has(it.w), it => { if (st.mods.has(it.w)) st.mods.delete(it.w); else st.mods.add(it.w); refreshInspector(); dirtyAll(); });
  const viewItems = [
    { label: 'Zoom 4×', k: 'zoom', val: 4 }, { label: '6×', k: 'zoom', val: 6 }, { label: '8×', k: 'zoom', val: 8 },
    { label: 'Hull ↑', k: 'heading', val: 0 }, { label: '→', k: 'heading', val: 90 }, { label: '↓', k: 'heading', val: 180 }, { label: '←', k: 'heading', val: 270 },
    { label: 'Scan', k: 'turretMode', val: 'sweep' }, { label: 'Forward', k: 'turretMode', val: 'forward' },
    { label: 'Hit box', k: 'box', val: true, toggle: true }, { label: 'Fixtures', k: 'fixtures', val: true, toggle: true },
  ];
  chipRow(document.getElementById('ins-view'), viewItems, it => it.toggle ? !!st[it.k] : st[it.k] === it.val, it => {
    if (it.toggle) st[it.k] = !st[it.k];
    else st[it.k] = it.val;
    if (it.k === 'zoom') sizeInspector();
    refreshInspector();
    dirtyAll();
  });
  const pickBtn = document.getElementById('ins-pick');
  const p = picks[st.chassis];
  const on = !!(p && p.pick === st.col);
  pickBtn.setAttribute('aria-pressed', on ? 'true' : 'false');
  pickBtn.textContent = on ? `Picked for ${ch.name}` : `Pick for ${ch.name}`;
  const facts = document.getElementById('ins-facts');
  facts.innerHTML = '';
  const add = (k, val) => {
    const dt = document.createElement('dt');
    dt.textContent = k;
    const dd = document.createElement('dd');
    dd.textContent = val;
    facts.appendChild(dt);
    facts.appendChild(dd);
  };
  add('Role', ch.role);
  add('Hull', `${ch.fp[2] - ch.fp[0] + 1} × ${ch.fp[3] - ch.fp[1] + 1} design px (hit box kept)`);
  add('Guns', ch.guns === 2 ? `twin ${ch.gun}, ±${ch.lat} px apart` : `one ${ch.gun}`);
  if (d) {
    add('Moves on', { tracks: 'tracks', hover: 'grav pads (hovers)', quad: 'four track pods' }[d.locomotion] || d.locomotion);
    const lamps = (d.hull_lights || []).concat(d.turret_lights || []);
    const kinds = {};
    for (const l of lamps) kinds[l.kind] = (kinds[l.kind] || 0) + 1;
    add('Lights', Object.entries(kinds).map(([kk, n]) => `${n} ${kk}`).join(', ') || 'none');
    const ms = d.muzzles || [];
    add('Muzzle', ms.length ? ms.map(m => `${-m[1]} px ahead`).slice(0, 1).join('') + (ms.length > 1 ? ' (twin)' : '') : '—');
  }
}

function sizeInspector() {
  const cssW = Math.min(560, document.getElementById('inspector').clientWidth || 420);
  const cssH = Math.round(Math.max(260, S * st.zoom * 0.9));
  if (insView) sizeView(insView, cssW, cssH);
}

function buildInspector() {
  const canvas = document.getElementById('ins-canvas');
  insView = makeView(canvas, 420, 320, drawInspector, { fps: 30 });
  insView.always = false;
  sizeInspector();
  canvas.addEventListener('pointermove', e => {
    const r = canvas.getBoundingClientRect();
    st.aim = [(e.clientX - r.left) / r.width * insView.W, (e.clientY - r.top) / r.height * insView.H];
  });
  canvas.addEventListener('pointerleave', () => { st.aim = null; });
  canvas.addEventListener('click', fire);
  document.getElementById('ins-fire').addEventListener('click', fire);
  document.getElementById('ins-pick').addEventListener('click', () => togglePick(st.chassis, st.col));
  window.addEventListener('resize', () => { sizeInspector(); });
  refreshInspector();
}

function fire() {
  st.fireAt = performance.now() / 1000;
  if (insView) insView.dirty = true;
}

// Weapon module states over a firing cycle that started `dt` seconds ago.
function modStates(mods, dt, now) {
  const out = {};
  const firing = dt >= 0 && dt < 1.2;
  for (const w of mods) {
    let s = 0;
    if (w === 'minigun') s = firing && dt < 0.7 ? 1 + Math.floor(dt / 0.05) % 3 : 0;
    else if (w === 'missiles') s = !firing ? 0 : Math.min(4, 1 + Math.floor(dt / 0.14));
    else if (w === 'plasma') s = firing && dt < 0.12 ? 1 : firing && dt < 0.24 ? 2 : 0;
    else if (w === 'laser') s = firing && dt < 0.12 ? 1 : firing && dt < 0.4 ? 2 : 0;
    else if (w === 'flame') s = firing && dt < 0.9 ? 2 + Math.floor(now * 14) % 2 : Math.floor(now * 6) % 2;
    out[w] = s;
  }
  return out;
}

function firingPose(t, dt) {
  t.pose = 0;
  t.flash = 0;
  const twin = muzzles(t).length > 1;
  if (dt < 0 || dt > 0.4) return;
  if (twin) {
    if (dt < 0.09) { t.pose = 1; if (dt < 0.06) t.flash = 1; }
    else if (dt < 0.13) t.pose = 0;
    else if (dt < 0.22) { t.pose = 2; if (dt < 0.19) t.flash = 2; }
  } else {
    if (dt < 0.1) { t.pose = 1; if (dt < 0.06) t.flash = 3; }
    else if (dt < 0.2) t.pose = 2;
  }
}

function drawInspector(v, now) {
  begin(v, now);
  const k = st.zoom * dpr();
  ground(v, k, CHASSIS.findIndex(c => c.key === st.chassis) * 11 + 5);
  const cx = v.W / 2, cy = v.H / 2 + k * 2;
  const t = { col: st.col, chassis: st.chassis, team: st.team, tier: st.tier, wreck: st.wreck, hull: st.heading, frame: 0, pose: 0 };
  t.frame = st.motion && st.drive ? Math.floor(now * 8) % 4 : 0;
  let turret = st.heading;
  if (st.aim) turret = Math.atan2(st.aim[0] - cx, -(st.aim[1] - cy)) * 180 / Math.PI;
  else if (st.turretMode === 'sweep' && st.motion) turret = st.heading + Math.sin(now * 0.8) * 35;
  t.turret = turret;
  const dt = performance.now() / 1000 - st.fireAt;
  if (!st.wreck) {
    firingPose(t, dt);
    if (st.mods.size) {
      t.mods = modStates(st.mods, dt, now);
      const fx = {};
      if (st.mods.has('laser') && t.mods.laser === 2) fx.laser = 1;
      if (st.mods.has('flame') && t.mods.flame >= 2) fx.flame = now;
      if (fx.laser || fx.flame) t.fx = fx;
    }
    if (st.mods.size && dt >= 0 && dt < 1.2) v.dirty = true;
  }
  if (dt >= 0 && dt < 1.3) v.dirty = true;
  drawTank(v, t, cx, cy, k);
  finish(v);
  const g = v.g;
  if (st.box) {
    const ch = CH[st.chassis];
    g.save();
    g.translate(Math.round(cx), Math.round(cy));
    g.rotate(st.heading * Math.PI / 180);
    g.strokeStyle = 'rgba(255, 90, 54, 0.9)';
    g.lineWidth = Math.max(1, dpr());
    const [x0, y0, x1, y1] = ch.fp;
    g.strokeRect(x0 * k + 0.5, y0 * k + 0.5, (x1 - x0 + 1) * k - 1, (y1 - y0 + 1) * k - 1);
    g.strokeStyle = 'rgba(238, 163, 67, 0.95)';
    g.beginPath();
    g.moveTo(-3 * k, 0); g.lineTo(3 * k, 0); g.moveTo(0, -3 * k); g.lineTo(0, 3 * k);
    g.stroke();
    g.restore();
  }
  if (st.fixtures) {
    const d = des(st.col, st.chassis);
    if (d) {
      const put = (l, rot) => {
        const p = rot2(l.x, l.y, rot);
        g.save();
        g.strokeStyle = l.kind === 'head' || l.kind === 'spot' ? '#ffffff' : l.kind === 'tail' ? '#ff5a36' : '#eea343';
        g.lineWidth = Math.max(1, dpr());
        g.beginPath();
        g.arc(cx + p[0] * k, cy + p[1] * k, 1.6 * k, 0, Math.PI * 2);
        g.stroke();
        g.restore();
      };
      for (const l of d.hull_lights || []) put(l, t.hull);
      for (const l of d.turret_lights || []) put(l, t.turret);
    }
  }
}

// ---------------------------------------------------------------------------
// Sections that follow the inspected tank
// ---------------------------------------------------------------------------
let depViews = [];

function clearDep() {
  for (const v of depViews) VIEWS.delete(v);
  depViews = [];
}

function smallCard(parent, title, sub, draw, opts) {
  const card = document.createElement('div');
  card.className = 'card';
  card.innerHTML = '<canvas></canvas><div class="cap"><b></b><span></span></div>';
  card.querySelector('b').textContent = title;
  card.querySelector('span').textContent = sub || '';
  if (opts && opts.rng) {
    const r = document.createElement('span');
    r.className = 'rng';
    r.textContent = opts.rng;
    card.querySelector('.cap').appendChild(r);
  }
  parent.appendChild(card);
  let w = (opts && opts.w) || cellSize();
  let h = (opts && opts.h) || cellSize();
  if (opts && opts.fill) {
    w = Math.max(120, Math.floor(card.clientWidth - 2));
    h = Math.round(w * opts.fill);
  }
  const v = makeView(card.querySelector('canvas'), w, h, draw, opts);
  depViews.push(v);
  return v;
}

function rebuildDependent() {
  clearDep();
  buildDamage();
  buildWeapons();
  buildLights();
  buildAnatomy();
}

// The inspected design's own sheet, labelled.
function buildAnatomy() {
  const canvas = document.getElementById('anatomy-canvas');
  if (canvas._view) VIEWS.delete(canvas._view);
  const col = st.col === 'today' ? LINE_KEYS[0] : st.col;
  const d = des(col, st.chassis);
  if (!d) return;
  const z = 2;
  const pad = 22;
  const groups = [
    ['Hull · pristine', 0, 4], ['Scuffed', 4, 4], ['Damaged', 8, 4], ['Critical', 12, 4], ['Wrecks', 16, 4],
    ['Turret · rest, recoil ×2, per tier', 20, 12], ['Broken', 32, 1],
  ];
  let modCols = 0;
  for (const w of WEAPONS) modCols += WSTATES[w];
  const cssW = 33 * S * z + 90;
  const cssH = pad + 2 * S * z + 34 + pad + 2 * S * z + 10;
  const v = makeView(canvas, cssW, cssH, (v, now) => {
    begin(v, now);
    const r = dpr();
    const g = v.g;
    const a = A.design[d.id];
    const ti = TEAMS.indexOf(st.team);
    g.fillStyle = '#0b0d0f';
    g.fillRect(0, 0, v.W, v.H);
    const x0 = 90 * r;
    const cz = S * z * r;
    g.font = `${Math.round(12 * r)}px "Barlow Condensed", sans-serif`;
    g.textBaseline = 'middle';
    const label = (txt, x, y, color) => { g.fillStyle = color || 'rgba(236,232,223,0.75)'; g.fillText(txt.toUpperCase(), x, y); };
    for (const [name, c0, n] of groups) {
      label(name, x0 + c0 * cz + 4 * r, (pad / 2) * r);
      g.fillStyle = 'rgba(255,255,255,0.06)';
      g.fillRect(x0 + c0 * cz, pad * r, n * cz - 2 * r, 2 * cz);
    }
    const y1 = pad * r;
    // Paint on the real grass, light on black.
    const grass = A.ground.grass;
    for (let c = 0; c < 33; c++) {
      g.drawImage(grass, 100 + c * 7, 100, S, S, x0 + c * cz, y1, cz - 2 * r, cz);
    }
    g.drawImage(a.img, 0, ti * 2 * S, 33 * S, S, x0, y1, 33 * cz, cz);
    g.drawImage(a.img, 0, (ti * 2 + 1) * S, 33 * S, S, x0, y1 + cz, 33 * cz, cz);
    label('Paint', 8 * r, y1 + cz / 2);
    label('Light', 8 * r, y1 + cz * 1.5);
    // Modules.
    const y2 = y1 + 2 * cz + 34 * r;
    let c = 0;
    for (const w of WEAPONS) {
      label(WNAME[w], x0 + c * cz + 4 * r, y2 - 12 * r);
      c += WSTATES[w];
    }
    for (let i = 0; i < modCols; i++) g.drawImage(grass, 140 + i * 9, 140, S, S, x0 + i * cz, y2, cz - 2 * r, cz);
    g.drawImage(a.img, 0, 6 * S, modCols * S, S, x0, y2, modCols * cz, cz);
    g.drawImage(a.img, 0, 7 * S, modCols * S, S, x0, y2 + cz, modCols * cz, cz);
    label('Modules', 8 * r, y2 + cz / 2);
    label('Light', 8 * r, y2 + cz * 1.5);
    finish(v);
  }, { fps: 1, lightMode: 'day' });
  depViews.push(v);
  const lede = document.getElementById('anatomy-lede');
  lede.textContent = `${CH[st.chassis].name} · ${colTitle(col)} “${d.codename}”, in the ${st.team === 'enemy' ? 'enemy' : st.team === 'p1' ? 'player 1' : 'player 2'} colours: the paint, then the light layer the night adds on top. Forty design pixels a cell, drawn at twice that in the game, like today's thirty-two. 33 cells a team, plus 19 module cells shared by all three.`;
}

function buildDamage() {
  const ch = CH[st.chassis];
  const col = st.col;
  const d = des(col, st.chassis);
  document.getElementById('dmg-new-label').textContent = d ? `${ch.name} · ${colTitle(col)} “${d.codename}”` : `${ch.name} · today`;
  const elN = document.getElementById('dmg-new');
  const elT = document.getElementById('dmg-today');
  elN.innerHTML = '';
  elT.innerHTML = '';
  const seed = CHASSIS.indexOf(ch) * 5 + 1;
  const mk = (parent, colKey, tier, wreck, title, sub, rng) => smallCard(parent, title, sub, (v, now) => {
    begin(v, now);
    const k = st.scale * dpr();
    ground(v, k, seed + tier * 3 + (wreck ? WRECKS.indexOf(wreck) + 9 : 0));
    const t = { col: colKey, chassis: st.chassis, team: st.team, tier, wreck, hull: 0, frame: 0, pose: 0 };
    idle(t, now, seed);
    if (wreck) t.turret = 20;
    drawTank(v, t, v.W / 2, v.H / 2 + k, k);
    finish(v);
  }, { rng, fps: 8 });
  if (d) {
    for (let i = 0; i < 4; i++) mk(elN, col, i, null, TIER_NAME[i], '', TIER_RANGE[i]);
    for (const w of WRECKS) mk(elN, col, 3, w, WRECK_NAME[w], 'wreck', '100 %');
  } else {
    const p = document.createElement('p');
    p.className = 'muted';
    p.textContent = 'Select one of the four new lines in the lineup to see its damage ladder here.';
    elN.appendChild(p);
  }
  mk(elT, 'today', 0, null, 'Pristine', '', '0–29 %');
  mk(elT, 'today', 1, null, 'Light', '', '30–74 %');
  mk(elT, 'today', 3, null, 'Disabled', '', '75–99 %');
  for (const w of WRECKS) mk(elT, 'today', 3, w, WRECK_NAME[w], 'wreck', '100 %');
}

const WEAPON_TEXT = {
  minigun: 'A burst of bullets; the hot barrel cycles through three frames while it fires (the game already cycles them).',
  missiles: 'Four tubes that empty through a volley and refill one by one on the reload.',
  plasma: 'Coils at the main gun: dim when idle, bright as it charges, white as a bolt leaves.',
  laser: 'An emitter lens: dark red idle, charging, then the beam.',
  flame: 'A pilot light flickers while held; the nozzle roars while the trigger is down.',
};

// Where a module sits, in words, from the design's hardpoint.
function mountText(d, w) {
  const hp = (d.hardpoints || {})[w];
  if (!hp) return '';
  if (w === 'plasma') return 'On the main gun';
  const [x, y] = hp;
  const side = x >= 2 ? 'right' : x <= -3 ? 'left' : '';
  const fore = y <= -3 ? 'front' : y >= 2 ? 'rear' : '';
  if (!side) return fore === 'rear' ? 'Roof, behind the pivot' : fore === 'front' ? 'Roof, ahead of the pivot' : 'Roof, over the pivot';
  return `${side[0].toUpperCase() + side.slice(1)} ${fore ? fore + ' ' : ''}${fore ? 'quarter' : 'cheek'}`;
}

function buildWeapons() {
  const el = document.getElementById('weapons-grid');
  el.innerHTML = '';
  const col = st.col === 'today' ? LINE_KEYS[0] : st.col;
  const d = des(col, st.chassis);
  const ch = CH[st.chassis];
  if (!d) {
    el.innerHTML = '<p class="muted">No design to show.</p>';
    return;
  }
  const seed = CHASSIS.indexOf(ch) * 3 + 2;
  const items = WEAPONS.map(w => ({ key: w, title: WNAME[w], sub: `${mountText(d, w)} · ${WEAPON_TEXT[w]}`, mods: [w] }));
  items.push({ key: 'all', title: 'Full loadout', sub: 'All five at once: the worst case a tank that collects everything can reach. Each keeps its own mount.', mods: WEAPONS.slice() });
  const w = Math.round(cellSize() * 1.9);
  const h = Math.round(cellSize() * 1.25);
  for (const it of items) {
    smallCard(el, it.title, it.sub, (v, now) => {
      begin(v, now);
      const k = st.scale * dpr();
      ground(v, k, seed + WEAPONS.indexOf(it.key) * 13 + 40);
      const period = 2.6;
      const dt = st.motion ? ((now + WEAPONS.indexOf(it.key) * 0.43) % period) - 0.6 : -1;
      const t = { col, chassis: st.chassis, team: st.team, tier: 0, wreck: null, hull: 0, frame: st.motion ? Math.floor(now * 7) % 4 : 0, pose: 0 };
      t.turret = st.motion ? Math.sin(now * 0.5 + seed) * 12 : 0;
      t.mods = modStates(it.mods, dt, now);
      const fx = {};
      if (it.mods.includes('laser') && t.mods.laser === 2) fx.laser = 1;
      if (it.mods.includes('flame') && t.mods.flame >= 2) fx.flame = now;
      if (fx.laser || fx.flame) t.fx = fx;
      drawTank(v, t, v.W / 2, v.H * 0.62, k);
      finish(v);
    }, { w, h, fps: 20, fill: 0.5 });
  }
  const note = document.createElement('p');
  note.className = 'muted';
  note.style.gridColumn = '1 / -1';
  note.textContent = `Modules as drawn for ${ch.name} · ${colTitle(col)} “${d.codename}”. Every line draws its own versions of the five, and every design places them on its own turret.`;
  el.appendChild(note);
}

function buildLights() {
  const canvas = document.getElementById('lights-canvas');
  const old = canvas._view;
  if (old) VIEWS.delete(old);
  const cols = COLS.map(c => c.key);
  const cw = Math.round((S + 30) * st.scale);
  const cssW = Math.min(cw * cols.length, document.querySelector('.wrap').clientWidth - 2);
  const per = cssW / cols.length;
  const cssH = Math.round((S + 60) * st.scale);
  const v = makeView(canvas, cssW, cssH, (v, now) => {
    begin(v, now);
    const k = st.scale * dpr();
    ground(v, k, 77);
    cols.forEach((col, i) => {
      const t = { col, chassis: st.chassis, team: st.team, tier: 0, wreck: null, hull: 0, pose: 0 };
      idle(t, now, i * 2.3);
      drawTank(v, t, (i + 0.5) * per * dpr(), v.H * 0.72, k);
    });
    finish(v);
    const g = v.g;
    g.font = `${Math.round(12 * dpr())}px "Barlow Condensed", sans-serif`;
    g.fillStyle = 'rgba(236,232,223,0.8)';
    g.textAlign = 'center';
    cols.forEach((col, i) => g.fillText(colTitle(col).toUpperCase(), (i + 0.5) * per * dpr(), v.H - 8 * dpr()));
  }, { fps: 15, lightMode: 'night' });
  depViews.push(v);
}

function buildLightLegend() {
  const items = [
    ['#fff2d0', 'Headlights', 'a cone ahead of the hull; warm white on a player, amber on an enemy'],
    ['#ff421a', 'Tail lights', 'red, at the rear corners'],
    ['#4d9be6', 'Markers', 'strips, pads and rings in the chassis accent; the team colour on a player'],
    ['#ff421a', 'Sensor eye', 'red on an enemy turret, the team colour on a player'],
    ['#eea343', 'Beacons', 'amber, turning with the hull (Foundry)'],
    ['#93ece2', 'Engines and reactors', 'thrusters, grav pads and cores (Skimmer, Prototype)'],
    ['#ff5a36', 'Warning lamp', 'the one light left blinking at critical damage'],
    ['#eea343', 'Embers and fire', 'what a gutted or cooked-off wreck throws at night'],
  ];
  const el = document.getElementById('lights-legend');
  el.innerHTML = '';
  for (const [c, b, s] of items) {
    const d = document.createElement('div');
    d.innerHTML = `<span class="swatch" style="background:${c}"></span><span><b></b> <span></span></span>`;
    d.querySelector('b').textContent = b;
    d.querySelector('span span').textContent = '— ' + s;
    el.appendChild(d);
  }
}

// ---------------------------------------------------------------------------
// Review: the rebuilt designs and the player colours
// ---------------------------------------------------------------------------
const colours = { p3: 'green', p4: 'white' };
let reviewViews = [];
let rebuiltRefreshers = [];

const REBUILT = [
  { chassis: 'breaker', note: '“Rebuild this that more matches vanguard style”',
    what: 'The separate dozer blade on steel arms is gone. The ram is built into the bow: a thick prow with two steel ram horns, on the same fenders, lips, glacis and grilled deck as the rest of the line, under the warden\'s clean hex turret and a heavy gun.' },
  { chassis: 'glacier', note: '“rebuild glacier in style of vanguard but simpler”',
    what: 'The plough, the bands and the coolant tank are gone: the flak\'s compact hull and the assault\'s box turret with one long gun. The white accent strip across the glacis is the only arctic hint, and it glows cold at night.' },
];

function teamOf(team) {
  return team === 'p3' ? colours.p3 : team === 'p4' ? colours.p4 : team;
}

function buildReview() {
  for (const v of reviewViews) VIEWS.delete(v);
  reviewViews = [];
  buildRebuilt();
  buildColourPick();
  buildColours();
}

function buildRebuilt() {
  const el = document.getElementById('rebuilt-grid');
  el.innerHTML = '';
  rebuiltRefreshers = [];
  for (const it of REBUILT) {
    const before = DESIGNS['prev.' + it.chassis];
    const after = DESIGNS['vanguard.' + it.chassis];
    const card = document.createElement('div');
    card.className = 'rb';
    card.innerHTML = '<div class="rb-head"><b></b><span></span></div><div class="rb-pair"><div><canvas></canvas><div class="rb-cap"></div></div><div><canvas></canvas><div class="rb-cap new"></div></div></div><div class="rb-foot"><p></p><button class="btn ghost pick" aria-pressed="false"></button></div>';
    card.querySelector('b').textContent = `${CH[it.chassis].name} · “${after ? after.codename : ''}”`;
    card.querySelector('.rb-head span').textContent = 'Your note: ' + it.note;
    card.querySelector('.rb-foot p').textContent = it.what;
    const caps = card.querySelectorAll('.rb-cap');
    caps[0].textContent = 'Before';
    caps[1].textContent = 'After';
    const btn = card.querySelector('.rb-foot button');
    const refresh = () => {
      const p = picks[it.chassis];
      const on = !!(p && p.pick === 'vanguard');
      btn.setAttribute('aria-pressed', on ? 'true' : 'false');
      btn.textContent = on ? 'Rebuild picked' : 'Pick the rebuild';
    };
    btn.addEventListener('click', () => { togglePick(it.chassis, 'vanguard'); });
    refresh();
    rebuiltRefreshers.push(refresh);
    el.appendChild(card);
    const cvs = card.querySelectorAll('canvas');
    [before, after].forEach((d, i) => {
      if (!d) return;
      const holder = cvs[i].parentElement;
      const w = Math.max(150, Math.floor(holder.clientWidth));
      const h = Math.round(Math.min(280, w * 0.66));
      const v = makeView(cvs[i], w, h, (v, now) => {
        begin(v, now);
        const k = Math.min(8, st.scale * 2) * dpr();
        ground(v, k, 40 + i * 7 + (it.chassis === 'glacier' ? 20 : 0));
        const t = idle({ design: d, col: 'vanguard', chassis: it.chassis, team: st.team, tier: 0, wreck: null, hull: 0 }, now, 3 + i, { fire: true });
        drawTank(v, t, v.W * 0.5, v.H * 0.56, k);
        finish(v);
      }, { fps: 12 });
      reviewViews.push(v);
    });
  }
}

function buildColourPick() {
  for (const key of ['p3', 'p4']) {
    const seg = document.getElementById('pick-' + key);
    seg.innerHTML = '';
    for (const c of P34) {
      const b = document.createElement('button');
      b.dataset.v = c;
      b.innerHTML = `<span class="swatch" style="background:${TEAM_HEX[c]}"></span> `;
      b.appendChild(document.createTextNode(TEAM_NAME[c]));
      b.setAttribute('aria-pressed', colours[key] === c ? 'true' : 'false');
      b.addEventListener('click', () => setColour(key, c));
      seg.appendChild(b);
    }
  }
}

function setColour(key, c) {
  const other = key === 'p3' ? 'p4' : 'p3';
  if (colours[other] === c) colours[other] = colours[key];
  colours[key] = c;
  buildColourPick();
  updateColourHeads();
  writeColours();
  if (F.view) F.view.dirty = true;
}

function buildColours() {
  const el = document.getElementById('colours-grid');
  el.innerHTML = '';
  const cw = (S + 6) * st.scale;
  el.style.gridTemplateColumns = `auto ${TEAMS.length * cw}px`;
  const corner = document.createElement('div');
  corner.className = 'hd';
  el.appendChild(corner);
  const head = document.createElement('div');
  head.style.display = 'grid';
  head.style.gridTemplateColumns = `repeat(${TEAMS.length}, ${cw}px)`;
  for (const team of TEAMS) {
    const h = document.createElement('div');
    h.className = 'hd';
    h.dataset.team = team;
    h.innerHTML = `<span class="swatch" style="background:${TEAM_HEX[team]}"></span><span class="nm"></span><span class="tagp"></span>`;
    h.querySelector('.nm').textContent = TEAM_NAME[team];
    head.appendChild(h);
  }
  el.appendChild(head);
  CHASSIS.forEach((ch, row) => {
    const name = document.createElement('div');
    name.className = 'ch';
    name.textContent = ch.name;
    el.appendChild(name);
    const c = document.createElement('canvas');
    el.appendChild(c);
    const v = makeView(c, TEAMS.length * cw, (S + 4) * st.scale, (v, now) => {
      begin(v, now);
      const k = st.scale * dpr();
      ground(v, k, row * 5 + 3);
      TEAMS.forEach((team, i) => {
        const t = idle({ col: 'vanguard', chassis: ch.key, team, tier: 0, wreck: null, hull: 0 }, now, row * 3 + i);
        drawTank(v, t, (i + 0.5) * cw * dpr(), v.H / 2 + k, k);
      });
      finish(v);
    }, { fps: 10 });
    reviewViews.push(v);
  });
  updateColourHeads();
}

function updateColourHeads() {
  for (const h of document.querySelectorAll('#colours-grid .hd[data-team]')) {
    const team = h.dataset.team;
    const tag = team === 'p1' ? 'P1' : team === 'p2' ? 'P2' : colours.p3 === team ? 'P3' : colours.p4 === team ? 'P4' : '';
    const el = h.querySelector('.tagp');
    el.textContent = tag;
    el.hidden = !tag;
  }
}

function setColourStatus(s) {
  const el = document.getElementById('colour-status');
  if (el) el.textContent = s;
}

async function writeColours() {
  try { localStorage.setItem('bb-motorpool-colours', JSON.stringify(colours)); } catch (e) { /* ignore */ }
  if (!db || !dbWritable) {
    setColourStatus(db ? '' : 'Kept in this browser');
    return;
  }
  setColourStatus('Saving…');
  try {
    await db.doc('colors/players').set({ p3: colours.p3, p4: colours.p4, at: new Date().toISOString() });
    setColourStatus('Saved');
  } catch (e) {
    setColourStatus('Could not save just now');
  }
}

function loadLocalColours() {
  try {
    const c = JSON.parse(localStorage.getItem('bb-motorpool-colours') || 'null');
    if (c && P34.includes(c.p3) && P34.includes(c.p4) && c.p3 !== c.p4) Object.assign(colours, c);
  } catch (e) { /* ignore */ }
}

// ---------------------------------------------------------------------------
// The four lines
// ---------------------------------------------------------------------------
let lineViews = [];

function buildLines() {
  for (const v of lineViews) VIEWS.delete(v);
  lineViews = [];
  const el = document.getElementById('lines-list');
  el.innerHTML = '';
  for (const line of LINES) {
    const block = document.createElement('div');
    block.className = 'line-block';
    block.innerHTML = `<div class="lb-head"><span class="swatch" style="background:${COLOR[line.key]}"></span><h3></h3><p></p></div><div class="line-row"></div>`;
    block.querySelector('h3').textContent = line.title;
    block.querySelector('p').textContent = line.notes || line.tagline;
    const row = block.querySelector('.line-row');
    const cs = cellSize();
    el.appendChild(block);
    const avail = row.clientWidth || document.querySelector('.wrap').clientWidth;
    const per = [12, 6, 4, 3, 2].find(n => n * cs <= avail) || 2;
    const colW = Math.floor(avail / per);
    row.style.gridTemplateColumns = `repeat(${per}, ${colW}px)`;
    CHASSIS.forEach((ch, i) => {
      const d = des(line.key, ch.key);
      const lc = document.createElement('div');
      lc.className = 'lc';
      lc.title = `${ch.name} · ${line.title}`;
      lc.innerHTML = '<canvas></canvas><div class="lcap"></div>';
      lc.querySelector('.lcap').textContent = `${ch.name}${d ? ' · ' + d.codename : ''}`;
      lc.addEventListener('click', () => { select(line.key, ch.key, false); document.getElementById('lineup').scrollIntoView({ behavior: 'smooth' }); });
      row.appendChild(lc);
      const seed = i * 5 + LINES.indexOf(line) * 17;
      const v = makeView(lc.querySelector('canvas'), colW - 1, cs, (v, now) => {
        begin(v, now);
        const k = st.scale * dpr();
        ground(v, k, seed);
        const t = idle({ col: line.key, chassis: ch.key, team: st.team, tier: 0, wreck: null, hull: 0 }, now, seed, { fire: true });
        drawTank(v, t, v.W / 2, v.H / 2 + k, k);
        finish(v);
      }, { fps: 12 });
      lineViews.push(v);
    });
  }
}

let resizeTimer = null;
window.addEventListener('resize', () => {
  clearTimeout(resizeTimer);
  resizeTimer = setTimeout(() => { buildLines(); rebuildDependent(); buildReview(); }, 250);
});

// ---------------------------------------------------------------------------
// Field test: tanks patrolling the default map on its cell grid, the way
// the game moves them - four directions, turning in place.
// ---------------------------------------------------------------------------
const F = {
  cols: DATA.field.cols, rows: DATA.field.rows, cell: 32,
  blocked: new Set(DATA.field.blocked), cast: 'picks', zoom: 1, tanks: [], wrecks: [], smoke: [], tracers: [], marks: [], seed: 7, view: null, lastT: 0,
};
const MARK_LIFE = 9;
const SPEED = { light: 74, medium: 64, heavy: 54, super: 44 };
const CLASS_OF = c => CH[c].tier;

function open(c, r) {
  return c >= 0 && r >= 0 && c < F.cols && r < F.rows && !F.blocked.has(r * F.cols + c);
}

function passable(c, r, cls) {
  if (!open(c, r)) return false;
  if (cls === 'super') {
    for (let dc = -1; dc <= 1; dc++) for (let dr = -1; dr <= 1; dr++) if (!open(c + dc, r + dr)) return false;
  } else if (cls === 'heavy') {
    if (!(open(c - 1, r) || open(c + 1, r)) || !(open(c, r - 1) || open(c, r + 1))) return false;
  }
  return true;
}

function rng32(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6D2B79F5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function castCol(chassis) {
  if (F.cast === 'today') return 'today';
  if (F.cast === 'picks') {
    const p = picks[chassis];
    if (p && p.pick) return p.pick;
    return LINE_KEYS[0];
  }
  return F.cast;
}

function setupField() {
  const rnd = rng32(F.seed * 7919 + 11);
  const taken = [];
  const farFrom = (c, r) => taken.every(([a, b]) => Math.abs(a - c) + Math.abs(b - r) >= 4);
  const spot = cls => {
    for (let i = 0; i < 400; i++) {
      const c = 1 + Math.floor(rnd() * (F.cols - 2));
      const r = 1 + Math.floor(rnd() * (F.rows - 2));
      if (passable(c, r, cls) && farFrom(c, r)) { taken.push([c, r]); return [c, r]; }
    }
    for (let i = 0; i < 400; i++) {
      const c = Math.floor(rnd() * F.cols), r = Math.floor(rnd() * F.rows);
      if (open(c, r)) { taken.push([c, r]); return [c, r]; }
    }
    return [2, 2];
  };
  const live = [['assault', 'p1'], ['scout', 'p2'], ['flak', 'p3'], ['warden', 'p4'], ['breaker', 'enemy'], ['longbow', 'enemy'], ['glacier', 'enemy'], ['titan', 'enemy'], ['leviathan', 'enemy']];
  const dead = [['wraith', 'gutted'], ['ravager', 'blown'], ['obelisk', 'cookoff']];
  F.wrecks = dead.map(([chassis, wreck], i) => {
    const [c, r] = spot(CLASS_OF(chassis));
    return { chassis, wreck, team: 'enemy', x: c * 32 + 16, y: r * 32 + 16, hull: [0, 90, 180, 270][Math.floor(rnd() * 4)] + (rnd() - 0.5) * 16, turret: rnd() * 360 };
  });
  F.tanks = live.map(([chassis, team]) => {
    const cls = CLASS_OF(chassis);
    const [c, r] = spot(cls);
    const dir = Math.floor(rnd() * 4);
    return {
      chassis, team, cls, c, r, tc: c, tr: r, x: c * 32 + 16, y: r * 32 + 16, dir, hull: dir * 90, turret: dir * 90,
      dist: 0, frame: 0, cool: 1 + rnd() * 2, fireT: -9, rnd: rng32(Math.floor(rnd() * 1e9)), smokeT: 0,
    };
  });
  F.smoke = [];
  F.tracers = [];
  F.marks = [];
}

const DIRS = [[0, -1], [1, 0], [0, 1], [-1, 0]];

function angDiff(a, b) {
  let d = (b - a) % 360;
  if (d > 180) d -= 360;
  if (d < -180) d += 360;
  return d;
}

function stepField(dt, now) {
  for (const t of F.tanks) {
    const cx = t.tc * 32 + 16, cy = t.tr * 32 + 16;
    const want = t.dir * 90;
    const dh = angDiff(t.hull, want);
    const turn = 260 * dt;
    t.hull += Math.abs(dh) <= turn ? dh : Math.sign(dh) * turn;
    if (Math.abs(angDiff(t.hull, want)) < 6) {
      const sp = SPEED[t.cls] * dt;
      const dx = cx - t.x, dy = cy - t.y;
      const dist = Math.hypot(dx, dy);
      if (dist <= sp) {
        t.x = cx; t.y = cy; t.c = t.tc; t.r = t.tr;
        t.dist += dist;
        // Next cell: keep going mostly, turn sometimes, reverse only when stuck.
        const opts = [];
        for (let d = 0; d < 4; d++) {
          const nc = t.c + DIRS[d][0], nr = t.r + DIRS[d][1];
          if (!passable(nc, nr, t.cls)) continue;
          if (F.tanks.some(o => o !== t && ((o.tc === nc && o.tr === nr) || (o.c === nc && o.r === nr)))) continue;
          if (F.wrecks.some(w => Math.abs(w.x - (nc * 32 + 16)) < 30 && Math.abs(w.y - (nr * 32 + 16)) < 30)) continue;
          const back = d === (t.dir + 2) % 4;
          opts.push([d, d === t.dir ? 5 : back ? 0.15 : 1]);
        }
        if (opts.length) {
          let tot = opts.reduce((a, o) => a + o[1], 0);
          let pick = t.rnd() * tot;
          for (const [d, w] of opts) { pick -= w; if (pick <= 0) { t.dir = d; break; } }
          t.tc = t.c + DIRS[t.dir][0];
          t.tr = t.r + DIRS[t.dir][1];
        }
      } else {
        t.x += dx / dist * sp;
        t.y += dy / dist * sp;
        t.dist += sp;
      }
    }
    t.frame = Math.floor(t.dist / 3) % 4;
    if (t.dist - (t.markAt || 0) >= 6) {
      t.markAt = t.dist;
      const dd = des(castCol(t.chassis), t.chassis);
      F.marks.push({ x: t.x, y: t.y, rot: t.hull, style: dd ? dd.marks : 'tread', fp: CH[t.chassis].fp, age: 0 });
      if (F.marks.length > 700) F.marks.splice(0, F.marks.length - 700);
    }
    // Turret: the nearest opponent in range, else a slow scan.
    let best = null, bd = 290;
    for (const o of F.tanks) {
      if (o === t || (o.team === 'enemy') === (t.team === 'enemy')) continue;
      const d = Math.hypot(o.x - t.x, o.y - t.y);
      if (d < bd) { bd = d; best = o; }
    }
    let aim = best ? Math.atan2(best.x - t.x, -(best.y - t.y)) * 180 / Math.PI : t.hull + Math.sin(now * 0.6 + t.x * 0.01) * 30;
    const da = angDiff(t.turret, aim);
    const tt = 170 * dt;
    t.turret += Math.abs(da) <= tt ? da : Math.sign(da) * tt;
    t.cool -= dt;
    if (best && Math.abs(angDiff(t.turret, aim)) < 5 && t.cool <= 0) {
      t.cool = 1.6 + t.rnd() * 2.2;
      t.fireT = now;
      const ms = muzzles({ col: castCol(t.chassis), chassis: t.chassis });
      ms.forEach((m, i) => {
        const p = rot2(m[0], m[1], t.turret);
        const a = (t.turret - 90) * Math.PI / 180;
        F.tracers.push({ x: t.x + p[0] * 2, y: t.y + p[1] * 2, vx: Math.cos(a) * 420, vy: Math.sin(a) * 420, life: Math.min(0.55, bd / 420), delay: i * 0.12 });
      });
    }
    // Exhaust smoke from Foundry stacks, while moving.
    const d = des(castCol(t.chassis), t.chassis);
    if (d && d.emitters && d.emitters.smoke && st.motion) {
      t.smokeT -= dt;
      if (t.smokeT <= 0) {
        t.smokeT = 0.18 + t.rnd() * 0.2;
        for (const [ex, ey] of d.emitters.smoke) {
          const p = rot2(ex, ey, t.hull);
          F.smoke.push({ x: t.x + p[0] * 2, y: t.y + p[1] * 2, vx: (t.rnd() - 0.5) * 6, vy: -8 - t.rnd() * 6, age: 0, life: 1.2 + t.rnd(), s: 3 + t.rnd() * 2, dark: 0.5 });
        }
      }
    }
  }
  for (const w of F.wrecks) {
    if (w.wreck === 'husk') continue;
    if (Math.random() < dt * (w.wreck === 'gutted' ? 7 : 4)) {
      F.smoke.push({ x: w.x + (Math.random() - 0.5) * 16, y: w.y + (Math.random() - 0.5) * 16, vx: (Math.random() - 0.5) * 5, vy: -10 - Math.random() * 8, age: 0, life: 1.8 + Math.random() * 1.4, s: 4 + Math.random() * 4, dark: 0.35 });
    }
  }
  for (const m of F.marks) m.age += dt;
  F.marks = F.marks.filter(m => m.age < MARK_LIFE);
  for (const p of F.smoke) { p.age += dt; p.x += p.vx * dt; p.y += p.vy * dt; p.vx *= 0.98; }
  F.smoke = F.smoke.filter(p => p.age < p.life);
  for (const tr of F.tracers) {
    if (tr.delay > 0) { tr.delay -= dt; continue; }
    tr.x += tr.vx * dt; tr.y += tr.vy * dt; tr.life -= dt;
  }
  F.tracers = F.tracers.filter(tr => tr.life > 0);
}

function drawField(v, now) {
  const dt = Math.min(0.05, F.lastT ? now - F.lastT : 0.016);
  F.lastT = now;
  if (st.motion) stepField(dt, now);
  begin(v, now);
  const z = F.zoom * dpr();
  const g = v.g;
  const img = A.field[st.ground] || A.field.grass;
  g.drawImage(img, 0, 0, img.width, img.height, 0, 0, v.W, v.H);
  const k = 2 * z;
  drawMarks(g, z, k);
  for (const w of F.wrecks) {
    drawTank(v, { col: castCol(w.chassis), chassis: w.chassis, team: w.team, tier: 3, wreck: w.wreck, hull: w.hull, turret: w.turret, frame: 0, pose: 0 }, w.x * z, w.y * z, k);
  }
  const order = F.tanks.slice().sort((a, b) => a.y - b.y);
  for (const t of order) {
    const col = castCol(t.chassis);
    const tank = { col, chassis: t.chassis, team: teamOf(t.team), tier: 0, wreck: null, hull: t.hull, turret: t.turret, frame: t.frame, pose: 0 };
    firingPose(tank, now - t.fireT);
    drawTank(v, tank, t.x * z, t.y * z, k);
  }
  for (const tr of F.tracers) {
    if (tr.delay > 0) continue;
    const a = Math.atan2(tr.vy, tr.vx);
    for (const ctx of v.lit ? [g, v.eg] : [g]) {
      ctx.save();
      ctx.translate(tr.x * z, tr.y * z);
      ctx.rotate(a);
      ctx.fillStyle = '#eea343';
      ctx.fillRect(-6 * z, -1 * z, 8 * z, 2 * z);
      ctx.fillStyle = '#ffffff';
      ctx.fillRect(0, -1 * z, 2 * z, 2 * z);
      ctx.restore();
    }
    if (v.lit) v.lights.push({ x: tr.x * z, y: tr.y * z, r: 14 * k, c: [1, 0.7, 0.35], a: 0.8 });
  }
  for (const p of F.smoke) {
    const f = p.age / p.life;
    const a = (1 - f) * 0.55;
    const s = Math.round((p.s + f * 6) * z / 2) * 2;
    const shade = Math.round(60 + 80 * p.dark * (1 - f));
    g.fillStyle = `rgba(${shade},${shade},${shade},${a})`;
    g.fillRect(Math.round(p.x * z / 2) * 2 - s / 2, Math.round(p.y * z / 2) * 2 - s / 2, s, s);
  }
  finish(v);
}

// Ground marks: what each locomotion leaves behind, fading out.
function drawMarks(g, z, k) {
  for (const m of F.marks) {
    const f = 1 - m.age / MARK_LIFE;
    g.save();
    g.translate(m.x * z, m.y * z);
    g.rotate(m.rot * Math.PI / 180);
    const [x0, , x1] = m.fp;
    if (m.style === 'wash') {
      g.fillStyle = `rgba(214, 226, 200, ${0.10 * f})`;
      g.fillRect((x0 + 2) * k, -1.5 * k, (x1 - x0 - 3) * k, 3 * k);
    } else if (m.style === 'quad') {
      g.fillStyle = `rgba(30, 25, 22, ${0.30 * f})`;
      for (const cx of [x0 + 2, x1 - 1]) g.fillRect(Math.round((cx - 1) * k), -1 * k, 2 * k, 2 * k);
    } else {
      const heavy = m.style === 'heavy';
      g.fillStyle = `rgba(30, 25, 22, ${(heavy ? 0.42 : 0.32) * f})`;
      const w = heavy ? 3 : 2;
      for (const cx of [x0 + 2.5, x1 - 1.5]) g.fillRect(Math.round((cx - w / 2) * k), -1.5 * k, w * k, 3 * k);
    }
    g.restore();
  }
}

function buildField() {
  const castEl = document.getElementById('field-cast');
  const items = [{ v: 'picks', label: 'Your picks' }].concat(LINES.map(l => ({ v: l.key, label: l.title }))).concat([{ v: 'today', label: 'Today' }]);
  castEl.innerHTML = '';
  for (const it of items) {
    const b = document.createElement('button');
    b.textContent = it.label;
    b.dataset.v = it.v;
    b.setAttribute('aria-pressed', F.cast === it.v ? 'true' : 'false');
    b.addEventListener('click', () => {
      F.cast = it.v;
      for (const x of castEl.children) x.setAttribute('aria-pressed', x.dataset.v === F.cast ? 'true' : 'false');
      F.view.dirty = true;
    });
    castEl.appendChild(b);
  }
  const zoomEl = document.getElementById('field-zoom');
  for (const b of zoomEl.children) {
    b.addEventListener('click', () => {
      F.zoom = parseFloat(b.dataset.v);
      for (const x of zoomEl.children) x.setAttribute('aria-pressed', x === b ? 'true' : 'false');
      sizeView(F.view, Math.round(F.cols * 32 * F.zoom), Math.round(F.rows * 32 * F.zoom));
    });
  }
  document.getElementById('field-reset').addEventListener('click', () => { F.seed += 1; setupField(); F.view.dirty = true; });
  setupField();
  F.view = makeView(document.getElementById('field-canvas'), F.cols * 32, F.rows * 32, drawField, { fps: 30 });
}

// ---------------------------------------------------------------------------
// Density study
// ---------------------------------------------------------------------------
function buildDensity() {
  const el = document.getElementById('density-grid');
  el.innerHTML = '';
  if (!DATA.density) {
    const p = document.createElement('p');
    p.className = 'muted';
    p.textContent = 'The density study is still being drawn.';
    el.appendChild(p);
    return;
  }
  const D = DATA.density;
  document.getElementById('density-lede').textContent = D.lede;
  D.items.forEach((item, i) => {
    const card = document.createElement('div');
    card.className = 'card';
    card.innerHTML = '<canvas></canvas><div class="cap"><b></b><span></span></div>';
    card.querySelector('b').textContent = item.title;
    card.querySelector('span').textContent = item.caption;
    el.appendChild(card);
    const css = Math.max(160, Math.floor(card.clientWidth - 2));
    const a = A.density[item.img];
    const cell = item.cell;
    const view = makeView(card.querySelector('canvas'), css, Math.round(css * 0.62), (v, now) => {
      begin(v, now);
      const f = 3 * dpr();              // device px per field px
      ground(v, 2 * f, 91 + i);
      const k = f * 80 / cell;          // a 40 cell is 2 field px a pixel, an 80 cell 1
      const x = v.W / 2, y = v.H / 2 + 2 * f;
      const frame = st.motion ? Math.floor(now * 7) % 4 : 0;
      const tur = st.motion ? Math.sin(now * 0.55) * 22 : 0;
      const layers = [[frame * cell, 0], [4 * cell, tur]];
      const g = v.g;
      g.save();
      g.globalAlpha = 0.486;
      for (const [sx, rot] of layers) blit(g, a.sh, sx, 0, cell, x + 0.595 * 3 * f, y + 0.48 * 3 * f, rot, k);
      g.restore();
      for (const [sx, rot] of layers) blit(g, a.img, sx, 0, cell, x, y, rot, k);
      finish(v);
    }, { fps: 12 });
    depViews.push(view);
  });
}

// ---------------------------------------------------------------------------
// Picks: a db document per chassis, read back by Claude.
// ---------------------------------------------------------------------------
const picks = {};
let db = null;
let dbWritable = true;
const saving = {};

function loadLocalPicks() {
  try {
    const raw = JSON.parse(localStorage.getItem('bb-motorpool-picks') || 'null');
    if (raw) Object.assign(picks, raw);
  } catch (e) { /* ignore */ }
}

function saveLocalPicks() {
  try { localStorage.setItem('bb-motorpool-picks', JSON.stringify(picks)); } catch (e) { /* ignore */ }
}

function setStatus(s) {
  document.getElementById('save-status').textContent = s;
}

async function writePick(chassis) {
  saveLocalPicks();
  if (!db || !dbWritable) return;
  const p = picks[chassis] || {};
  const body = { pick: p.pick || null, stars: p.stars || [], note: p.note || '', at: new Date().toISOString() };
  if (saving[chassis]) { saving[chassis].again = true; return; }
  saving[chassis] = { again: false };
  setStatus('Saving…');
  try {
    await db.collection('picks').doc(chassis).set(body);
    setStatus('Saved');
  } catch (e) {
    if (e && e.code === 'invalid_argument') {
      dbWritable = false;
      setStatus('This view can read picks but not change them. Copy picks to keep yours.');
    } else {
      setStatus('Could not save just now; your pick is kept on this page.');
    }
  }
  const again = saving[chassis].again;
  delete saving[chassis];
  if (again) writePick(chassis);
}

function togglePick(chassis, col) {
  const p = picks[chassis] || (picks[chassis] = { pick: null, stars: [], note: '' });
  p.pick = p.pick === col ? null : col;
  for (const r of rebuiltRefreshers) r();
  refreshMatrixMarks();
  refreshInspector();
  renderPicks();
  writePick(chassis);
  if (F.view) F.view.dirty = true;
}

function pickAll(col) {
  for (const ch of CHASSIS) {
    const p = picks[ch.key] || (picks[ch.key] = { pick: null, stars: [], note: '' });
    p.pick = col;
    writePick(ch.key);
  }
  refreshMatrixMarks();
  refreshInspector();
  renderPicks();
}

function toggleStar(chassis, col) {
  const p = picks[chassis] || (picks[chassis] = { pick: null, stars: [], note: '' });
  const s = new Set(p.stars || []);
  if (s.has(col)) s.delete(col); else s.add(col);
  p.stars = Array.from(s);
  renderPicks();
  writePick(chassis);
}

const noteTimers = {};
function renderPicks() {
  const tbl = document.getElementById('picks-table');
  const focused = document.activeElement && document.activeElement.dataset ? document.activeElement.dataset.chassis : null;
  if (focused && tbl.contains(document.activeElement)) {
    updatePickButtons();
    return;
  }
  tbl.innerHTML = '<thead><tr><th>Chassis</th><th>Pick</th><th>Also like</th><th>Note</th></tr></thead>';
  const tb = document.createElement('tbody');
  for (const ch of CHASSIS) {
    const p = picks[ch.key] || {};
    const tr = document.createElement('tr');
    tr.innerHTML = '<td class="cn"></td><td><div class="opt pick-opts"></div></td><td><div class="opt star-opts"></div></td><td><input type="text"></td>';
    tr.querySelector('.cn').textContent = ch.name;
    const po = tr.querySelector('.pick-opts');
    for (const c of COLS) {
      const b = document.createElement('button');
      b.textContent = c.title;
      b.dataset.col = c.key;
      b.dataset.chassis = ch.key;
      b.className = 'pk';
      b.setAttribute('aria-pressed', p.pick === c.key ? 'true' : 'false');
      b.addEventListener('click', () => togglePick(ch.key, c.key));
      po.appendChild(b);
    }
    const so = tr.querySelector('.star-opts');
    for (const c of LINES) {
      const b = document.createElement('button');
      b.className = 'star';
      b.textContent = '★ ' + c.title;
      b.dataset.col = c.key;
      b.dataset.chassis = ch.key;
      b.setAttribute('aria-pressed', (p.stars || []).includes(c.key) ? 'true' : 'false');
      b.addEventListener('click', () => toggleStar(ch.key, c.key));
      so.appendChild(b);
    }
    const inp = tr.querySelector('input');
    inp.id = 'note-' + ch.key;
    inp.dataset.chassis = ch.key;
    inp.placeholder = 'What to change…';
    inp.value = p.note || '';
    inp.setAttribute('aria-label', `Note for ${ch.name}`);
    inp.addEventListener('input', () => {
      const q = picks[ch.key] || (picks[ch.key] = { pick: null, stars: [], note: '' });
      q.note = inp.value;
      clearTimeout(noteTimers[ch.key]);
      noteTimers[ch.key] = setTimeout(() => writePick(ch.key), 700);
    });
    tb.appendChild(tr);
  }
  tbl.appendChild(tb);
  updateCount();
}

function updatePickButtons() {
  for (const b of document.querySelectorAll('#picks-table button')) {
    const p = picks[b.dataset.chassis] || {};
    if (b.classList.contains('star')) b.setAttribute('aria-pressed', (p.stars || []).includes(b.dataset.col) ? 'true' : 'false');
    else b.setAttribute('aria-pressed', p.pick === b.dataset.col ? 'true' : 'false');
  }
  updateCount();
}

function updateCount() {
  const n = CHASSIS.filter(c => picks[c.key] && picks[c.key].pick).length;
  document.getElementById('picks-count').textContent = `${n} of ${CHASSIS.length} picked`;
}

function picksJSON() {
  const out = { picks: {}, colours: { p1: 'blue', p2: 'pink', p3: colours.p3, p4: colours.p4 }, note: generalNote };
  for (const ch of CHASSIS) {
    const p = picks[ch.key] || {};
    out.picks[ch.key] = { pick: p.pick || null, design: p.pick && p.pick !== 'today' ? (des(p.pick, ch.key) || {}).codename || null : null, stars: p.stars || [], note: p.note || '' };
  }
  return JSON.stringify(out, null, 2);
}

let generalNote = '';
let noteTimer = null;

async function connectDb() {
  loadLocalPicks();
  renderPicks();
  refreshMatrixMarks();
  let use = null;
  try {
    use = window.claude && window.claude.use ? await window.claude.use('db') : null;
  } catch (e) { use = null; }
  db = use;
  if (!db) {
    setStatus('Saving is not available in this view; picks stay in this browser. Use Copy picks to keep them.');
    return;
  }
  setStatus('Connected');
  try {
    db.collection('picks').onSnapshot(snap => {
      for (const doc of snap.docs) {
        const data = doc.data();
        if (!data || saving[doc.id]) continue;
        const local = picks[doc.id] || {};
        const noteFocused = document.activeElement && document.activeElement.id === 'note-' + doc.id;
        picks[doc.id] = { pick: data.pick || null, stars: data.stars || [], note: noteFocused ? local.note : (data.note || '') };
      }
      saveLocalPicks();
      for (const r of rebuiltRefreshers) r();
      refreshMatrixMarks();
      refreshInspector();
      renderPicks();
    }, e => setStatus('Live updates stopped; reload to reconnect.'));
    db.doc('colors/players').onSnapshot(snap => {
      if (!snap.exists) return;
      const c = snap.data() || {};
      if (P34.includes(c.p3) && P34.includes(c.p4) && c.p3 !== c.p4) {
        colours.p3 = c.p3;
        colours.p4 = c.p4;
        buildColourPick();
        updateColourHeads();
        if (F.view) F.view.dirty = true;
      }
    }, () => {});
    db.doc('notes/general').onSnapshot(snap => {
      if (!snap.exists) return;
      const ta = document.getElementById('gen-note');
      if (document.activeElement === ta) return;
      generalNote = (snap.data() || {}).text || '';
      ta.value = generalNote;
    }, () => {});
  } catch (e) {
    setStatus('Could not open the picks store; picks stay in this browser.');
  }
}

function wireGeneralNote() {
  const ta = document.getElementById('gen-note');
  try { ta.value = generalNote = localStorage.getItem('bb-motorpool-note') || ''; } catch (e) { /* ignore */ }
  ta.addEventListener('input', () => {
    generalNote = ta.value;
    try { localStorage.setItem('bb-motorpool-note', generalNote); } catch (e) { /* ignore */ }
    clearTimeout(noteTimer);
    noteTimer = setTimeout(async () => {
      if (!db || !dbWritable) return;
      setStatus('Saving…');
      try {
        await db.doc('notes/general').set({ text: generalNote, at: new Date().toISOString() });
        setStatus('Saved');
      } catch (e) {
        setStatus('Could not save the note just now.');
      }
    }, 800);
  });
  document.getElementById('copy-picks').addEventListener('click', async () => {
    const text = picksJSON();
    try {
      await navigator.clipboard.writeText(text);
      setStatus('Picks copied');
    } catch (e) {
      const pre = document.createElement('textarea');
      pre.value = text;
      pre.style.width = '100%';
      pre.style.minHeight = '160px';
      document.getElementById('picks').appendChild(pre);
      pre.select();
      setStatus('Select all and copy from the box below.');
    }
  });
}

// ---------------------------------------------------------------------------
// Toolbar
// ---------------------------------------------------------------------------
function wireToolbar() {
  for (const seg of document.querySelectorAll('.toolbar .seg')) {
    const key = seg.dataset.ctl;
    const cur = () => key === 'scale' ? String(st.scale) : key === 'motion' ? (st.motion ? 'on' : 'off') : st[key];
    for (const b of seg.children) b.setAttribute('aria-pressed', b.dataset.v === cur() ? 'true' : 'false');
    seg.addEventListener('click', e => {
      const b = e.target.closest('button');
      if (!b) return;
      const val = b.dataset.v;
      if (key === 'scale') st.scale = parseInt(val, 10);
      else if (key === 'motion') st.motion = val === 'on';
      else st[key] = val;
      for (const x of seg.children) x.setAttribute('aria-pressed', x === b ? 'true' : 'false');
      saveView();
      if (key === 'scale') {
        buildReview();
        buildMatrix();
        buildLines();
        rebuildDependent();
        buildDensity();
      }
      dirtyAll();
    });
  }
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------
async function boot() {
  buildLegend();
  buildLightLegend();
  wireToolbar();
  await loadAssets();
  loadLocalColours();
  buildReview();
  buildMatrix();
  buildInspector();
  buildField();
  buildLines();
  rebuildDependent();
  buildDensity();
  wireGeneralNote();
  connectDb();
  requestAnimationFrame(loop);
}

boot().catch(err => {
  console.error(err);
  const p = document.createElement('p');
  p.className = 'note';
  p.textContent = 'The sprites failed to load: ' + (err && err.message ? err.message : err);
  document.querySelector('.wrap').prepend(p);
});
})();
