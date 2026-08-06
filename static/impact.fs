#version 330

in vec2 fragTexCoord;
out vec4 finalColor;

uniform sampler2D texture0;   // the rendered scene
uniform vec2 center;          // hit point, in 0..1 UV coords
uniform float time;           // seconds since the impact started
uniform vec2 resolution;      // screen size, to keep the pulse round

uniform float speed;          // pulse growth, UV units/sec
uniform float width;          // thickness of the distorted band, UV units
uniform float strength;       // how hard the pulse bends the image, UV units
uniform float duration;       // seconds the effect plays before fully fading

// A shell impact: a single sharp outward punch (not the death shockwave's
// push/pull wobble) plus a bright, fast-decaying spark flash at the hit
// point, so a hit reads as a quick "thwack" distinct from the rolling kill
// ring and the muzzle's soft heat-shimmer.
void main() {
    vec2 toPixel = fragTexCoord - center;

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
        uv -= normalize(toPixel) * amount;
    }
    vec4 color = texture(texture0, uv);

    // Bright warm spark at the impact point, decaying faster than the pulse
    // itself so it reads as an instantaneous flash rather than a glow. Mixed
    // toward the spark color (not added) so it stays visible against a
    // bright background too - additive alone all but disappears against the
    // near-white RAYWHITE battlefield floor (a shell that misses every tank
    // and hits the boundary wall lands on open ground, not a dark tank
    // sprite), since white plus more brightness just clips back to white.
    float flashRadius = width * 3.0;
    float flash = smoothstep(flashRadius, 0.0, dist) * fade * fade;
    color.rgb = mix(color.rgb, vec3(1.0, 0.55, 0.15), flash);

    finalColor = color;
}
