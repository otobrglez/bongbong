# PRD: bongbong in more than one language

Status: agreed with the owner 2026-09-29 (the decisions are in section 8),
surveyed against `feature/coop-ng-2`. Nothing is built. The font is decided by
the phase 0 screenshots; three smaller questions in section 8 stay open with a
recommendation each.

Contents

1. The decision, in one paragraph
2. Goals and non-goals
3. What we have today (verified 2026-09-29)
4. Design, by area
5. Numbers
6. Phased plan
7. Risks, mitigations, stop conditions
8. Decisions for the owner
9. References

## 1. The decision, in one paragraph

Every string a player reads moves out of the code and into one catalogue per
language, embedded in the binary the way `SHIPPED_MAPS` embeds maps, resolved
through a headless `text` module that `hud.rs`, `lobby.rs`, `editor/` and
`net/round.rs` read the way they read `tuning()`. The catalogue format is
Fluent, because the author's own language has a dual and Fluent's plural rules
already know that. raylib's default font is replaced by one pixel font family
whose atlas is built at startup from the codepoints the loaded language actually
uses, so a language costs a text file and no art. The language comes from the
platform - the OS on desktop, iOS and Android, the page's `navigator.language`
on the web - overridable by `--lang` and `?lang=`; there is no in-game picker.
English and Slovenian ship first. The room server never sees a translated string: its refusals
become codes the client renders. English stays the source language and the only
spelling of every key, protocol word and data name (`Tool::name`,
`Mission::name`, `TankKind::name`, the map format), so the dev tools, the probe
and the wire are untouched.

## 2. Goals and non-goals

Goals

- A new language is one `.ftl` file plus, where its script needs it, a font
  pack. No Rust change, no layout change, no art.
- Every platform the game ships on picks the player's language on its own:
  macOS, Linux, Windows (`bongbong` archives), the web (bongbong.io and the PR
  previews), iOS (TestFlight), Android (the APK).
- The same picture on every platform for the same language: the text module and
  the font are shared code, not per-platform code.
- Layout does not break when a label is longer than its English: every text box
  has a budget, every language is checked against every budget in `cargo test`,
  and a label that still does not fit shrinks or truncates by a rule rather
  than overflowing into its neighbour.
- Scripts beyond Latin render: Cyrillic and Greek from the first release,
  Chinese, Japanese and Korean when their font pack is added.
- The room server, the probe, the dev server, `bbmcp`, the map format and the
  seeded replays are unchanged: not one string in `simulation/` or `net/wire.rs`
  is human language.

Non-goals

- **No right-to-left text** (Arabic, Hebrew) and **no shaped scripts** (Arabic,
  Devanagari, Thai). raylib draws one glyph per codepoint left to right and
  knows nothing of bidi or shaping. Adding a shaping engine is a separate
  project; the design leaves the door open by keeping every draw behind
  `render/text.rs`.
- No locale formatting of numbers, dates or units. The game shows ping in
  milliseconds and a countdown in seconds as plain digits; that stays.
- No translation of map files, map names, chassis data keys or the dev-server
  protocol. `maps/*.toml` stay English; a map's *display* name is a catalogue
  lookup keyed by its slug where one exists and the slug itself otherwise.
- No voice, no audio (there is none).
- No translation of chat. `Said` is not drawn today; when it is, it is user
  content and passes through untouched.
- **No in-game language picker and no saved preference** (owner's decision,
  section 8). The game speaks the platform's language; `--lang` and `?lang=`
  are the developer's and the tester's overrides. A picker would need a
  settings surface the game does not have, and a phone already has one. The
  design keeps the switch cheap - the catalogue is a snapshot swapped at the
  frame boundary, the atlas a rebuild - so a picker can be added later without
  reopening anything here.
- Not the store listings. App Store and Play Store descriptions are localized
  in their consoles, not in this repo.

## 3. What we have today (verified 2026-09-29)

**No localization exists.** No locale, language or i18n code in `src/`,
`server/`, `site/` or `tools/`; every string is an English literal at the draw
site. About 200 of them:

