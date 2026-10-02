#version 100
// GLSL ES 100 port of ../muzzle_flash.fs for the web build. See
// shockwave.fs in this directory for why this file exists; keep both in
// sync with their ../*.fs counterparts.

precision mediump float;

varying vec2 fragTexCoord;

uniform sampler2D texture0;   // the rendered scene
uniform vec2 center;          // hit point, in the ripple frame
uniform float time;           // seconds since the flash started
uniform vec2 resolution;      // the ripple frame (the standard field), to keep the puff round

// The scene target in the ripple frame (`Camera::ripple_view`, the
// standard field's units): its texture coordinate t is the ripple point
// viewUv + t * viewUvSize, viewUv standing at the target's bottom-left
// corner, where every centre is measured from too - so a ripple is the
// same size in world pixels on every map, and the numbers stay small on a
// large one.
uniform vec2 viewUv;
uniform vec2 viewUvSize;

uniform float speed;          // front growth, ripple units/sec
uniform float width;          // thickness of the pushed band, ripple units
uniform float strength;       // how hard the puff shoves the image, ripple units
uniform float duration;       // seconds the effect plays before fully fading

void main() {
    vec2 toPixel = viewUv + fragTexCoord * viewUvSize - center;

    // aspect-correct so the puff is round, not an ellipse
    vec2 corrected = toPixel;
    corrected.x *= resolution.x / resolution.y;
    float dist = length(corrected);

    float radius = time * speed;
    float diff = dist - radius;      // where am I relative to the front?

    // Unlike the kill shockwave's symmetric wobble, this only pushes ahead
    // of the front (never pulls in behind it) - a single one-sided shove
    // that reads as a hot puff of air rather than a rolling wave.
    float front = smoothstep(width, 0.0, max(diff, 0.0));
    float amount = front * strength;

    // Fade fast and non-linearly (squared) so the flash snaps off instead
    // of lingering - a split-second punch rather than a slow-decaying blast.
    float fade = 1.0 - clamp(time / duration, 0.0, 1.0);
    amount *= fade * fade;

    vec2 uv = fragTexCoord;
    if (dist > 0.0001) {
        uv -= normalize(toPixel) * amount / viewUvSize;
    }
    gl_FragColor = texture2D(texture0, uv);
}
