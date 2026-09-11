#version 330

in vec2 fragTexCoord;
out vec4 finalColor;

uniform sampler2D texture0;   // the rendered scene
uniform vec2 center;          // hit point, in 0..1 UV coords (unused here)
uniform float time;           // seconds since the shockwave started (unused here)

// Up to SHOCK_MAX (lib.rs) ripples at once - a chained barrel cascade
// pushes one per link, and a tank dying in the middle of it must not be
// shoved out by them. An unused slot has gain 0.
uniform vec2 centers[4];
uniform float times[4];
uniform float gains[4];       // per-ripple strength; 0 = slot unused
uniform vec2 resolution;      // screen size, to keep the ring round

uniform float speed;          // ring growth, UV units/sec
uniform float width;          // thickness of the distorted band, UV units
uniform float strength;       // how hard the ring bends the image, UV units
uniform float duration;       // seconds the effect plays before fully fading

void main() {
    // Accumulate every live ripple's displacement, then sample the scene
    // exactly once. Blitting N times instead would re-distort the previous
    // pass's already-distorted output - artefacts compound - and cost N
    // full-screen samples. `gains[i] == 0.0` is an unused slot; a
    // zero-length loop bound is not portable in GLSL ES 100, so the loop
    // always runs SHOCK_MAX times and an empty slot simply adds nothing.
    vec2 offset = vec2(0.0);
    for (int i = 0; i < 4; i++) {
        vec2 toPixel = fragTexCoord - centers[i];

        // aspect-correct so the ring is a circle, not an ellipse
        vec2 corrected = toPixel;
        corrected.x *= resolution.x / resolution.y;
        float dist = length(corrected);

        float radius = times[i] * speed;
        float diff = dist - radius;      // where am I relative to the ring?

        // strength peaks at the ring, fades on both sides
        float ring = smoothstep(width, 0.0, abs(diff));

        // sine gives the push-in/push-out wobble of a real wave
        float amount = ring * strength * gains[i] * sin(diff / width * 3.14159);

        // fade this ripple out over `duration` seconds
        amount *= 1.0 - clamp(times[i] / duration, 0.0, 1.0);

        if (dist > 0.0001) {
            offset -= normalize(toPixel) * amount;
        }
    }

    finalColor = texture(texture0, fragTexCoord + offset);
}