| Where | Roughly | Examples |
|---|---|---|
| HUD bar and round (`render/hud.rs`, `render/game.rs`, `hud.rs`, `mode.rs`) | 30 | `SPEED`, `SHIELD`, `FROG`, `LEAVE`, `YOU WIN`, `YOU LOSE`, `Restarting in`, `Back to the lobby in`, `PAUSED`, the two dialogs |
| Mission and wave banners (`level.rs:29-35`, `simulation/waves.rs:118-123`) | 5 | `PROTECT THE FROG!`, `HUNT THE FROG!`, `DESTROY!`, `WAVE N`, `FINAL WAVE` |
| Lobby (`lobby.rs`, `render/lobby.rs`) | 40 | six stage titles, eight subtitles, `HOST A ROOM`, `I'M READY`, `REMATCH`, `+N MORE`, `AWAY`/`HOST`/`READY`/`WAITING` |
| Online status and notes (`net/round.rs:579-604`, `net/client.rs`, `net/native.rs`, `net/web.rs`) | 25 | `CONNECTING`, `ASKING FOR A SEAT`, `OFFLINE: {reason}`, the close reasons |
| Room server refusals (`server/src/{conn,hub,room}.rs`) | 25 | `room full`, `the host removed you from this room`, `no room X here` |
| Editor (`editor/mod.rs`, `editor/render.rs`) | 80 | the bar, 33 dropdown labels, 12 settings rows, the popups, status messages, `map.rs` load errors |
| Touch hints, labels, version | 5 | `DRAG TO STEER`, `TAP TO FIRE`, `P1`..`P8` |

Three facts shape the design more than the count:

1. **The font cannot show most languages.** Nothing loads a font; every draw is
   `draw_text` with raylib's default (`render/lobby.rs:5-8` says so). That font
   is 224 glyphs, U+0000..U+00FF (`rtext.c:163-166`): Basic Latin and the
   Latin-1 Supplement. Slovenian's č, š and ž are Latin Extended-A and draw as
   `?` today, as does every Cyrillic, Greek and CJK codepoint. A custom font is
   not an option in this project, it is the first step.
2. **Width is guessed, not measured, and guessed in bytes.** The HUD uses
   `CHAR_W = 12` at size 18 and `CHAR_W_SMALL = 7` at size 10
   (`render/hud.rs:38-41`), buttons `label.len() * 11` (`render/hud.rs:455,490`),
   the lobby `CHAR_W_18 = 11` (`render/lobby.rs:39`), touch hints `len * 5`
   (`touch.rs:262`). `.len()` is bytes, so `ČAKAM` is already mis-centred. Only
   the banners, the status line and the version stamp go through
   `rl.measure_text` (`render/game.rs:198-247`), and only because they are drawn
   where a `RaylibHandle` is in reach. The editor alone counts characters and
   truncates with `~` (`editor/render.rs:41-53`).
3. **Some text is built where no language belongs.** `waves.rs` formats
   `WAVE N`/`FINAL WAVE` inside the simulation; `Mission::banner` in `level.rs`
   is an English sentence; the HUD title is `mission.name().to_ascii_uppercase()`
   - a data key shown as a word; the lobby's chassis column is `TankKind::name`
   from `tuning::TANK_NAMES`; the settings popup prints `Theme::name`,
   `SpawnKind::name`, `Tier::name`. And the room server sends its refusals as
   English sentences in `Lobby::Error { message }` (`net/wire.rs:817`), which the
   lobby shows verbatim as its subtitle (`lobby.rs:525`).

What is already right:

- Text input is Unicode: `get_char_pressed` yields `char`s (`app.rs:1228,1254`),
  the server caps nicknames by characters (`NICK_MAX`, `server/src/room.rs`),
  and the room code entry deliberately accepts ASCII only (`lobby.rs:384-386`),
  which stays: codes are `CODE_ALPHABET`, not words.
