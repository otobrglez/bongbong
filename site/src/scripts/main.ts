// Entry point for the game page (src/pages/index.astro). Astro bundles and
// minifies this with its imports; the Emscripten glue stays a separate
// classic script, loaded with `defer` after this one so `window.Module`
// exists when it runs.

import { loadingPanel } from "./loading-panel";
import { installModule } from "./runtime";
import { installInputShims } from "./input";
import { initTuningPanel } from "./tuning-panel";
import { installFullscreenToggle } from "./fullscreen";

const canvas = document.getElementById("canvas") as HTMLCanvasElement | null;
const loading = loadingPanel();

installModule(canvas, loading, (module) => {
  // The runtime is up but nothing has been drawn yet; wait for the frame
  // after the next one so the panel lifts on a picture rather than on the
  // black canvas underneath it.
  window.requestAnimationFrame(() => window.requestAnimationFrame(loading.done));
  try { initTuningPanel(module); } catch (e) { console.error("tuning panel:", e); }
});
installInputShims(canvas);
installFullscreenToggle();
