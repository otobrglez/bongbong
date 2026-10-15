// The game page's own words, in the language the game will speak
// (docs/localization-prd.md section 4.7). The game picks its language from
// `?lang=` and then the browser's list (src/text.rs's `choose`); the page
// applies the same rule to its words around and below the canvas, so the
// loading panel, the key hints and the sections under the game match what
// the game shows once it is up. No routing per language: the game is one
// page. The privacy, support and subscribe pages are prose and English
// (src/layouts/Page.astro).
//
// Elements carry `data-i18n="key"` for their text, `data-i18n-title`,
// `data-i18n-alt` and `data-i18n-label` (aria-label) for those attributes;
// `applyStrings` fills them once the language is known. The tuning panel is
// a developer surface and stays in English.

export type Lang = "en" | "sl";

const SHIPPED: Lang[] = ["en", "sl"];

type Strings = {
  title: string;
  /** HTML: the full-screen glyph and the label, which a touch screen
   *  hides (index.astro), each in its own span. */
  fullscreen: string;
  fullscreenTitle: string;
  /** HTML, as `fullscreen`. */
  exitFullscreen: string;
  /** The touch screen's toggle for the panel of the page's links, closed and open. */
  moreButton: string;
  closeButton: string;
  updates: string;
  loading: string;
  fetching: string;
  starting: string;
  preparing: (done: number, total: number) => string;
  /** The card over the stage on a phone held upright, and what it adds. */
  turn: string;
  turnNote: string;
  or: string;
  more: string;
  /** HTML: carries the `<kbd>` markup. */
  keysOne: string;
  keysTwo: string;
  moreAbout: string;
  appBetter: string;
  appBetterNote: string;
  badgeAlt: string;
  inTitle: string;
  inLead: string;
  campaign: string;
  campaignText: string;
  coop: string;
  coopText: string;
  build: string;
  buildText: string;
  takeTitle: string;
  takeLead: string;
  ios: string;
  iosText: string;
  iosLink: string;
  iosSoon: string;
  latest: string;
  whatsNew: string;
  allReleases: string;
  newsTitle: string;
  newsLead: string;
  email: string;
  subscribeButton: string;
  kitNote: string;
  newsDone: string;
  newsFailed: string;
  support: string;
  privacy: string;
  source: string;
  backToGame: string;
};

/** The keys whose words carry markup, put in as HTML. */
const HTML_KEYS = new Set<keyof Strings>(["fullscreen", "exitFullscreen", "keysOne", "keysTwo"]);

const EN: Strings = {
  title: "BongBong!",
  fullscreen: '<span class="glyph" aria-hidden="true">&#9974;</span> <span class="label">Full screen</span>',
  fullscreenTitle: "Play full screen",
  exitFullscreen: '<span class="glyph" aria-hidden="true">&#9974;</span> <span class="label">Exit full screen</span>',
  moreButton: "≡ More",
  closeButton: "✕ Close",
  updates: "Get updates",
  loading: "Loading…",
  fetching: "Fetching the game",
  starting: "Starting",
  preparing: (done, total) => `Preparing (${done}/${total})`,
  turn: "Turn your phone sideways to play.",
  turnNote: "The round is already running.",
  or: "or",
  more: "More",
  keysOne: "<kbd>←↑↓→</kbd> drive · <kbd>Space</kbd> fire · <kbd>R</kbd> restart",
  keysTwo: "Player 2: <kbd>WASD</kbd> + <kbd>Left Shift</kbd>",
  moreAbout: "More about BongBong",
  appBetter: "Better as an app",
  appBetterNote: "Full screen, no browser bars, plays offline.",
  badgeAlt: "Download on the App Store",
  inTitle: "What's in it",
  inLead: "Twelve tanks, seven bongs and a battlefield that burns, smokes and falls apart around you.",
  campaign: "Campaign and free play",
  campaignText: "Level by level, or any map with as many enemies as you dare.",
  coop: "Online co-op",
  coopText: "Host a room, share its code, and fight side by side, up to eight tanks.",
  build: "Build your battlefield",
  buildText: "Paint walls, water and woods, then press play on what you made.",
  takeTitle: "Take it with you",
  takeLead: "The same game on every screen. The browser is just the quickest way in.",
  ios: "iPhone and iPad",
  iosText: "Left thumb steers, right thumb fires. Plays offline.",
  iosLink: "Download from the App Store",
  iosSoon: "Coming soon to the App Store.",
  latest: "Latest release",
  whatsNew: "What's new",
  allReleases: "All releases",
  newsTitle: "Get a dispatch when a new version ships",
  newsLead: "New tanks, new maps, new ways to blow things up.",
  email: "Email",
  subscribeButton: "Subscribe",
  kitNote: "Sent with Kit. Unsubscribe from any email.",
  newsDone: "Nearly there: check your inbox to confirm.",
  newsFailed: "That didn't go through. Try again in a moment.",
  support: "Support",
  privacy: "Privacy",
  source: "Source",
  backToGame: "▲ Back to the game",
};

