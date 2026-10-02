// Whether this browser asks for reduced motion, for the game's one motion
// switch (src/motion.rs, docs/large-maps-follow-camera.md section 6): the
// `prefers-reduced-motion` media query, which follows the system setting
// (Reduce Motion on macOS and iOS, "Show animations" off on Windows,
// "Remove animations" on Android).
//
// The `window.bbLang` pattern (see room.ts): the page publishes the fact
// before the runtime starts and the game reads it once, at startup
// (src/app.rs's `page_string`). "reduce" or "no-preference"; empty where
// the browser cannot answer, which the game takes as full motion.

export function installMotion(): void {
  let answer = "";
  try {
    if (typeof window.matchMedia === "function") {
      if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
        answer = "reduce";
      } else if (window.matchMedia("(prefers-reduced-motion: no-preference)").matches) {
        answer = "no-preference";
      }
    }
  } catch {
    answer = "";
  }
  window.bbMotion = answer;
}
