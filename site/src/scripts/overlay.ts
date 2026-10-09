// Where the page's own controls (Full screen, Subscribe) stand over the
// canvas, published for the game (src/hud.rs `PageOverlay`, read once a
// frame by src/app/web.rs): `window.bbOverlay` is `top x y width height`
// in CSS pixels from the canvas's corner.
//
// `x y width height` is the controls' box, which the game's off-screen
// arrows keep off wherever it is. `top` is the band along the canvas's top
// the game's chrome keeps out of, the way it keeps out of a safe area: on a
// touch screen the controls stand there (index.astro), so the corner
// clusters, the panels and the builder's bar are laid out below them; with
// a mouse they keep to the bottom-left corner and take no band.

/// The same test the page's CSS moves the controls by.
const TOUCH = "(hover: none) and (pointer: coarse)";

/// Between the controls' bottom edge and the chrome below them.
const GAP_PX = 3;

/// The short side of an iPad mini 6 or later, in CSS pixels.
const IPAD_MINI_SHORT_PX = 744;

/// A full-size iPad, whose CSS pixel - its point - is a 132nd of an inch
/// rather than a phone's 160th (src/indicators.rs `IPAD_POINTS_PER_MM`).
/// iPadOS's Safari asks for the desktop site and calls itself a Mac, but a
/// Mac has no touch points; an older one says "iPad". An iPad mini 6 or
/// later keeps a phone's measure; a mini 5 (768 x 1024) cannot be told from
/// a 9.7" iPad and is taken for one.
function fullSizeIpad(): boolean {
  try {
    const ua = navigator.userAgent || "";
    const ipad = /\biPad\b/.test(ua) || (/\bMacintosh\b/.test(ua) && navigator.maxTouchPoints > 1);
    return ipad && Math.min(screen.width, screen.height) !== IPAD_MINI_SHORT_PX;
  } catch {
    return false;
  }
}

export function installOverlay(): void {
  window.bbOverlay = "";
  const touch = window.matchMedia ? window.matchMedia(TOUCH) : null;
  // Whether this is a touch screen, read once by the game at startup
  // (src/app.rs `screen`): a phone's or a tablet's browser frames the view
  // as the app on that device does, rather than at a desktop's 96 CSS
  // pixels to the inch.
  window.bbTouch = touch && touch.matches ? "1" : "";
  // Whether this is a full-size iPad, read once by the game at startup:
  // its builder measures a cell under a finger in millimetres
  // (src/app.rs `touch_points_per_mm`).
  window.bbIpad = fullSizeIpad() ? "1" : "";
  const canvas = document.getElementById("canvas");
  const controls = document.querySelector<HTMLElement>(".overlay-controls");
  if (!canvas || !controls) return;

  const publish = () => {
    const c = canvas.getBoundingClientRect();
    const o = controls.getBoundingClientRect();
    if (c.width < 1 || c.height < 1 || o.width < 1 || o.height < 1) {
      window.bbOverlay = "";
      return;
    }
    const x = o.left - c.left;
    const y = o.top - c.top;
    const top = touch && touch.matches ? Math.max(0, y + o.height + GAP_PX) : 0;
    window.bbOverlay = [top, x, y, o.width, o.height].map((v) => Math.round(v * 100) / 100).join(" ");
  };

  // The box moves with the canvas (full screen, immersive mode, a resize,
  // a rotation) and changes size with its labels (Exit full screen); a
  // switch of pointer moves it from one corner to the top.
  if (typeof ResizeObserver === "function") {
    const watch = new ResizeObserver(publish);
    watch.observe(canvas);
    watch.observe(controls);
  }
  window.addEventListener("resize", publish);
  document.addEventListener("fullscreenchange", publish);
  document.addEventListener("webkitfullscreenchange", publish);
  touch?.addEventListener?.("change", publish);
  publish();
}
