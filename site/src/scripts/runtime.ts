// The Emscripten `Module` object: the page's side of the contract with the
// generated glue (`/game/bongbong.js`, built by `just build-web`). The glue
// is a classic script that reads the *global* `Module` when it runs, so
// `installModule` must have run first - index.astro loads the glue with
// `defer`, which the browser executes after this (module) script, in
// document order.

import { preparing } from "./strings";

import type { LoadingPanel } from "./loading-panel";

type CType = "number" | "string" | "boolean" | "array" | null;

export interface EmscriptenModule {
  canvas: HTMLCanvasElement | null;
  locateFile(path: string): string;
  print(text: string): void;
  printErr(text: string): void;
  setStatus(text: string): void;
  monitorRunDependencies(left: number): void;
  onRuntimeInitialized(): void;
  /** High-water mark of `monitorRunDependencies`, for a stable denominator. */
  totalDependencies?: number;
  /** Exported by the glue once the runtime is up (`-sEXPORTED_RUNTIME_METHODS`). */
  ccall?(name: string, ret: CType, argTypes: CType[], args: unknown[]): unknown;
  /** `_bb_*` and everything else the glue attaches. */
  [key: string]: unknown;
}

/** Define `window.Module` for the glue and wire the loading panel to its
 *  progress hooks. `onReady` runs once the wasm runtime is initialised. */
export function installModule(
  canvas: HTMLCanvasElement | null,
  loading: LoadingPanel,
  onReady: (module: EmscriptenModule) => void,
): EmscriptenModule {
  const module: EmscriptenModule = {
    canvas,
    locateFile: (path) => `/game/${path}`,
    print: (t) => console.log(t),
    printErr: (t) => console.error(t),
    setStatus: (t) => loading.say(t),
    monitorRunDependencies(left) {
      module.totalDependencies = Math.max(module.totalDependencies || 0, left);
      loading.say(left ? preparing(module.totalDependencies - left, module.totalDependencies) : "");
    },
    onRuntimeInitialized() {
      onReady(module);
    },
  };
  window.Module = module;
  opaqueCanvas(canvas);
  keepAudioWithThePage();
  return module;
}

/** The game's canvas is opaque: its WebGL context is made without an alpha
 *  channel. The game blits each finished picture keeping the alpha already
 *  under it (`render::view::draw_opaque`), which a desktop window ignores
 *  but a browser composites - over a canvas with alpha the field would come
 *  out see-through, showing the page behind it. Without one nothing can. */
function opaqueCanvas(canvas: HTMLCanvasElement | null): void {
  if (!canvas) return;
  const get = canvas.getContext.bind(canvas) as (type: string, attrs?: object) => RenderingContext | null;
  const webgl = (type: string) => type === "webgl" || type === "webgl2" || type === "experimental-webgl";
  canvas.getContext = ((type: string, attrs?: object) =>
    get(type, webgl(type) ? { ...attrs, alpha: false } : attrs)) as typeof canvas.getContext;
}

/** The Web Audio contexts raylib's audio (miniaudio) plays the game's sound
 *  through (docs/sound-system-prd.md D4); miniaudio keeps them on
 *  `window.miniaudio`. */
function audioContexts(): AudioContext[] {
  const ma = (window as unknown as { miniaudio?: { devices?: ({ webaudio?: AudioContext } | null)[] } }).miniaudio;
  return (ma?.devices ?? []).flatMap((device) => (device?.webaudio ? [device.webaudio] : []));
}

// Browsers hold an AudioContext suspended until a user gesture. miniaudio
// resumes its own on a click or a touch; a key press - the keyboard plays the
// game - is a gesture too, so it resumes them here. Every gesture is heard,
// not just the first: the device opens once the wasm has loaded, which can
// be after the first press. A hidden tab suspends the sound (the game stops
// drawing there) and a shown one brings it back.
function keepAudioWithThePage(): void {
  let unlocked = false;
  const resume = () => {
    unlocked = true;
    for (const ctx of audioContexts()) {
      if (ctx.state === "suspended") void ctx.resume();
    }
  };
  for (const type of ["keydown", "pointerdown", "touchend"]) {
    window.addEventListener(type, resume, { capture: true });
  }
  document.addEventListener("visibilitychange", () => {
    for (const ctx of audioContexts()) {
      if (document.hidden) void ctx.suspend();
      else if (unlocked) void ctx.resume();
    }
  });
}
