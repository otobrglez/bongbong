#version 330

in vec2 fragTexCoord;
out vec4 finalColor;

uniform sampler2D texture0;   // the rendered scene

// Everything that bends the picture, in one pass (render/shockwave.rs):
// the ripples of the blasts, the puff of hot air at every muzzle and the
// punch of every shell that lands. Their displacements are summed and the
// scene is sampled exactly once - blitting once per effect would bend the
// last pass's already-bent output, and cost a sample each - and the sum
// moves the picture by whole 2 px blocks, a block at a time, so a bend
// keeps the field's pixel grid.

// Up to SHOCK_MAX (lib.rs) ripples at once - a chained barrel cascade
// pushes one per link, and a tank dying in the middle of it must not be
// shoved out by them. An unused slot has gain 0. The loop bounds are
// literal constants and every slot is always visited: GLSL ES 100 does
// not allow a uniform loop bound.
uniform vec2 centers[4];
uniform float times[4];
uniform float gains[4];       // per-ripple strength; 0 = slot unused
// Where each ring starts (ripple units) and how fast it runs, as a
// multiple of `speed`: 0 and 1 for every ripple that runs outward from its
// centre; a gravity well's snap starts at its reach and runs inward.
uniform float starts[4];
uniform float signs[4];
uniform vec2 resolution;      // the ripple frame (the standard field), to keep the ring round

// The scene target in the ripple frame (`Camera::ripple_view`, the
// standard field's units): its texture coordinate t is the ripple point
// viewUv + t * viewUvSize, viewUv standing at the target's bottom-left
// corner, where every centre is measured from too - so a ripple is the
// same size in world pixels on every map, and the numbers stay small on a
// large one.
uniform vec2 viewUv;
uniform vec2 viewUvSize;
uniform vec2 targetSize;      // the scene target, texels (a texel is a world pixel)

uniform float speed;          // ring growth, ripple units/sec
uniform float width;          // thickness of the distorted band, ripple units
uniform float strength;       // how hard the ring bends the image, ripple units
uniform float duration;       // seconds the effect plays before fully fading

// Up to FLASH_MAX (render/shockwave.rs) muzzle puffs and as many impact
// punches. A slot's time is the seconds since it started; one at or past
// its duration adds nothing, which is how an unused slot is said.
uniform vec2 puffs[4];
uniform float puffTimes[4];
uniform vec4 puff;            // a puff's speed, width, strength and duration
uniform vec2 punches[4];
uniform float punchTimes[4];
uniform vec4 punch;           // a punch's speed, width, strength and duration

// How far `toPixel` lies from the centre it is measured from,
// aspect-corrected so a ring is a circle, not an ellipse.
float reach(vec2 toPixel) {
    vec2 corrected = toPixel;
    corrected.x *= resolution.x / resolution.y;
    return length(corrected);
}

void main() {
    // The block this fragment is in, and the ripple point at its middle:
    // every fragment of a block bends alike.
    vec2 texel = fragTexCoord * targetSize;
    vec2 block = (floor(texel / 2.0) * 2.0 + 1.0) / targetSize;
    vec2 rippleUv = viewUv + block * viewUvSize;
    vec2 offset = vec2(0.0);

    for (int i = 0; i < 4; i++) {
        vec2 toPixel = rippleUv - centers[i];
        float dist = reach(toPixel);

        float radius = max(starts[i] + signs[i] * times[i] * speed, 0.0);
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

    // A muzzle's puff: unlike the ripple's symmetric wobble it only pushes
    // ahead of its front (never pulls in behind it) - a single one-sided
    // shove that reads as a hot puff of air rather than a rolling wave -
    // and fades squared, so it snaps off instead of lingering.
    for (int i = 0; i < 4; i++) {
        vec2 toPixel = rippleUv - puffs[i];
        float dist = reach(toPixel);
        float diff = dist - puffTimes[i] * puff.x;
        float front = smoothstep(puff.y, 0.0, max(diff, 0.0));
        float fade = 1.0 - clamp(puffTimes[i] / puff.w, 0.0, 1.0);
        if (dist > 0.0001) {
            offset -= normalize(toPixel) * front * puff.z * fade * fade;
        }
    }

    // A shell's impact: one thin band punched outward, with no wobble -
    // a quick "thwack", sharper than the rolling ring of a kill.
    for (int i = 0; i < 4; i++) {
        vec2 toPixel = rippleUv - punches[i];
        float dist = reach(toPixel);
        float diff = dist - punchTimes[i] * punch.x;
        float band = smoothstep(punch.y, 0.0, abs(diff));
        float fade = 1.0 - clamp(punchTimes[i] / punch.w, 0.0, 1.0);
        if (dist > 0.0001) {
            offset -= normalize(toPixel) * band * punch.z * fade;
        }
    }

    // The offset is in ripple units and the target spans viewUvSize of
    // them: in texels, to the nearest whole block.
    vec2 moved = floor(offset / viewUvSize * targetSize / 2.0 + 0.5) * 2.0;
    finalColor = texture(texture0, fragTexCoord + moved / targetSize);
}
