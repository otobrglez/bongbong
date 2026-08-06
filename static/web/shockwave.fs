#version 100
// GLSL ES 100 port of ../shockwave.fs for the web build (raylib's
// PLATFORM_WEB defaults to OpenGL ES2, which only understands GLSL ES 100 -
// see CLAUDE.md's web build section). Keep this in sync with ../shockwave.fs:
// same logic, only the syntax differs (varying vs in/out, texture2D vs
// texture, gl_FragColor vs finalColor).

precision mediump float;

varying vec2 fragTexCoord;

uniform sampler2D texture0;   // the rendered scene
uniform vec2 center;          // hit point, in 0..1 UV coords
uniform float time;           // seconds since the shockwave started
uniform vec2 resolution;      // screen size, to keep the ring round

uniform float speed;          // ring growth, UV units/sec
uniform float width;          // thickness of the distorted band, UV units
uniform float strength;       // how hard the ring bends the image, UV units
uniform float duration;       // seconds the effect plays before fully fading

void main() {
    vec2 toPixel = fragTexCoord - center;

    // aspect-correct so the ring is a circle, not an ellipse
    vec2 corrected = toPixel;
    corrected.x *= resolution.x / resolution.y;
    float dist = length(corrected);

    float radius = time * speed;
    float diff = dist - radius;      // where am I relative to the ring?

    // strength peaks at the ring, fades on both sides
    float ring = smoothstep(width, 0.0, abs(diff));

    // sine gives the push-in/push-out wobble of a real wave
    float amount = ring * strength * sin(diff / width * 3.14159);

    // fade the whole thing out over `duration` seconds
    amount *= 1.0 - clamp(time / duration, 0.0, 1.0);

    vec2 uv = fragTexCoord;
    if (dist > 0.0001) {
        uv -= normalize(toPixel) * amount;
    }
    gl_FragColor = texture2D(texture0, uv);
}