- Every asset directory ships whole: `--preload-file static@/static` on the
  web, `bundle.sh` on iOS, `package.sh` on Android. A `static/fonts/` and the
  embedded catalogues need no build change on any platform.
- The page-to-game channel exists (`page_string`, `app.rs:125-137`, the
  `bbInvite`/`bbToken` pattern) and so does a native-only thread of platform
  glue (`app/ios.rs`, `app/android.rs`) where a locale read belongs.
- The headless models are in place: `HudModel::gather`, `LobbyView`,
  `PlayChrome`, `BuilderInput`. Localization is a change to what those models
  carry, and the painters keep drawing what they are handed.

## 4. Design, by area

### 4.1 The catalogue: `src/text.rs`

One module, headless, compiled into every build including the server (which
uses only its key type). It owns:

- `Lang`: the language tag as a Unicode BCP 47 identifier (`unic-langid`), with
  the list of shipped languages `SHIPPED_LANGS: &[(&str, &str)]` - tag and
  `include_str!("../lang/<tag>.ftl")`, the `SHIPPED_MAPS` pattern. English is
  first and is the fallback for every missing message.
- `Catalogue`: a `FluentBundle` for the chosen language layered over English
  (a lookup falls through to English per message, never per language, so a
  half-translated file ships a mixed screen rather than an English one - the
  translator sees what is left). Built with `FluentBundle::new_concurrent`, so
  the snapshot below can cross threads.
- `text()`: a shared `Arc<Catalogue>` snapshot cloned under a lock that is
  released at once - `tuning()`'s discipline exactly, including the rule that
  a write happens only at the frame boundary (`app.rs` applies a pending
  language change beside `tuning::apply_pending`). Tests build local
  `Catalogue`s and never touch the global.
- The lookups: `text().get("lobby-host")` for a plain message,
  `text().fmt("lobby-more", &[("n", 3)])` for one with arguments. The key set
  is generated: `tools/textkeys.py` (or a build-time test) reads `lang/en.ftl`
  and pins `text::keys::LOBBY_HOST` style constants, so a typo is a compile
  error rather than an English fallback nobody notices. A message missing in
  English is a test failure.

**Why Fluent.** The strings with a number in them - `+N MORE`, `1 PLAYER`/`2
PLAYERS`, `Restarting in N`, `SEAT N`, `WAVE N`, `a-b of n` - are the ones a
key-value format gets wrong, and they get wrong in exactly the languages this
game's author speaks: Slovenian has four plural categories (one, two, few,
other), Russian and Polish three with rules on the last digit, French counts
zero as singular. Fluent carries CLDR plural rules (`intl_pluralrules`) and
lets the translator write the selector in the file:

```
lobby-more = { $n ->
    [one] +{ $n } MORE
   *[other] +{ $n } MORE
}
```

The whole stack (`fluent-bundle`, `fluent-syntax`, `fluent-langneg`,
`unic-langid`, `intl_pluralrules`, `intl-memoizer`) is pure Rust and builds for
`wasm32-unknown-emscripten`; it adds about ten crates and no C. The
alternative - TOML with `{n}` placeholders and a hand-written plural function
per language - is smaller and is what many indie games do; it is the right
answer if the language list is fixed at English plus two or three Western
European languages, and the wrong one the day Slovenian, Russian or Polish is
added. Slovenian is the second language this game ships, so the choice made
itself (section 8).

**Style rules the catalogue enforces.**

- The game's chrome is in capitals by design (`HOST A ROOM`, `YOU WIN`). The
  translator writes the final form; **the code never uppercases at runtime**.
  `to_ascii_uppercase` on a data key is what the HUD title does today and goes
  away: `Mission::name` stays the TOML spelling and `mission-protect-title` is
  its word. Runtime case mapping is locale-dependent (Turkish dotless i, German
  ß) and a capitals-only language pack for Georgian or CJK is meaningless.
- Every message carries a comment with where it is drawn and its **budget** in
  cells (section 4.3), so a translator sees `# HUD bar, 40 px slot, 5 cells`
  above `hud-speed = SPEED`.
