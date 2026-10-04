// Where the page's own controls stand over the canvas - Full screen, and
// with a mouse the subscribe link beside it - published for the game (src/hud.rs `PageOverlay`, read once a
// frame by src/app/web.rs): `window.bbOverlay` is `top x y width height`
// in CSS pixels from the canvas's corner.
//
// `x y width height` is the controls' box, which the game's off-screen
// arrows keep off wherever it is. `top` is the band along the canvas's top
// the game's chrome keeps out of, the way it keeps out of a safe area: on a
// touch screen the full-screen button stands there, its icon alone, and
// the subscribe link moves to the footer (index.astro), so the corner
// clusters, the panels and the builder's bar are laid out below it; with a
// mouse both keep to the bottom-left corner and take no band.

/// The same test the page's CSS moves the controls by.
const TOUCH = "(hover: none) and (pointer: coarse)";

/// Between the controls' bottom edge and the chrome below them.
const GAP_PX = 3;

export function installOverlay(): void {
  window.bbOverlay = "";
  const touch = window.matchMedia ? window.matchMedia(TOUCH) : null;
  // Whether this is a touch screen, read once by the game at startup
  // (src/app.rs `screen`): a phone's or a tablet's browser frames the view
  // as the app on that device does, rather than at a desktop's 96 CSS
  // pixels to the inch.
  window.bbTouch = touch && touch.matches ? "1" : "";
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
