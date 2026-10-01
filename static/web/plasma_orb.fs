#version 100

#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

// A plasma bolt in flight, drawn on one quad that carries the orb, its
// glow and its comet tail (render/shot_shaders.rs). The quad is rotated to
// face travel: in its own space x runs across and y runs back along the
// path, so the tail streams toward +y. Everything is in orb radii.
//
// The sphere is lit from the screen's top left whatever way the bolt flies
// (`rotation` turns the light back into the quad's space), and its surface
// pattern is 3D noise sampled on the sphere's normal after spinning it
// about a tilted axis - that is what makes it read as a turning ball
// rather than a flat disc. Two looks, picked by `style`:
//   0 teal   - electric: ridged veins of light crawling over a deep sea
//              green body, lightning filaments crackling off the rim.
//   1 purple - arcane: a domain-warped nebula swirling over a void-dark
//              core, spiral arms turning outside it, glints like stars.
// `seed` gives every bolt its own pattern; the CPU side also hands each a
// spin speed and axis of its own, so no two look alike.
//
// Drawn in the effects language (docs/effects.md): the bolt is worked out
// once per 2 px block of the field - found from the fragment's place on
// the field, not the quad's corners, so it sits on the field's grid
// whichever way it flies - every block takes the nearest step of the
// bolt's own ramp, and its glow and tail fade in quarters with their band
// edges dithered through the same 4x4 Bayer pattern as every other light.

varying vec2 fragTexCoord;
varying vec4 fragColor;

uniform vec2 orbPos;     // the orb's centre, world px
uniform float radiusPx;  // the orb's radius, px
uniform vec2 viewOrigin; // the world px at the render target's top-left corner
uniform float viewHeight; // the render target's height, px (a texel per world px): its y is flipped
uniform float time;      // round clock, seconds
uniform float spin;      // how far the surface has turned, radians
uniform float tilt;      // the spin axis's lean off vertical, radians
uniform float seed;      // per-bolt pattern offset
uniform float style;     // 0 teal/electric, 1 purple/arcane
uniform float rotation;  // the quad's rotation on screen, radians
uniform float trail;     // tail length, in orb radii
uniform float breathe;   // glow and rim strength, breathing with the pulse
uniform vec3 cDeep;
uniform vec3 cBody;
uniform vec3 cBright;
uniform vec3 cHot;

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
        p = p * 2.03 + vec3(1.7, 9.2, 3.1);
        a *= 0.5;
    }
    return v;
}

vec2 rot2(vec2 v, float a) {
    float c = cos(a);
    float s = sin(a);
    return vec2(c * v.x - s * v.y, s * v.x + c * v.y);
}

// The 2x2 Bayer threshold, and the 4x4 one built from it: the pattern
// `pyro::bayer` dissolves with on the CPU, anchored to the field.
float bayer2(vec2 b) {
    vec2 m = mod(b, 2.0);
    return mod(2.0 * m.x + 3.0 * m.y, 4.0);
}

float bayer4(vec2 b) {
    return (4.0 * bayer2(b) + bayer2(floor(b / 2.0)) + 0.5) / 16.0;
}

// The step of the bolt's ramp - deep, body, bright, hot, white - nearest
// to `c`: never a blend of two.
vec3 onRamp(vec3 c) {
    vec3 best = cDeep;
    float bd = dot(c - cDeep, c - cDeep);
    float d = dot(c - cBody, c - cBody);
    if (d < bd) { bd = d; best = cBody; }
    d = dot(c - cBright, c - cBright);
    if (d < bd) { bd = d; best = cBright; }
    d = dot(c - cHot, c - cHot);
    if (d < bd) { bd = d; best = cHot; }
    d = dot(c - vec3(1.0), c - vec3(1.0));
    if (d < bd) { best = vec3(1.0); }
    return best;
}

