# Slovenščina. Pravila so v lang/en.ftl: velike črke v okvirju igre,
# šumniki se ob risanju zložijo na osnovno črko (Č -> C, Š -> S, Ž -> Z),
# vsak niz ima proračun v pikslih, ki ga preveri `cargo test`.

## Vrstica HUD

button-build = GRADI
button-play = IGRAJ
button-leave = VEN
button-online = SPLET

hud-weapon-offline = IZPAD OROŽJA
hud-rod-aim = NAMERI, NATO SPUSTI ZA UDAR
hud-rod-cancel = SPUSTI TUKAJ ZA PREKLIC
hud-well-anchor = ZNOVA SPROŽI ZA SIDRO
hud-edits-put-away = SPREMEMBE ODLOŽENE - UNDO JIH VRNE

players-title = Koliko igralcev?
players-keys = P1 puščice + preslednica    P2 WASD + levi Shift
players-touch = P1 vleci + tapni    P2 WASD + levi Shift
players-one = 1 IGRALEC
players-two = 2 IGRALCA

question-revert-title = Povrnem { $name }?
question-revert-sub = Tvoje spremembe bodo izbrisane, vrne se izvirnik.
question-revert-yes = POVRNI
question-revert-no = OBDRŽI
question-changed-title = Izvirnik { $name } je nov
question-changed-sub = Preklopim na izvirnik in zavržem tvoje spremembe?
question-changed-yes = PREKLOPI
question-changed-no = OBDRŽI MOJO

## Igra

round-won = ZMAGA
round-trained = OPRAVLJENO!
round-lost = PORAZ
round-restarting = Nova igra čez { $seconds }...
round-back-to-lobby = Nazaj v sobo čez { $seconds }...
paused = PAVZA

mission-protect = BRANI
mission-hunt = LOV
mission-destroy = UNIČI
mission-training = URJENJE

mission-protect-banner = BRANI BONGA!
mission-hunt-banner = ULOVI BONGA!
mission-destroy-banner = UNIČI VSE!
mission-training-banner = SLEDI BONGU!

wave-banner = VAL { $n }
wave-final = ZADNJI VAL

## Stopnje

level-number = STOPNJA { $n } / { $count }
result-time = ČAS { $time }
result-wrecks = UNIČENIH { $n } / { $total }
result-all-clear = VSE STOPNJE SO OPRAVLJENE!
result-again = ŠE ENKRAT
result-next = NASLEDNJA STOPNJA
result-first = NAZAJ NA ZAČETEK
result-levels = STOPNJE
result-next-in = NASLEDNJA ČEZ { $seconds }
result-again-in = ŠE ENKRAT ČEZ { $seconds }
levels-title = STOPNJE
levels-sub = Izberi stopnjo. Zmaga odpre naslednjo.
levels-back = NAZAJ
bar-level = STOPNJA

level-boot-camp = Vadišče
level-lotus-lagoon = Lotosova laguna
level-vulkan = Vulkan
level-glasshouses = Vrt s steklenjaki
level-oasis-bazaar = Bazar v oazi
level-carnival = Karneval
level-jungle-temple = Tempelj v džungli
level-hedge-maze = Labirint živih mej
level-archipelago = Otočje
level-harbor-lights = Luči pristanišča
level-serpent-river = Kačja reka
level-no-mans-land = Nikogaršnje ozemlje
level-scrapyard = Odpad
level-black-gold = Črno zlato
level-castle-moat = Grajski jarek
level-grand-campaign = Veliki pohod

seat-label = P{ $n }

touch-steer = VLECI ZA VOŽNJO
touch-fire = TAPNI ZA STREL

## Soba

lobby-title-start = SPLETNA IGRA
lobby-title-code = PRIDRUŽI SE SOBI
lobby-title-waiting = POVEZOVANJE S SOBO
lobby-title-closed = SOBE NI VEČ
lobby-title-host = TVOJA SOBA
lobby-title-guest = V SOBI

lobby-sub-start = Ustvari sobo in deli kodo, ali se pridruži obstoječi.
lobby-sub-code = Pet znakov iz kode, ki si jo dobil.
lobby-sub-waiting = { $host }...
lobby-sub-closed = Nihče več ne posluša.
lobby-sub-host = Skeniraj kodo ali jo preberi glasno. ZAČNI, ko so vsi pripravljeni.
lobby-sub-guest = Čakamo, da gostitelj začne igro.
lobby-outcome-won = ZMAGA.
lobby-outcome-lost = PORAZ.
lobby-outcome-over = KONEC IGRE.
lobby-rematch-host = REVANŠA, ko so vsi pripravljeni.
lobby-rematch-guest = Čakamo na gostiteljevo revanšo.

