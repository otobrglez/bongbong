#version 330

// The weather's sky pass (docs/weather.md, render/weather.rs): the air over
// the finished field - over the tanks, the shots and the fireballs - drawn
// on the 2 px block grid: heat haze bending whole rows, drifting fog banks
// and blowing sand in steps with dithered edges - a gust's wall of it sweeping
// the field -, rain streaks slanting down, snow in three depths, the white of
// a lightning strike. Fog, sand, rain and snow
// are lit by the light map, so at night a headlight's beam shows in the
// fog and the rain glitters where a fire burns.

in vec2 fragTexCoord;
in vec4 fragColor;
out vec4 finalColor;

uniform sampler2D texture0;   // the lit field
uniform sampler2D lightMap;   // the light map, halved: 1.0 stored is 2.0 of light
uniform vec2 viewOrigin;      // the world px at the target's top-left corner
uniform vec2 viewSize;        // the target, px: a texel per world px
uniform float time;           // round seconds
uniform float lit;            // 1 when the light map is this frame's
uniform vec3 ambient;         // the light the field is lit by where no lamp is
uniform float rain;
uniform float rainSpeed;      // px per second
uniform float rainSlant;      // x px per px fallen
uniform float fog;
uniform float fogDrift;
uniform float sand;
uniform float sandWind;
uniform float snow;
uniform float haze;
uniform float hazeAmp;        // px
uniform float hazeSpeed;
uniform float flash;          // lightning, 0..1
uniform float clearRadius;    // fog and sand thin out this near a seat; 0 = nowhere
uniform vec2 seats[8];        // every seat's tank, world px
uniform float seatCount;
uniform float gustOn;         // 1 while a sandstorm's gust crosses the field
uniform float gustSince;      // seconds since its front left the field's upwind corner
uniform vec2 gustDir;         // the way it blows (unit)
uniform float gustFront;      // px/s its front crosses at
uniform float gustLen;        // seconds it blows at any one point

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

// How hard the gust blows at p, 0 to 1: `weather::Gust::strength_at`'s
// band, so the wall of sand is drawn exactly where the hulls feel it - a
// fast rise behind the front, a slow dying away.
float gustAt(vec2 p) {
    if (gustOn < 0.5) {
        return 0.0;
    }
    float age = (gustSince - dot(p, gustDir) / gustFront) / gustLen;
    if (age < 0.0 || age >= 1.0) {
        return 0.0;
    }
    return smoothstep(0.0, 0.2, age) * (1.0 - smoothstep(0.35, 1.0, age));
}

// 1 far from every seat's tank, easing down to a quarter beside one.
float sightMask(vec2 p) {
    if (clearRadius <= 0.0) {
        return 1.0;
    }
    float s = 1.0;
    for (int i = 0; i < 8; i++) {
        if (float(i) >= seatCount) {
            break;
        }
        s = min(s, smoothstep(clearRadius * 0.45, clearRadius, length(p - seats[i])));
    }
    return mix(0.25, 1.0, s);
}

// One depth of rain: streaks one block wide in sheared space, where every
// drop stands still and the whole pattern falls.
vec3 rainLayer(vec3 col, vec2 pb, float i, vec3 tint) {
    vec2 cs = vec2(12.0 + i * 6.0, 96.0 + i * 40.0);
    float speed = rainSpeed * (0.8 + i * 0.35);
    vec2 r = vec2(pb.x - pb.y * rainSlant, pb.y - time * speed);
    vec2 cell = floor(r / cs);
    vec2 f = r - cell * cs;
    if (hash(cell + i * 17.0) > rain * (0.55 + 0.25 * i)) {
        return col;
    }
    float xo = floor(hash(cell + 3.1) * (cs.x / 2.0 - 1.0)) * 2.0 + 1.0;
    float yo = hash(cell + 7.7) * cs.y * 0.5;
    float len = 8.0 + i * 7.0;
    float along = f.y - yo;
    if (abs(f.x - xo) < 1.0 && along > 0.0 && along < len) {
        col = mix(col, tint, (0.22 + 0.16 * i) * (0.3 + 0.7 * along / len));
    }
    return col;
}

// One depth of snow: a flake per cell, drifting as it falls.
vec3 flakes(vec3 col, vec2 pb, float i, float amount, vec3 tint) {
    float cs = 26.0 + i * 12.0;
    float speed = 26.0 + i * 22.0;
    vec2 r = vec2(pb.x - time * (8.0 + i * 6.0), pb.y - time * speed);
    vec2 cell = floor(r / cs);
    float h = hash(cell + i * 31.0);
    if (h > amount) {
        return col;
    }
    vec2 c = cell * cs + vec2(hash(cell + 1.1), hash(cell + 2.2)) * (cs - 8.0) + 4.0;
    c.x += sin(time * (1.1 + h) + h * 6.28) * 5.0;
    c = floor(c / 2.0) * 2.0 + 1.0;
    float size = i > 1.5 ? 2.0 : 1.0;
    vec2 d = abs(r - c);
    if (max(d.x, d.y) < size) {
        col = mix(col, tint, 0.65 + 0.12 * i);
    }
    return col;
}

