// Keyboard and pointer shims between the browser and raylib's GLFW layer.

/** Arrow keys and Space are game controls, but browsers also bind them to
 *  page scrolling by default - on viewports too short to fit the whole
 *  layout, that scroll becomes visible and fights the tank's movement. */
const GAME_KEYS = new Set(["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Space"]);

/** Whether `target` takes typing: the newsletter's email box, the tuning
 *  panel's fields. */
function editable(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement;
}

export function installInputShims(canvas: HTMLCanvasElement | null): void {
  // A key typed into one of the page's own fields is the field's, never the
  // game's. Emscripten's GLFW listens on the window in the capture phase
  // (its `glfwInit`, when the runtime starts) and would drive the tank, fire
  // on Space, restart on R and swallow Backspace and Tab; the shims below
  // would keep the arrows from moving the caret. This listener is on the
  // same window and phase and is added first - this module runs before the
  // deferred glue - so stopping the event here keeps every later listener
  // from seeing it, while the field still takes the key.
  for (const type of ["keydown", "keypress", "keyup"]) {
    window.addEventListener(type, (e) => { if (editable(e.target)) e.stopImmediatePropagation(); }, true);
  }

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