lobby-map = ZEMLJEVID
lobby-mission = MISIJA

seat-away = ODSOTEN
seat-host = GOSTITELJ
seat-ready = PRIPRAVLJEN
seat-waiting = ČAKA
seat-empty = PRAZNO
lobby-more = { $n ->
    [one] +{ $n } VEČ
    [two] +{ $n } VEČ
    [few] +{ $n } VEČ
   *[other] +{ $n } VEČ
}

lobby-host = USTVARI SOBO
lobby-join = PRIDRUŽI SE
lobby-back = NAZAJ
lobby-close = ZAPRI
lobby-delete = IZBRIŠI
lobby-confirm = PRIDRUŽI SE
lobby-ready = PRIPRAVLJEN
lobby-im-ready = PRIPRAVLJEN!
lobby-start = ZAČNI
lobby-rematch = REVANŠA
lobby-leave = ZAPUSTI
lobby-kick = ODSTRANI

code-error-length = koda sobe ima { $expected } znakov, ne { $got }
code-error-character = '{ $char }' ni del kode sobe

## Vrstica stanja spletne igre

status-label-room = SOBA
status-connecting = { $room } - POVEZOVANJE
status-greeting = { $room } - PROŠNJA ZA SEDEŽ
status-lobby = { $room } { $code } - SEDEŽ { $seat } - V SOBI
status-ping = PING { $ms } MS
status-buffer = { $room } { $code } - SEDEŽ { $seat }{ $rtt } - ZALOGA { $ms } MS
status-waiting = { $room } { $code } - SEDEŽ { $seat }{ $rtt } - ČAKANJE NA SOBO
status-offline = { $room } - BREZ POVEZAVE: { $reason }
note-tuning-refused = nastavitve sobe niso bile sprejete: { $detail }
note-welcome-refused = igre v sobi ni bilo mogoče zgraditi: { $detail }
note-not-cleared = ta zemljevid še ni preigran: zmagaj ga z IGRAJ v graditelju, nato ga shrani

## Zavrnitve strežnika

refusal-already-in-room = že si v sobi
refusal-not-in-room = nisi v sobi
refusal-not-yours = to sporočilo pošilja le strežnik
refusal-bad-message = soba tega ni razumela: { $detail }
refusal-bad-map = slab zemljevid: { $detail }
refusal-bad-code = to ni koda sobe: { $detail }
refusal-no-such-room = sobe { $code } ni
refusal-room-gone = sobe { $code } ni več
refusal-server-draining = strežnik se zapira zaradi ponovnega zagona; poskusi čez minuto
refusal-server-full = strežnik je poln ({ $rooms } sob)
refusal-room-full = soba je polna: { $seats } sedežev
refusal-already-started = igra se je že začela; soba ne sprejema novih igralcev
refusal-kicked = gostitelj te je odstranil iz sobe
refusal-reconnected = povezava je bila prevzeta iz druge seje
refusal-grace-over = predolgo odsoten; sedež je sproščen
refusal-left-room = zapustil si sobo
refusal-only-host-starts = igro začne le gostitelj
refusal-only-host-kicks = igralce odstranjuje le gostitelj
refusal-kick-self = gostitelj ne more odstraniti sebe
refusal-no-such-seat = tega sedeža ni
refusal-in-progress = igra je v teku
refusal-not-ready = { $nick } ni pripravljen
refusal-server-restarting = strežnik se znova zaganja; ustvari novo sobo čez minuto
refusal-room-closed = soba se je zaprla

## Graditelj

editor-back = NAZAJ
editor-file = MENI
editor-map = IGRA
editor-fit = VSE
editor-day = DAN
editor-play-here = IGRAJ TU
editor-check = PREGLED

check-title = PREGLED ZEMLJEVIDA
check-hint = Izberi težavo, da jo vidiš na zemljevidu. POPRAVI naredi spremembo, ki jo terja.
check-none = NI TEŽAV
check-fix = POPRAVI
check-cleared = PREIGRANO
check-not-cleared = ŠE NI PREIGRANO
check-par = ČAS AVTORJA { $time }
check-cleared-hint = Zmagan z IGRAJ, tak kot je. Vsaka sprememba je nova različica.
check-not-cleared-hint = Zmagaj ga z IGRAJ, sam in brez sprememb, da ga lahko gostiš v sobi.

