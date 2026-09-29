#version 100

#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

// One hit, drawn on a square quad centred on the impact point
// (render/shot_shaders.rs, the impacts `fx.rs` keeps). `t` runs 0..1 over
// the hit's life; `p` below is the quad in units of its half width, and
// `dir` is the way the shot was travelling, so debris and spray can fly
// back toward the shooter the way a real hit splashes. Six looks:
//   0 shell  - a white flash, a noisy fireball that cools through yellow,
//              orange and red into smoke, a thin shock ring, and hot
//              debris streaks thrown back and to the sides;
//   1 bullet - a sharp four-point star, a few ricochet sparks in a cone
//              back toward the gun, a puff of dust;
//   2 plasma, electric (teal) - a burst of light, an expanding ring with
//              a crackling edge, lightning forking out from the centre;
//   3 plasma, arcane (purple) - the same ring, but a vortex collapsing
//              inward through it and stars left twinkling;
//   4 laser  - a molten splash: a white-hot core, droplets thrown back
//              along the beam, a heat bloom in the beam's colour;
//   5 ooze   - a bio slush glob bursting flat: a lumpy blob that spreads
//              and settles with a wet rim and a lit sheen, droplets thrown
//              all round, a thin splash ring. No white-hot core - it is a
//              liquid, not a fire.
// cA/cB are the shot's own colours for the energy looks and the ooze.

varying vec2 fragTexCoord;
varying vec4 fragColor;

uniform float t;
uniform float style;
uniform vec2 dir;
uniform float seed;
uniform vec3 cA;   // bright colour
uniform vec3 cB;   // body colour

float hash1(float n) { return fract(sin(n * 127.1 + seed * 311.7) * 43758.5453); }

float hash(vec3 p) {
    p = fract(p * 0.3183099 + vec3(0.71, 0.113, 0.419));
    p *= 17.0;
    return fract(p.x * p.y * p.z * (p.x + p.y + p.z));
}

float noise(vec3 x) {
    vec3 i = floor(x);
    vec3 f = fract(x);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(mix(hash(i + vec3(0, 0, 0)), hash(i + vec3(1, 0, 0)), f.x),
                   mix(hash(i + vec3(0, 1, 0)), hash(i + vec3(1, 1, 0)), f.x), f.y),
               mix(mix(hash(i + vec3(0, 0, 1)), hash(i + vec3(1, 0, 1)), f.x),
                   mix(hash(i + vec3(0, 1, 1)), hash(i + vec3(1, 1, 1)), f.x), f.y), f.z);
}

float fbm(vec3 p) {
    float v = 0.0;
    float a = 0.5;
    for (int i = 0; i < 4; i++) {
        v += a * noise(p);
        p = p * 2.02 + vec3(5.1, 1.3, 7.7);
        a *= 0.5;
    }
    return v;
}

float easeOut(float x) {
    x = clamp(x, 0.0, 1.0);
    return 1.0 - (1.0 - x) * (1.0 - x) * (1.0 - x);
}

// Distance from p to the segment a-b.
float seg(vec2 p, vec2 a, vec2 b) {
    vec2 pa = p - a;
    vec2 ba = b - a;
    float h = clamp(dot(pa, ba) / max(dot(ba, ba), 0.00001), 0.0, 1.0);
    return length(pa - ba * h);
}

// Accumulate a layer over what is already there (straight alpha, "over").
void over(inout vec4 acc, vec3 c, float a) {
    a = clamp(a, 0.0, 1.0);
    acc.rgb = mix(acc.rgb, c, a / max(acc.a + a * (1.0 - acc.a), 0.0001) * (1.0 - acc.a) + a * acc.a / max(acc.a + a * (1.0 - acc.a), 0.0001));
    acc.a = acc.a + a * (1.0 - acc.a);
}

// Streaks flying out from the centre: `n` of them, each at its own
// hashed angle - `back` of them biased into a cone round `-dir` - and
// speed, `len` long, thinning and fading as they go.
float streaks(vec2 p, int n, float spread, float back, float reach, float len, float width) {
    float best = 0.0;
    float baseA = atan(-dir.y, -dir.x);
    for (int i = 0; i < 16; i++) {
        if (i >= n) break;
        float fi = float(i);
        float a = hash1(fi) * 6.2831853;
        if (hash1(fi + 40.0) < back) {
            a = baseA + (hash1(fi + 80.0) - 0.5) * 2.0 * spread;
        }
        float speed = 0.55 + 0.45 * hash1(fi + 20.0);
        float head = easeOut(t * 1.2) * reach * speed;
        float tail = max(head - len * (1.0 - t), 0.0);
        vec2 d = vec2(cos(a), sin(a));
        float w = width * (1.0 - t);
        float s = 1.0 - smoothstep(0.0, w, seg(p, d * tail, d * head));
        best = max(best, s);
    }
    return best;
}

