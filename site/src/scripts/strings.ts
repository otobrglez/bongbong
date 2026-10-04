// The page's own words, in the language the game will speak
// (docs/localization-prd.md section 4.7). The game picks its language from
// `?lang=` and then the browser's list (src/text.rs's `choose`); the page
// applies the same rule to the dozen strings around the canvas, so the
// loading panel, the key hints and the footer match what the game shows
// once it is up. No routing per language: the game is one page.
//
// Elements carry `data-i18n="key"`; `applyStrings` fills them once the
// language is known. The tuning panel is a developer surface and stays in
// English.

export type Lang = "en" | "sl";

const SHIPPED: Lang[] = ["en", "sl"];

type Strings = {
  title: string;
  fullscreen: string;
  fullscreenTitle: string;
  exitFullscreen: string;
  subscribe: string;
  loading: string;
  fetching: string;
  starting: string;
  preparing: (done: number, total: number) => string;
  /** The card over the stage on a phone held upright. */
  turn: string;
  /** HTML: carries the `<kbd>` markup. */
  keysOne: string;
  keysTwo: string;
  /** HTML: carries the link. */
  feedback: string;
};

const EN: Strings = {
  title: "BongBong!",
  fullscreen: "Full screen",
  fullscreenTitle: "Play full screen",
  exitFullscreen: "Exit full screen",
  subscribe: "Subscribe for updates.",
  loading: "Loading…",
  fetching: "Fetching the game",
  starting: "Starting",
  preparing: (done, total) => `Preparing (${done}/${total})`,
  turn: "Turn your phone sideways to play.",
  keysOne: "Arrows to drive, <kbd>Space</kbd> to fire, <kbd>r</kbd> to restart.",
  keysTwo:
    "Two players: Arrows + <kbd>Space</kbd> for player 1, <kbd>W</kbd><kbd>A</kbd><kbd>S</kbd><kbd>D</kbd> + <kbd>Left Shift</kbd> for player 2.",
  feedback: 'Feedback and suggestions are welcome via <a href="https://github.com/otobrglez">@otobrglez</a>.',
};

const SL: Strings = {
  title: "BongBong!",
  fullscreen: "Celoten zaslon",
  fullscreenTitle: "Igraj na celotnem zaslonu",
  exitFullscreen: "Zapri celoten zaslon",
  subscribe: "Naroči se na novice.",
  loading: "Nalaganje…",
  fetching: "Prenašam igro",
  starting: "Zaganjam",
  preparing: (done, total) => `Pripravljam (${done}/${total})`,
  turn: "Za igranje obrni telefon v ležeči položaj.",
  keysOne: "Puščice za vožnjo, <kbd>preslednica</kbd> za strel, <kbd>r</kbd> za novo igro.",
  keysTwo:
    "Dva igralca: puščice + <kbd>preslednica</kbd> za igralca 1, <kbd>W</kbd><kbd>A</kbd><kbd>S</kbd><kbd>D</kbd> + <kbd>levi Shift</kbd> za igralca 2.",
  feedback: 'Odzivi in predlogi so dobrodošli pri <a href="https://github.com/otobrglez">@otobrglez</a>.',
};

const TABLES: Record<Lang, Strings> = { en: EN, sl: SL };

/** The shipped language `tag` names: itself, or the one with its language
 *  subtag (`sl-SI` names `sl`); nothing for a language the site has not
 *  got. The game's `text::shipped`, for the page. */
function shipped(tag: string): Lang | undefined {
  const t = tag.trim().toLowerCase().replace("_", "-");
  if (!t) return undefined;
  const exact = SHIPPED.find((s) => s === t);
  if (exact) return exact;
  const language = t.split("-")[0];
  return SHIPPED.find((s) => s.split("-")[0] === language);
}

/** The language the page and the game will speak: `?lang=` when it names
 *  a shipped language, else the first of the browser's preferred
 *  languages that does, else English. */
export function pickLanguage(): Lang {
  let explicit: string | null = null;
  try {
    explicit = new URLSearchParams(location.search).get("lang");
  } catch {
    explicit = null;
  }
  const asked = explicit ? shipped(explicit) : undefined;
  if (asked) return asked;
  let preferred: readonly string[] = [];
  try {
    preferred = navigator.languages?.length ? navigator.languages : navigator.language ? [navigator.language] : [];
  } catch {
    preferred = [];
  }
  for (const tag of preferred) {
    const found = shipped(tag);
    if (found) return found;
  }
  return "en";
}

let current: Lang = "en";

/** The word for `key` in the language picked by `applyStrings`. */
export function t<K extends Exclude<keyof Strings, "preparing">>(key: K): string {
  return TABLES[current][key];
}

/** The loading panel's progress line. */
export function preparing(done: number, total: number): string {
  return TABLES[current].preparing(done, total);
}

/** Pick the language and put its words into every `data-i18n` element,
 *  the document's `lang` and its title. Called before anything else on
 *  the page writes text, so nothing flashes English first. */
export function applyStrings(): Lang {
  current = pickLanguage();
  const table = TABLES[current];
  document.documentElement.lang = current;
  document.title = table.title;
  for (const el of document.querySelectorAll<HTMLElement>("[data-i18n]")) {
    const key = el.dataset.i18n as keyof Strings | undefined;
    if (!key || key === "preparing") continue;
    const value = table[key];
    // The key hints and the footer carry markup.
    if (key === "keysOne" || key === "keysTwo" || key === "feedback") {
      el.innerHTML = value;
    } else {
      el.textContent = value;
    }
  }
  for (const el of document.querySelectorAll<HTMLElement>("[data-i18n-title]")) {
    const key = el.dataset.i18nTitle as keyof Strings | undefined;
    if (key && key !== "preparing") el.title = table[key];
  }
  return current;
}