- Placeholders are named (`{ $code }`, `{ $seat }`, `{ $ms }`), never
  positional, so a translator can reorder them.

### 4.2 The font: `render/text.rs`

raylib's default font goes; one pixel font family comes in, loaded from
`static/fonts/` through `load_font_from_memory(.., font_size, Some(chars))`
(`sola-raylib/raylib/src/core/text.rs:189`). The `chars` argument is what
makes this cheap: **the atlas is built from the codepoints the loaded catalogue
contains**, plus ASCII, the digits, `CODE_ALPHABET` and the punctuation the
status line uses. A 200-message language is a few hundred codepoints in Latin
or Cyrillic and one to two thousand in Chinese - one texture either way, built
once at startup and again on a language change.

**Which font.** The candidates, all free for commercial use (verify the
`LICENSE` in the release vendored):

| Font | Licence | Scripts | Native size | Note |
|---|---|---|---|---|
| Fusion Pixel (TakWolf) | OFL 1.1 | Latin, Latin Ext, Cyrillic, Greek, Chinese (Simplified and Traditional), Japanese, Korean | 8, 10, 12 px | Ships a **monospaced** and a proportional build; one family for every script this PRD admits. The recommendation. |
| Ark Pixel | OFL 1.1 | Latin, Cyrillic, Greek, CJK | 10, 12, 16 px | Fusion Pixel's ancestor; the same coverage, fewer sizes. |
| Press Start 2P | OFL 1.1 | Latin, Cyrillic, Greek | 8 px | The classic arcade look, but every glyph is a full square: `HOST A ROOM` would not fit a 200 px button. |
| GNU Unifont | GPLv2+ with font exception, or OFL 1.1 | The whole Basic Multilingual Plane | 16 px (8 px half-width) | The universal fallback; utilitarian rather than arcade. |

