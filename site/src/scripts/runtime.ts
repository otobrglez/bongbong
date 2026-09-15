// The Emscripten `Module` object: the page's side of the contract with the
// generated glue (`/game/bongbong.js`, built by `just build-web`). The glue
// is a classic script that reads the *global* `Module` when it runs, so
// `installModule` must have run first - index.astro loads the glue with
// `defer`, which the browser executes after this (module) script, in
// document order.

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
  /** Reserved for when audio arrives (docs: "Audio: none yet"). */
  audioContext?: AudioContext;
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
      loading.say(left ? `Preparing (${module.totalDependencies - left}/${module.totalDependencies})` : "");
    },
    onRuntimeInitialized() {
      onReady(module);
    },
  };
  window.Module = module;
  resumeAudioOnFirstClick(module);
  return module;
}

// Browsers keep an AudioContext created before any user gesture suspended;
// the first click is the gesture that lets it run.
function resumeAudioOnFirstClick(module: EmscriptenModule): void {
  window.addEventListener(
    "click",
    () => {
      const ctx = module.audioContext;
      if (ctx && ctx.state === "suspended") void ctx.resume();
    },
    { once: true },
  );
}
