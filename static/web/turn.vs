#version 100
// GLSL ES 100 port of ../turn.vs for the web, iOS and Android builds; keep
// both in sync.

attribute vec3 vertexPosition;
attribute vec2 vertexTexCoord;   // design pixels from the pivot, along the screen's axes
attribute vec3 vertexNormal;     // the cell's corner on the sheet (design pixels) and the turn (radians)
attribute vec4 vertexColor;

varying vec2 fragTexCoord;
varying vec4 fragColor;
varying vec2 fragCell;
varying vec2 fragTurn;
varying vec4 fragShear;

uniform mat4 mvp;

// A turned sprite's quad (`render::turn`): the batch's normal carries what
// is the same at all four corners, so quads of one sheet share a draw call.
void main() {
    fragTexCoord = vertexTexCoord;
    fragColor = vertexColor;
    fragCell = vertexNormal.xy;
    fragTurn = vec2(cos(vertexNormal.z), sin(vertexNormal.z));
    // The turn as whole quarters and what is left, 45 degrees at most
    // either way: the quarters' cos and sin, and the two shears of the rest.
    float quarters = floor(vertexNormal.z / 1.5707963 + 0.5);
    float rest = vertexNormal.z - quarters * 1.5707963;
    fragShear = vec4(tan(rest * 0.5), sin(rest), floor(cos(quarters * 1.5707963) + 0.5), floor(sin(quarters * 1.5707963) + 0.5));
    gl_Position = mvp * vec4(vertexPosition, 1.0);
}
