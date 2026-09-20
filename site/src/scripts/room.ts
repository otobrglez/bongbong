// What the game needs from the page it is running in, to reach an online
// co-op room (docs/online-coop-prd.md §4.10): the link this page was
// opened on, and a token that says which player this is.
//
// The pattern is `window.bbShift`'s (see input.ts): the page publishes a
// fact on `window`, the game reads it through `emscripten_run_script`
// (src/app.rs's `page_string`). A browser has no argv, so these two
// globals are the whole of the web build's command line - `--join CODE`,
// `--rooms URL` and the device token, all at once.

/** The device this browser is, across visits. Kept in localStorage,
 *  which the PRD names as the browser's half of identity. */
const DEVICE_KEY = "bongbong.device";

/** Which tab of it. Kept in sessionStorage, which is per tab and
 *  survives a reload: a refresh reclaims the same seat, a second tab is
 *  a second player. localStorage alone could not tell the two apart -
 *  it is shared by every tab on the origin, so both would ask the room
 *  for the same seat and one would take it from the other. */
const TAB_KEY = "bongbong.tab";

/** Sixteen hex characters of randomness. `crypto` is there in every
 *  browser that runs the game; the fallback only keeps a page in an
 *  exotic one from having no token at all. */
function randomId(): string {
  const bytes = new Uint8Array(8);
  if (globalThis.crypto?.getRandomValues) {
    crypto.getRandomValues(bytes);
  } else {
    for (let i = 0; i < bytes.length; i++) bytes[i] = Math.floor(Math.random() * 256);
  }
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

/** The id kept under `key` in the store `open` returns, minted on first
 *  use. Storage is not always there to be had - a private window, a
 *  browser with site data blocked, an iframe without access - and both
 *  opening it and reading it throw rather than answer, so a failure
 *  falls back to an id that lives as long as this page does. That still
 *  joins a room; it just does not survive a reload. */
function kept(open: () => Storage, key: string): string {
  try {
    const store = open();
    const had = store.getItem(key);
    if (had) return had;
    const minted = randomId();
    store.setItem(key, minted);
    return minted;
  } catch {
    return randomId();
  }
}

export function installRoom(): void {
  // The URL the game reads its room out of, whole. The rule that writes
  // an invite lives in the game (src/net/rooms.rs's `join_url`, which is
  // also what the lobby's QR carries), so the rule that reads one back
  // lives beside it and is unit-tested there rather than here. The
  // page's part is the one thing only the page can do: hand over a
  // location wasm cannot ask for itself. The fragment goes, since it is
  // never part of an invite.
  let here = "";
  try {
    here = location.href.split("#")[0];
  } catch {
    here = "";
  }
  window.bbInvite = here;

  // The reconnect key a room knows this player by: the device, then the
  // tab. Two tabs are two seats; a reload of either is the same seat
  // coming back.
  window.bbToken = `${kept(() => localStorage, DEVICE_KEY)}-${kept(() => sessionStorage, TAB_KEY)}`;
}