// One depth of blowing grains: short horizontal streaks on the wind.
vec3 grains(vec3 col, vec2 pb, float i, float amount, vec3 tint) {
    vec2 cs = vec2(48.0 + i * 16.0, 10.0 + i * 4.0);
    vec2 r = vec2(pb.x - time * (340.0 + i * 160.0) * sandWind, pb.y + sin(pb.x * 0.01 + time) * 6.0);
    vec2 cell = floor(r / cs);
    vec2 f = r - cell * cs;
    if (hash(cell + i * 7.0) > amount) {
        return col;
    }
    float yo = floor(hash(cell + 1.7) * (cs.y / 2.0)) * 2.0 + 1.0;
    float xo = hash(cell + 5.3) * cs.x * 0.5;
    if (abs(f.y - yo) < 1.0 && f.x > xo && f.x < xo + 4.0 + i * 4.0) {
        col = mix(col, tint, 0.55);
    }
    return col;
}

void main() {
    // World pixels, y down (the render texture is read flipped).
    vec2 p = viewOrigin + vec2(fragTexCoord.x, 1.0 - fragTexCoord.y) * viewSize;
    vec2 blk = floor(p / 2.0);
    vec2 pb = blk * 2.0 + 1.0;

    // Heat haze: rows in 2 px bands shift by whole blocks in a rising wave.
    vec2 uv = fragTexCoord;
    if (haze > 0.0) {
        float n = vnoise(vec2(pb.x * 0.02, pb.y * 0.045 + time * 1.4 * hazeSpeed));
        float w = sin(pb.y * 0.11 - time * 3.6 * hazeSpeed + n * 6.28);
        uv.x += floor(w * haze * hazeAmp * 0.5 + 0.5) * 2.0 / viewSize.x;
    }
    vec3 col = texture(texture0, uv).rgb;
    vec3 light = lit > 0.5 ? texture(lightMap, fragTexCoord).rgb * 2.0 : ambient;
    float bright = lum(light);

    if (fog > 0.0) {
        vec2 q = pb * 0.0045 + vec2(time * 0.02, time * 0.006) * fogDrift;
        float n = fbm(q * vec2(1.0, 1.6));
        float a = (smoothstep(0.28, 0.72, n) * 0.7 + 0.22) * fog * sightMask(p);
        a = band(a, 4.0, blk, false);
        vec3 fc = vec3(0.84, 0.87, 0.9) * min(light, vec3(1.25));
        col = mix(col, fc, min(a, 0.9));
    }

    if (sand > 0.0) {
        // A gust is a wall of thicker sand sweeping the field, and it
        // fills the clear ring round every tank as it passes.
        float g = gustAt(pb);
        vec2 q = vec2(pb.x * 0.004 - time * 0.5 * sandWind, pb.y * 0.013);
        float n = fbm(q);
        float n2 = vnoise(q * 3.0 + vec2(-time * 1.4 * sandWind, 0.0));
        float a = (0.3 + 0.5 * smoothstep(0.3, 0.75, n) + 0.15 * n2) * sand * mix(sightMask(p), 1.0, g);
        a += g * (0.26 + 0.18 * n2) * min(sand * 1.4, 1.0);
        a = band(min(a, 0.9), 5.0, blk, false);
        float shade = clamp(bright * 1.1, 0.25, 1.2);
        vec3 sc = vec3(0.84, 0.66, 0.42) * (0.9 + 0.2 * n2) * shade;
        col = mix(col, sc, a);
        for (int i = 0; i < 2; i++) {
            col = grains(col, pb, float(i), sand * (0.6 + 0.9 * g), vec3(0.93, 0.8, 0.58) * shade);
        }
    }

    if (rain > 0.0) {
        vec3 tint = vec3(0.78, 0.85, 0.97) * min(light * 1.2 + 0.05, vec3(1.4));
        for (int i = 0; i < 3; i++) {
            col = rainLayer(col, pb, float(i), tint);
        }
    }

    if (snow > 0.0) {
        vec3 tint = vec3(0.96, 0.98, 1.0) * min(light * 1.1 + 0.1, vec3(1.3));
        for (int i = 0; i < 3; i++) {
            col = flakes(col, pb, float(i), snow * 0.55, tint);
        }
    }

    col += vec3(0.75, 0.82, 1.0) * flash * 0.12;
    finalColor = vec4(col, 1.0);
}