const SL: Strings = {
  title: "BongBong!",
  fullscreen: '<span class="glyph" aria-hidden="true">&#9974;</span> <span class="label">Celoten zaslon</span>',
  fullscreenTitle: "Igraj na celotnem zaslonu",
  exitFullscreen: '<span class="glyph" aria-hidden="true">&#9974;</span> <span class="label">Zapri celoten zaslon</span>',
  moreButton: "≡ Več",
  closeButton: "✕ Zapri",
  updates: "Prejmi novice",
  loading: "Nalaganje…",
  fetching: "Prenašam igro",
  starting: "Zaganjam",
  preparing: (done, total) => `Pripravljam (${done}/${total})`,
  turn: "Za igranje obrni telefon v ležeči položaj.",
  turnNote: "Igra že teče.",
  or: "ali",
  more: "Več",
  keysOne: "<kbd>←↑↓→</kbd> vožnja · <kbd>preslednica</kbd> strel · <kbd>R</kbd> nova igra",
  keysTwo: "Igralec 2: <kbd>WASD</kbd> + <kbd>levi Shift</kbd>",
  moreAbout: "Več o igri BongBong",
  appBetter: "Še boljša kot aplikacija",
  appBetterNote: "Celoten zaslon, brez vrstic brskalnika, deluje brez povezave.",
  badgeAlt: "Prenesi iz trgovine App Store",
  inTitle: "Kaj te čaka",
  inLead: "Dvanajst tankov, sedem bongov in bojišče, ki okoli tebe gori, se kadi in razpada.",
  campaign: "Kampanja in prosta igra",
  campaignText: "Stopnjo za stopnjo ali katerikoli zemljevid s toliko sovražniki, kolikor si upaš.",
  coop: "Skupaj na spletu",
  coopText: "Odpri sobo, deli njeno kodo in se bori z ramo ob rami, do osem tankov.",
  build: "Zgradi svoje bojišče",
  buildText: "Nariši zidove, vodo in gozd, nato zaigraj na svojem bojišču.",
  takeTitle: "Vzemi jo s seboj",
  takeLead: "Ista igra na vsakem zaslonu. Brskalnik je le najhitrejša pot do nje.",
  ios: "iPhone in iPad",
  iosText: "Levi palec vozi, desni strelja. Deluje tudi brez povezave.",
  iosLink: "Prenesi iz trgovine App Store",
  iosSoon: "Kmalu v trgovini App Store.",
  latest: "Zadnja izdaja",
  whatsNew: "Novosti",
  allReleases: "Vse izdaje",
  newsTitle: "Prejmi sporočilo, ko izide nova različica",
  newsLead: "Novi tanki, novi zemljevidi, novi načini, kako kaj razstreliti.",
  email: "E-pošta",
  subscribeButton: "Naroči se",
  kitNote: "Pošilja Kit. Odjava je v vsakem sporočilu.",
  newsDone: "Še korak: potrdi naročnino v svojem nabiralniku.",
  newsFailed: "Ni uspelo. Poskusi znova čez trenutek.",
  support: "Pomoč",
  privacy: "Zasebnost",
  source: "Izvorna koda",
  backToGame: "▲ Nazaj v igro",
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

type Words = Exclude<keyof Strings, "preparing">;

/** The word for `key` in the language picked by `applyStrings` - English
 *  on a page that never picks one. */
export function t(key: Words): string {
  return TABLES[current][key];
}

/** The loading panel's progress line. */
export function preparing(done: number, total: number): string {
  return TABLES[current].preparing(done, total);
}

/** The key an element names in `attribute`, if it is a word the table has. */
function wordOf(el: HTMLElement, attribute: string): Words | undefined {
  const key = el.getAttribute(attribute);
  return key && key !== "preparing" && key in TABLES.en ? (key as Words) : undefined;
}

/** Pick the language and put its words into every `data-i18n*` element,
 *  the document's `lang` and its title. Called before anything else on
 *  the page writes text, so nothing flashes English first. */
export function applyStrings(): Lang {
  current = pickLanguage();
  const table = TABLES[current];
  document.documentElement.lang = current;
  document.title = table.title;
  for (const el of document.querySelectorAll<HTMLElement>("[data-i18n]")) {
    const key = wordOf(el, "data-i18n");
    if (!key) continue;
    if (HTML_KEYS.has(key)) {
      el.innerHTML = table[key];
    } else {
      el.textContent = table[key];
    }
  }
  const attributes: [string, string][] = [["data-i18n-title", "title"], ["data-i18n-alt", "alt"], ["data-i18n-label", "aria-label"]];
  for (const [marker, attribute] of attributes) {
    for (const el of document.querySelectorAll<HTMLElement>(`[${marker}]`)) {
      const key = wordOf(el, marker);
      if (key) el.setAttribute(attribute, table[key]);
    }
  }
  return current;
}
