#version 330

// The weather's light pass (docs/weather.md, render/weather.rs): the field
// as pass 1 painted it, under daylight, multiplied by the light map - the
// ambient every pixel starts at plus every headlight, fire, portal, blast
// and shot in the round - on the 2 px block grid the art is drawn on. The
// shots' glows are drawn after this pass, so they stay hot in the dark.

in vec2 fragTexCoord;
in vec4 fragColor;
out vec4 finalColor;

uniform sampler2D texture0;   // the field before its lights
uniform sampler2D lightMap;   // the light map, halved: 1.0 stored is 2.0 of light
uniform vec2 fieldSize;       // the field, px: the sun's gradient spans it
uniform vec2 viewOrigin;      // the world px at the target's top-left corner
uniform vec2 viewSize;        // the target, px: a texel per world px
uniform vec3 ambient;         // the light the map was cleared to
uniform float bands;          // steps per unit of light above the ambient; 0 = smooth
uniform float dither;         // 1 = dither between the steps (2x2 Bayer)
uniform float darkness;       // how dark the ambient is, 0..1
uniform float vignette;       // how much the view darkens toward its edges
uniform vec3 sun;             // the low sun's warmth at the west edge, gone by the east

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

void main() {
    // The target's pixels and the world's, y down (the render texture
    // is read flipped): the steps keep to the world's blocks, the
    // vignette frames the view.
    vec2 vp = vec2(fragTexCoord.x, 1.0 - fragTexCoord.y) * viewSize;
    vec2 p = viewOrigin + vp;
    vec2 blk = floor(p / 2.0);
    vec2 pb = blk * 2.0 + 1.0;
    vec2 vb = pb - viewOrigin;
    vec3 base = texture(texture0, fragTexCoord).rgb;

    vec3 light;
    if (bands > 0.0) {
        // One reading per 2 px block, stepped above the ambient: the dark
        // itself stays flat, only what the lamps add is banded, by its
        // brightness so a lamp keeps its colour from step to step.
        vec3 l = texture(lightMap, vec2(vb.x / viewSize.x, 1.0 - vb.y / viewSize.y)).rgb * 2.0;
        vec3 lamp = max(l - ambient, vec3(0.0));
        float strength = dot(lamp, vec3(0.299, 0.587, 0.114));
        if (strength > 0.001) {
            lamp *= band(strength, bands, blk, dither < 0.5) / strength;
        }
        light = ambient + lamp;
    } else {
        light = texture(lightMap, fragTexCoord).rgb * 2.0;
    }
    // The sun is not a lamp: its gradient spans the field, and it stays
    // smooth rather than stepped.
    light += sun * clamp(1.0 - p.x / fieldSize.x, 0.0, 1.0);

    // Colour fades where it is dark: moonlight leaves a blue-grey field,
    // and a lamp brings the colour back where it falls.
    float lum = dot(light, vec3(0.299, 0.587, 0.114));
    float dim = clamp(1.0 - lum, 0.0, 1.0) * darkness;
    float grey = dot(base, vec3(0.299, 0.587, 0.114));
    vec3 col = mix(base, vec3(grey) * vec3(0.78, 0.88, 1.12), 0.6 * dim);
    col *= light;

    float v = length((vp / viewSize - 0.5) * vec2(1.25, 1.0));
    col *= 1.0 - vignette * smoothstep(0.45, 1.05, v);
    finalColor = vec4(col, 1.0);
}