void main() {
    vec2 p = (fragTexCoord - 0.5) * 2.0;
    float d = length(p);
    float ang = atan(p.y, p.x);
    vec4 acc = vec4(0.0);

    if (style < 0.5) {
        // --- shell ---
        // Smoke: a dark puff that swells and thins over the second half.
        float sr = 0.3 + 0.45 * easeOut(t);
        float sn = fbm(vec3(p * 2.2, seed + t * 1.5));
        float smoke = smoothstep(sr, sr * 0.55, d + (sn - 0.5) * 0.35) * smoothstep(0.15, 0.45, t) * (1.0 - t) * 0.85;
        over(acc, mix(vec3(0.16, 0.14, 0.13), vec3(0.32, 0.3, 0.28), sn), smoke);
        // Fireball: a noisy ball that grows fast and cools.
        float fr = 0.18 + 0.42 * easeOut(t * 1.6);
        float fn = fbm(vec3(cos(ang) * 1.8, sin(ang) * 1.8, seed + t * 4.0));
        float edge = fr * (0.75 + 0.5 * fn);
        float inside = smoothstep(edge, edge * 0.7, d);
        float heat = clamp((1.0 - d / max(edge, 0.001)) * 1.3 * (1.0 - t * 1.3) + (1.0 - t) * 0.2, 0.0, 1.0);
        vec3 fire = heat > 0.66 ? mix(vec3(1.0, 0.8, 0.3), vec3(1.0, 0.98, 0.85), (heat - 0.66) / 0.34)
                  : heat > 0.33 ? mix(vec3(0.95, 0.35, 0.08), vec3(1.0, 0.8, 0.3), (heat - 0.33) / 0.33)
                  : mix(vec3(0.35, 0.1, 0.06), vec3(0.95, 0.35, 0.08), heat / 0.33);
        over(acc, fire, inside * (1.0 - smoothstep(0.55, 0.85, t)));
        // Shock ring, quick and thin.
        float rr = 0.15 + 0.85 * easeOut(t * 2.2);
        float ring = (1.0 - smoothstep(0.0, 0.035, abs(d - rr))) * (1.0 - clamp(t * 2.2, 0.0, 1.0));
        over(acc, vec3(1.0, 0.92, 0.75), ring * 0.8);
        // Debris, mostly thrown back and to the sides.
        float deb = streaks(p, 12, 1.1, 0.7, 0.95, 0.28, 0.035);
        over(acc, vec3(1.0, 0.78, 0.35), deb);
        // The flash.
        float fl = (1.0 - smoothstep(0.0, 0.12, t)) * smoothstep(0.35, 0.0, d);
        over(acc, vec3(1.0, 1.0, 0.95), fl);
    } else if (style < 1.5) {
        // --- bullet ---
        float puff = smoothstep(0.55, 0.1, d) * smoothstep(0.1, 0.4, t) * (1.0 - t) * 0.5;
        over(acc, vec3(0.55, 0.5, 0.42), puff * (0.6 + 0.4 * fbm(vec3(p * 4.0, seed))));
        float sp = streaks(p, 7, 0.8, 0.85, 1.0, 0.4, 0.07);
        over(acc, vec3(1.0, 0.85, 0.45), sp);
        float k = 1.0 - smoothstep(0.0, 0.45, t);
        float rot = seed * 3.0;
        vec2 q = vec2(cos(rot) * p.x - sin(rot) * p.y, sin(rot) * p.x + cos(rot) * p.y);
        float star = max(1.0 - smoothstep(0.0, 0.06, abs(q.x)) , 1.0 - smoothstep(0.0, 0.06, abs(q.y))) * smoothstep(0.75 * k + 0.05, 0.0, d);
        over(acc, vec3(1.0, 0.95, 0.7), star * k);
        over(acc, vec3(1.0, 1.0, 0.9), smoothstep(0.3 * k + 0.02, 0.0, d) * k);
    } else if (style < 3.5) {
        // --- plasma ---
        bool arcane = style > 2.5;
        float rr = 0.12 + 0.78 * easeOut(t);
        float wobble = (noise(vec3(ang * 3.0, t * 12.0, seed)) - 0.5) * 0.12;
        float band = 1.0 - smoothstep(0.0, 0.07 + 0.05 * (1.0 - t), abs(d - rr - wobble));
        float fill = smoothstep(rr, 0.0, d) * (1.0 - t) * 0.6;
        over(acc, cB, fill);
        if (arcane) {
            // A vortex collapsing inward inside the ring.
            float spiral = sin(ang * 3.0 + log(max(d, 0.001)) * 7.0 + t * 14.0 + seed);
            float v = smoothstep(0.5, 1.0, spiral) * smoothstep(rr, 0.05, d) * (1.0 - t);
            over(acc, cA, v);
            float stars = step(0.975, hash(vec3(floor(p * 9.0), floor(t * 10.0) + seed))) * smoothstep(rr + 0.2, 0.0, d);
            over(acc, vec3(1.0), stars * (1.0 - t));
        } else {
            // Lightning forking from the centre to the ring.
            float bolt = 0.0;
            for (int i = 0; i < 6; i++) {
                float a0 = float(i) / 6.0 * 6.2831853 + seed + floor(t * 20.0) * 0.7;
                float jag = (noise(vec3(d * 9.0, float(i) * 3.1, floor(t * 20.0) + seed)) - 0.5) * 0.9;
                float da = abs(mod(ang - a0 - jag * d + 3.14159, 6.2831853) - 3.14159) * d;
                bolt = max(bolt, (1.0 - smoothstep(0.0, 0.03, da)) * step(d, rr));
            }
            over(acc, mix(cA, vec3(1.0), 0.4), bolt * (1.0 - t));
        }
        over(acc, cA, band * (1.0 - t * 0.7));
        over(acc, mix(cA, vec3(1.0), 0.6), (1.0 - smoothstep(0.0, 0.18, t)) * smoothstep(0.3, 0.0, d));
    } else if (style < 4.5) {
        // --- laser ---
        float glow = smoothstep(0.9, 0.0, d) * (1.0 - t) * 0.55;
        over(acc, cB, glow);
        float drops = streaks(p, 10, 0.9, 0.9, 0.9, 0.18, 0.045);
        over(acc, mix(cA, vec3(1.0, 0.9, 0.6), 0.5), drops);
        float core = smoothstep(0.32 * (1.0 - t) + 0.04, 0.0, d);
        over(acc, vec3(1.0, 1.0, 0.95), core);
    } else {
        // --- ooze ---
        // The blob: a lumpy edge that spreads fast then settles, fading
        // over the last third as the puddle underneath takes over.
        float sr = 0.22 + 0.4 * easeOut(t * 1.4);
        float sn = fbm(vec3(cos(ang) * 2.0, sin(ang) * 2.0, seed));
        float edge = sr * (0.7 + 0.55 * sn);
        float fade = 1.0 - smoothstep(0.6, 1.0, t);
        float blob = smoothstep(edge, edge * 0.8, d) * fade;
        over(acc, cB, blob * 0.85);
        // A wet rim just inside the edge and a sheen toward the light.
        float rim = (1.0 - smoothstep(0.0, 0.05, abs(d - edge * 0.86))) * fade;
        over(acc, cA, rim * 0.7);
        float sheen = smoothstep(edge * 0.75, 0.0, length(p + vec2(0.12, 0.12) * sr)) * fade;
        over(acc, mix(cA, vec3(1.0), 0.3), sheen * 0.5);
        // Droplets thrown all round, and a thin ring of splash.
        float drops = streaks(p, 12, 3.14159, 0.0, 0.95, 0.12, 0.05);
        over(acc, cA, drops);
        float rr = 0.1 + 0.8 * easeOut(t * 2.0);
        float ring = (1.0 - smoothstep(0.0, 0.03, abs(d - rr))) * (1.0 - clamp(t * 2.0, 0.0, 1.0));
        over(acc, mix(cA, vec3(1.0), 0.5), ring * 0.5);
    }

    gl_FragColor = vec4(acc.rgb, acc.a * fragColor.a);
}
