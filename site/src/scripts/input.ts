// Keyboard and pointer shims between the browser and raylib's GLFW layer.

/** Arrow keys and Space are game controls, but browsers also bind them to
 *  page scrolling by default - on viewports too short to fit the whole
 *  layout, that scroll becomes visible and fights the tank's movement. */
const GAME_KEYS = new Set(["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Space"]);

export function installInputShims(canvas: HTMLCanvasElement | null): void {
  // Right button is the builder's eraser, never the browser's menu.
  canvas?.addEventListener("contextmenu", (e) => e.preventDefault());

  window.addEventListener(
    "keydown",
    (e) => { if (GAME_KEYS.has(e.code)) e.preventDefault(); },
    { passive: false },
  );

  // Whether the left Shift key - player 2's fire key - is held. Emscripten's
  // GLFW layer reports the DOM Shift key as the *left* Shift whichever
  // side was pressed (it maps keyCode 0x10 and never reads event.code), so
  // through raylib a Right Shift would fire player 2's tank on the web
  // only. The game reads this flag once a frame instead (bit 1 =
  // ShiftLeft; see src/app.rs's left_shift_down). Cleared on blur so a
  // Shift held while the tab loses focus does not stick.
  window.bbShift = 0;
  window.addEventListener("keydown", (e) => { if (e.code === "ShiftLeft") window.bbShift |= 1; });
  window.addEventListener("keyup", (e) => { if (e.code === "ShiftLeft") window.bbShift &= ~1; });
  window.addEventListener("blur", () => { window.bbShift = 0; });
}
