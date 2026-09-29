# Slovenščina. Pravila so v lang/en.ftl: velike črke v okvirju igre,
# šumniki se ob risanju zložijo na osnovno črko (Č -> C, Š -> S, Ž -> Z),
# vsak niz ima proračun v pikslih, ki ga preveri `cargo test`.

## Vrstica HUD

hud-speed = TEMPO
hud-shield = ŠČIT
hud-frog = ŽABA

button-build = GRADI
button-play = IGRAJ
button-leave = VEN
button-online = SPLET

players-title = Koliko igralcev?
players-keys = P1 puščice + preslednica    P2 WASD + levi Shift
players-one = 1 IGRALEC
players-two = 2 IGRALCA

leave-title = Zapustiš to igro?
leave-sub = Napredek bo izgubljen. Zemljevid ostane.
leave-confirm = ZAPUSTI IGRO
leave-stay = IGRAJ NAPREJ

## Igra

round-won = ZMAGA
round-lost = PORAZ
round-restarting = Nova igra čez { $seconds }...
round-back-to-lobby = Nazaj v sobo čez { $seconds }...
paused = PAVZA

mission-protect = BRANI
mission-hunt = LOV
mission-destroy = UNIČI

mission-protect-banner = BRANI ŽABO!
mission-hunt-banner = ULOVI ŽABO!
mission-destroy-banner = UNIČI VSE!

wave-banner = VAL { $n }
wave-final = ZADNJI VAL

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

editor-build = GRADI
editor-file = MENI
editor-map = IGRA
editor-tool = ORODJE

category-wall = ZID
category-prop = OVIRA
category-ground = TLA
category-actor = FIGURA
category-pickup = BONUS

file-load = NALOŽI...
file-save = SHRANI
file-save-as = SHRANI KOT...
file-clear = POČISTI

settings-tanks = TANKI
settings-tank = TANK
settings-tank2 = TANK 2
settings-mission = MISIJA
settings-spawn = PRIHOD
settings-waves = VALOVI
settings-size = VELIKOST
settings-growth = RAST
settings-tier-start = RAZRED OD
settings-tier-end = RAZRED DO
settings-theme = TEMA
settings-reset = PONASTAVI
settings-auto = samodejno
settings-cli = (cli)

editor-save-as = Shrani kot:
editor-save-hint = Enter shrani, Esc prekliče
editor-no-maps = ni zemljevidov
editor-shipped = vgrajen
editor-page = { $from }-{ $to } od { $n }  (kolešček)
editor-untitled = neimenovan

editor-saved = shranjeno v { $name }.toml
editor-loaded = naloženo: { $name }
editor-saving-unavailable = shranjevanje v tej izdaji ni na voljo: spremembe ostanejo v pomnilniku do konca seje
editor-no-name = zemljevid še nima imena: uporabi SHRANI KOT
editor-bad-name = ime "{ $name }" sme imeti le črke, števke, - in _

tool-brick = opeka
tool-iron = železo
tool-wood = les
tool-glass = steklo
tool-sandbag = vreča peska
tool-barrel = sod
tool-fence = ograja
tool-tree = drevo
tool-pine = smreka
tool-oil_drum = sod olja
tool-fuel_drum = sod goriva
tool-oil_trail = sled olja
tool-road = cesta
tool-water = voda
tool-tall_grass = visoka trava
tool-gate = vrata
tool-portal = portal
tool-start = začetek p1
tool-start2 = začetek p2
tool-frog = žaba
tool-enemy_frog = sovražna žaba
tool-health = zdravje
tool-ammo = strelivo
tool-laser = laser
tool-minigun = mitraljez
tool-plasma = plazma
tool-missiles = rakete
tool-speedup = pospešek
tool-shield = ščit
tool-flamethrower = metalec ognja
tool-frog_health = paket za žabo
tool-tower_pack = popravilo
tool-tesla = tesla stolp
tool-tesla_enemy = sovr. tesla
tool-gun_tower = strojnica
tool-gun_tower_enemy = s. strojnica
tool-bio_slush = bio brozga
tool-bio_slush_enemy = s. brozga
tool-eraser = radirka

tool-short-tall_grass = trava
tool-short-oil_drum = olje
tool-short-fuel_drum = gorivo
tool-short-oil_trail = olje
tool-short-enemy_frog = s.žaba
tool-short-flamethrower = ogenj
tool-short-frog_health = žaba+
tool-short-tower_pack = stolp+
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
