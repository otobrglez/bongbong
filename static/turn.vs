#version 330

in vec3 vertexPosition;
in vec2 vertexTexCoord;   // design pixels from the pivot, along the screen's axes
in vec3 vertexNormal;     // the cell's corner on the sheet (design pixels) and the turn (radians)
in vec4 vertexColor;

out vec2 fragTexCoord;
out vec4 fragColor;
out vec2 fragCell;
out vec2 fragTurn;
out vec4 fragShear;

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
