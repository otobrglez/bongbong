// The page's ways off the canvas (index.astro).
//
// On a touch screen the band along the canvas's top holds only Full screen
// and More (overlay.ts says why the band is narrow); More opens a panel of
// the rest - the App Store, the newsletter, the sections below the game,
// Support and Privacy. Tapping More again, outside the panel or pressing
// Escape closes it. The round runs on behind it.
//
// A link from the game to the page below it (`data-leave-game`: Get
// updates, More about BongBong) first takes the game out of full screen,
// where the `.game` box is all that shows, then scrolls once the page is
// laid out again. A phone held sideways has no other way down: the canvas
// fills the screen and takes every swipe (`touch-action: none`).

import { leaveFullscreen } from "./fullscreen";
import { t } from "./strings";

export function installMorePanel(): void {
  const toggle = document.getElementById("more-toggle");
  const panel = document.getElementById("more-panel");

  const setOpen = (open: boolean) => {
    if (!toggle || !panel) return;
    panel.hidden = !open;
    toggle.setAttribute("aria-expanded", String(open));
    toggle.textContent = open ? t("closeButton") : t("moreButton");
  };

  if (toggle && panel) {
    toggle.addEventListener("click", (e) => {
      // Never let the toggle double as a game tap (fullscreen.ts).
      e.preventDefault();
      e.stopPropagation();
      setOpen(panel.hidden);
    });
    document.addEventListener("keydown", (e) => {
      if (e.key === "Escape" && !panel.hidden) setOpen(false);
    });
    document.addEventListener(
      "pointerdown",
      (e) => {
        const target = e.target as Node | null;
        if (!panel.hidden && target && !panel.contains(target) && !toggle.contains(target)) setOpen(false);
      },
      true,
    );
  }

  const smooth = !window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
  for (const link of document.querySelectorAll<HTMLAnchorElement>("a[data-leave-game]")) {
    link.addEventListener("click", (e) => {
      const target = link.hash ? document.getElementById(link.hash.slice(1)) : null;
      if (!target) return;
      e.preventDefault();
      e.stopPropagation();
      setOpen(false);
      leaveFullscreen();
      window.requestAnimationFrame(() =>
        window.requestAnimationFrame(() => target.scrollIntoView({ behavior: smooth ? "smooth" : "auto", block: "start" })),
      );
    });
  }
}
