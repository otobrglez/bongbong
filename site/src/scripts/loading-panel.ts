// The loading panel, driven by Emscripten's own progress reporting.
//
// Two hooks, because they cover different halves of the wait and either
// one alone leaves a silent stretch. `setStatus` carries the byte
// counts while `bongbong.data` streams in ("Downloading data... (a/b)");
// `monitorRunDependencies` counts the preload work that follows, which
// reports no bytes at all. Neither fires for the wasm fetch/compile
// itself, which is why the bar starts as an indeterminate sweep rather
// than at zero.

const PROGRESS_RE = /([\d.]+)\s*\/\s*([\d.]+)/;

export interface LoadingPanel {
  /** Feed one of Emscripten's status lines; an empty string means "starting". */
  say(text: string): void;
  /** Fill the bar, fade the panel out and take it out of the layout. */
  done(): void;
}

export function loadingPanel(): LoadingPanel {
  const panel = document.getElementById("loading");
  const bar = document.getElementById("loading-bar");
  const note = document.getElementById("loading-note");
  let finished = false;

  function say(text: string): void {
    if (finished || !note) return;
    const m = text ? text.match(PROGRESS_RE) : null;
    if (m && bar) {
      const done = parseFloat(m[1]);
      const total = parseFloat(m[2]);
      if (total > 0) {
        bar.classList.add("measured");
        bar.style.width = `${Math.max(3, Math.min(100, (done / total) * 100))}%`;
      }
    }
    // Emscripten's own wording is developer-facing; keep the panel's
    // note to the one thing a player cares about.
    note.textContent = text ? "Fetching the game" : "Starting";
  }

  function done(): void {
    if (finished || !panel) return;
    finished = true;
    if (bar) {
      bar.classList.add("measured");
      bar.style.width = "100%";
    }
    panel.classList.add("done");
    // Match the CSS fade, then take it out of the layout entirely so
    // it can never eat a tap meant for the canvas.
    window.setTimeout(() => { panel.hidden = true; }, 300);
  }

  return { say, done };
}
