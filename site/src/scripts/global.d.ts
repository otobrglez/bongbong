// Globals the page shares with the Emscripten glue (`public/game/bongbong.js`)
// and with the game itself (src/app.rs).
import type { EmscriptenModule } from "./runtime";

declare global {
  interface Window {
    /** Emscripten's configuration object. The glue reads the global `Module`
     *  when it loads, so the page must have defined it by then. */
    Module: EmscriptenModule;
    /** Bit 1 = ShiftLeft held; read once a frame by `app.rs`'s
     *  `left_shift_down`. See `input.ts`. */
    bbShift: number;
    /** The URL this page was opened on, fragment stripped: the web
     *  build's command line, parsed once at startup by `app.rs` through
     *  `net::rooms::Invite`. See `room.ts`. */
    bbInvite: string;
    /** This tab's reconnect key in a room, read once at startup by
     *  `app.rs`. See `room.ts`. */
    bbToken: string;
  }

  // Old WebKit's prefixed Fullscreen API (see fullscreen.ts).
  interface Document {
    webkitFullscreenElement?: Element | null;
    webkitExitFullscreen?: () => Promise<void> | void;
  }

  interface HTMLElement {
    webkitRequestFullscreen?: () => Promise<void> | void;
  }
}

export {};
