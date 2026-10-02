#version 100

#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

// The flamethrower's stream as a jet of burning liquid fuel, drawn on one
// quad laid from the nozzle down the stream (render/shot_shaders.rs), in
// the effects language (docs/effects.md): the stream is worked out once
// per 2 px block of the field - found from the fragment's place on the
// field, not the quad's corners, so it sits on the field's grid whatever
// way the stream points - and coloured in the fire ramp's flat steps, its
// ragged edge dithered through the same 4x4 Bayer pattern as every other
// dissolve. In the stream's own frame `s` runs 0 at the nozzle to 1 at the
// reach and `x` runs -1..1 across the cone's full width at the far end.
//
// Near the nozzle the fuel is a tight, bright rope; it whips gently (a
// wave travelling outward), carries slugs of fuel down its length (width
// pulses moving outward), and only past about the middle does it break up
// and bloom into rolling fire that cools from white through gold and
// orange to red, with a ragged edge at the tip.

varying vec2 fragTexCoord;
varying vec4 fragColor;

uniform float time;
uniform float seed;
uniform float reach;       // stream length, px (the bloom and the slug spacing are in px)
uniform vec2 origin;       // the nozzle, world px
uniform vec2 dir;          // down the stream, a unit vector
uniform float halfWidth;   // the quad's half width, px
uniform vec2 viewOrigin;   // the world px at the render target's top-left corner
uniform float viewHeight;  // the render target's height, px (a texel per world px): its y is flipped

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
        p = p * 2.07 + vec3(3.1, 1.7, 7.3);
        a *= 0.5;
    }
    return v;
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

// The fire ramp (`pyro::FIRE`), one flat step per band of heat.
vec3 fire(float h) {
    if (h > 0.78) return vec3(1.0, 1.0, 1.0);
    if (h > 0.62) return vec3(1.0, 0.886, 0.627);
    if (h > 0.46) return vec3(0.933, 0.639, 0.263);
    if (h > 0.32) return vec3(0.863, 0.612, 0.29);
    if (h > 0.18) return vec3(0.894, 0.259, 0.098);
    if (h > 0.08) return vec3(0.612, 0.208, 0.153);
    return vec3(0.506, 0.184, 0.153);
}

void main() {
    // The block this fragment is in, in world px, and its centre in the
    // stream's frame.
    vec2 world = viewOrigin + vec2(gl_FragCoord.x, viewHeight - gl_FragCoord.y);
    vec2 blk = floor(world / 2.0);
    vec2 d = (blk + 0.5) * 2.0 - origin;
    vec2 across = vec2(-dir.y, dir.x);
    float px = dot(d, dir);
    float s = px / max(reach, 1.0);
    float x = dot(d, across) / max(halfWidth, 1.0);
    if (s < 0.0 || s > 1.0) {
        discard;
    }

    // The rope's half width, as a fraction of the far-end cone half width:
    // thin and steady out to the middle, then blooming.
    float bloom = smoothstep(0.3, 1.0, s);
    float billow = fbm(vec3(s * 3.0 - time * 2.5, seed, time * 0.6));
    float rope = mix(0.05, 0.09, s) + bloom * pow(bloom, 0.3) * (0.3 + 0.3 * billow);
    // Slugs of fuel moving down the rope.
    rope *= 0.85 + 0.2 * sin(px * 0.22 - time * 42.0 + seed);
    // A whip travelling outward along the stream.
    float c = sin(px * 0.045 - time * 16.0 + seed) * 0.06 * s;

    // Rolling fire texture scrolling outward, rougher as it blooms.
    float flow = fbm(vec3(x * 3.0, px * 0.06 - time * 9.0, seed + time * 0.8));
    float dist = abs(x - c) / max(rope, 0.001);
    dist += (flow - 0.5) * (0.25 + 1.1 * bloom);

    float body = smoothstep(1.0, 0.55, dist);
    float core = smoothstep(0.5, 0.0, abs(x - c) / max(rope * 0.45, 0.001)) * (1.0 - smoothstep(0.35, 0.75, s));

    // Heat: hottest in the core near the nozzle, cooling outward and along;
    // the band edges dithered a little so the steps read as drawn.
    float heat = clamp(core * 1.2 + body * (1.0 - 0.9 * s) * (0.55 + 0.6 * flow), 0.0, 1.0);
    float threshold = bayer4(blk);
    heat += (threshold - 0.5) * 0.1;

    // Coverage: the rope is solid; the bloom thins at its ragged edge and
    // the whole stream fades out over its last stretch - kept or dropped
    // per block through the Bayer pattern, never translucent.
    float a = max(body, core);
    a *= 1.0 - smoothstep(0.78, 1.0, s + (flow - 0.5) * 0.3);
    a *= smoothstep(0.0, 0.03, s);
    if (a <= threshold) {
        discard;
    }
    gl_FragColor = vec4(fire(heat), fragColor.a);
}