The recommendation is Fusion Pixel's **monospaced** build at its 10 px size:
Latin, Cyrillic and Greek glyphs are half-width (5 px), CJK full-width (10 px).
That is the property section 4.3 rests on - width is a count, not a
measurement. The look changes: the default font is a 10 px design too, so the
sizes in use map to whole multiples (10 small, 20 for the bar's 18, 30 for the
dialog title's 28, 40 for the code entry, 50 and 70 for the banners' 48 and
72) and every draw scales the one atlas by an integer, which raylib does
crisply with nearest filtering. This is a visual pass over the HUD, the lobby
and the editor bar, and it is done **first, as a spike with screenshots** of
the same screens in the current font and two candidates, published for a
decision before any string moves (the project's rule for visual choices).

**Packs.** The Latin/Cyrillic/Greek subset of a 10 px pixel TTF is on the order
of 100 to 200 KB and ships in every build. The CJK glyphs are megabytes and
are a second file, `static/fonts/cjk-10.ttf`, added only with the first CJK
language (phase 3). **When it comes it ships in every download** (owner's
decision): `--preload-file static@/static` takes it like every other asset,
one loading path on every platform, at the cost of one to three megabytes on
a `.data` file that already carries the sheets. `tools/fontpack.py` (pyftsubset under the
same nix invocation the sprite generators use) writes both files from the
vendored full font, subset to the scripts each pack carries; it is a generator
like `gen_tanks.py`, run by hand, its outputs committed.

**Glyphs the atlas lacks** draw as `?` - raylib's behaviour, kept. Two places
can hit it: nicknames and, later, chat. When a roster arrives with a codepoint
the atlas has not got, `render/text.rs` rebuilds the atlas with the roster's
codepoints added, at most once per roster change; a codepoint outside every
shipped pack stays `?`. Section 8 asks whether to restrict nicknames instead.

**The API the painters see** is small and mirrors what they do now:
`draw(d, &str, x, y, size, color)`, `draw_centered(d, &str, rect, size,
color)`, `fit(&str, cells, size) -> Cow<str>` (truncate with `~` as the editor
does), and the headless `width(&str, size)` from `text.rs`. `MeasureTextEx` is
never needed, so a closure without a `RaylibHandle` can centre text, which is
what `render/game.rs:941-944` cannot do today.

### 4.3 Width, budgets and the fixed layouts

The bar, the lobby panel and the editor bar are fixed slot tables pinned by
tests (`render/hud.rs` `SLOTS_*` and `bar_tests`, `lobby.rs` `button_rect` and
`lobby_tests`, `editor/mod.rs` `SLOT_*` and `editor::render::bar_tests`). They
stay fixed - a phone needs buttons that do not move - and the text adapts to
them, by three rules:

1. **Width is a headless function of the string.** `text::width(s, size)` sums
   a per-codepoint advance: half a cell for every codepoint East Asian Width
   calls narrow, a whole cell for wide and fullwidth, zero for combining marks.
   This replaces every `len() * CHAR_W` and `chars().count() * 0.61`, and is
   the same number the font draws because the font is monospaced per class.
   `hud.rs`, `lobby.rs` and `editor/` can centre, right-align and fit without
   raylib, and their tests can assert on it.
2. **Every box has a budget** in cells, stated once beside the geometry it
   comes from (`MODE_BUTTON_W = 72` at size 20 is 7 cells; a 40 px `BAR_SLOT_W`
   at size 10 is 8 cells; a 200 px lobby button at size 20 is 20 cells minus
   padding). `text_tests::every_language_fits_every_budget` loads each shipped
   catalogue, resolves each message with its widest plausible arguments (the
   largest seat number, a five-letter code, `999 MS`) and fails naming the
   language, the key, the width and the budget. A language file that does not
   fit does not merge, the way a probe fixture over its ceiling does not.
3. **Where a budget is tight by nature, the fit rule is declared**, not
   improvised: the bar's small labels (`SPEED`, `SHIELD`, `FROG`) fall to the
   next smaller size before truncating; button labels truncate with `~` at
   their budget; titles and subtitles in the lobby wrap to a second line, for
   which the panel's `Room` face has the room (the subtitle sits at
   `bottom.y - 18` today, one line); banners are measured and centred already.
   The translator's comment names the rule, so `HOST A ROOM` in German
   (`RAUM ERSTELLEN`, 14 cells in 20) is written to fit rather than trusted to
   the truncator.

The two `bar_tests` that pin English widths (`"DESTROY 12/12".len() * CHAR_W`,
`"BUILD".len() * 11`) become budget assertions over every language.

### 4.4 Strings that leave the simulation and the wire

- `waves.rs` stops formatting text. `RoundState`/`Game` expose the banner as
  data - `WaveBanner { wave: u16, is_final: bool, time_left }` - and the HUD
  model formats it. The wire already carries the number and the timer, so
  `net/encode.rs` and the replica are unchanged; the `WAVE N` string never
  travelled.
- `Mission::banner()` becomes a key (`mission-protect-banner`); `Mission::name`,
  `SpawnKind::name`, `Tier::name`, `Theme::name`, `TankKind::name`,
  `Tool::name` and `PickupKind`'s spellings **stay English**. They are the
  map format, the CLI, the dev-server protocol and the probe's output. Each
  gets a display lookup keyed by the data name: `tank-scout`, `theme-desert`,
  `tool-sandbag`, `mission-hunt`. The editor's `label()`/`short_label()`
  tables become such lookups.
- **The room server speaks codes.** `Lobby::Error { message: String }` becomes
  `Lobby::Error { code: Refusal, args: Vec<String> }` with `Refusal` an enum
  in `net/wire.rs` (`RoomFull`, `RoomGone { code }`, `Kicked`, `AlreadyStarted`,
  `ServerDraining`, `BadMap { detail }`, ...), and `ClientEvent::Refused`
  carries it; the lobby resolves `refusal-room-full` from the catalogue. The
  client's own close reasons (`net/client.rs:317`, `net/native.rs`,
  `net/web.rs`, `net/transport.rs`) become a `CloseReason` enum the same way.
  This is a `PROTOCOL_VERSION` bump, shipped with the version tag that ships
  the client and the image together, as every protocol change is. Free-text
  detail (a TOML parse error in `BadMap`) stays a string argument and is shown
  as is: it is for the developer, not the player.