lint-unreachable-bong = BONG JE NEDOSEGLJIV
lint-unreachable-pickup = BONUS JE NEDOSEGLJIV
lint-gated-pickup = BONUS ZA ZIDOVI
lint-disconnected-region = ODREZANO OBMOČJE
lint-boxed-in-cell = ZAPRTO POLJE
lint-spawn-band-too-tight = NI PROSTORA ZA SOVRAŽNIKE
lint-planner-physics-mismatch = POT SKOZI ZID
lint-narrow-corridor = OZEK PREHOD
lint-gate-not-on-edge = VRATA NISO NA ROBU
lint-gate-blocked = POT OD VRAT JE ZAPRTA
lint-waves-no-gates = VALOVI BREZ VRAT
lint-hunt-missing-enemy-bong = LOV BREZ SOVRAŽNEGA BONGA
lint-enemy-bong-unreachable = SOVRAŽNI BONG JE NEDOSEGLJIV
lint-no-start = NI ZAČETKA IGRALCA
lint-start-penned = ZAČETEK JE ZAPRT
lint-player2-unreachable = IGRALEC 2 JE ODREZAN
lint-players-too-close = ZAČETKA STA PREBLIZU
lint-portal-alone = OSAMLJEN PORTAL
lint-portal-blocked = PORTAL JE ZAPRT
lint-tower-at-start = STOLP POKRIVA ZAČETEK
lint-tower-no-reach = STOLP NE DOSEŽE NIČESAR
lint-too-many-towers = PREVEČ STOLPOV NA ENI STRANI
lint-training-door = VRATA, KI SE NE ODPREJO
lint-narrow-trail = OZKA STEZA
editor-tool = ORODJE

category-wall = ZID
category-prop = OVIRA
category-ground = TLA
category-plant = RASTJE
category-actor = FIGURA
category-pickup = BONUS

file-load = NALOŽI...
file-save = SHRANI
file-save-as = SHRANI KOT...
file-clear = POČISTI
file-revert = IZVIRNIK

settings-group-round = BOJ
settings-group-tanks = TANKI
settings-group-field = POLJE
settings-group-sky = NEBO
settings-sky-hint = VSAK BOJ IZBERE ENO · BREZ = JASNO
settings-tanks = TANKI
settings-tank = TANK
settings-tank2 = TANK 2
settings-bong = TVOJ BONG
settings-enemy-bong = TUJI BONG
settings-mission = MISIJA
settings-spawn = PRIHOD
settings-waves = VALOVI
settings-size = VELIKOST
settings-growth = RAST
settings-tier-start = RAZRED OD
settings-tier-end = RAZRED DO
settings-theme = TEMA
settings-width = ŠIRINA
settings-height = VIŠINA
settings-anchor = SIDRO
settings-reset = PONASTAVI
settings-auto = SAMODEJNO
settings-auto-short = SAM.
settings-cli = (cli)

editor-save-as = Shrani kot:
editor-save-hint = Enter shrani, Esc prekliče
editor-save-hint-touch = Tapni SHRANI, zunaj za preklic
editor-no-maps = ni zemljevidov
editor-shipped = vgrajen
editor-modified = spremenjen
editor-page = { $from }-{ $to } od { $n }  (kolešček)
editor-page-touch = { $from }-{ $to } od { $n }  (tapni < ali >)
editor-untitled = neimenovan

