# PRD: bongbong in more than one language

Status: agreed with the owner 2026-09-29 (the decisions are in section 8)
and built the same day on this branch - phases 1 and 2; section 8b lists
where the build departs from the text. Three smaller questions in
section 8 stay open with a recommendation each.

Contents

1. The decision, in one paragraph
2. Goals and non-goals
3. What we have today (verified 2026-09-29)
4. Design, by area
5. Numbers
6. Phased plan
7. Risks, mitigations, stop conditions
8. Decisions
8b. As built
9. Parked: a font for other scripts
10. References

## 1. The decision, in one paragraph

Every string a player reads moves out of the code and into one catalogue per
language, embedded in the binary the way `SHIPPED_MAPS` embeds maps, resolved
through a headless `text` module that `hud.rs`, `lobby.rs`, `editor/` and
`net/round.rs` read the way they read `tuning()`. The catalogue format is
Fluent, because the author's own language has a dual and Fluent's plural rules
already know that. **raylib's default font stays.** It draws Basic Latin and
Latin-1, and every letter outside that set folds to its base letter before it
is drawn - Č to C, Š to S, Ž to Z - by one table in the text module, so a
language costs a text file and no art, and the scripts the game can show are
the Latin ones until a font is ever adopted (parked, section 9). The language comes from the
platform - the OS on desktop, iOS and Android, the page's `navigator.language`
on the web - overridable by `--lang` and `?lang=`; there is no in-game picker.
English and Slovenian ship first. The room server never sees a translated string: its refusals
become codes the client renders. English stays the source language and the only
spelling of every key, protocol word and data name (`Tool::name`,
`Mission::name`, `TankKind::name`, the map format), so the dev tools, the probe
and the wire are untouched.

## 2. Goals and non-goals

Goals

- A new language is one `.ftl` file plus, where its alphabet needs it, rows in
  the fold table. No Rust change beyond that, no layout change, no art.
- Every platform the game ships on picks the player's language on its own:
  macOS, Linux, Windows (`bongbong` archives), the web (bongbong.io and the PR
  previews), iOS (TestFlight), Android (the APK).
- The same picture on every platform for the same language: the text module and
  the fold are shared code, not per-platform code.
- Layout does not break when a label is longer than its English: every text box
  has a budget, every language is checked against every budget in `cargo test`,
  and a label that still does not fit shrinks or truncates by a rule rather
  than overflowing into its neighbour.
- Every Latin-alphabet language is drawable with the font the game has: what
  Latin-1 lacks (č, š, ž, ł, ő, ř, ă, ...) folds to the letter the font has,
  and a test proves every shipped message drawable.
- The room server, the probe, the dev server, `bbmcp`, the map format and the
  seeded replays are unchanged: not one string in `simulation/` or `net/wire.rs`
  is human language.

Non-goals

- **No custom font, and so no script beyond Latin** (owner's decision, section
  8). Cyrillic, Greek, Chinese, Japanese and Korean have no glyphs in raylib's
  default font and cannot be folded to letters that mean the same thing. They
  wait for a font, whose design is parked in section 9 so that adopting one
  later reopens nothing else in this document. Right-to-left and shaped
  scripts (Arabic, Hebrew, Devanagari, Thai) are further out still: raylib
  draws one glyph per codepoint left to right and knows nothing of bidi or
  shaping.
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
  frame boundary - so a picker can be added later without reopening anything
  here.
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

1. **The font draws Latin-1 and nothing else.** Nothing loads a font; every
   draw is `draw_text` with raylib's default (`render/lobby.rs:5-8` says so).
   That font is 224 glyphs, U+0000..U+00FF (`rtext.c:163-166`): Basic Latin
   and the Latin-1 Supplement, so é, ü, ñ, ß and à draw, while Slovenian's č,
   š and ž are Latin Extended-A and draw as `?` today, as does every Cyrillic,
   Greek and CJK codepoint. Every string has to be brought inside that set
   before it reaches `draw_text`.
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
  ß), and the translator knows better than a table which words of their
  language carry capitals well.