- `OnlineRound::status()` and `note` (`net/round.rs`) are presentation and
  resolve through `text()`; `net/` is headless and the catalogue is headless,
  so nothing moves.
- Nothing in `simulation/` ever calls `text()`. A test greps for it, the way
  `determinism_tests` guard the RNG.

### 4.5 Where the language comes from

Resolution order, first match wins, once at startup:

1. An explicit request: `--lang <tag>` on native; `?lang=<tag>` on the web,
   read through `Invite::parse`'s sibling so a preview link can carry it. This
   is how a tester sees Slovenian on an English machine and how `lang-shots`
   renders every language; a player never needs it.
2. **The platform's language list**, which is the whole story for a player:
   - **Desktop**: `sys-locale` (macOS through CoreFoundation, Windows through
     `GetUserDefaultLocaleName`, Linux from `LC_ALL`/`LC_MESSAGES`/`LANG`). One
     small pure-Rust crate; a hand-rolled `LANG` read is the fallback if it
     is refused.
   - **Web**: the page publishes `window.bbLang = navigator.languages.join(",")`
     in `site/src/scripts/room.ts` next to `bbInvite`, and `app.rs` reads it
     once through `page_string`. The page's own `<html lang>` follows the same
     value.
   - **iOS**: `SDL_GetPreferredLocales()` - the app already runs on SDL3, the
     header is in the vendored prefix (`SDL3/SDL_locale.h`), one FFI
     declaration in `app/ios.rs`.
   - **Android**: `AConfiguration_getLanguage`/`getCountry` off
     `GetAndroidApp()->config` (raylib exports the handle,
     `rcore_android.c:349`), with `persist.sys.locale` through the
     `__system_property_get` `app/android.rs` already declares as the
     fallback.
3. English.

Matching is Fluent's language negotiation (`fluent-langneg`): the platform's
list against `SHIPPED_LANGS`, so `sl-SI` finds `sl`, `pt-PT` finds `pt-BR`
before English, `zh-TW` finds `zh-Hant` and never `zh-Hans`. The chosen tag is
in `status` (dev server) and the version stamp line, so a bug report says which
language the screen was in.

### 4.6 Switching at runtime

There is no picker (section 2), but the switch itself is built, because the
dev server's `lang` tool and `lang-shots` need it: a language change is
staged like a tuning patch and applied at the next
frame boundary - the catalogue snapshot swaps, the atlas rebuilds, and the
models re-gather on the next frame, since nothing else holds a string. If a
picker is ever wanted, it is a `LANGUAGE` row in the players dialog and a
stepper on the lobby's `Start` face over this switch, and nothing more.

### 4.7 The web page and platform metadata

- `site/src/pages/index.astro` has a dozen strings (the loading panel, the
  full-screen label, the key hints, the footer) and `lang="en"`. Phase 3 gives
  `site/src/scripts/` a `strings.ts` keyed by the same tags and sets
  `document.documentElement.lang`. No routing per language: the game is one
  page.
- The app name **BongBong** is a brand and is not translated, so
  `Info.plist`'s `CFBundleDisplayName` and the Android `android:label` stay as
  they are. iOS gets `CFBundleLocalizations` listing the shipped tags so the
  system knows the app speaks them; Android needs nothing.
