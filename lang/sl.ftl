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
settings-auto = samodejno
settings-auto-short = sam.
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

editor-saved = shranjeno v { $name }.toml
editor-loaded = naloženo: { $name }
editor-saving-unavailable = shranjevanje v tej izdaji ni na voljo: spremembe ostanejo v pomnilniku do konca seje
editor-no-name = zemljevid še nima imena: uporabi SHRANI KOT
editor-bad-name = ime "{ $name }" sme imeti le črke, števke, - in _
editor-kept = shranjeno: { $name }
editor-kept-modified = spremembe shranjene: { $name }
editor-kept-original = { $name } je spet izvirnik
editor-reverted = { $name } je spet izvirnik
editor-copied = { $n ->
    [one] kopirana { $n } celica
    [two] kopirani { $n } celici
    [few] kopirane { $n } celice
   *[other] kopiranih { $n } celic
}
editor-cut = { $n ->
    [one] izrezana { $n } celica
    [two] izrezani { $n } celici
    [few] izrezane { $n } celice
   *[other] izrezanih { $n } celic
}
editor-stamp-kept = shranjeno kot { $name } med VZORCI
editor-stamp-empty = ničesar za shraniti: izbor je prazen
editor-fill-too-large = preveliko za polnjenje: več kot { $n } celic

editor-brush = ČOPIČ
brush-pen = svinčnik
brush-rect = pravokotnik
brush-fill = polnjenje
brush-scatter = raztros
brush-stamps = vzorci...

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

tool-brick = opeka
tool-iron = železo
tool-wood = les
tool-glass = steklo
tool-concrete = beton
tool-sandbag = vreča peska
tool-barrel = sod
tool-fence = ograja
tool-target = tarča
tool-tree = drevo
tool-pine = iglavec
tool-spruce = smreka
tool-scots = rdeči bor
tool-fir = mlada jelka
tool-birch = breza
tool-willow = vrba
tool-palm = datljevec
tool-snag = suho drevo
tool-bush = grm
tool-berry_bush = jagodičje
tool-juniper = brin
tool-fern = praprot
tool-autumn_bush = jesenski grm
tool-reeds = trstičje
tool-forest = gozdni čopič
tool-oil_drum = sod olja
tool-fuel_drum = sod goriva
tool-oil_trail = sled olja
tool-road = cesta
tool-water = voda
tool-lava = lava
tool-tall_grass = visoka trava
tool-gate = vrata
tool-portal = portal
tool-volcano = vulkan
tool-lamp = lučka
tool-start = začetek p1
tool-start2 = začetek p2
tool-bong = bong
tool-enemy_bong = sovražni bong
tool-health = zdravje
tool-ammo = strelivo
tool-laser = laser
tool-minigun = mitraljez
tool-plasma = plazma
tool-missiles = rakete
tool-speedup = pospešek
tool-shield = ščit
tool-flamethrower = metalec ognja
tool-bong_health = paket za bonga
tool-tower_pack = popravilo
tool-heat_shield = toplotni ščit
tool-grenades = granate
tool-sonic_hammer = zvočno kladivo
tool-emp_burst = emp sunek
tool-gauss_rail = gaussov top
tool-fpv_swarm = roj dronov
tool-rod_from_god = božja palica
tool-gravity_well = težnostna jama
tool-air_heli = zrak: helikopter
tool-air_jet = zrak: lovec
tool-tesla = tesla stolp
tool-tesla_enemy = sovr. tesla
tool-gun_tower = strojnica
tool-gun_tower_enemy = s. strojnica
tool-bio_slush = bio brozga
tool-bio_slush_enemy = s. brozga
tool-eraser = radirka
tool-select = izbor

tool-short-tall_grass = trava
tool-short-forest = gozd
tool-short-oil_drum = olje
tool-short-fuel_drum = gorivo
tool-short-oil_trail = olje
tool-short-enemy_bong = s.bong
tool-short-flamethrower = ogenj
tool-short-bong_health = bong+
tool-short-tower_pack = stolp+
tool-short-heat_shield = ščit+
tool-short-grenades = granat
tool-short-sonic_hammer = zvok
tool-short-emp_burst = emp
tool-short-gauss_rail = tir
tool-short-fpv_swarm = dron
tool-short-rod_from_god = palica
tool-short-gravity_well = jama
tool-short-air_heli = heli
tool-short-air_jet = lovec
tool-short-volcano = vulkan
tool-short-tesla = tesla
tool-short-tesla_enemy = s.tsl
tool-short-gun_tower = strel
tool-short-gun_tower_enemy = s.str
tool-short-bio_slush = bio
tool-short-bio_slush_enemy = s.bio
tool-short-sandbag = vreča
tool-short-start = start p1
tool-short-start2 = start p2

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
tier-light = lahki
tier-medium = srednji
tier-heavy = težki
tier-super = super

tank-scout = skavt
tank-assault = assault
tank-breaker = breaker
tank-longbow = longbow
tank-flak = flak
tank-wraith = wraith
tank-warden = warden
tank-ravager = ravager
tank-glacier = glacier
tank-obelisk = obelisk
tank-titan = titan
tank-leviathan = leviathan

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