- Every message carries a comment with where it is drawn, its **budget**
  (section 4.3) and a note that it is drawn folded (section 4.2), so a
  translator sees `# HUD bar, 40 px slot, about 6 letters` above
  `hud-speed = SPEED`.
- Placeholders are named (`{ $code }`, `{ $seat }`, `{ $ms }`), never
  positional, so a translator can reorder them.

### 4.2 The glyphs: the default font and the fold

**No font is added** (owner's decision, section 8). raylib's default font
draws every string, as it does today, and the text module makes every string
drawable by **folding** each codepoint the font has no glyph for onto the
Latin-1 letter it is built on:

```
Č Ć → C    Š → S    Ž → Z    Đ → D    Ł → L    Ő → Ö    Ř → R    Ă → A ...
č ć → c    š → s    ž → z    đ → d    ł → l    ő → ö    ř → r    ă → a ...
```

`text::fold(&str) -> Cow<str>` is one table in `text.rs`: Latin Extended-A
and the parts of Extended-B and Additional that European alphabets use, each
row the base letter (or the Latin-1 letter nearest it - ő to ö, not o, because
ö is in the font). A codepoint in the table folds; a codepoint the font has
(U+0020..U+007E, U+00A0..U+00FF) passes; anything else becomes `?`, which is
what raylib would draw anyway and is now visible in a test rather than on a
phone. The table is hand-written, not a Unicode normalisation crate: the
languages this game ships need a few dozen rows, a new language brings its
rows in the same PR as its `.ftl`, and the test below says which are missing.

**Where it runs.** Once, when the catalogue snapshot is built: every message
is folded as it is resolved, so `hud.rs`, `lobby.rs` and the tests see the
text as the screen will show it and the width rule (4.3) counts the right
letters. And once more on user text as it enters a model - a nickname in the
roster, a map name from a file - through the same function, so nothing that
reaches a painter carries a glyph the font lacks. Painters do not fold; they
draw what the model hands them. Room codes are `CODE_ALPHABET` already and
never touch the table.

**What it costs the reader.** Slovenian without diacritics is legible and
familiar - every Slovene has typed it on a keyboard without them - and in the
capitals the chrome uses (`ZACNI`, `CAKAM`, `IGRALEC 2`) it reads as the same
arcade voice as the English. It is a compromise the translator writes toward:
where a folded word turns ambiguous (`čas`/`cas` is not, `šal`/`sal` is), a
synonym is chosen. The comment above each message says the text will be drawn
folded, so nobody translates for diacritics that will not appear.

**The test that replaces the font work.** `text_tests::every_message_is_drawable`
resolves every message of every shipped language and fails on any codepoint
that is neither in the font nor in the fold table, naming the language, the
key and the character - so adding Polish and forgetting ą is a red test, not
a `?` in the lobby. A sibling test pins the fold of a sentence in each shipped
language.

**The API the painters see** is small and mirrors what they do now:
`draw(d, &str, x, y, size, color)`, `draw_centered(d, &str, rect, size,
color)`, `fit(&str, max_px, size) -> Cow<str>` (truncate with `~` as the editor
does), and the headless `width(&str, size)` from `text.rs`, so a closure
without a `RaylibHandle` can centre text, which is what `render/game.rs:941-944`
cannot do today.

### 4.3 Width, budgets and the fixed layouts

The bar, the lobby panel and the editor bar are fixed slot tables pinned by
tests (`render/hud.rs` `SLOTS_*` and `bar_tests`, `lobby.rs` `button_rect` and
`lobby_tests`, `editor/mod.rs` `SLOT_*` and `editor::render::bar_tests`). They
stay fixed - a phone needs buttons that do not move - and the text adapts to
them, by three rules:

1. **Width is a headless function of the string, and exact.** raylib's default
   font is proportional, and its 224 glyph widths are a constant table in
   `rtext.c` (`charsWidth[224]`, 10 px tall, `DrawText` scaling them by
   `size / 10` and adding a spacing of the same factor). `text::width(s, size)`
   carries that table and that rule, so it returns the pixels `MeasureText`
   would, with no `RaylibHandle`, for a string the fold has already brought
   into the font's set. This replaces every `len() * CHAR_W`, `len() * 11` and
   `chars().count() * 0.61` - all of them guesses, and the byte-counting ones
   wrong for any non-ASCII letter the font does draw. `hud.rs`, `lobby.rs` and
   `editor/` can centre, right-align and fit without raylib, their tests can
   assert on it, and a `render`-only test pins the table against `MeasureText`
   for every glyph, the way `math::raylib_tests` pins the vector arithmetic.
2. **Every box has a budget** in pixels at its size, stated once beside the
   geometry it comes from (`MODE_BUTTON_W = 72` less padding at size 18; a
   40 px `BAR_SLOT_W` at size 10; a 200 px lobby button at size 18). The
   translator's comment carries it as a character count against the font's
   average advance, which is what a person can count.
   `text_tests::every_language_fits_every_budget` loads each shipped
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
   The translator's comment names the rule, so `HOST A ROOM` in Slovenian
   (`USTVARI SOBO`, twelve letters against a 200 px button) is written to fit
   rather than trusted to the truncator.

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
frame boundary - the catalogue snapshot swaps and the models re-gather on the
next frame, since nothing else holds a string. If a
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
- The dev server takes `lang` on `restart` and as a live tool (the catalogue
  swap is a frame-boundary write), and `just lang-shots` renders
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
| Fold table | a few dozen rows for Slovenian and its neighbours; under 200 for every Latin alphabet in Europe |
| Default font width table | 224 constants copied from `rtext.c` |
| Assets added | none; the download size does not move |
| New crates | Fluent's six plus `sys-locale`; all pure Rust, wasm-clean |
| Protocol | one `PROTOCOL_VERSION` bump (refusal codes) |
| Draw sites to convert | roughly 120 `draw_text` calls across `render/`, `editor/render.rs`, `touch.rs`, `app.rs` |

## 6. Phased plan

1. **English, localizable.** `text.rs` (catalogue, fold, width),
   `render/text.rs` (the draw helpers), `lang/en.ftl`, the generated keys,
   every literal moved, `width()` replacing every estimate, budgets and their
   test, the wave banner and mission banner moved out of `simulation/`,
   refusal codes on the wire (protocol bump). Ships as an English-only release
   that looks and behaves exactly as before - the font is the same, the
   centring is now exact where it was a guess. This is the large mechanical
   change and touches `lobby.rs`, `hud.rs` and `net/round.rs`, which the co-op
   branch is still editing: **it starts after `feature/coop-ng-2` lands**, or
   it is a merge nobody wants.
2. **Platform language and Slovenian.** `--lang`, `?lang=`, the four platform
   reads, negotiation, the Slovenian rows of the fold table and `lang/sl.ftl`
   - the one translation of the first release, reviewed by the author, with
   its `lang-shots` in the PR. The Slovenian pass is what proves the budgets,
   the fold and the fit rules on a real language before any other is drafted;
   a further Latin-alphabet language is a PR of one file and its fold rows,
   whenever one is wanted.
3. **The page.** `site/` strings and `<html lang>`, following the same tags.
   Other scripts are not a phase; they are section 9.

## 7. Risks, mitigations, stop conditions

- **Folded Slovenian reads as a compromise.** `ČAKAM` drawn as `CAKAM` is
  legible to every Slovene and looks like the SMS Slovenian everyone has
  typed, but it is not the language written properly, and a reviewer may
  want the diacritics back. Mitigation: the translator writes toward the
  fold (synonyms where a folded word turns ambiguous), the capitals hide most
  of the loss, and the way back is a font (section 9), which changes nothing
  in the catalogue - the `.ftl` keeps its diacritics, only the fold stops
  firing. Stop condition: if folded Slovenian is judged unacceptable on the
  phase 2 screenshots, section 9 is unparked before any third language.
- **German and Finnish are long.** Budgets catch it in CI; the fit rules
  degrade gracefully; the translator sees the budget in the file. Residual
  risk is a screen that fits and reads badly, which `lang-shots` exists to
  show.
- **A protocol bump strands old clients.** Already the rule: client and image
  ship on one version tag and refuse each other by name on a mismatch.
- **Merge conflict with the co-op branch.** Phase 1 rewrites the files the
  branch is in. Sequenced after it, above.
- **`?` in nicknames.** A Cyrillic or Thai nickname folds to nothing and
  draws as `?????`, exactly as it does today. Either accept it, or restrict
  the alphabet at the server to what the fold can draw (section 8).
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
2. **First release: English and Slovenian.** No tier-1 list. One translation
   the author can review, and the budgets proven on a real language before
   any other is drafted. Further languages are one file each, added when
   wanted.
3. **No custom font. Letters outside the default font fold to their base
   letter** - Č to C, Š to S, Ž to Z - for now. The scripts the game can show
   are therefore the Latin ones; the font that would open the others is
   parked in section 9. (An earlier draft of this document proposed a pixel
   font family with a per-language atlas and a CJK pack in every download;
   that decision is withdrawn with the font.)
4. **No in-game picker.** The platform's language, with `--lang` and `?lang=`
   as the overrides for testing. No saved preference either, since there is
   nothing to save.

Open, each with the recommendation the text above proceeds under:

5. **Nicknames**: any Unicode, `?` beyond what the fold can draw (recommended,
   keeps the server ignorant of fonts and is today's behaviour), or an
   alphabet the server enforces.
6. **Capitals**: the chrome stays all-capitals in every language, written so by
   the translator (recommended - it also hides most of what the fold takes
   away), or the design drops capitals now that `to_uppercase` is off the
   table.
7. **Translation source**: machine-drafted and speaker-reviewed (recommended -
   for Slovenian the reviewer is the author), or speaker-written from the
   start.

## 8b. As built (2026-09-29)

Phases 1 and 2 are in the branch this document ships on. What the build
does that the text above does not say, or says differently:

- **`src/text.rs` holds everything**: the catalogue, `fold`, `width`,
  `fit`, the negotiation and the platform hooks. There is no
  `render/text.rs`: every painter measures with `text::width` and draws
  with raylib's `draw_text` as before, so the raylib half needed nothing.
- **Keys are a macro, not a generator.** `keys! { LOBBY_HOST = "lobby-host"; ... }`
  in `text.rs` expands to the constants and `keys::ALL`, and
  `the_keys_and_the_english_file_name_the_same_messages` holds the list
  and `lang/en.ftl` together both ways. A data family (`tool-*`, `tank-*`,
  `mission-*`, `theme-*`, `spawn-*`, `tier-*`, `status-label-*`) is looked
  up by `Catalogue::named` and checked against the code's own lists.
- **No `fluent-langneg`.** Its current release keys on a different tag
  crate than `fluent-bundle`; `text::shipped`/`negotiate` are the few
  lines needed (an exact tag, then the language subtag), so the
  dependency is `fluent-bundle` and `unic-langid` plus `sys-locale` on
  the desktop. `FluentBundle::new_concurrent` with isolation marks off.
- **Width is exact.** The default font's 224 glyph widths were copied from
  `rtext.c` and `width` applies `DrawText`'s scale and spacing, so the
  budgets are pixels and the pins in `render::hud::bar_tests` and
  `editor::render::bar_tests` measure the real string.
- **The refusal codes** are `net::wire::Refusal`, twenty-five variants
  with a `Display` in English for the server's logs and tests, carried
  in `Lobby::Error { refusal }`; `text::refusal` is the client's words.
  The client's own close reasons (`Closed::reason`) stay strings: they
  name addresses and socket errors and are shown as the detail they are,
  under the localized `OFFLINE` label. `PROTOCOL_VERSION` is 7.
- **The status line's label** is `OnlineRound::new`'s `label` looked up
  as `status-label-<label>`: the game's `ROOM` is a word, the rig's `RIG`
  falls back to itself.
- **Android** reads `persist.sys.locale` and `ro.product.locale` through
  the `__system_property_get` the entry already declares rather than
  `AConfiguration_getLanguage`, which needs the activity handle raylib
  does not export.
- **The dev server's `lang` tool** reports and switches the language;
  `status.language` carries it. `just lang-shots` is not built: the review
  flow is `lang {tag: "sl"}` then `screenshot` through `bbmcp`, by hand.
- **Slovenian** (`lang/sl.ftl`) is the author's to review. Where a
  Slovenian word did not fit its budget the shorter one was chosen
  (`TEMPO` for `SPEED` in the 38 px gauge label, `VEN` for the bar's
  `LEAVE` and the `KICK` button, `RAZRED OD`/`RAZRED DO` for the tier
  rows); `UNDO`/`REDO` and `PING`/`MS` are left as they are. The editor's
  `FILE` reads `MENI` and its `MAP` button `IGRA`.
- **The site's own strings** (phase 3) are not touched; the page
  publishes `window.bbLang` and that is all it does for now.

## 9. Parked: a font for other scripts

Kept here so that adopting a font later is a decision about one asset, not a
reopening of this design. Nothing in sections 4.1 to 4.8 depends on the
default font except the fold table and the width table, and both are behind
`text.rs`.

- **Mechanism.** raylib loads a TTF with a chosen glyph set:
  `load_font_from_memory(.., size, Some(chars))`
  (`sola-raylib/raylib/src/core/text.rs:189`). With the catalogue in hand,
  `chars` is every codepoint the loaded language uses plus ASCII and
  `CODE_ALPHABET`, so the atlas is one small texture built at startup - a
  few hundred glyphs for a Latin or Cyrillic language, one to two thousand
  for Chinese. The fold then applies only to codepoints the *loaded* font
  lacks, and `width()` reads the loaded font's advances instead of the
  default's table; every painter and every budget stays as it is.
- **Candidates**, all under licences that allow a commercial game (check the
  vendored release's `LICENSE`): Fusion Pixel (OFL 1.1; Latin, Cyrillic,
  Greek, Chinese, Japanese, Korean; 8, 10 and 12 px; a monospaced build in
  which Latin is half-width and CJK full-width, which keeps width a count),
  its ancestor Ark Pixel (OFL 1.1), Press Start 2P (OFL 1.1; Latin, Cyrillic,
  Greek; square glyphs too wide for the bar's slots) and GNU Unifont
  (GPLv2+ with font exception or OFL 1.1; the whole Basic Multilingual Plane;
  utilitarian). A 10 px design maps the sizes in use onto integer multiples
  (10, 20, 30, 40, 50, 70) and scales crisply with nearest filtering.
- **Costs.** A Latin/Cyrillic/Greek subset is 100 to 200 KB; CJK glyphs are
  one to three megabytes and `--preload-file static@/static` puts everything
  in `static/` into every web download, so a CJK pack is a download-size
  decision as much as a font one. A font changes the game's face on every
  screen, so it is chosen on side-by-side screenshots of the bar, the lobby
  and the editor (the project's rule for visual choices), never in the
  abstract. Glyphs outside the loaded atlas still draw as `?`; a roster with
  an unknown codepoint would rebuild the atlas once per roster change.
- **What unparks it.** Folded Slovenian judged unacceptable (section 7), or a
  request for any non-Latin language.

## 10. References

- `docs/hud-and-builder-layout-design.md` - the fixed slot tables the budgets
  are derived from.
- `docs/online-coop-prd.md` §4.10 (the lobby), §4.7 (the room server's
  refusals), §4.4 (`PROTOCOL_VERSION`).
- `docs/runtime-tuning-design.md` - the `tuning()` snapshot and
  frame-boundary write `text()` copies.
- `docs/mapshot-prd.md` - the render-to-files recipe `lang-shots` follows.
- raylib `rtext.c` - `LoadFontDefault` (224 glyphs, U+0000..U+00FF, the
  `charsWidth[224]` table `width()` copies) and, for section 9,
  `LoadFontFromMemory` with a codepoint list
  (`sola-raylib/raylib/src/core/text.rs:189`).
- Project Fluent: <https://projectfluent.org/>, `fluent-rs`
  (<https://github.com/projectfluent/fluent-rs>).
- Section 9's candidates: Fusion Pixel Font
  (<https://github.com/TakWolf/fusion-pixel-font>), Ark Pixel Font
  (<https://github.com/TakWolf/ark-pixel-font>), Press Start 2P, GNU Unifont.
- SDL3 `SDL_GetPreferredLocales`, Android NDK `AConfiguration_getLanguage`,
  `sys-locale` crate.
