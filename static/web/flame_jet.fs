#version 100

#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

// The flamethrower's stream as a jet of burning liquid fuel, drawn on one
// quad laid from the nozzle down the stream (render/shot_shaders.rs). In
// the quad's own space `s` runs 0 at the nozzle to 1 at the reach and `x`
// runs -1..1 across the cone's full width at the far end.
//
// Near the nozzle the fuel is a tight, bright rope; it whips gently (a
// wave travelling outward), carries slugs of fuel down its length (width
// pulses moving outward), and only past about the middle does it break up
// and bloom into rolling fire that cools from white through yellow and
// orange to red, with a smoky ragged edge at the tip.

varying vec2 fragTexCoord;
varying vec4 fragColor;

uniform float time;
uniform float seed;
uniform float reach;    // stream length, px (the bloom and the slug spacing are in px)

float hash(vec3 p) {
    p = fract(p * 0.3183099 + vec3(0.71, 0.113, 0.419));
    p *= 17.0;
    return fract(p.x * p.y * p.z * (p.x + p.y + p.z));
}

float noise(vec3 x) {
    vec3 i = floor(x);
    vec3 f = fract(x);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(mix(hash(i + vec3(0, 0, 0)), hash(i + vec3(1, 0, 0)), f.x),
                   mix(hash(i + vec3(0, 1, 0)), hash(i + vec3(1, 1, 0)), f.x), f.y),
               mix(mix(hash(i + vec3(0, 0, 1)), hash(i + vec3(1, 0, 1)), f.x),
                   mix(hash(i + vec3(0, 1, 1)), hash(i + vec3(1, 1, 1)), f.x), f.y), f.z);
}

float fbm(vec3 p) {
    float v = 0.0;
    float a = 0.5;
    for (int i = 0; i < 4; i++) {
        v += a * noise(p);
        p = p * 2.07 + vec3(3.1, 1.7, 7.3);
        a *= 0.5;
    }
    return v;
}

void main() {
    float s = fragTexCoord.y;
    float x = (fragTexCoord.x - 0.5) * 2.0;
    float px = s * reach;

    // The rope's half width, as a fraction of the far-end cone half width:
    // thin and steady out to the middle, then blooming.
    float bloom = smoothstep(0.3, 1.0, s);
    float billow = fbm(vec3(s * 3.0 - time * 2.5, seed, time * 0.6));
    float rope = mix(0.05, 0.09, s) + bloom * pow(bloom, 0.3) * (0.3 + 0.3 * billow);
    // Slugs of fuel moving down the rope.
    rope *= 0.85 + 0.2 * sin(px * 0.22 - time * 42.0 + seed);
    // A whip travelling outward along the stream.
    float c = sin(px * 0.045 - time * 16.0 + seed) * 0.06 * s;

    // Rolling fire texture scrolling outward, rougher as it blooms.
    float flow = fbm(vec3(x * 3.0, px * 0.06 - time * 9.0, seed + time * 0.8));
    float dist = abs(x - c) / max(rope, 0.001);
    dist += (flow - 0.5) * (0.25 + 1.1 * bloom);

    float body = smoothstep(1.0, 0.55, dist);
    float core = smoothstep(0.5, 0.0, abs(x - c) / max(rope * 0.45, 0.001)) * (1.0 - smoothstep(0.35, 0.75, s));

    // Heat: hottest in the core near the nozzle, cooling outward and along.
    float heat = clamp(core * 1.2 + body * (1.0 - 0.9 * s) * (0.55 + 0.6 * flow), 0.0, 1.0);
    vec3 col;
    if (heat > 0.75) {
        col = mix(vec3(1.0, 0.84, 0.35), vec3(1.0, 0.98, 0.86), (heat - 0.75) / 0.25);
    } else if (heat > 0.45) {
        col = mix(vec3(1.0, 0.5, 0.1), vec3(1.0, 0.84, 0.35), (heat - 0.45) / 0.3);
    } else if (heat > 0.18) {
        col = mix(vec3(0.78, 0.16, 0.05), vec3(1.0, 0.5, 0.1), (heat - 0.18) / 0.27);
    } else {
        col = mix(vec3(0.22, 0.12, 0.1), vec3(0.78, 0.16, 0.05), heat / 0.18);
    }

    // Coverage: the rope is solid; the bloom thins at its ragged edge and
    // the whole stream fades out over its last stretch.
    float a = max(body, core);
    a *= 1.0 - smoothstep(0.78, 1.0, s + (flow - 0.5) * 0.3);
    a *= smoothstep(0.0, 0.03, s);
    gl_FragColor = vec4(col, clamp(a, 0.0, 1.0) * fragColor.a);
}