void main() {
    // The block this fragment is in, in world px, and its centre in the
    // quad's own space, in orb radii.
    vec2 world = viewOrigin + vec2(gl_FragCoord.x, viewHeight - gl_FragCoord.y);
    vec2 blk = floor(world / 2.0);
    vec2 p = rot2((blk + 0.5) * 2.0 - orbPos, -rotation) / max(radiusPx, 1.0);
    float d = length(p);
    vec3 seedv = vec3(seed, seed * 1.37, seed * 0.71);

    // Accumulate straight colour and coverage for everything outside the
    // ball; the ball itself is opaque and replaces it.
    vec3 glowCol = mix(cBody, cBright, 0.45);

    // Comet tail: a tapering stream behind the orb, rippling and flowing
    // back along the path.
    float ty = p.y;
    float tail = 0.0;
    if (ty > 0.0 && ty < trail) {
        float t = ty / trail;
        float sway = sin(ty * 2.3 - time * 13.0 + seed) * 0.14 * t;
        float w = mix(0.85, 0.05, pow(t, 0.8));
        float x = abs(p.x + sway);
        float flow = fbm(vec3(p.x * 2.5, ty * 1.6 - time * 7.0, seed));
        tail = smoothstep(w, w * 0.15, x) * pow(1.0 - t, 1.4) * (0.55 + 0.75 * flow);
    }

    // Glow round the ball.
    float glow = exp(-max(d - 1.0, 0.0) * 3.0) * 0.7 * breathe * step(1.0, d);

    // What each look adds outside the ball.
    float extra = 0.0;
    vec3 extraCol = cHot;
    float ang = atan(p.y, p.x);
    vec2 sp = rot2(p, rotation);
    if (style < 0.5) {
        // Lightning: ridges of noise in (angle, radius), flickering.
        float n = noise(vec3(ang * 2.2 + seed, d * 3.5 - time * 9.0, floor(time * 18.0) * 0.37 + seed));
        float arc = pow(1.0 - abs(2.0 * n - 1.0), 18.0);
        extra = arc * smoothstep(2.1, 1.05, d) * step(1.0, d) * 1.4;
        extraCol = mix(cBright, cHot, 0.6);
    } else {
        // Spiral arms turning round the ball, in screen space so they
        // turn the same whichever way the bolt flies.
        float sa = atan(sp.y, sp.x);
        float arms = sin(sa * 3.0 - spin * 1.6 + log(max(d, 0.001)) * 5.0 + seed);
        float band = smoothstep(0.55, 1.0, arms) * smoothstep(2.2, 1.1, d) * step(1.0, d);
        float stars = step(0.985, hash(vec3(floor(sp * 7.0), floor(time * 6.0) + seed))) * smoothstep(2.3, 1.2, d);
        extra = band * 0.8 + stars * 1.2;
        extraCol = mix(cBright, cHot, stars);
    }

    float outA = clamp(glow + tail * 0.9 + extra, 0.0, 1.0);
    vec3 outRgb = (glowCol * glow + mix(cBright, cBody, clamp(ty / max(trail, 0.001), 0.0, 1.0)) * tail * 0.9 + extraCol * extra)
                / max(glow + tail * 0.9 + extra, 0.0001);

    // The ball.
    if (d < 1.0) {
        float z = sqrt(1.0 - d * d);
        vec3 n = vec3(p.x, p.y, z);
        // Lighting in screen space: light from the top left and in front.
        vec3 ns = vec3(rot2(n.xy, rotation), n.z);
        vec3 L = normalize(vec3(-0.55, -0.65, 0.55));
        float diff = max(dot(ns, L), 0.0);
        vec3 H = normalize(L + vec3(0.0, 0.0, 1.0));
        float spec = pow(max(dot(ns, H), 0.0), 38.0);
        float rim = pow(1.0 - z, 2.4);

        // Surface point spun about the tilted axis.
        vec3 q = vec3(ns.x, ns.y, ns.z);
        q.yz = rot2(q.yz, tilt);
        q.xz = rot2(q.xz, spin);

        vec3 col;
        if (style < 0.5) {
            float body = fbm(q * 2.1 + seedv);
            float ridge = 1.0 - abs(2.0 * fbm(q * 3.2 + seedv * 2.0 + vec3(0.0, 0.0, time * 0.35)) - 1.0);
            float veins = pow(ridge, 7.0);
            col = mix(cDeep, cBody, smoothstep(0.25, 0.75, body));
            col *= 0.35 + 0.85 * diff;
            col += cBright * veins * 1.3 + cHot * pow(veins, 3.0) * 0.9;
        } else {
            vec3 w = vec3(fbm(q * 1.7 + seedv), fbm(q * 1.7 + seedv + 5.2), fbm(q * 1.7 + seedv + 9.1));
            float neb = fbm(q * 1.9 + w * 2.2 + vec3(0.0, 0.0, time * 0.15));
            col = mix(cDeep * 0.55, cBody, smoothstep(0.3, 0.7, neb));
            col *= 0.4 + 0.8 * diff;
            col += cBright * smoothstep(0.58, 0.78, neb) * 1.1;
            float twinkle = step(0.965, hash(floor(q * 14.0) + seedv)) * (0.5 + 0.5 * sin(time * 9.0 + seed * 7.0));
            col += cHot * twinkle;
        }
        col += cBright * rim * 1.1 * breathe;
        col += cHot * spec * 0.9;
        col += cBright * pow(max(1.0 - d, 0.0), 2.0) * 0.25;

        float edge = smoothstep(1.0, 0.93, d);
        outRgb = mix(outRgb, col, edge);
        outA = mix(outA, 1.0, edge);
    }

    // Coverage in quarters, the band edges dithered; colour in the ramp's
    // flat steps, the band edges dithered too.
    float threshold = bayer4(blk);
    float a = floor(clamp(outA, 0.0, 1.0) * 4.0 + threshold) / 4.0;
    if (a <= 0.0) {
        discard;
    }
    gl_FragColor = vec4(onRamp(min(outRgb, vec3(1.0)) + (threshold - 0.5) * 0.12), a * fragColor.a);
}