editor-saved = SHRANJENO V { $name }.toml
editor-loaded = NALOŽENO: { $name }
editor-saving-unavailable = SHRANJEVANJE V TEJ IZDAJI NI NA VOLJO: SPREMEMBE OSTANEJO V POMNILNIKU DO KONCA SEJE
editor-no-name = ZEMLJEVID ŠE NIMA IMENA: UPORABI SHRANI KOT
editor-bad-name = IME "{ $name }" SME IMETI LE ČRKE, ŠTEVKE, - IN _
editor-kept = SHRANJENO: { $name }
editor-kept-modified = SPREMEMBE SHRANJENE: { $name }
editor-kept-original = { $name } JE SPET IZVIRNIK
editor-reverted = { $name } JE SPET IZVIRNIK
editor-copied = { $n ->
    [one] KOPIRANA { $n } CELICA
    [two] KOPIRANI { $n } CELICI
    [few] KOPIRANE { $n } CELICE
   *[other] KOPIRANIH { $n } CELIC
}
editor-cut = { $n ->
    [one] IZREZANA { $n } CELICA
    [two] IZREZANI { $n } CELICI
    [few] IZREZANE { $n } CELICE
   *[other] IZREZANIH { $n } CELIC
}
editor-stamp-kept = SHRANJENO KOT { $name } MED VZORCI
editor-stamp-empty = NIČESAR ZA SHRANITI: IZBOR JE PRAZEN
editor-fill-too-large = PREVELIKO ZA POLNJENJE: VEČ KOT { $n } CELIC

editor-brush = ČOPIČ
brush-pen = SVINČNIK
brush-rect = PRAVOKOTNIK
brush-fill = POLNJENJE
brush-scatter = RAZTROS
brush-stamps = VZORCI...

select-copy = KOPIRAJ
select-cut = IZREŽI
select-paste = PRILEPI
select-delete = IZBRIŠI
select-save-stamp = + VZOREC
select-stamps = VZORCI
select-place = POLOŽI
select-cancel = PREKLIČI

stamp-fort = trdnjava
stamp-bunker = bunker
stamp-river-bend = rečni zavoj
stamp-saved = moj vzorec { $n }

tool-brick = OPEKA
tool-iron = ŽELEZO
tool-wood = LES
tool-glass = STEKLO
tool-concrete = BETON
tool-sandbag = VREČA PESKA
tool-barrel = SOD
tool-fence = OGRAJA
tool-target = TARČA
tool-tree = DREVO
tool-pine = IGLAVEC
tool-spruce = SMREKA
tool-scots = RDEČI BOR
tool-fir = MLADA JELKA
tool-birch = BREZA
tool-willow = VRBA
tool-palm = DATLJEVEC
tool-snag = SUHO DREVO
tool-bush = GRM
tool-berry_bush = JAGODIČJE
tool-juniper = BRIN
tool-fern = PRAPROT
tool-autumn_bush = JESENSKI GRM
tool-reeds = TRSTIČJE
tool-forest = GOZDNI ČOPIČ
tool-oil_drum = SOD OLJA
tool-fuel_drum = SOD GORIVA
tool-oil_trail = SLED OLJA
tool-road = CESTA
tool-water = VODA
tool-lava = LAVA
tool-tall_grass = VISOKA TRAVA
tool-gate = VRATA
tool-portal = PORTAL
tool-volcano = VULKAN
tool-lamp = LUČKA
tool-start = ZAČETEK P1
tool-start2 = ZAČETEK P2
tool-bong = BONG
tool-enemy_bong = SOVRAŽNI BONG
tool-health = ZDRAVJE
tool-ammo = STRELIVO
tool-laser = LASER
tool-minigun = MITRALJEZ
tool-plasma = PLAZMA
tool-missiles = RAKETE
tool-speedup = POSPEŠEK
tool-shield = ŠČIT
tool-flamethrower = PLAMENOMET
tool-bong_health = PAKET BONGA
tool-tower_pack = POPRAVILO
tool-heat_shield = TOPLOTNI ŠČIT
tool-grenades = GRANATE
tool-sonic_hammer = ZV. KLADIVO
tool-emp_burst = EMP SUNEK
tool-gauss_rail = GAUSSOV TOP
tool-fpv_swarm = ROJ DRONOV
tool-rod_from_god = BOŽJA PALICA
tool-gravity_well = TEŽN. JAMA
tool-air_heli = ZRAK: HELI
tool-air_jet = ZRAK: LOVEC
tool-tesla = TESLA STOLP
tool-tesla_enemy = SOVR. TESLA
tool-gun_tower = STROJNICA
tool-gun_tower_enemy = S. STROJNICA
tool-bio_slush = BIO BROZGA
tool-bio_slush_enemy = S. BROZGA
tool-eraser = RADIRKA
tool-select = IZBOR