- Key hints (`P2 WASD + L.Shift`, the editor's `Enter to save, Esc to cancel`)
  name physical keys. They are translated as words, and the AZERTY problem
  (WASD is ZQSD there) is a keyboard-layout question, not a language one, and
  stays out of scope.

### 4.8 Tooling and tests

- `lang/en.ftl` is the source of truth. A new string is a message in it plus
  a generated key constant; the constant's absence is the compile error that
  finds every draw site.
- `text_tests`: every shipped language parses; every message English has, the
  language has or falls back (a report, not a failure, so a partial language
  can ship on purpose - the failure is a message in a language that English
  has not got); every budget is met (4.3); every placeholder English uses, the
  translation uses; no message is unreferenced by a key constant; `simulation/`
  and `net/wire.rs` never name `text()`.
- `hud_tests`, `lobby_tests` and the two `bar_tests` run their fit assertions
  over `SHIPPED_LANGS`, not over English.
- The dev server takes `lang` on `restart` and as a live tool (`Live`, since
  the atlas rebuild is a frame-boundary write), and `just lang-shots` renders
  the HUD, the two dialogs, every lobby face and the editor bar in every
  language to `target/lang-shots/<tag>/`, the thumbnail recipe's shape - the
  review surface for a translation PR, since a test cannot judge whether
  `RAUM ERSTELLEN` reads well in a 200 px button.
- Translation workflow: a draft per language generated from `en.ftl` with its
  comments (the budget and the screen the string is on are the context a
  machine translator needs), reviewed by a speaker, checked by the tests and
  the screenshots. Weblate or Crowdin read Fluent natively if a community
  ever contributes; nothing here depends on either.

## 5. Numbers

| What | About |
|---|---|
| Player-visible strings today | 200 |
| Catalogue per language | 10 to 15 KB of `.ftl`, embedded |
| Latin + Cyrillic + Greek font pack, 10 px | 100 to 200 KB, in every build |
| CJK font pack, 10 px, subset to the common-use sets | 1 to 3 MB, phase 3, in every download once added |
| Atlas at startup | one texture; under 512 x 512 for a CJK language, far less for Latin |
| New crates | Fluent's six plus `sys-locale`; all pure Rust, wasm-clean |
| Protocol | one `PROTOCOL_VERSION` bump (refusal codes) |
| Draw sites to convert | roughly 120 `draw_text` calls across `render/`, `editor/render.rs`, `touch.rs`, `app.rs` |

## 6. Phased plan

0. **Font spike** (a day or two). Load two candidate fonts behind a flag, draw
   the bar, the lobby `Room` face, a dialog and the editor bar in each and in
   the current font, publish the comparison, decide. Nothing merges but the
   decision and the vendored font.
1. **English, localizable.** `text.rs`, `render/text.rs`, `lang/en.ftl`, the
   generated keys, every literal moved, `width()` replacing every estimate,
   budgets and their test, the wave banner and mission banner moved out of
   `simulation/`, refusal codes on the wire (protocol bump). Ships as an
   English-only release that looks different (the font) and behaves the same.
   This is the large mechanical change and touches `lobby.rs`, `hud.rs` and
   `net/round.rs`, which the co-op branch is still editing: **it starts after
   `feature/coop-ng-2` lands**, or it is a merge nobody wants.
2. **Platform language and Slovenian.** `--lang`, `?lang=`, the four platform
   reads, negotiation, and `lang/sl.ftl` - the one translation of the first
   release, reviewed by the author, with its `lang-shots` in the PR. The
   Slovenian pass is what proves the budgets and the fit rules on a real
   language before any other is drafted; a further language is a PR of one
   file each, whenever one is wanted.
3. **CJK and the page.** The second font pack in every download,
   `zh-Hans`/`ja`/`ko` when asked for, `site/` strings and `<html lang>`.

## 7. Risks, mitigations, stop conditions

- **The font changes the game's face.** Every screen looks different after
  phase 1. Mitigation: phase 0 decides on screenshots, not in the abstract;
  the sizes map to integer multiples so nothing blurs. Stop condition: if no
  candidate reads as well as the default at 10 px in the bar's small labels,
  keep raylib's default for `[A-Za-z0-9]` and use the pixel font only for
  glyphs it lacks - a two-font fallback `render/text.rs` can hide behind the
  same API, at the price of two atlases.
- **German, Russian and Finnish are long.** Budgets catch it in CI; the fit
  rules degrade gracefully; the translator sees the budget in the file.
  Residual risk is a screen that fits and reads badly, which `lang-shots`
  exists to show.
- **Web download grows.** Phase 1 adds under 200 KB to a `.data` file that
  already carries the sheets. CJK is the real cost, decided (section 8) as a
  one-to-three megabyte addition to every download when it arrives; measure
  the `.data` size in that PR and say it in the release notes.
- **A protocol bump strands old clients.** Already the rule: client and image
  ship on one version tag and refuse each other by name on a mismatch.
- **Merge conflict with the co-op branch.** Phase 1 rewrites the files the
  branch is in. Sequenced after it, above.
- **`?` in nicknames.** A Thai nickname on a Latin atlas is `?????`. The
  roster-time atlas extension covers every script a shipped pack has; beyond
  that, either accept `?` or restrict the alphabet at the server (section 8).
- **Fluent's dependency tree.** Six crates for plurals, decided (section 8).
  The `text()` API hides the format, so if the tree ever becomes a problem
  the eight plural messages are the whole cost of a hand-rolled replacement.
- **Translations rot.** A new English message with no translation falls back
  to English per message, visibly, and the completeness report names it; the
  screenshot recipe makes a partial language reviewable. Nothing crashes on a
  missing key.

## 8. Decisions

Taken by the owner on 2026-09-29:

1. **Format: Fluent.** Plural rules for Slovenian, Russian and Polish come
   with the crate; translators' tools read the files. The TOML alternative
   was considered and rejected as right only for a short Western European
   list.
2. **First release: English and Slovenian.** No tier-1 list. One font pack,
   one translation the author can review, and the budgets proven on a real
   language before any other is drafted. Further languages are one file each,
   added when wanted.
3. **CJK ships in every download** once a CJK language is added. One loading
   path on every platform; the megabytes are accepted.
4. **No in-game picker.** The platform's language, with `--lang` and `?lang=`
   as the overrides for testing. No saved preference either, since there is
   nothing to save.

Open, each with the recommendation the text above proceeds under:

5. **Font**: Fusion Pixel monospaced 10 px (one family, every script,
   half/full-width so width is a count), or Ark Pixel, or Press Start 2P for
   the look at the cost of every budget. Decided on the phase 0 screenshots.
6. **Nicknames**: any Unicode, `?` beyond the shipped scripts (recommended,
   keeps the server ignorant of fonts), or an alphabet the server enforces.
7. **Capitals**: the chrome stays all-capitals in every language, written so by
   the translator (recommended), or the design drops capitals now that
   `to_uppercase` is off the table.
8. **Translation source**: machine-drafted and speaker-reviewed (recommended -
   for Slovenian the reviewer is the author), or speaker-written from the
   start.

## 9. References

- `docs/hud-and-builder-layout-design.md` - the fixed slot tables the budgets
  are derived from.
- `docs/online-coop-prd.md` §4.10 (the lobby), §4.7 (the room server's
  refusals), §4.4 (`PROTOCOL_VERSION`).
- `docs/runtime-tuning-design.md` - the `tuning()` snapshot and
  frame-boundary write `text()` copies.
- `docs/mapshot-prd.md` - the generator-and-committed-output pattern the font
  packs and `lang-shots` follow.
- raylib `rtext.c` (`LoadFontDefault`, 224 glyphs; `LoadFontFromMemory`
  with a codepoint list), `sola-raylib/raylib/src/core/text.rs:189`.
- Project Fluent: <https://projectfluent.org/>, `fluent-rs`
  (<https://github.com/projectfluent/fluent-rs>).
- Fusion Pixel Font (<https://github.com/TakWolf/fusion-pixel-font>), Ark
  Pixel Font (<https://github.com/TakWolf/ark-pixel-font>), Press Start 2P,
  GNU Unifont.
- Unicode East Asian Width (UAX #11) - the half/full-width rule `width()`
  implements.
- SDL3 `SDL_GetPreferredLocales`, Android NDK `AConfiguration_getLanguage`,
  `sys-locale` crate.
