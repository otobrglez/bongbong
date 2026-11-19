#version 330

in vec2 fragTexCoord;   // design pixels from the pivot, along the screen's axes
in vec4 fragColor;
in vec2 fragCell;       // the cell's corner on the sheet, design pixels
in vec2 fragTurn;       // cos and sin of the turn
in vec4 fragShear;   // the rest of the turn past its whole quarters as two shears, the quarters' cos and sin
out vec4 finalColor;

uniform sampler2D texture0;   // the sheet, sampled nearest
uniform vec2 texel;           // one design pixel in texture coordinates
uniform vec2 cell;            // the cell's size, design pixels
uniform vec2 pivot;           // the pivot from the cell's corner, design pixels
uniform float corner;         // how far a corner is cut along its two edges, 0 (never) to 1 (the whole quarter)
uniform float shear;          // 1: every design pixel moved whole (three shears); 0: the pixel under the block's centre

// The cell's design pixel `i`, clear outside the cell: a neighbouring
// cell's art is never read.
vec4 at(vec2 i) {
    if (i.x < 0.0 || i.y < 0.0 || i.x >= cell.x || i.y >= cell.y) return vec4(0.0);
    return texture(texture0, (fragCell + i + 0.5) * texel);
}

// Whether two design pixels are one colour; all clear pixels are.
bool same(vec4 a, vec4 b) {
    if (a.a < 0.004 && b.a < 0.004) return true;
    vec4 d = abs(a - b);
    return max(max(d.r, d.g), max(d.b, d.a)) < 0.004;
}

// A sprite turned to any angle on its own design-pixel grid (D-5, the
// crisp turn). The fragment takes the block of the screen it lies in -
// blocks as large as a design pixel is drawn, laid from the pivot, where
// the sprite's own pixels lie at rest - and the whole block shows one
// design pixel, so a turned sprite is built of the same whole blocks as a
// resting one and of no colour its cell does not hold.
//
// Sheared, the turn past its whole quarters is three shears (Paeth's
// rotation), each sliding whole rows or columns of blocks a whole number
// of blocks: every design pixel of the cell lands on exactly one block,
// so nothing is dropped or doubled and a line one pixel wide stays one.
//
// Otherwise the block shows the design pixel under its centre once the
// turn is undone; where the centre falls in a corner of a pixel whose two
// neighbours there are one colour, that colour takes the corner (Scale2x's
// rule, the enlargement RotSprite turns).
void main() {
    vec2 lead = fract(pivot);
    vec2 block = floor(fragTexCoord + lead) + 0.5 - lead;
    if (shear > 0.5) {
        float across = block.x + floor(fragShear.x * block.y + 0.5);
        float down = block.y - floor(fragShear.y * across + 0.5);
        across += floor(fragShear.x * down + 0.5);
        finalColor = at(floor(pivot + vec2(fragShear.z * across + fragShear.w * down, fragShear.z * down - fragShear.w * across))) * fragColor;
        return;
    }
    vec2 p = pivot + vec2(fragTurn.x * block.x + fragTurn.y * block.y, fragTurn.x * block.y - fragTurn.y * block.x);
    vec2 i = floor(p);
    vec2 f = p - i;
    vec2 side = vec2(f.x < 0.5 ? -1.0 : 1.0, f.y < 0.5 ? -1.0 : 1.0);
    vec2 edge = 0.5 - abs(f - 0.5);
    vec4 colour = at(i);
    vec4 h = at(i + vec2(side.x, 0.0));
    vec4 v = at(i + vec2(0.0, side.y));
    vec4 ho = at(i - vec2(side.x, 0.0));
    vec4 vo = at(i - vec2(0.0, side.y));
    if (edge.x + edge.y < corner && same(h, v) && !same(h, vo) && !same(v, ho)) colour = h;
    finalColor = colour * fragColor;
}
