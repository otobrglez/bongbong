// Full-screen toggle.
//
// Three outcomes, because no single mechanism works everywhere:
//
//   1. `requestFullscreen` - desktop, Android, iPadOS. Real full screen:
//      the browser's own chrome goes away.
//   2. The `.immersive` class - Safari on iPhone, which has no element
//      Fullscreen API at all (only <video> can go fullscreen there).
//      This takes the whole viewport but *cannot* hide Safari's URL bar;
//      no page can. It is a platform restriction, not a bug.
//   3. Add to Home Screen - the only way to get a chrome-free window on
//      an iPhone; `apple-mobile-web-app-capable` in the head makes the
//      launched app run standalone. Deliberately not advertised on the
//      page: it is a wall of text over the game explaining a platform
//      limitation the player cannot do anything about mid-round.

export function installFullscreenToggle(): void {
  const game = document.querySelector<HTMLElement>(".game");
  const canvas = document.getElementById("canvas");
  const button = document.getElementById("fullscreen-toggle");
  if (!game || !button) return;
  const request = game.requestFullscreen || game.webkitRequestFullscreen;
  const exit = document.exitFullscreen || document.webkitExitFullscreen;

  const real = () => !!(document.fullscreenElement || document.webkitFullscreenElement);
  const active = () => real() || game.classList.contains("immersive");
  const label = () => {
    button.innerHTML = active() ? "&#9974; Exit full screen" : "&#9974; Full screen";
  };
  const fallback = () => {
    game.classList.add("immersive");
    label();
  };

  function enter(): void {
    if (!request) return fallback();
    let p: Promise<void> | void;
    try {
      p = request.call(game);
    } catch {
      return fallback();
    }
    // Old WebKit's prefixed version returns undefined rather than a
    // promise, so `.then` on it would throw and abort the whole
    // handler - leaving neither real full screen nor the fallback.
    if (p && typeof p.then === "function") {
      p.then(label, fallback);
    } else {
      setTimeout(() => { if (real()) label(); else fallback(); }, 120);
    }
  }

  button.addEventListener("click", (e) => {
    // Never let the toggle double as a game tap.
    e.preventDefault();
    e.stopPropagation();
    if (active()) {
      if (real() && exit) {
        try { void exit.call(document); } catch { /* already out */ }
      }
      game.classList.remove("immersive");
      label();
    } else {
      enter();
    }
    canvas?.focus();
  });
  document.addEventListener("fullscreenchange", label);
  document.addEventListener("webkitfullscreenchange", label);
  label();
}
