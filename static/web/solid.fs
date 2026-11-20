#version 100
// GLSL ES 100 port of ../solid.fs for the web, iOS and Android builds; keep
// both in sync. The sheet's coordinates want more than mediump's ten bits
// (a tank sheet is 2400 texels tall), so highp wherever the driver has it.

#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

varying vec2 fragTexCoord;
varying vec4 fragColor;

uniform sampler2D texture0;   // the sheet, sampled nearest

// A sprite's shape in one flat colour (render/solid.rs): the tint's colour
// wherever the sheet has paint, as opaque as the paint and the tint
// together - a hit's white silhouette over the hull it landed on, as
// bright on a dark chassis as on a pale one.
void main() {
    gl_FragColor = vec4(fragColor.rgb, texture2D(texture0, fragTexCoord).a * fragColor.a);
}