tool-short-tall_grass = TRAVA
tool-short-forest = GOZD
tool-short-oil_drum = OLJE
tool-short-fuel_drum = GORIVO
tool-short-oil_trail = OLJE
tool-short-enemy_bong = S.BONG
tool-short-flamethrower = OGENJ
tool-short-bong_health = BONG+
tool-short-tower_pack = STOLP+
tool-short-heat_shield = ŠČIT+
tool-short-grenades = GRANAT
tool-short-sonic_hammer = ZVOK
tool-short-emp_burst = EMP
tool-short-gauss_rail = TIR
tool-short-fpv_swarm = DRON
tool-short-rod_from_god = PALICA
tool-short-gravity_well = JAMA
tool-short-air_heli = HELI
tool-short-air_jet = LOVEC
tool-short-volcano = VULKAN
tool-short-tesla = TESLA
tool-short-tesla_enemy = S.TSL
tool-short-gun_tower = STREL
tool-short-gun_tower_enemy = S.STR
tool-short-bio_slush = BIO
tool-short-bio_slush_enemy = S.BIO
tool-short-sandbag = VREČA
tool-short-start = START P1
tool-short-start2 = START P2

## Imena podatkov

theme-grass = trava
theme-desert = puščava
weather-clear = jasno
weather-night = noč
weather-dusk = mrak
weather-rain = dež
weather-storm = nevihta
weather-fog = megla
weather-sandstorm = vihar
weather-snow = sneg
weather-heat_haze = vročina
weather-random = naključno
spawn-band = pas
spawn-waves = valovi
tier-light = LAHKI
tier-medium = SREDNJI
tier-heavy = TEŽKI
tier-super = SUPER

tank-scout = SKAVT
tank-assault = ASSAULT
tank-breaker = BREAKER
tank-longbow = LONGBOW
tank-flak = FLAK
tank-wraith = WRAITH
tank-warden = WARDEN
tank-ravager = RAVAGER
tank-glacier = GLACIER
tank-obelisk = OBELISK
tank-titan = TITAN
tank-leviathan = LEVIATHAN

## Bongove vrstice v urjenju (docs/training-stage.md). { $name } je
## ime bonga, ki govori (`bong-name-*`).
bong-name-spike = Bong Spike
bong-name-oggle = Bong Oggle
bong-name-gloop = Bong Gloop
bong-name-puff = Bong Puff
bong-name-capi = Bong Capi
bong-name-blip = Bong Blip
bong-name-fuzz = Bong Fuzz
bong-hello = Živjo! Jaz sem { $name }.
bong-drive = <ARROWS> za vožnjo. Pred zavojem popusti.
bong-drive-touch = <STICK> Za vožnjo vleci z levim palcem.
bong-flags = Zapelji čez tri zastavice.
bong-flags-nudge = Tri zastavice. Potem se vrata odprejo.
bong-crates = Brez granat. Zaleti se v zaboje.
bong-health = Rdeči križ popravi oklep.
bong-crates-nudge = Zaboj s strelivom je na severu.
bong-fire = <SPACE> strelja, kamor kaže top.
bong-fire-touch = <TAP> desno polovico za strel.
bong-materials = Les hitro poči. Opeka zdrži več.
bong-iron = Železo nikoli. Ustreli opečni zid!
bong-fire-nudge = Obrni se k zidu, pritisni <SPACE>.
bong-fire-nudge-touch = Obrni se k zidu in <TAP>.
bong-minigun = Drži strel. Hitro porablja strelivo.
bong-wall = Lep strel! Za mano.
bong-drums = Sodi eksplodirajo. Streljaj od daleč.
bong-drum-kinds = Rdeči gorijo. Sivi letijo.
bong-oil = Ogenj steče po razliti nafti.
bong-drums-nudge = Ustreli sod. Opazuj verižno reakcijo!
bong-boom = Bum! Eksplozija podre tudi zid. Za mano.
bong-pad = To je moj dom. Kar naprej.
bong-ow = Au! To je priletelo z vzhoda!
bong-kit = Hitro, poberi moj zaboj!
bong-kit-nudge = Moj zaboj! Zeleni!
bong-healed = Bolje! Če padem, izgubiva.
bong-enemy = Prihaja!
bong-line-up = Poravnaj se in streljaj!
bong-enemy-nudge = Poravnaj se z njim!
bong-only-me = Hoče samo mene. Streljaj!
bong-chomp = Hrsk!
bong-shield = Najprej ščit.
bong-shoots-back = Ta strelja nazaj!
bong-wave-nudge = Še zadnji. Zmoreš.
bong-down = Tank je uničen! Nazaj k vratom.
bong-ready = Vse je nared.
