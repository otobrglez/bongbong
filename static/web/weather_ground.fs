#version 100
// GLSL ES 100 port of ../weather_ground.fs for the web build and the phones (raylib's
// OpenGL ES 2 path only understands GLSL ES 100 - see CLAUDE.md's web build
// section). Keep this in sync with ../weather_ground.fs: same logic, only the syntax
// differs (varying vs in/out, texture2D vs texture, gl_FragColor vs
// finalColor). High precision where the GPU has it: the weather's noise
// hashes positions and the round clock, which a 16-bit float cannot hold.

#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

// The weather's ground pass (docs/weather.md, render/weather.rs): the bare
// ground - the tileset alone, before any tread mark, scorch, tile or tank
// is drawn over it - with the sky's mark on it: snow cover and ice, or wet
// earth, puddles on the road, splashes and rings on the water. Everything
// drawn after it stands on it, so a tank leaves dark tracks in the snow
// and a wall stays dry. Water is the cell mask's word and the pixel's own
// blue together, so a shore's grass in a water cell is ground.

varying vec2 fragTexCoord;
varying vec4 fragColor;

uniform sampler2D texture0;   // the bare ground
uniform sampler2D cellMask;   // per map cell: r water (0 dry, 0.5 ford, 1 deep or ice), g road
uniform float frozen;         // 1 when the rules froze the water: the ice is solid
uniform vec2 viewOrigin;      // the world px at the target's top-left corner
uniform vec2 viewSize;        // the target, px: a texel per world px
uniform vec2 cells;           // the mask's size, in cells
uniform vec2 cellOrigin;      // the world cell the mask's first texel is: 0 for a field's own
uniform float time;           // round seconds
uniform float rain;           // how hard it rains
uniform float splashRate;     // how often drops splash
uniform float snow;           // how much of the ground is covered, 0..1

