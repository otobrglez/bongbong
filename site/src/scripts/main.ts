// Entry point for the game page (src/pages/index.astro). Astro bundles and
// minifies this with its imports; the Emscripten glue stays a separate
// classic script, loaded with `defer` after this one so `window.Module`
// exists when it runs.

import { loadingPanel } from "./loading-panel";
import { installModule } from "./runtime";
import { installInputShims } from "./input";
import { installRoom } from "./room";
import { installMotion } from "./motion";
import { initTuningPanel } from "./tuning-panel";
import { installFullscreenToggle } from "./fullscreen";
import { installOverlay } from "./overlay";
import { applyStrings } from "./strings";

// The page's own words first, in the language the game will pick from the
// same URL and browser list, so nothing around the canvas flashes English.
applyStrings();

const canvas = document.getElementById("canvas") as HTMLCanvasElement | null;
const loading = loadingPanel();

// The room this page was opened on, published before the runtime starts:
// the game reads it once, at startup, the way a desktop build reads its
// command line.
installRoom();
// Whether the browser asks for reduced motion, read once at startup too.
installMotion();

installModule(canvas, loading, (module) => {
  // The runtime is up but nothing has been drawn yet; wait for the frame
  // after the next one so the panel lifts on a picture rather than on the
  // black canvas underneath it.
  window.requestAnimationFrame(() => window.requestAnimationFrame(loading.done));
  try { initTuningPanel(module); } catch (e) { console.error("tuning panel:", e); }
});
installInputShims(canvas);
installFullscreenToggle();
// After the labels and the full-screen button are in, so the box it
// publishes is the one on screen.
installOverlay();
