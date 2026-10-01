#version 100
// GLSL ES 100 port of ../impact.fs for the web build. See shockwave.fs in
// this directory for why this file exists; keep both in sync with their
// ../*.fs counterparts.

precision mediump float;

varying vec2 fragTexCoord;

uniform sampler2D texture0;   // the rendered scene
uniform vec2 center;          // hit point, in 0..1 UV coords
uniform float time;           // seconds since the impact started
uniform vec2 resolution;      // the field's size, to keep the pulse round

// The part of the field the scene target holds (`Camera::field_uv`): its
// texture coordinate t is field UV viewUv + t * viewUvSize - (0, 0) and
// (1, 1) for the whole field. A ripple is measured on the field, whatever
// part of it is on screen.
uniform vec2 viewUv;
uniform vec2 viewUvSize;

uniform float speed;          // pulse growth, UV units/sec
uniform float width;          // thickness of the distorted band, UV units
uniform float strength;       // how hard the pulse bends the image, UV units
uniform float duration;       // seconds the effect plays before fully fading

// A shell impact: a single sharp outward punch (not the death shockwave's
// push/pull wobble), so a hit reads as a quick "thwack" distinct from the
// rolling kill ring and the muzzle's soft heat-shimmer. The flash itself is
// the hit's burst, drawn in blocks (burst.rs); this only bends the picture.
void main() {
    vec2 toPixel = viewUv + fragTexCoord * viewUvSize - center;

    // aspect-correct so the pulse is a circle, not an ellipse
    vec2 corrected = toPixel;
    corrected.x *= resolution.x / resolution.y;
    float dist = length(corrected);

    float t = clamp(time / duration, 0.0, 1.0);
    float fade = 1.0 - t;

    float radius = time * speed;
    float diff = dist - radius;

    // strength peaks at the pulse band, fades on both sides
    float band = smoothstep(width, 0.0, abs(diff));

    // one-sided outward punch (no sine wobble) - sharper and more percussive
    // than the death shockwave's push/pull ripple
    float amount = band * strength * fade;

    vec2 uv = fragTexCoord;
    if (dist > 0.0001) {
        uv -= normalize(toPixel) * amount / viewUvSize;
    }
    vec4 color = texture2D(texture0, uv);

    gl_FragColor = color;
}
