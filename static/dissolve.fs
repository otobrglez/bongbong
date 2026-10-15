#version 330

in vec2 fragTexCoord;
out vec4 finalColor;

uniform sampler2D texture0;   // the screen the round that ended was kept on
uniform float progress;       // how far the dissolve has gone, 0 to 1
uniform float blockPx;        // a block's side, framebuffer pixels

// The screen a round ended on, dissolving into the next round's
// (render/dissolve.rs): a block of it stays while its 4x4 Bayer threshold
// - pyro::bayer's matrix - is at or over `progress` and is gone after, so
// the new round shows through block by block and the two never blend.
// Drawn opaque: the kept screen's alpha is what translucent draws left in
// it, not a coverage.

// The 2x2 Bayer step of a block's low bits, each 0 or 1: 0 2 / 3 1.
float bayer2(vec2 p) {
    return 2.0 * p.x + 3.0 * p.y - 4.0 * p.x * p.y;
}

void main() {
    vec2 b = floor(gl_FragCoord.xy / blockPx);
    float m = 4.0 * bayer2(mod(b, 2.0)) + bayer2(mod(floor(b / 2.0), 2.0));
    if ((m + 0.5) / 16.0 < progress) discard;
    finalColor = vec4(texture(texture0, fragTexCoord).rgb, 1.0);
}