// Hashes of wrapped lattice points: every input stays small, so the
// pattern holds up at any clock and on a 16-bit float.
float hash(vec2 p) {
    vec3 p3 = fract(mod(p.xyx, 289.0) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

float vnoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash(i);
    float b = hash(i + vec2(1.0, 0.0));
    float c = hash(i + vec2(0.0, 1.0));
    float d = hash(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

float fbm(vec2 p) {
    float s = 0.0;
    float a = 0.5;
    for (int i = 0; i < 4; i++) {
        s += a * vnoise(p);
        p = p * 2.03 + vec2(17.1, 9.3);
        a *= 0.5;
    }
    return s;
}

float bayer(vec2 blk) {
    vec2 m = mod(blk, 2.0);
    return (mod(2.0 * m.x + 3.0 * m.y, 4.0) + 0.5) / 4.0;
}

// `x` in `levels` steps, each step edge dithered by the 2x2 Bayer pattern
// across the middle third of the step either side of it, so a gradient
// reads as drawn bands with pixel-art edges rather than as a halftone.
// `hard` steps without the dither.
float band(float x, float levels, vec2 blk, bool hard) {
    float s = x * levels;
    float i = floor(s);
    float f = clamp((s - i - 0.5) * 3.0 + 0.5, 0.0, 1.0);
    return (i + step(hard ? 0.5 : bayer(blk), f)) / levels;
}

float lum(vec3 c) {
    return dot(c, vec3(0.299, 0.587, 0.114));
}

// The map cell under field pixel p: cell (c, r) is centred on (c, r) * 32,
// and the mask starts at cell `cellOrigin`.
vec4 cellAt(vec2 p) {
    return texture2D(cellMask, (floor((p + 16.0) / 32.0) - cellOrigin + 0.5) / cells);
}

// One expanding ring per `g` px cell now and then: a drop's splash on the
// ground, its rings on water. `chance` of the cell's turns ring at all;
// each lasts `life` seconds and grows to `reach` px.
vec3 drops(vec3 col, vec2 pb, float g, float chance, float life, float reach, float alpha) {
    vec2 cell = floor(pb / g);
    float period = (0.5 + hash(cell * 1.7 + 5.0) * 0.6) / max(splashRate, 0.05);
    float tt = time + hash(cell + 9.0) * period;
    float age = mod(tt, period);
    float turn = floor(tt / period);
    if (age > life || hash(cell + turn * 0.37 + 1.3) > chance) {
        return col;
    }
    vec2 at = cell * g + floor((0.2 + 0.6 * vec2(hash(cell + turn + 2.1), hash(cell + turn + 4.3))) * g / 2.0) * 2.0 + 1.0;
    float k = age / life;
    float radius = 1.0 + k * reach;
    if (abs(length(pb - at) - radius) < 1.2) {
        col = mix(col, vec3(0.84, 0.9, 1.0), (1.0 - k) * alpha);
    }
    return col;
}

void main() {
    // World pixels, y down (the render texture is read flipped).
    vec2 p = viewOrigin + vec2(fragTexCoord.x, 1.0 - fragTexCoord.y) * viewSize;
    vec2 blk = floor(p / 2.0);
    vec2 pb = blk * 2.0 + 1.0;
    vec3 base = texture2D(texture0, fragTexCoord).rgb;
    vec3 col = base;
    vec4 m = cellAt(p);
    bool water = m.r > 0.25 && base.b > base.r + 0.12 && base.b > 0.3;
    bool road = m.g > 0.5;

    if (rain > 0.0) {
        float wet = min(rain, 1.0);
        if (water) {
            col = drops(col, pb, 16.0, min(0.9, 0.55 * rain), 0.45, 8.0, 0.45);
        } else {
            // Wet earth reads darker and cooler.
            col = mix(col, col * vec3(0.78, 0.83, 0.9), wet);
            if (road && vnoise(pb * 0.07 + 3.1) > 0.56) {
                // A puddle: the grey sky in it, rings where drops land,
                // a glint now and then.
                col = mix(col, vec3(0.3, 0.37, 0.5), 0.5 * wet);
                col = drops(col, pb, 10.0, min(0.9, 0.6 * rain), 0.4, 4.0, 0.5);
                if (hash(blk + floor(time * 5.0)) > 0.985) {
                    col += 0.1 * wet;
                }
            }
            col = drops(col, pb, 22.0, min(0.9, 0.3 * rain), 0.2, 4.0, 0.6);
        }
    }

    if (snow > 0.0) {
        float s = clamp(snow, 0.0, 1.0);
        if (water) {
            // Ice: pale and bluish, with the odd crack and glint.
            vec3 ice = vec3(0.72, 0.86, 0.95) + 0.05 * (hash(blk) - 0.5);
            float crack = 1.0 - smoothstep(0.0, 0.025, abs(vnoise(pb * 0.045 + 7.0) - 0.5));
            ice = mix(ice, vec3(0.58, 0.72, 0.84), crack * 0.6);
            ice = mix(ice, vec3(0.94, 0.98, 1.0), step(0.985, hash(floor(p / vec2(6.0, 2.0)))) * 0.6);
            // Frozen by the rules, the ice is whole: nothing of the water
            // moving under it shows, since a hull drives on it.
            col = mix(col, ice, frozen > 0.5 ? 1.0 : min(0.9, s * 1.1));
        } else {
            // Snow lies where the noise is low enough for this much of it,
            // in three steps with dithered edges; the ground's own pattern shows
            // through as relief.
            float n = fbm(pb * 0.035);
            float cover = smoothstep(0.62 - 0.5 * s, 0.8 - 0.5 * s, n);
            cover = band(cover, 3.0, blk, false);
            if (road) {
                cover *= 0.8;
            }
            vec3 flake = vec3(0.92, 0.95, 1.0) * (0.86 + 0.23 * lum(base));
            col = mix(col, flake, cover * 0.86);
        }
    }

    gl_FragColor = vec4(col, 1.0);
}
