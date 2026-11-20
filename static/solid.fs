#version 330

in vec2 fragTexCoord;
in vec4 fragColor;
out vec4 finalColor;

uniform sampler2D texture0;   // the sheet, sampled nearest

// A sprite's shape in one flat colour (render/solid.rs): the tint's colour
// wherever the sheet has paint, as opaque as the paint and the tint
// together - a hit's white silhouette over the hull it landed on, as
// bright on a dark chassis as on a pale one.
void main() {
    finalColor = vec4(fragColor.rgb, texture(texture0, fragTexCoord).a * fragColor.a);
}
