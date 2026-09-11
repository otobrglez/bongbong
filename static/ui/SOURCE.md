# Map editor UI icons — provenance

`eraser.png` is **third-party art**, one icon (`Skillicon4_06.png`) copied
from craftpix.net's free "Skill 32x32 Icons for Cyberpunk Game" pack
(`craftpix-net-741764-free-skill-32x32-icons-for-cyberpunk-game`, kept
outside this repo under `../bongbong-assets/`), not produced by bongbong's
own generators — same "hand-authored third-party art, plain file copy, not
part of the `tools/spritegen/` pipeline" treatment as `static/punyworld/`.

- License terms: <https://craftpix.net/file-licenses/> (per the pack's own
  `License.txt`) — check those terms before any redistribution beyond local
  development.
- Only ever drawn by the map builder (`src/editor/`), which is part of every
  build now that the builder is a mode of the game (docs/game-editor-fusion.md);
  bundled with the rest of `static/`. See docs/map-editor-design.md's
  "Eraser icon" section for its origin.
- 32×32 px, used as-is (no recolor/retint pass, unlike `punyworld`'s).
