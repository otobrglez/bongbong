//! The game's three modes and the switch between them
//! (docs/game-editor-fusion.md, section 5; docs/online-coop-prd.md §4.5):
//! `Session` owns the `Game`, the `MapEditor` and which driver is live,
//! plus the "leave this round?" question in between and the "how many
//! players?" one (docs/two-players.md). Both the window (`app.rs`) and
//! the dev server go through the methods here, so a tool and a click are
//! the same path. No `RaylibHandle` anywhere in this file: drawing stays
//! in `game.rs`, `hud.rs` and `editor.rs`.
//!
//! Play runs `Game::update` in process, Build runs the map builder,
//! Lobby is the room screen (`lobby.rs`) and Online draws a
//! `net::round::OnlineRound`'s replica - a round the room server
//! simulates and this window only ever draws. The local round is left
//! exactly where it stood: `playing()` is Play's alone, so nothing about
//! the lobby or an online round can start or stop a local one.
//!
//! Lobby and Online hand back and forth on the same seat: the room says
//! the round has begun and the window goes Online, the room says it is
//! over and the window comes back to the screen it came from, code, QR
//! and roster intact, where the host can ask for a rematch. Only
//! `leave_online` gives the seat up.
//!
//! The local round is a level when its map is one (`levels.rs`,
//! docs/levels.md): its end screen then counts down to `NEXT LEVEL` after
//! a win and `PLAY AGAIN` after a loss (`follow_countdown`), its buttons
//! take either at once, and a win is progress for the store. Any other
//! map is free play and restarts on its own. The level select
//! (`level_select.rs`) stands over the frozen round the way the dialogs
//! do, and is the way back to a level already won.

use crate::ai::Intent;
use crate::editor::{BuilderFrame, BuilderInput, EditorAction, MapEditor};
use crate::hud::{result_layout, LevelBanner, NextLevel, PlayChrome, ResultButtons, ResultView};
use crate::level_select::{LevelSelect, SelectAction, SelectInput};
use crate::levels::Campaign;
use crate::lobby::{Lobby, LobbyAction, LobbyInput, RoomPhase, RoomView};
use crate::map::MapFile;
use crate::mapstore::Question;
use crate::net::client::{RoomSetup, Target};
use crate::net::round::AnyRound;
use crate::net::rooms::{RoomCode, RoomsHost, SiteBase};
use crate::simulation::{Game, Outcome, PlayerCount};
use crate::Rect;

/// Which mode the window is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Driver {
    Play,
    Build,
    /// The lobby: hosting or joining a room, before any round
    /// (`lobby.rs`).
    Lobby,
    /// A room's round, drawn from its snapshots and never updated here.
    Online,
}

impl Driver {
    pub fn name(self) -> &'static str {
        match self {
            Driver::Play => "play",
            Driver::Build => "build",
            Driver::Lobby => "lobby",
            Driver::Online => "online",
        }
    }
}

/// Whether `game`'s round is its map played as the map is authored, under
/// the knobs `t`: one seat, the map's own start and its own sky, and no
/// enemy count, mission, spawn plan or chassis from the command line, a
/// tool or the `player_tank` knob (read as it stands at the win; the round
/// was set up under it unless it was moved since).
fn played_as_authored(game: &Game, t: &crate::tuning::Tuning) -> bool {
    // The map's sky, or the one of its skies this seed picks - or night,
    // once the map's `nightfall` has passed: what the round is fought
    // under when no `--weather` puts another in.
    let key = if game.night_has_fallen() { crate::map::Weather::Night.into() } else { game.map.weather };
    let own_sky = crate::weather::in_force(key, game.round_seed(), true, &crate::tuning::Tuning::DEFAULT);
    game.players == PlayerCount::ONE
        && game.start_override.is_none()
        && game.enemy_count_override.is_none()
        && game.level_overrides == crate::level::LevelOverrides::default()
        && game.player_row_override.is_none()
        && t.player_tank == crate::tuning::Tuning::DEFAULT.player_tank
        && game.weather() == own_sky
}

/// Where PLAY HERE puts seat 1 in `game`, a round just set up on the
/// builder's map from the map's own start: the cell nearest `near` - the
/// middle of the builder's view - that a tank can be put down on
/// (`Game::drop_cell`: spawn-legal clearance, out of deep water, off a
/// portal) in the part of the nav grid the map's own start is in, so the
/// test is driven where the round is played and never from a pocket the
/// router cannot leave (Unreal's Play From Here trap). `None` where there
/// is no such cell.
pub fn play_here_cell(game: &Game, near: crate::math::Vec2) -> Option<(i32, i32)> {
    let start = game.seat_pose(0)?.position;
    let (width, height) = game.map.field_size();
    let grid = game.nav_grid(width, height);
    let parts = grid.components();
    game.drop_cell(&grid, near, |_, p| parts.connected(&grid, start, p))
}

/// The whole windowed session: a round and the map it came from, in
/// whichever mode is live.
pub struct Session {
    pub driver: Driver,
    pub game: Game,
    pub builder: MapEditor,
    /// The leave-round dialog is open (play mode only). While it is, the
    /// round is frozen because `advance` is not called - no simulation
    /// pause flag is touched.
    pub dialog: bool,
    /// The players dialog (`1 PLAYER` / `2 PLAYERS`) is open, play mode
    /// only; the round is frozen the same way. Never open together with
    /// `dialog`. The chosen count itself lives on `Game::players`, which
    /// every restart keeps, so it is the session's setting.
    pub players_dialog: bool,
    /// The seat in a room and the replica it draws, from the moment the
    /// lobby opens one: the second `Game` of the session, and the only
    /// one this window does not simulate. `None` until then.
    pub online: Option<AnyRound>,
    /// The lobby screen, from `press_online` until it is closed. It
    /// outlives no round: leaving a room comes back through it.
    pub lobby: Option<Lobby>,
    /// Which rooms server this session talks to (`--rooms`,
    /// `BONGBONG_ROOMS`): the lobby's invite link is built from it.
    /// `app.rs` sets it once at startup; the cluster's is the default.
    pub rooms: RoomsHost,
    /// Where this build is served from, which is where the lobby's QR
    /// sends a scan. The deployed site unless the web build's page says
    /// otherwise (`app.rs` reads `window.bbInvite`), which is what keeps
    /// a PR preview's invite inside that preview.
    pub site: SiteBase,
    /// The name this player takes into a room: `--nick`, else
    /// `Player #` and six letters and digits read off the token
    /// (`net::client::anonymous_nick`).
    pub nick: String,
    /// The reconnect key this session takes into a room
    /// (`net::client::Identity::device_token`): the same token in a
    /// later join reclaims the same seat for the room's life, so it has
    /// to be this player's and nobody else's. A desktop build given a
    /// `--nick` derives it from the nickname, which is what makes two
    /// `--nick`s on one machine two seats; a build given none - every
    /// phone - mints one for the run (`Identity::anonymous`), which is
    /// what makes two such windows two seats; the web page mints a random
    /// one and keeps it per tab (`site/src/scripts/room.ts`), which is
    /// what makes two tabs two players. `app.rs` sets it once at startup.
    pub token: String,
    /// The levels and how far this player has got (`levels.rs`), set by
    /// `set_campaign`. `None` - a test's session, a tool's - makes every
    /// map free play.
    pub campaign: Option<Campaign>,
    /// The level select (`level_select.rs`), from `press_levels` until a
    /// level starts or it is closed: play mode only, never together with
    /// a dialog, and the round behind it frozen the dialogs' way.
    pub level_select: Option<LevelSelect>,
    /// A question about a kept map (`mapstore`, BB-33), over the round or
    /// the builder: FILE's REVERT TO ORIGINAL, or a modified copy whose
    /// original this build ships changed (`watch_originals`). While it is
    /// up nothing else takes a press and the round is frozen the dialogs'
    /// way.
    pub question: Option<Question>,
    /// The maps `watch_originals` has looked at this session: each is
    /// asked about once.
    originals_checked: std::collections::BTreeSet<String>,
    /// The world rectangle the local round was last drawn showing - the
    /// whole field for an arena, the followed view on a field map - which
    /// `app.rs` notes every frame of play. BUILD opens the builder's
    /// camera on it (docs/large-maps-follow-camera.md §9).
    pub play_view: Option<crate::math::Rectangle>,
    /// Whether this window draws the play minimap (`minimap.rs`,
    /// docs/large-maps-follow-camera.md §7, §15): its screen shows one -
    /// not a phone's, as `minimap_show` says - and its view shows less than
    /// the whole field. `app.rs` sets it every frame, before the hit tests;
    /// `play_chrome` lays the minimap's slot out from it, so the painter,
    /// the hit tests and the dev server read one geometry. False where no
    /// window does (a test, a headless tool).
    pub minimap_on: bool,
    /// The clear check (docs/large-maps-patterns.md, "Clear check before
    /// sharing"): the revision of the builder's canvas (`MapFile::revision`)
    /// its plain `PLAY` started the local round on. A win on that revision,
    /// played as the map is authored (`note_outcome`), clears it in the
    /// builder. `PLAY HERE`, a level started, another map or seat count
    /// put in the round's place take it away: those rounds clear nothing.
    pub clear_attempt: Option<u64>,
    /// Whether the local round's win has been put to the clear check: once,
    /// on the first frame of the win; a round seen playing again lets the
    /// next win be put.
    clear_noted: bool,
    /// The fade through black between local rounds, the way an end screen
    /// takes, and the real time since the round was decided (`Curtain`).
    curtain: Curtain,
}

/// The fade through black between local rounds and the end's slowed
/// picture, headless (`Session::tick_curtain`, `curtain`, `time_scale`). The
/// fade out is read off the end screen's own clock - its last
/// `level_fade_seconds` or `retry_fade_seconds` - so it lands black on the
/// frame the next round starts; the fade in runs on real time from there.
#[derive(Clone, Copy, Debug, Default)]
struct Curtain {
    /// The way a press on the end screen chose, taken when its clock runs
    /// out behind the fade out (`press_result`, `enter_result`).
    pending: Option<Pending>,
    /// The fade in's seconds left, and its length.
    fade_in: f32,
    fade_in_of: f32,
    /// The local round's frame last frame: a round that starts over by
    /// itself (free play, the R key) is seen by its frame going back.
    last_frame: u64,
    /// Real seconds since the local round was decided, for the hold and
    /// the slow motion on the deciding blow; `None` while it plays.
    since_end: Option<f32>,
}

/// The way an end screen was told to take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    NextLevel,
    PlayAgain,
}

/// A session reads as its round: the dev server, its tests and `main.rs`
/// call `Game`'s accessors on it directly.
impl std::ops::Deref for Session {
    type Target = Game;
    fn deref(&self) -> &Game {
        &self.game
    }
}

impl std::ops::DerefMut for Session {
    fn deref_mut(&mut self) -> &mut Game {
        &mut self.game
    }
}

impl Session {
    /// `game` must be initialised; the builder is seeded from its map.
    /// The field size is always the map's (`MapFile::field_size`), so a
    /// round and the builder's canvas never need to be told it.
    pub fn new(game: Game) -> Self {
        let builder = MapEditor::new(game.map.clone());
        // Nobody until `app.rs` says who: a token of this session's own,
        // never one another client could be holding.
        let anonymous = crate::net::client::Identity::anonymous();
        Session {
            driver: Driver::Play,
            game,
            builder,
            dialog: false,
            players_dialog: false,
            online: None,
            lobby: None,
            rooms: RoomsHost::deployed(),
            site: SiteBase::deployed(),
            nick: anonymous.nick,
            token: anonymous.device_token,
            campaign: None,
            level_select: None,
            question: None,
            originals_checked: std::collections::BTreeSet::new(),
            play_view: None,
            minimap_on: false,
            clear_attempt: None,
            clear_noted: false,
            curtain: Curtain::default(),
        }
    }

    /// Put the levels on the session: from here on a round on a level's
    /// map is that level.
    pub fn set_campaign(&mut self, campaign: Campaign) {
        self.campaign = Some(campaign);
        self.sync_level();
    }

    /// Which level the local round is, if its map is one of the levels
    /// (`levels::Campaign::position`).
    pub fn level(&self) -> Option<usize> {
        self.campaign.as_ref()?.position(self.game.map.name.as_deref())
    }

    /// A level's end screen counts down into a hold, for
    /// `follow_countdown` to take its way; free play's - and a test
    /// round's from the builder's spot (`play_here`) - restarts on its
    /// own. Called wherever the round's map or start changes.
    fn sync_level(&mut self) {
        self.game.hold_end_screen = self.level_round().is_some();
    }

    /// The level the local round plays for: its map's (`level`), unless
    /// the round is a test from the builder's spot (`play_here`), which
    /// wins no level and leads to no other.
    fn level_round(&self) -> Option<usize> {
        self.level().filter(|_| self.game.start_override.is_none())
    }

    /// Start level `i` - its map as last edited this session - on the
    /// field and in the builder, as a new document there, and a fresh
    /// round on it (a `--seed` stays pinned, the banner shows).
    pub fn start_level(&mut self, i: usize) -> Result<(), String> {
        let map = self.campaign.as_ref().ok_or("this session has no levels")?.map(i)?;
        self.builder.leave();
        self.builder.open(map.clone());
        self.game.map = map;
        self.game.start_override = None;
        self.clear_attempt = None;
        self.sync_level();
        self.dialog = false;
        self.players_dialog = false;
        self.level_select = None;
        let (width, height) = self.game.map.field_size();
        self.game.init(width, height);
        self.open_curtain(crate::tuning::tuning().level_fade_seconds);
        Ok(())
    }

    /// `PLAY AGAIN`: the same map, a fresh round - what the R key does.
    /// Play mode only.
    pub fn play_again(&mut self) {
        if self.driver == Driver::Play {
            self.dialog = false;
            self.players_dialog = false;
            let (width, height) = self.game.map.field_size();
            self.game.init(width, height);
            self.open_curtain(crate::tuning::tuning().retry_fade_seconds);
        }
    }

    /// `NEXT LEVEL`: after a won level, the one after it - the first again
    /// after the last. A no-op anywhere else; answers whether it moved.
    pub fn next_level(&mut self) -> bool {
        let Some(i) = self.level_round() else { return false };
        if self.driver != Driver::Play || self.game.outcome() != Outcome::Won {
            return false;
        }
        let next = self.campaign.as_ref().map_or(i, |c| c.next(i));
        match self.start_level(next) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("[levels] level {}: {e}", self.campaign.as_ref().map_or(next + 1, |c| c.levels.number(next)));
                false
            }
        }
    }

    /// Count a won level as progress and put a win to the clear check.
    /// Called after every frame's steps: only the first call after a win
    /// moves anything, and `take_progress` hands the move to the store. A
    /// round started from the builder's spot (`play_here`) is a test and
    /// wins nothing.
    pub fn note_outcome(&mut self) {
        match self.game.outcome() {
            Outcome::Playing => self.clear_noted = false,
            Outcome::Lost => {}
            Outcome::Won if self.game.start_override.is_some() => {}
            Outcome::Won => {
                if let (Some(i), Some(campaign)) = (self.level_round(), self.campaign.as_mut()) {
                    campaign.won(i);
                }
                if !std::mem::replace(&mut self.clear_noted, true) {
                    self.note_clear();
                }
            }
        }
    }

    /// The clear check on a win: the revision `PLAY` started the round on
    /// (`clear_attempt`) is cleared when the round is that revision
    /// `played_as_authored`, and its par is the round clock at the win. The
    /// builder keeps it (`MapEditor::note_clear`), the best par of its
    /// wins.
    fn note_clear(&mut self) {
        let Some(revision) = self.clear_attempt else { return };
        let game = &self.game;
        if played_as_authored(game, &crate::tuning::tuning()) && game.map.revision() == revision {
            let seconds = game.round_stats().seconds as f64;
            self.builder.note_clear(revision, seconds);
        }
    }

    /// The progress to write, once per change: the map name of the
    /// furthest level reached.
    pub fn take_progress(&mut self) -> Option<String> {
        self.campaign.as_mut()?.take_unsaved()
    }

    /// The end screen's content: set once a local round is decided, with
    /// a level's buttons and its countdown on a level (never on a test
    /// round from the builder's spot, whose end screen is free play's).
    fn result_view(&self) -> Option<ResultView> {
        if self.game.outcome() == Outcome::Playing {
            return None;
        }
        let buttons = self.level_round().zip(self.campaign.as_ref()).map(|(i, campaign)| {
            let next = (self.game.outcome() == Outcome::Won).then(|| {
                if campaign.is_last(i) { NextLevel::FirstAgain { levels: campaign.levels.len() } } else { NextLevel::Next }
            });
            // Whole seconds, never 0: at zero the screen is taking its
            // way (`follow_countdown`), which it does the same frame.
            let left = self.game.restart_countdown().min(self.game.end_beats().countdown);
            let countdown = (!matches!(next, Some(NextLevel::FirstAgain { .. }))).then(|| left.ceil().max(1.0) as u32);
            ResultButtons { next, countdown }
        });
        Some(ResultView { stats: self.game.round_stats(), seats: self.game.players.count(), buttons })
    }

    /// A level's end screen whose countdown has run out takes the way it
    /// counted down to: the next level after a win, the same one again
    /// after a loss. The last level's win counts nothing down and waits
    /// for a button. Called after every frame's steps, like
    /// `note_outcome`; a screen behind a dialog, the level select or the
    /// builder never moves on by itself, since nothing counts it down.
    /// Answers whether a round started.
    pub fn follow_countdown(&mut self) -> bool {
        if !self.playing() || self.game.restart_countdown() > 0.0 {
            return false;
        }
        match self.curtain.pending.take() {
            Some(Pending::NextLevel) => return self.next_level(),
            Some(Pending::PlayAgain) => {
                self.play_again();
                return true;
            }
            None => {}
        }
        match self.result_view().and_then(|view| view.buttons) {
            Some(ResultButtons { countdown: Some(_), next: Some(_) }) => self.next_level(),
            Some(ResultButtons { countdown: Some(_), next: None }) => {
                self.play_again();
                true
            }
            _ => false,
        }
    }

    /// A press at `p` on a level's end screen, in UI points with the
    /// screen laid out in the chrome's `area` (`hud::UiFrame`): `LEVELS`,
    /// `PLAY AGAIN` or the way on. Answers whether it landed on a button;
    /// a press anywhere else is the caller's.
    pub fn press_result(&mut self, p: crate::math::Vec2, area: Rect) -> bool {
        if !self.playing() || self.game.outcome() == Outcome::Playing {
            return false;
        }
        // The finale: any press goes straight to the verdict, and one
        // while the end screen eases in or the screen is fading out is
        // claimed and does nothing - no button is there to be pressed.
        if self.game.in_finale() {
            self.game.skip_finale();
            return true;
        }
        if !self.verdict_ready() || self.curtain.pending.is_some() {
            return true;
        }
        let Some(view) = self.result_view() else { return false };
        let Some(rects) = result_layout(area, &view).buttons else { return false };
        if rects.again.contains(p) {
            self.take_way(Pending::PlayAgain);
            true
        } else if rects.next.is_some_and(|r| r.contains(p)) {
            self.take_way(Pending::NextLevel);
            true
        } else if rects.levels.contains(p) {
            self.press_levels();
            true
        } else {
            false
        }
    }

    /// The level select: open it over the round, which stands still
    /// behind it (`playing()` is false while it is up), or close it if it
    /// already is - the HUD's level button, the end screen's `LEVELS` and
    /// Esc. Play mode with levels only; a dialog that was asking closes.
    /// Answers whether it is open.
    pub fn press_levels(&mut self) -> bool {
        if self.level_select.take().is_some() {
            return false;
        }
        let Some(campaign) = &self.campaign else { return false };
        if self.driver != Driver::Play {
            return false;
        }
        self.dialog = false;
        self.players_dialog = false;
        self.level_select = Some(LevelSelect::open(self.level(), campaign.open_to()));
        true
    }

    /// One frame of the level select (`input` in UI points, the panel
    /// centred in the chrome's `area`): its hit tests and keys, and
    /// whatever they ask. Answers whether a level started - a new round,
    /// with its banner.
    pub fn update_level_select(&mut self, input: &SelectInput, area: Rect) -> bool {
        let (Some(select), Some(campaign)) = (&mut self.level_select, &self.campaign) else { return false };
        match select.update(input, area, campaign.levels.len(), campaign.open_to()) {
            SelectAction::Stay => false,
            SelectAction::Close => {
                self.level_select = None;
                false
            }
            SelectAction::Start(i) => match self.start_level(i) {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("[levels] level {}: {e}", i + 1);
                    self.level_select = None;
                    false
                }
            },
        }
    }

    /// The number of the level on the field, for the HUD's level button
    /// in the mission word's place: a local round in play mode on a
    /// level. The painter and every hit test read it, so the button is
    /// pressable exactly where it is drawn.
    pub fn level_button(&self) -> Option<usize> {
        let levels = &self.campaign.as_ref()?.levels;
        (self.driver == Driver::Play).then(|| self.level()).flatten().map(|i| levels.number(i))
    }

    /// Enter on a level's end screen: the way on after a win, `PLAY
    /// AGAIN` after a loss. Answers whether there was a screen to answer.
    pub fn enter_result(&mut self) -> bool {
        if !self.playing() || self.game.outcome() == Outcome::Playing {
            return false;
        }
        if self.game.in_finale() {
            self.game.skip_finale();
            return true;
        }
        if !self.verdict_ready() || self.curtain.pending.is_some() {
            return false;
        }
        match self.result_view().and_then(|view| view.buttons) {
            Some(ResultButtons { next: Some(_), .. }) => {
                self.take_way(Pending::NextLevel);
                true
            }
            Some(ResultButtons { next: None, .. }) => {
                self.take_way(Pending::PlayAgain);
                true
            }
            None => false,
        }
    }

    /// The end screen's press for a tool, which runs no frames between a
    /// request and its answer (the dev server's `click`): the finale and
    /// the fade are skipped and the way pressed is taken at once, so the
    /// answer describes the round it led to.
    pub fn press_result_at_once(&mut self, p: crate::math::Vec2, area: Rect) -> bool {
        if self.playing() {
            self.game.skip_to_verdict();
        }
        let hit = self.press_result(p, area);
        self.take_way_now();
        hit
    }

    /// `enter_result` for a tool, as `press_result_at_once`.
    pub fn enter_result_at_once(&mut self) -> bool {
        if self.playing() {
            self.game.skip_to_verdict();
        }
        let took = self.enter_result();
        self.take_way_now();
        took
    }

    /// Take a way pressed on the end screen without its fade out.
    fn take_way_now(&mut self) {
        if self.curtain.pending.is_some() {
            self.game.hurry_end(0.0);
            self.follow_countdown();
        }
    }

    /// Whether a decided round's end screen has eased in far enough for
    /// its buttons to take presses.
    fn verdict_ready(&self) -> bool {
        let beats = self.game.end_beats();
        self.game.since_end().is_some_and(|s| s >= beats.finale + beats.fade * 0.5)
    }

    /// Take the end screen's `way` behind the fade out: its clock runs
    /// down to the fade's length and `follow_countdown` takes the way when
    /// it is out.
    fn take_way(&mut self, way: Pending) {
        let t = crate::tuning::tuning();
        let fade = match way {
            Pending::NextLevel => t.level_fade_seconds,
            Pending::PlayAgain => t.retry_fade_seconds,
        };
        self.curtain.pending = Some(way);
        self.game.hurry_end(fade);
    }

    /// Start the fade in onto a round just started, `seconds` long.
    fn open_curtain(&mut self, seconds: f32) {
        self.curtain.pending = None;
        self.curtain.fade_in = seconds;
        self.curtain.fade_in_of = seconds;
        self.curtain.last_frame = self.game.frame();
    }

    /// One frame of the curtain, `dt` real seconds: the fade in runs down,
    /// a local round that started over by itself (free play's end, the R
    /// key) fades in, and the time since the round was decided counts.
    /// `app.rs` calls it once a frame, after `follow_countdown`.
    pub fn tick_curtain(&mut self, dt: f32) {
        let frame = self.game.frame();
        if self.driver == Driver::Play && frame < self.curtain.last_frame && self.curtain.fade_in <= 0.0 {
            self.open_curtain(crate::tuning::tuning().retry_fade_seconds);
        }
        self.curtain.last_frame = frame;
        self.curtain.fade_in = (self.curtain.fade_in - dt).max(0.0);
        self.curtain.since_end = match (self.game.outcome(), self.curtain.since_end) {
            (Outcome::Playing, _) => None,
            (_, since) if !self.playing() => since,
            (_, since) => Some(since.unwrap_or(0.0) + dt),
        };
    }

    /// How dark the curtain is, 0 to 1, over a local round in play mode:
    /// the fade out over the end screen's last seconds where it is about
    /// to take a way by itself or was told to, the fade in after.
    pub fn curtain(&self) -> f32 {
        if self.driver != Driver::Play {
            return 0.0;
        }
        let fade_in = if self.curtain.fade_in_of > 0.0 { self.curtain.fade_in / self.curtain.fade_in_of } else { 0.0 };
        let t = crate::tuning::tuning();
        let way = self.curtain.pending.or_else(|| {
            if self.game.outcome() == Outcome::Playing {
                return None;
            }
            if !self.game.hold_end_screen {
                // Free play restarts itself on the same map.
                return Some(Pending::PlayAgain);
            }
            match self.result_view().and_then(|view| view.buttons) {
                Some(ResultButtons { countdown: Some(_), next: Some(_) }) => Some(Pending::NextLevel),
                Some(ResultButtons { countdown: Some(_), next: None }) => Some(Pending::PlayAgain),
                _ => None,
            }
        });
        let fade_out = way.map_or(0.0, |way| {
            let seconds = match way {
                Pending::NextLevel => t.level_fade_seconds,
                Pending::PlayAgain => t.retry_fade_seconds,
            };
            let left = self.game.restart_countdown();
            if seconds > 0.0 && left < seconds { 1.0 - left / seconds } else { 0.0 }
        });
        fade_out.max(fade_in).clamp(0.0, 1.0)
    }

    /// The share of real time the local round runs at: held for
    /// `round_hitstop_seconds` on the blow that decided it, then
    /// `round_slowmo_scale` for `round_slowmo_seconds`, easing back over
    /// the last of them. 1 under reduced motion and while the round plays.
    pub fn time_scale(&self) -> f32 {
        let Some(since) = self.curtain.since_end else { return 1.0 };
        if crate::motion::reduced() {
            return 1.0;
        }
        let t = crate::tuning::tuning();
        if since < t.round_hitstop_seconds {
            return 0.0;
        }
        let slow = since - t.round_hitstop_seconds;
        if slow >= t.round_slowmo_seconds {
            return 1.0;
        }
        // The last 40 % of the slow motion eases back to full speed.
        let back = (slow / t.round_slowmo_seconds - 0.6).max(0.0) / 0.4;
        t.round_slowmo_scale + (1.0 - t.round_slowmo_scale) * back
    }

    pub fn mode(&self) -> Driver {
        self.driver
    }

    /// Whether `Game::update` should run this frame: play mode with no
    /// question pending.
    pub fn playing(&self) -> bool {
        self.driver == Driver::Play && !self.dialog && !self.players_dialog && self.level_select.is_none() && self.question.is_none()
    }

    /// A round the player could still lose by leaving: anything but the
    /// end screen.
    pub fn round_in_progress(&self) -> bool {
        self.game.outcome() == Outcome::Playing
    }

    /// The `BUILD` button (or `Tab` in play mode): ask when a round is in
    /// progress, switch at once on the end screen. In build mode this is
    /// a no-op. Returns the mode afterwards.
    pub fn press_build(&mut self) -> Driver {
        if self.driver == Driver::Play {
            if self.level_select.take().is_some() {
                // As with the players question: the level select closes.
            } else if self.players_dialog {
                // A press anywhere else closes the players question.
                self.players_dialog = false;
            } else if self.dialog {
                // Pressing the button again while asked is "keep playing".
                self.dialog = false;
            } else if self.round_in_progress() {
                self.dialog = true;
            } else {
                self.enter_build();
            }
        }
        self.driver
    }

    /// Answer the leave dialog. A no-op when it is not open.
    pub fn answer_dialog(&mut self, leave: bool) -> Driver {
        if self.dialog {
            self.dialog = false;
            if leave {
                self.enter_build();
            }
        }
        self.driver
    }

    /// Into the builder, its camera opening on what the round showed: the
    /// whole canvas after an arena, the followed view's world after a
    /// field map - while the canvas is the round's map's size, so the view
    /// still names the same ground.
    fn enter_build(&mut self) {
        self.dialog = false;
        self.players_dialog = false;
        self.driver = Driver::Build;
        if let Some(view) = self.play_view
            && self.builder.map().field_size() == self.game.map.field_size()
        {
            self.builder.look_at(view);
        }
    }

    /// The players button in the HUD: open the players dialog, or close it
    /// if it is already up. Play mode only, and a no-op while the leave
    /// dialog is asking or the level select is up - one question at a
    /// time. Works on the end screen
    /// too (the restart countdown waits). Returns whether it is open. Never
    /// opens where two players are not offered (`TWO_PLAYERS_AVAILABLE`).
    pub fn press_players(&mut self) -> bool {
        if crate::TWO_PLAYERS_AVAILABLE
            && self.driver == Driver::Play
            && !self.dialog
            && self.level_select.is_none()
            && self.game.map.training.is_none()
        {
            self.players_dialog = !self.players_dialog;
        }
        self.players_dialog
    }

    pub fn close_players_dialog(&mut self) {
        self.players_dialog = false;
    }

    /// Answer the players dialog: a different count restarts the round at
    /// once in that mode (the same path as the R key - a `--seed` stays
    /// pinned, the banner shows), the current count just closes it. A
    /// no-op when it is not open. Returns the count afterwards.
    pub fn answer_players(&mut self, count: PlayerCount) -> PlayerCount {
        if self.players_dialog {
            self.players_dialog = false;
            if count != self.game.players {
                self.game.players = count;
                self.clear_attempt = None;
                let (width, height) = self.game.map.field_size();
                self.game.init(width, height);
            }
        }
        self.game.players
    }

    /// `PLAY` from the builder: the edited map becomes the round's map and
    /// a fresh round starts from the map's own start (a `--seed` stays
    /// pinned, the mission banner shows as on any restart); a win in it
    /// clears the canvas's revision (`clear_attempt`). A no-op in play
    /// mode.
    pub fn play(&mut self) -> Driver {
        if self.driver == Driver::Build {
            // Nothing under way in the builder - a menu, a rectangle half
            // drawn, a finger - is there on the next BUILD.
            self.builder.leave();
            self.game.map = self.builder.map().clone();
            // A level's canvas is that level for the rest of the session.
            if let Some(campaign) = &mut self.campaign {
                campaign.remember_edit(&self.game.map);
            }
            self.game.start_override = None;
            self.sync_level();
            // The canvas carries no stamp, so the round's map is the
            // builder's revision.
            self.clear_attempt = Some(self.builder.revision());
            let (width, height) = self.game.map.field_size();
            self.game.init(width, height);
            self.driver = Driver::Play;
            self.dialog = false;
            self.players_dialog = false;
        }
        self.driver
    }

    /// `PLAY HERE` from the builder (docs/large-maps-patterns.md, "Play
    /// from here"): `play`, with seat 1 put down on the cell nearest the
    /// middle of the builder's view a tank can start on in the part of the
    /// map its own start drives in (`play_here_cell`), where the round's
    /// camera opens. The map and its history are untouched: the spot is
    /// the round's (`Game::start_override`), kept by its restarts and gone
    /// with the next `PLAY`. Seat 2 on a couch starts beside seat 1. A
    /// round like this is a test, not a clear or a level won. With no such
    /// cell anywhere, it is a plain `PLAY`. A no-op in play mode.
    pub fn play_here(&mut self) -> Driver {
        if self.driver != Driver::Build {
            return self.driver;
        }
        let vp = self.builder.viewport();
        let near = self.builder.camera().center(&vp);
        self.play();
        if let Some(cell) = play_here_cell(&self.game, near) {
            // The same round as the one just set up, its start moved: the
            // seed it drew is pinned for the one `init` and given back. A
            // test: it clears nothing, and its end screen restarts it.
            self.clear_attempt = None;
            let pinned = self.game.seed_override;
            self.game.seed_override = Some(self.game.round_seed());
            self.game.start_override = Some(cell);
            self.sync_level();
            let (width, height) = self.game.map.field_size();
            self.game.init(width, height);
            self.game.seed_override = pinned;
        }
        self.driver
    }

    /// The `Tab` key: `press_build` in play mode, `play` in build mode -
    /// except while the builder's Save prompt is taking text, when a key
    /// is a character and not a command. Online it is inert: the round
    /// belongs to the room, and leaving it is `leave_online`.
    pub fn toggle(&mut self) -> Driver {
        match self.driver {
            Driver::Play => self.press_build(),
            Driver::Build if self.builder.text_entry_open() => self.driver,
            Driver::Build => self.play(),
            Driver::Lobby | Driver::Online => self.driver,
        }
    }

    /// The HUD's `ONLINE` button: open the lobby over the local round,
    /// which is left exactly where it stands (nothing here calls
    /// `Game::update`, and `playing()` is false from this frame on).
    /// A no-op where a build cannot reach a room, and while the builder
    /// is up - the lobby is play's button. Returns the mode afterwards.
    pub fn press_online(&mut self) -> Driver {
        if !crate::ONLINE_AVAILABLE || self.driver != Driver::Play {
            return self.driver;
        }
        self.dialog = false;
        self.players_dialog = false;
        self.level_select = None;
        self.lobby = Some(Lobby::new(self.site.clone(), self.rooms.clone()));
        self.driver = Driver::Lobby;
        self.driver
    }

    /// Open the lobby on a room that has already been dialled - the
    /// command line's `--host`, `--join CODE` and `--rig`. The screen
    /// shows the code and the QR while the room fills up, and hands over
    /// to `Driver::Online` the moment the round begins.
    pub fn open_lobby_with(&mut self, round: AnyRound) {
        self.dialog = false;
        self.players_dialog = false;
        self.lobby = Some(Lobby::new(self.site.clone(), self.rooms.clone()));
        self.online = Some(round);
        self.driver = Driver::Lobby;
    }

    /// Put `round` in the session's seat, whatever opened it.
    pub fn attach_round(&mut self, round: AnyRound) {
        self.online = Some(round);
    }

    /// Dial a room for the lobby's `HOST` or `JOIN`, or for the room a
    /// link named, through `net::client::connect` - the same socket the
    /// command line opens. A build that cannot reach one never gets
    /// here: every caller is gated on `ONLINE_AVAILABLE`.
    #[cfg(feature = "online")]
    fn dial(&mut self, target: Target) {
        let identity = crate::net::client::Identity::new(self.nick.clone(), self.token.clone());
        let client = crate::net::client::connect(&self.rooms, identity, target);
        self.attach_round(crate::net::round::OnlineRound::new(client, "ROOM"));
    }

    #[cfg(not(feature = "online"))]
    fn dial(&mut self, _target: Target) {}

    /// Take the seat the link this build was opened on names: the web
    /// build's `/j/AK7QX` or `?join=AK7QX` (`net::rooms::Invite`,
    /// docs/online-coop-prd.md §4.10), read once at start the way
    /// `--join CODE` is.
    ///
    /// The lobby opens on the room rather than on its opening face -
    /// there is nothing to choose, the link chose - and hands over to
    /// `Driver::Online` the moment the host starts the round. The local
    /// round is built and left standing as usual, so `LEAVE` comes out
    /// at a game.
    pub fn join_room(&mut self, code: RoomCode) {
        if !crate::ONLINE_AVAILABLE {
            return;
        }
        self.dialog = false;
        self.players_dialog = false;
        self.lobby = Some(Lobby::new(self.site.clone(), self.rooms.clone()));
        self.dial(Target::Join(code));
        self.driver = Driver::Lobby;
    }

    /// Take the window into `round` without the lobby - the primitive
    /// `open_lobby_with` and the hand-over from the lobby both use. The
    /// local round and the builder are left exactly as they stand.
    pub fn go_online(&mut self, round: AnyRound) {
        self.dialog = false;
        self.players_dialog = false;
        self.lobby = None;
        self.online = Some(round);
        self.driver = Driver::Online;
    }

    /// Give up the seat, close the lobby and come back to the local
    /// round, which has been standing still the whole time (nothing
    /// online ever calls `Game::update`). A no-op in Play and Build.
    pub fn leave_online(&mut self) -> Driver {
        if let Some(round) = &mut self.online {
            round.leave();
        }
        self.online = None;
        self.lobby = None;
        if matches!(self.driver, Driver::Online | Driver::Lobby) {
            self.driver = Driver::Play;
        }
        self.driver
    }

    /// One frame of the lobby: what the room says, then the screen's own
    /// hit tests - the panel centred in the chrome's `area`, UI points -
    /// then whatever it asked for. `app.rs` and the dev server both fill
    /// the same `LobbyInput`, so a tool's click lands on the hit test a
    /// finger does.
    ///
    /// The room's round is polled from here, which is what makes a seat
    /// fill up and a roster arrive while the screen is on; the seat sends
    /// no intent, before a round or after one, since `OnlineRound` sends
    /// only while the room's round is playing. The frame the
    /// room starts the round the window hands over to `Driver::Online`
    /// and the replica is what is drawn. Returns the mode afterwards.
    pub fn update_lobby(&mut self, input: &LobbyInput, area: Rect, dt: f32) -> Driver {
        if self.driver != Driver::Lobby {
            return self.driver;
        }
        if let Some(round) = &mut self.online {
            round.frame(&Intent::default(), dt);
        }
        let room = self.online.as_ref().map(RoomView::of);
        let Some(lobby) = &mut self.lobby else { return self.driver };
        match lobby.update(input, area, room.as_ref()) {
            LobbyAction::None => {}
            LobbyAction::Host { map, mission } => {
                self.dial(Target::Host(RoomSetup { map, map_toml: None, mission, seed: None }));
            }
            // The screen only offers `JOIN` on a code it has parsed, so a
            // refusal here is not something a player can reach.
            LobbyAction::Join { code } => {
                if let Ok(code) = RoomCode::parse(&code) {
                    self.dial(Target::Join(code));
                }
            }
            LobbyAction::Ready => {
                if let Some(round) = &mut self.online {
                    round.ready();
                }
            }
            LobbyAction::Start => {
                if let Some(round) = &mut self.online {
                    round.start_round();
                }
            }
            LobbyAction::Kick { seat } => {
                if let Some(round) = &mut self.online {
                    round.kick(seat);
                }
            }
            LobbyAction::Leave => {
                self.leave_online();
            }
        }
        // The round has begun: the replica is the picture from here on.
        if self.driver == Driver::Lobby && room.is_some_and(|r| r.phase == RoomPhase::Playing) {
            self.lobby = None;
            self.driver = Driver::Online;
        }
        self.driver
    }

    /// One rendered frame of the online round: what arrived, this seat's
    /// `intent` (seat 0 of the frame's `Input`), and the picture between
    /// two snapshots. A no-op in the other modes.
    pub fn update_online(&mut self, intent: &Intent, dt: f32) {
        if self.driver != Driver::Online {
            return;
        }
        if let Some(round) = &mut self.online {
            round.frame(intent, dt);
        }
        // The room ticks its round through the end screen and says so
        // when the countdown has run out, so the banner has played by
        // the time the window comes back to the lobby. The seat and the
        // room are kept - the same roster, code and QR are on screen a
        // frame later, with the host's button reading `REMATCH`.
        if self.online.as_ref().is_some_and(|r| r.ended().is_some()) {
            self.back_to_lobby();
        }
    }

    /// Put the room screen back over the round without giving the seat
    /// up: a fresh `Lobby` on the session's rooms host, since everything
    /// about the room itself is read off the client each frame.
    fn back_to_lobby(&mut self) {
        self.lobby = Some(Lobby::new(self.site.clone(), self.rooms.clone()));
        self.driver = Driver::Lobby;
    }

    /// The round on screen: the room's replica in online mode - the local
    /// one until a `Welcome` has built it - and the session's own
    /// otherwise. Everything that draws (`Game::render`, the HUD, `fx`)
    /// reads this rather than `game`.
    pub fn shown(&self) -> &Game {
        match self.driver {
            Driver::Online => self.online.as_ref().and_then(AnyRound::game).unwrap_or(&self.game),
            _ => &self.game,
        }
    }

    /// The round on screen, to write a *drawing* flag on: the dev
    /// server's overlays have to land on the `Game` that is drawn.
    /// Nothing writes simulation state through here - an online round's
    /// belongs to the room.
    pub fn shown_mut(&mut self) -> &mut Game {
        if self.driver == Driver::Online && self.online.as_ref().is_some_and(|r| r.game().is_some()) {
            return self.online.as_mut().and_then(AnyRound::game_mut).expect("just checked");
        }
        &mut self.game
    }

    /// Replace the session's map wholesale - the dev server's `restart`
    /// with a `map`/`map_toml`: the game's map for the round it is about
    /// to start, and the builder's canvas and baseline.
    pub fn replace_map(&mut self, map: MapFile) {
        self.game.map = map.clone();
        self.game.start_override = None;
        self.clear_attempt = None;
        self.builder.load(map);
        self.sync_level();
    }

    /// One frame of the builder, in build mode, on the window `frame`
    /// describes: `PLAY` starts the round, `PLAY HERE` starts it from the
    /// middle of the builder's view.
    pub fn update_builder(&mut self, input: &BuilderInput, frame: &BuilderFrame) {
        if self.driver != Driver::Build {
            return;
        }
        // A question takes every press, Enter and Esc while it is up.
        if self.question.is_some() {
            if input.pressed
                && let Some(p) = input.pointer
            {
                self.press_question(frame.to_ui(p), frame.ui.area);
            } else if input.enter {
                self.answer_question(true);
            } else if input.escape {
                self.answer_question(false);
            }
            return;
        }
        // The CHECK panel lints the canvas the way PLAY would set it up.
        self.builder.lint_setup = crate::maplint::LintSetup::of(&self.game);
        match self.builder.update(input, frame) {
            EditorAction::None => {}
            EditorAction::Play => {
                self.play();
            }
            EditorAction::PlayHere => {
                self.play_here();
            }
        }
        if let Some(question) = self.builder.take_question() {
            self.question = Some(question);
        }
    }

    /// Ask about a modified copy whose original this build ships changed
    /// (`mapstore::Store::stale`), once per map a session plays or builds
    /// on: the round's map and the builder's canvas, in play or build
    /// mode. `app.rs` calls it every frame; nothing with map modding off.
    pub fn watch_originals(&mut self) {
        let Some(store) = self.builder.store() else { return };
        if self.question.is_some() || !matches!(self.driver, Driver::Play | Driver::Build) {
            return;
        }
        for name in [self.game.map.name.clone(), self.builder.map().name.clone()].into_iter().flatten() {
            if self.originals_checked.insert(name.clone()) && store.stale(&name) {
                self.dialog = false;
                self.players_dialog = false;
                self.level_select = None;
                self.question = Some(Question::OriginalChanged { name });
                return;
            }
        }
    }

    /// A press on the question, in UI points over the chrome's `area`:
    /// its first button answers yes, its second or anywhere off the panel
    /// no. Answers whether there was a question to take it.
    pub fn press_question(&mut self, at: crate::math::Vec2, area: Rect) -> bool {
        if self.question.is_none() {
            return false;
        }
        let r = crate::hud::question_rects(area);
        if r.yes.contains(at) {
            self.answer_question(true);
        } else if r.no.contains(at) || !r.panel.contains(at) {
            self.answer_question(false);
        }
        true
    }

    /// Answer the question. Yes forgets the map's modified copy and puts
    /// the original in its place - on the builder's canvas as a new
    /// document, and in a round on that map, which starts over on it. No
    /// to a changed original keeps the copy and asks no more about it. A
    /// no-op when nothing is asked.
    pub fn answer_question(&mut self, yes: bool) {
        let Some(question) = self.question.take() else { return };
        let Some(store) = self.builder.store() else { return };
        let name = question.name().to_string();
        if !yes {
            if let Question::OriginalChanged { .. } = question
                && let Err(e) = store.keep(&name)
            {
                eprintln!("[mapstore] {name}: {e}");
            }
            return;
        }
        if let Err(e) = store.revert(&name) {
            eprintln!("[mapstore] {name}: {e}");
            self.builder.set_status(e);
            return;
        }
        if let Some(campaign) = &mut self.campaign {
            campaign.forget_edit(&name);
        }
        let Some(mut original) = crate::mapstore::original(&name).and_then(|text| MapFile::from_toml_str(text).ok()) else { return };
        original.name = Some(name.clone());
        if self.builder.map().name.as_deref() == Some(name.as_str()) {
            self.builder.open(original.clone());
            self.builder.set_status(crate::text::text().fmt(crate::text::keys::EDITOR_REVERTED, &[("name", name.as_str().into())]));
        }
        if self.driver == Driver::Play && self.game.map.name.as_deref() == Some(name.as_str()) {
            match self.level() {
                Some(i) => {
                    if let Err(e) = self.start_level(i) {
                        eprintln!("[levels] {e}");
                    }
                }
                None => {
                    self.game.map = original;
                    self.game.start_override = None;
                    self.clear_attempt = None;
                    self.sync_level();
                    let (width, height) = self.game.map.field_size();
                    self.game.init(width, height);
                }
            }
        }
    }

    /// The field the live mode is showing: the round's map in play mode,
    /// the builder's canvas in build mode (a map loaded into the builder
    /// may be a different size until PLAY makes it the round's).
    pub fn field_size(&self) -> (f32, f32) {
        match self.driver {
            Driver::Play | Driver::Lobby => self.game.map.field_size(),
            Driver::Build => self.builder.map().field_size(),
            Driver::Online => self.shown().map.field_size(),
        }
    }

    /// What `Game::render` should draw around the world this frame: the
    /// corner clusters in play mode and in an online round. An online
    /// round shows none of the local buttons - the round is the room's to
    /// restart and the builder is not part of it - and carries a status
    /// line instead, until the lobby screen replaces it.
    pub fn play_chrome(&self) -> PlayChrome {
        match self.driver {
            Driver::Online => PlayChrome {
                hud: true,
                status: self.online.as_ref().map(AnyRound::status),
                // The one button a room's round carries: the way back to
                // the local one on a build with no Esc key.
                leave_button: true,
                // The vitals are this seat's, whichever seat the room
                // gave it; the others are the compact strip's.
                seat: self.online.as_ref().and_then(AnyRound::seat),
                prompt: crate::hud::rod_prompt(self.shown(), self.online.as_ref().and_then(AnyRound::seat)),
                // A room's round does not restart where it stands: the
                // end screen counts down to the lobby it came from.
                countdown_label: Some(crate::text::keys::ROUND_BACK_TO_LOBBY),
                minimap: self.minimap_slot(),
                lamp_row: self.shown().lamps_in_play() || !self.shown().lava().is_empty(),
                ..PlayChrome::default()
            },
            Driver::Lobby => {
                let room = self.online.as_ref().map(RoomView::of);
                PlayChrome {
                    lobby: self.lobby.as_ref().map(|lobby| lobby.view(room.as_ref())),
                    ..PlayChrome::default()
                }
            }
            _ => PlayChrome {
                // The builder draws its own bar and none of this.
                hud: self.driver == Driver::Play,
                build_button: true,
                // A training round seats one, so it offers no second.
                players_button: crate::TWO_PLAYERS_AVAILABLE && self.game.map.training.is_none(),
                online_button: crate::ONLINE_AVAILABLE,
                restart_button: !crate::KEYBOARD_AVAILABLE,
                leave_button: false,
                // The P key's stand-in, which a phone has no other way to.
                pause_button: true,
                paused: self.game.paused,
                seat: None,
                leave_dialog: self.dialog,
                players_dialog: self.players_dialog,
                question: self.question.clone(),
                status: None,
                prompt: crate::hud::rod_prompt(&self.game, 0..self.game.players.count().min(2) as u8),
                lobby: None,
                countdown_label: None,
                level: self.level().zip(self.campaign.as_ref()).and_then(|(i, campaign)| {
                    let level = campaign.levels.get(i)?;
                    Some(LevelBanner { number: campaign.levels.number(i), count: campaign.levels.last_number(), title: level.title() })
                }),
                result: self.result_view(),
                curtain: self.curtain(),
                level_button: self.level_button(),
                levels: self.level_select.as_ref().zip(self.campaign.as_ref()).map(|(select, campaign)| select.view(campaign, self.level())),
                minimap: (self.driver == Driver::Play).then(|| self.minimap_slot()).flatten(),
                lamp_row: self.game.lamps_in_play() || !self.game.lava().is_empty(),
            },
        }
    }

    /// The minimap's size in points (`minimap::MinimapRules::size_pt`) for
    /// the round on screen, where this window draws one (`minimap_on`).
    fn minimap_slot(&self) -> Option<(f32, f32)> {
        self.minimap_on.then(|| crate::minimap::MinimapRules::current().size_pt(self.shown().map.field_size()))
    }

    /// The live buttons of whatever stands over the round and takes a press
    /// before the corners do - the level select's open tiles and BACK, a
    /// dialog's two, a level's end screen's, the lobby's - by name, in the
    /// UI points of `ui`, from the same geometry the painter and the hit
    /// tests read. What the dev server's `status.ui.screen_buttons` and the
    /// web build's `bb_ui_json` report.
    pub fn screen_buttons(&self, ui: &crate::hud::UiFrame) -> Vec<(String, crate::math::Rectangle)> {
        use crate::hud::{leave_dialog_rects, players_dialog_rects};
        use crate::level_select::{back_rect, tile_rect, TileState};
        let chrome = self.play_chrome();
        let mut out = Vec::new();
        if chrome.question.is_some() {
            let r = crate::hud::question_rects(ui.area);
            out.extend([("yes".to_string(), r.yes), ("no".to_string(), r.no)]);
        } else if let Some(levels) = &chrome.levels {
            for (i, tile) in levels.tiles.iter().enumerate().filter(|(_, t)| t.state != TileState::Locked) {
                out.push((format!("level_{}", tile.number), tile_rect(ui.area, i)));
            }
            out.push(("back".to_string(), back_rect(ui.area)));
        } else if chrome.players_dialog {
            let r = players_dialog_rects(ui.area);
            out.extend([("one".to_string(), r.one), ("two".to_string(), r.two)]);
        } else if chrome.leave_dialog {
            let r = leave_dialog_rects(ui.area);
            out.extend([("leave".to_string(), r.leave), ("stay".to_string(), r.stay)]);
        } else if self.driver == Driver::Play
            && let Some(r) = chrome.result.as_ref().and_then(|view| result_layout(ui.area, view).buttons)
        {
            // Only play draws the end screen: the builder over a decided
            // round shows none of its buttons.
            out.extend([("levels".to_string(), r.levels), ("again".to_string(), r.again)]);
            out.extend(r.next.map(|next| ("next".to_string(), next)));
        }
        if let Some(lobby) = &chrome.lobby {
            for b in lobby.buttons.iter().filter(|b| b.enabled) {
                out.push((b.button.name(), crate::lobby::button_rect(ui.area, b.button)));
            }
        }
        out
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;
    use crate::editor::Tool;
    use crate::map::CellObject;
    use crate::obstacle::{Material, Obstacle};

    const W: f32 = crate::DEFAULT_SCREEN_WIDTH as f32;
    const H: f32 = crate::DEFAULT_SCREEN_HEIGHT as f32;

    fn session() -> Session {
        let mut game = Game::default();
        game.enemy_count_override = Some(1);
        game.seed_override = Some(7);
        game.map = MapFile::from_toml_str(include_str!("../maps/default.toml")).expect("default map parses");
        game.init(W, H);
        Session::new(game)
    }

    /// `session` on the shipped `default` map, named as a level opens it,
    /// keeping its maps in a store of its own (`mapstore::Memory`) in which
    /// `default` has a modified copy - a wall at (1, 1) - made from another
    /// original than this build's, when `stale`.
    fn modded_session(stale: bool) -> (Session, crate::mapstore::Store<'static>) {
        use crate::mapstore::{Backend, Memory, Store};
        let memory: &'static Memory = Box::leak(Box::new(Memory::default()));
        let store = Store(memory);
        let mut edited = MapFile::from_toml_str(crate::mapstore::original("default").expect("shipped")).expect("parses");
        edited.set_cell(1, 1, CellObject::Wall { material: Material::Iron });
        store.save("default", &edited).expect("kept");
        if stale {
            let text = memory.read("default-modd").expect("kept");
            let body = text.split_once('\n').expect("a header").1;
            memory.write("default-modd", &format!("# modified from 0000000000000000\n{body}")).expect("writes");
        }
        let mut s = session();
        let mut map = MapFile::from_toml_str(&store.source("default").expect("opens")).expect("parses");
        map.name = Some("default".to_string());
        s.game.map = map.clone();
        s.game.init(W, H);
        s.builder.open(map);
        s.builder.use_store(Some(store));
        (s, store)
    }

    /// A modified copy made from the original this build ships asks
    /// nothing; one made from another asks once, freezing the round, and
    /// SWITCH forgets the copy and starts the round over on the original,
    /// in the builder too.
    #[test]
    fn a_changed_original_asks_and_switch_takes_it() {
        let (mut s, _) = modded_session(false);
        s.watch_originals();
        assert_eq!(s.question, None, "made from this build's original");

        let (mut s, store) = modded_session(true);
        s.watch_originals();
        assert_eq!(s.question, Some(Question::OriginalChanged { name: "default".to_string() }));
        assert!(!s.playing(), "the round stands still behind the question");
        let r = crate::hud::question_rects(Rect::new(0.0, 0.0, 800.0, 400.0));
        assert!(s.press_question(crate::math::Vec2::new(r.yes.x + 4.0, r.yes.y + 4.0), Rect::new(0.0, 0.0, 800.0, 400.0)));
        assert_eq!(s.question, None);
        assert!(!store.is_modified("default"), "the copy is gone");
        let original = MapFile::from_toml_str(crate::mapstore::original("default").expect("shipped")).expect("parses");
        assert_eq!(s.game.map.cell(1, 1), original.cell(1, 1), "the round is on the original");
        assert_eq!(s.builder.map().cell(1, 1), original.cell(1, 1), "and so is the builder");
        assert!(s.playing());
        s.watch_originals();
        assert_eq!(s.question, None, "asked once");
    }

    /// KEEP MINE keeps the modified copy, now as made from this build's
    /// original, so it is not asked about again.
    #[test]
    fn keep_mine_keeps_the_copy_and_asks_no_more() {
        let (mut s, store) = modded_session(true);
        s.watch_originals();
        assert!(s.question.is_some());
        s.answer_question(false);
        assert_eq!(s.question, None);
        assert!(store.is_modified("default") && !store.stale("default"));
        assert_eq!(s.game.map.cell(1, 1), Some(&CellObject::Wall { material: Material::Iron }), "the round stays on the copy");
    }

    /// FILE > REVERT MAP in the builder asks; REVERT forgets the copy and
    /// opens the original on the canvas, KEEP changes nothing.
    #[test]
    fn revert_in_the_builder_asks_first() {
        let (mut s, store) = modded_session(false);
        s.driver = Driver::Build;
        s.question = Some(Question::Revert { name: "default".to_string() });
        s.answer_question(false);
        assert!(store.is_modified("default"), "KEEP changes nothing");
        s.question = Some(Question::Revert { name: "default".to_string() });
        s.answer_question(true);
        assert!(!store.is_modified("default"));
        let original = MapFile::from_toml_str(crate::mapstore::original("default").expect("shipped")).expect("parses");
        assert_eq!(s.builder.map().cell(1, 1), original.cell(1, 1), "the canvas is the original");
        assert!(!s.builder.dirty(), "as a new document");
    }

    /// `session` on a map shown whole: a 34 x 17 arena with a start and a
    /// frog, which no shipped map is.
    fn arena_session() -> Session {
        let mut game = Game::default();
        game.enemy_count_override = Some(1);
        game.seed_override = Some(7);
        game.map = MapFile::from_toml_str("version = 1\ncells.\"5,5\" = { kind = \"start\" }\ncells.\"30,15\" = { kind = \"frog\" }\n")
            .expect("arena map parses");
        game.init(W, H);
        Session::new(game)
    }

    #[test]
    fn build_asks_mid_round_and_switches_at_once_on_the_end_screen() {
        let mut s = session();
        assert_eq!(s.press_build(), Driver::Play);
        assert!(s.dialog && !s.playing());
        assert_eq!(s.answer_dialog(false), Driver::Play);
        assert!(!s.dialog && s.playing());
        s.press_build();
        assert_eq!(s.answer_dialog(true), Driver::Build);
        assert!(!s.playing());
        // Back in play on the end screen: no question.
        s.play();
        s.game.debug_kill(0).expect("player slot exists");
        s.game.update(crate::simulation::Input::default(), crate::PHYSICS_FIXED_DT, W, H);
        assert_ne!(s.game.outcome(), Outcome::Playing);
        assert_eq!(s.press_build(), Driver::Build);
        assert!(!s.dialog);
    }

    /// BUILD opens the builder's camera on what the round last showed: a
    /// followed view's world, or FIT for the whole field.
    #[test]
    fn build_opens_the_builders_camera_on_what_play_showed() {
        let mut s = session();
        s.play_view = Some(crate::math::Rectangle::new(100.0, 96.0, 400.0, 200.0));
        s.press_build();
        s.answer_dialog(true);
        assert_eq!(s.mode(), Driver::Build);
        let vp = s.builder.viewport();
        assert!(!s.builder.camera().is_fit());
        let c = s.builder.camera().center(&vp);
        assert!((c.x - 300.0).abs() < 1.0 && (c.y - 196.0).abs() < 1.0, "{c:?}");
        let r = s.builder.camera().view(&vp).rect();
        assert!(r.x <= 101.0 && r.y <= 97.0 && r.x + r.width >= 499.0 && r.y + r.height >= 295.0, "the view holds what play showed: {r:?}");
        // The whole field again: FIT.
        s.play();
        let (w, h) = s.game.map.field_size();
        s.play_view = Some(crate::math::Rectangle::new(0.0, 0.0, w, h));
        s.press_build();
        s.answer_dialog(true);
        assert!(s.builder.camera().is_fit());
    }

    #[test]
    fn play_starts_a_round_on_the_edited_map_and_edits_survive_the_round_trip() {
        let mut s = session();
        s.press_build();
        s.answer_dialog(true);
        s.builder.select_tool(Tool::Wall(Material::Iron));
        let cell = (20, 11);
        s.builder.stroke(&[cell], false);
        assert!(s.builder.dirty());
        assert_eq!(s.play(), Driver::Play);
        let at = crate::map::cell_to_world(cell.0, cell.1);
        let iron_there = s
            .game
            .world
            .query::<&Obstacle>()
            .iter()
            .any(|o| o.material == Material::Iron && (o.position - at).length() < 1.0);
        assert!(iron_there, "the new round was built from the builder's map");
        assert_eq!(s.game.map.cell(cell.0, cell.1), Some(&CellObject::Wall { material: Material::Iron }));
        // Build again: the canvas is still the edited map, not reseeded.
        s.press_build();
        s.answer_dialog(true);
        assert!(s.builder.dirty());
        assert_eq!(s.builder.map().cell(cell.0, cell.1), Some(&CellObject::Wall { material: Material::Iron }));
    }

    /// A menu open when a round starts is gone on the next BUILD, and Tab
    /// while the dev Save prompt is taking text is a character, not PLAY.
    #[test]
    fn play_closes_the_builders_menu_and_tab_yields_to_the_save_prompt() {
        use crate::editor::{BuilderFrame, BuilderInput};
        let mut s = session();
        s.press_build();
        s.answer_dialog(true);
        let frame = BuilderFrame::headless(s.builder.map().field_size(), s.builder.map().class());
        // A named button's middle, on the window (`named_buttons`).
        let at = |s: &Session, name: &str| {
            let r = s.builder.named_buttons(&frame).into_iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no {name}")).1;
            frame.ui.to_window(crate::math::Vec2::new(r.x + r.width / 2.0, r.y + r.height / 2.0))
        };
        let press = BuilderInput { pointer: Some(at(&s, "map")), pressed: true, held: true, ..Default::default() };
        s.update_builder(&press, &frame);
        assert_eq!(s.builder.open_menu(), Some("map"));
        assert_eq!(s.toggle(), Driver::Play);
        s.press_build();
        s.answer_dialog(true);
        assert_eq!(s.builder.open_menu(), None, "the settings panel stayed open across PLAY");

        // The Save-as prompt (FILE > SAVE AS...) takes text: Tab is a
        // character there, not PLAY.
        let press = BuilderInput { pointer: Some(at(&s, "file")), pressed: true, held: true, ..Default::default() };
        s.update_builder(&press, &frame);
        assert_eq!(s.builder.open_menu(), Some("file"));
        if crate::map::saving_available() {
            let press = BuilderInput { pointer: Some(at(&s, "save_as")), pressed: true, held: true, ..Default::default() };
            s.update_builder(&press, &frame);
            assert_eq!(s.builder.open_menu(), Some("save"));
            assert_eq!(s.toggle(), Driver::Build, "Tab in the Save prompt started a round");
            s.update_builder(&BuilderInput { escape: true, ..Default::default() }, &frame);
            assert_eq!(s.builder.open_menu(), None);
            assert_eq!(s.toggle(), Driver::Play);
        }
    }

    /// Nothing a finger had under way outlives the builder: a rectangle a
    /// finger is still drawing when the round starts (Tab, a second finger
    /// on PLAY, the dev server's `play`) is taken back, not filled; and the
    /// finger - still down when BUILD brings the builder back, or down
    /// again under the same id, as Android hands ids out again - is nobody's
    /// on the canvas: it taps, strokes and zooms nothing when it lifts.
    #[test]
    fn play_then_build_with_a_finger_leaves_nothing_pending() {
        use crate::editor::{BuilderFrame, BuilderInput, CanvasScreen, Shape};
        use crate::math::Vec2;
        use crate::touch::TouchPoint;
        for same_id in [true, false] {
            // An arena, so its cells are drawn large enough for a finger to
            // paint at FIT (`builder_paint_min_cell_mm`).
            let mut s = arena_session();
            s.press_build();
            s.answer_dialog(true);
            let ui = crate::hud::UiFrame::new((1600.0, 900.0), 1.0, 1.0, crate::hud::Insets::default(), true);
            let frame = BuilderFrame::new(ui, s.builder.map().field_size(), s.builder.map().class(), None);
            let screen = CanvasScreen { device_per_px: frame.view.scale * 2.0, points_per_px: frame.view.scale, coarse: false };
            s.update_builder(&BuilderInput { screen: Some(screen), ..Default::default() }, &frame);
            s.builder.select_tool(Tool::Wall(Material::Brick));
            s.builder.set_shape(Shape::Rect);
            let before = s.builder.map().clone();
            let finger = |id: i32, at: Vec2, pressed: bool| BuilderInput {
                pointer: Some(at),
                pressed,
                held: true,
                touches: vec![TouchPoint { id, pos: at }],
                dt: 1.0 / 60.0,
                ..Default::default()
            };
            let f = frame.layout.field;
            let middle = frame.view.to_window(Vec2::new(f.x + f.w / 2.0, f.y + f.h / 2.0));
            s.update_builder(&finger(0, middle, true), &frame);
            for i in 1..=10 {
                s.update_builder(&finger(0, Vec2::new(middle.x + 12.0 * i as f32, middle.y + 6.0 * i as f32), false), &frame);
            }
            assert!(s.builder.rect_stroke().is_some(), "a rectangle is being drawn");
            assert_eq!(s.toggle(), Driver::Play);
            assert!(s.builder.rect_stroke().is_none(), "the rectangle outlived the builder");
            assert_eq!(s.builder.map().cells, before.cells, "leaving filled the rectangle");
            s.press_build();
            assert_eq!(s.answer_dialog(true), Driver::Build);
            let camera = *s.builder.camera();
            let id = if same_id { 0 } else { 1 };
            let at = Vec2::new(middle.x - 200.0, middle.y - 100.0);
            for _ in 0..3 {
                s.update_builder(&finger(id, at, false), &frame);
            }
            s.update_builder(&BuilderInput { dt: 1.0 / 60.0, ..Default::default() }, &frame);
            assert_eq!(s.builder.map().cells, before.cells, "same id {same_id}: the finger painted");
            assert_eq!(s.builder.history().undo_depth(), 0);
            assert_eq!(*s.builder.camera(), camera, "same id {same_id}: the finger moved the view");
        }
    }

    #[test]
    fn the_players_dialog_freezes_the_round_and_yields_to_the_leave_dialog() {
        let mut s = session();
        assert!(s.press_players());
        assert!(s.players_dialog && !s.playing());
        // The BUILD button while it asks just closes it.
        assert_eq!(s.press_build(), Driver::Play);
        assert!(!s.players_dialog && !s.dialog && s.playing());
        // Pressed again: a toggle.
        assert!(s.press_players());
        assert!(!s.press_players());
        assert!(s.playing());
        // One question at a time: the leave dialog blocks the players button.
        s.press_build();
        assert!(s.dialog);
        assert!(!s.press_players());
        assert!(s.dialog && !s.players_dialog);
        s.answer_dialog(false);
        // In build mode the button does nothing.
        s.press_build();
        s.answer_dialog(true);
        assert!(!s.press_players());
    }

    #[test]
    fn answering_with_the_other_mode_restarts_in_it_and_the_same_mode_just_closes() {
        let mut s = session();
        for _ in 0..5 {
            s.game.update(crate::simulation::Input::default(), crate::PHYSICS_FIXED_DT, W, H);
        }
        assert_eq!(s.game.players, PlayerCount::ONE);
        assert!(s.game.seat(1).is_none());
        // Closed: answering is a no-op.
        assert_eq!(s.answer_players(PlayerCount::TWO), PlayerCount::ONE);
        assert!(s.game.seat(1).is_none());

        s.press_players();
        assert_eq!(s.answer_players(PlayerCount::TWO), PlayerCount::TWO);
        assert!(!s.players_dialog && s.playing());
        assert_eq!(s.game.frame(), 0, "a new round started");
        assert!(s.game.seat(1).is_some());
        for _ in 0..5 {
            s.game.update(crate::simulation::Input::default(), crate::PHYSICS_FIXED_DT, W, H);
        }
        // The same count: closes without a restart.
        s.press_players();
        assert_eq!(s.answer_players(PlayerCount::TWO), PlayerCount::TWO);
        assert_eq!(s.game.frame(), 5);
        assert!(!s.players_dialog);
        // The mode sticks: an R restart and a BUILD -> PLAY round trip keep it.
        s.game.update(crate::simulation::Input { restart_pressed: true, ..Default::default() }, crate::PHYSICS_FIXED_DT, W, H);
        assert_eq!(s.game.players, PlayerCount::TWO);
        assert!(s.game.seat(1).is_some());
        s.press_build();
        s.answer_dialog(true);
        s.play();
        assert_eq!(s.game.players, PlayerCount::TWO);
        assert!(s.game.seat(1).is_some());
        // And back to one.
        s.press_players();
        assert_eq!(s.answer_players(PlayerCount::ONE), PlayerCount::ONE);
        assert!(s.game.seat(1).is_none());
    }

    /// Online is a third driver, not a replacement: the local round and
    /// the builder are exactly where they were when it started, and
    /// `playing()` - the one predicate that decides whether
    /// `Game::update` runs - stays Play's alone.
    #[test]
    fn going_online_freezes_the_local_round_and_leaves_the_builder_alone() {
        use crate::net::client::{Identity, RoomClient, RoomSetup};
        use crate::net::loopback::{self, LinkQuality};
        use crate::net::round::OnlineRound;
        use crate::net::transport::Transport;

        let mut s = session();
        s.builder.select_tool(Tool::Wall(Material::Iron));
        s.builder.stroke(&[(20, 11)], false);
        for _ in 0..5 {
            s.game.update(crate::simulation::Input::default(), crate::PHYSICS_FIXED_DT, W, H);
        }
        let local = s.game.drawable_state();
        let canvas = s.builder.map().clone();

        // A room nobody answers: the driver is online, and the window
        // draws the local round until a `Welcome` builds a replica.
        let (link, _room) = loopback::pair(LinkQuality::PERFECT, 5);
        let client = RoomClient::host(
            Box::new(link) as Box<dyn Transport>,
            Identity::new("host", "tok"),
            RoomSetup::default(),
        );
        s.go_online(OnlineRound::new(client, "ROOM"));
        assert_eq!(s.mode(), Driver::Online);
        assert!(!s.playing(), "the local round only ever runs in play mode");
        assert!(std::ptr::eq(s.shown(), &s.game), "nothing to draw yet but the local round");

        // Frames of the online round change nothing about the local one.
        for _ in 0..10 {
            s.update_online(&crate::ai::Intent::default(), 1.0 / 60.0);
        }
        assert_eq!(s.game.frame(), 5, "the local round took a step");
        assert_eq!(s.game.drawable_state(), local);
        assert_eq!(s.builder.map(), &canvas, "the builder kept its edit");
        assert!(s.builder.dirty());

        // Play's own buttons are gone while the round belongs to a room.
        let chrome = s.play_chrome();
        assert!(!chrome.build_button && !chrome.players_button && !chrome.restart_button && !chrome.pause_button);
        assert!(chrome.status.is_some_and(|line| line.contains("ROOM")), "the status line names the mode");
        // Tab is inert; leaving comes back to the local round as it was.
        assert_eq!(s.toggle(), Driver::Online);
        assert_eq!(s.leave_online(), Driver::Play);
        assert!(s.online.is_none() && s.playing());
        assert_eq!(s.game.frame(), 5);
        s.game.update(crate::simulation::Input::default(), crate::PHYSICS_FIXED_DT, W, H);
        assert_eq!(s.game.frame(), 6, "the local round carries on where it stood");
    }

    /// The lobby is a mode like Build: the round it stands over is
    /// frozen because `playing()` is false and nothing calls `update`,
    /// the builder's canvas is untouched, and closing it comes out where
    /// the round stood.
    #[test]
    fn the_lobby_freezes_the_local_round_and_leaves_the_builder_alone() {
        use crate::lobby::{Button, LobbyInput, Stage, button_rect};
        use crate::net::rooms::RoomsHost;

        let mut s = session();
        s.rooms = RoomsHost::overriding("ws://127.0.0.1:4848");
        s.builder.select_tool(Tool::Wall(Material::Iron));
        s.builder.stroke(&[(20, 11)], false);
        for _ in 0..5 {
            s.game.update(crate::simulation::Input::default(), crate::PHYSICS_FIXED_DT, W, H);
        }
        let local = s.game.drawable_state();
        let canvas = s.builder.map().clone();

        if !crate::ONLINE_AVAILABLE {
            assert_eq!(s.press_online(), Driver::Play, "a build with no client never opens the lobby");
            return;
        }
        assert_eq!(s.press_online(), Driver::Lobby);
        assert!(!s.playing(), "the local round only ever runs in play mode");
        assert!(s.online.is_none(), "no room until a button asks for one");
        assert!(std::ptr::eq(s.shown(), &s.game), "the lobby stands over the local round");
        // Play's own buttons are gone; the lobby is what is drawn.
        let chrome = s.play_chrome();
        assert!(!chrome.build_button && !chrome.players_button && !chrome.online_button);
        let view = chrome.lobby.expect("the lobby is on screen");
        assert_eq!(view.stage, Stage::Start);
        assert_eq!(view.map, crate::map::SHIPPED_MAPS[0].0);

        // Frames of it change nothing about the round or the canvas, and
        // walking into the code entry and back out is all local.
        let field = area();
        let press = |b: Button| {
            let r = button_rect(field, b);
            LobbyInput {
                pointer: Some(crate::math::Vec2::new(r.x + r.width / 2.0, r.y + r.height / 2.0)),
                pressed: true,
                ..LobbyInput::default()
            }
        };
        s.update_lobby(&press(Button::Join), field, 1.0 / 60.0);
        for _ in 0..10 {
            s.update_lobby(&LobbyInput::default(), field, 1.0 / 60.0);
        }
        assert_eq!(s.play_chrome().lobby.expect("still up").stage, Stage::Code);
        assert_eq!(s.game.frame(), 5, "the local round took a step");
        assert_eq!(s.game.drawable_state(), local);
        assert_eq!(s.builder.map(), &canvas, "the builder kept its edit");
        assert!(s.builder.dirty());

        // CLOSE comes back to the local round exactly where it stood.
        s.update_lobby(&press(Button::Back), field, 1.0 / 60.0);
        s.update_lobby(&press(Button::Back), field, 1.0 / 60.0);
        assert_eq!(s.mode(), Driver::Play);
        assert!(s.lobby.is_none() && s.playing());
        assert_eq!(s.game.frame(), 5);
        s.game.update(crate::simulation::Input::default(), crate::PHYSICS_FIXED_DT, W, H);
        assert_eq!(s.game.frame(), 6, "the local round carries on where it stood");
        // Tab is inert while the lobby is up, and the builder's own mode
        // never offers it.
        s.press_online();
        assert_eq!(s.toggle(), Driver::Lobby);
        s.leave_online();
        s.press_build();
        s.answer_dialog(true);
        assert_eq!(s.press_online(), Driver::Build, "the lobby is the play bar's button");
    }

    /// A round dialled from the command line comes up in the lobby, and
    /// hands over to `Driver::Online` the frame the room starts.
    #[test]
    fn a_command_line_room_opens_the_lobby_and_hands_over_when_it_starts() {
        use crate::lobby::LobbyInput;
        use crate::net::client::{Identity, RoomClient, RoomSetup};
        use crate::net::loopback::{self, LinkQuality};
        use crate::net::round::OnlineRound;
        use crate::net::transport::Transport;

        let mut s = session();
        let (link, _room) = loopback::pair(LinkQuality::PERFECT, 5);
        let client = RoomClient::host(Box::new(link) as Box<dyn Transport>, Identity::new("host", "tok"), RoomSetup::default());
        s.open_lobby_with(OnlineRound::new(client, "ROOM"));
        assert_eq!(s.mode(), Driver::Lobby);
        assert!(!s.playing());
        let field = area();
        // Nobody answers, so the screen stays on "reaching the room" and
        // the local round is still the picture behind it.
        for _ in 0..5 {
            assert_eq!(s.update_lobby(&LobbyInput::default(), field, 1.0 / 60.0), Driver::Lobby);
        }
        assert!(s.play_chrome().lobby.is_some());
        assert!(s.play_chrome().status.is_none(), "the round's line waits for the round");
        // Leaving hangs up and comes out at the local round.
        assert_eq!(s.leave_online(), Driver::Play);
        assert!(s.online.is_none() && s.lobby.is_none() && s.playing());
    }

    /// A round the room plays out is not a dead end: the window comes
    /// back to the room screen on the same seat, the same code and the
    /// same roster, with the host's action reading `REMATCH` - and the
    /// local round is still standing exactly where the lobby left it.
    #[test]
    fn a_round_the_room_ends_hands_the_window_back_to_the_lobby() {
        use crate::lobby::{Button, LobbyInput, Stage};
        use crate::math::Vec2;
        use crate::net::client::{Identity, RoomClient, RoomSetup};
        use crate::net::codec::{self, Msg};
        use crate::net::loopback::{self, LinkQuality};
        use crate::net::round::OnlineRound;
        use crate::net::transport::Transport;
        use crate::net::wire::{Lobby as LobbyMsg, RosterSeat, RoundOutcome, Seat as WireSeat};
        use crate::net::{MAX_SEATS, encode};

        const DT: f32 = 1.0 / 60.0;
        let field = area();
        let mut s = session();
        let local = s.game.frame();

        // The room's side of a loopback link, answered by hand.
        let mut authority = Game::default();
        authority.seed_override = Some(0xB0B5);
        authority.enemy_count_override = Some(0);
        authority.map = MapFile::from_toml_str(include_str!("../maps/default.toml")).expect("default map parses");
        let (w, h) = authority.map.field_size();
        authority.init(w, h);
        let roster = vec![RosterSeat { seat: 0, nick: "oto".into(), chassis: 3, ready: false, connected: true }];
        let welcome = || {
            encode::welcome(&authority, 0, vec![WireSeat { seat: 0, nick: "oto".into(), chassis: 3 }], "{}".into(), [0; MAX_SEATS])
                .expect("the map serialises")
        };

        let (link, mut room) = loopback::pair(LinkQuality::PERFECT, 5);
        let client = RoomClient::host(Box::new(link) as Box<dyn Transport>, Identity::new("oto", "tok"), RoomSetup::default());
        s.open_lobby_with(OnlineRound::new(client, "ROOM"));
        let say = |room: &mut loopback::Loopback, msg: Msg| room.send(&codec::encode(&msg));
        say(&mut room, Msg::Lobby(LobbyMsg::RoomCreated { code: "AK7QX".into() }));
        say(&mut room, Msg::Lobby(LobbyMsg::Roster { host: 0, seats: roster.clone() }));
        say(&mut room, Msg::Lobby(LobbyMsg::Started));
        say(&mut room, Msg::Welcome(welcome()));
        assert_eq!(s.update_lobby(&LobbyInput::default(), field, DT), Driver::Online, "the round begins");

        // The room plays its end screen out and then says how it went.
        say(&mut room, Msg::Lobby(LobbyMsg::Ended { outcome: RoundOutcome::Won }));
        say(&mut room, Msg::Lobby(LobbyMsg::Roster { host: 0, seats: roster.clone() }));
        s.update_online(&Intent::default(), DT);
        assert_eq!(s.mode(), Driver::Lobby, "the end of the round comes back to the room screen");
        let seat = s.online.as_ref().expect("the seat is kept");
        assert_eq!(seat.seat(), Some(0));
        assert_eq!(seat.code(), Some("AK7QX"), "the same room, not a new one");
        assert_eq!(s.game.frame(), local, "the local round moved while the window was online");
        assert!(!s.playing());

        // The room face is back, on the same roster, saying how the round
        // went, with the host's action reading REMATCH.
        s.update_lobby(&LobbyInput::default(), field, DT);
        let view = s.play_chrome().lobby.expect("the room screen");
        assert_eq!(view.stage, Stage::Room);
        assert_eq!(view.code.as_deref(), Some("AK7QX"));
        assert_eq!(view.seats.len(), 1);
        assert!(view.sub.starts_with("ROUND WON"), "{}", view.sub);
        let start = view.buttons.iter().find(|b| b.button == Button::Start).expect("the host's action");
        assert_eq!(start.label, "REMATCH");
        assert!(start.enabled, "the only other seat is the host's own, so the rematch is live");

        // Pressing it is the `Start` the room already takes from a host.
        let rect = crate::lobby::button_rect(field, Button::Start);
        let tap = LobbyInput {
            pointer: Some(Vec2::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)),
            pressed: true,
            ..LobbyInput::default()
        };
        s.update_lobby(&tap, field, DT);
        let mut heard = Vec::new();
        room.drain(&mut heard);
        assert!(heard.iter().any(|m| matches!(m, Msg::Lobby(LobbyMsg::Start))), "the rematch never left: {heard:?}");

        // And the round the room starts is drawn like any other.
        say(&mut room, Msg::Lobby(LobbyMsg::Started));
        say(&mut room, Msg::Welcome(welcome()));
        assert_eq!(s.update_lobby(&LobbyInput::default(), field, DT), Driver::Online);
        assert!(s.play_chrome().status.is_some(), "the round's own line is back over the field");
    }

    /// A session in the builder on `map`, one enemy a round, the canvas
    /// at FIT.
    fn build_session(map: MapFile) -> Session {
        let mut game = Game::default();
        game.enemy_count_override = Some(1);
        game.seed_override = Some(7);
        game.map = map;
        let (w, h) = game.map.field_size();
        game.init(w, h);
        let mut s = Session::new(game);
        s.driver = Driver::Build;
        s
    }

    /// The standard field, its start at the far left.
    fn open_field() -> MapFile {
        let mut map = MapFile::new();
        map.set_cell(3, 8, CellObject::Start);
        map
    }

    fn iron(map: &mut MapFile, col: i32, row: i32) {
        map.set_cell(col, row, CellObject::Wall { material: Material::Iron });
    }

    /// Every cell PLAY HERE could have taken in `game`, by the rule it is
    /// documented with, checked the long way: legal against every tile
    /// and deep cell, reachable from the start, dry, off a portal.
    fn could_start_on(game: &Game, cell: (i32, i32)) -> bool {
        let (w, h) = game.map.field_size();
        let grid = game.nav_grid(w, h);
        let p = crate::map::cell_to_world(cell.0, cell.1);
        let mut obstacles: Vec<crate::Position> = game.world.query::<&Obstacle>().iter().map(|o| o.position).collect();
        obstacles.extend(game.water().deep_cells());
        let start = game.seat_pose(0).expect("player 1").position;
        crate::battlefield::enemy_spawn_legal(p, w, h, 0.0, f32::INFINITY, p, 0.0, &grid, &obstacles)
            && grid.components().connected(&grid, start, p)
            && game.water().depth_at(p) != crate::ground::Depth::Deep
    }

    /// PLAY HERE puts player 1 on the legal cell nearest the middle of the
    /// builder's view - no legal cell is nearer - and leaves the map and
    /// its history alone; the round's restarts keep the spot and PLAY
    /// goes back to the map's start.
    #[test]
    fn play_here_starts_on_the_legal_cell_nearest_the_view() {
        let mut map = open_field();
        // A wall right where the view's middle is: the cell under it and
        // the ring a tank's box needs round it are out.
        for (c, r) in [(16, 8), (17, 8), (18, 8), (17, 9)] {
            iron(&mut map, c, r);
        }
        let mut s = build_session(map.clone());
        let vp = s.builder.viewport();
        let near = s.builder.camera().center(&vp);
        let depth = s.builder.history().undo_depth();
        assert_eq!(s.play_here(), Driver::Play);
        let cell = s.game.start_override.expect("a spot");
        assert_eq!(s.game.seat_pose(0).unwrap().position, crate::map::cell_to_world(cell.0, cell.1), "player 1 stands on it");
        assert!(could_start_on(&s.game, cell), "{cell:?} is a legal start");
        let d = crate::map::cell_to_world(cell.0, cell.1).distance_to(near);
        assert!(d < 4.0 * 32.0, "near the middle of the view: {cell:?} is {d} px from {near:?}");
        for row in 0..=17 {
            for col in 0..=34 {
                let nearer = crate::map::cell_to_world(col, row).distance_to(near) < d - 1e-3;
                assert!(!(nearer && could_start_on(&s.game, (col, row))), "({col},{row}) is nearer and legal");
            }
        }
        assert_eq!(s.builder.map(), &map, "the map is untouched");
        assert_eq!(s.game.map, map);
        assert_eq!(s.builder.history().undo_depth(), depth, "and so is its history");
        // R starts the round over on the same spot; PLAY on the start.
        s.game.update(crate::simulation::Input { restart_pressed: true, ..Default::default() }, crate::PHYSICS_FIXED_DT, W, H);
        assert_eq!(s.game.seat_pose(0).unwrap().position, crate::map::cell_to_world(cell.0, cell.1));
        s.press_build();
        s.answer_dialog(true);
        s.play();
        assert_eq!(s.game.start_override, None);
        assert_eq!(s.game.seat_pose(0).unwrap().position, crate::map::cell_to_world(3, 8), "PLAY starts on the map's start");
    }

    /// Deep water and a pen the start never reaches are no place to start:
    /// with a lake over the middle of the view and an iron pen beside it,
    /// the spot is on the dry ground the start drives on.
    #[test]
    fn play_here_refuses_deep_water_and_a_penned_spot() {
        let mut map = open_field();
        // A lake over the middle of the field...
        for col in 13..=21 {
            for row in 5..=12 {
                map.set_cell(col, row, CellObject::Water);
            }
        }
        // ... and an iron pen just right of it, its inside open but cut
        // off from the start.
        for row in 3..=14 {
            iron(&mut map, 23, row);
            iron(&mut map, 31, row);
        }
        for col in 23..=31 {
            iron(&mut map, col, 3);
            iron(&mut map, col, 14);
        }
        let mut s = build_session(map);
        // Look at the middle of the pen: its inside is the nearest ground.
        let vp = s.builder.viewport();
        s.builder.frame_camera(crate::math::Vec2::new(27.0 * 32.0, 8.5 * 32.0), 3.0);
        let near = s.builder.camera().center(&vp);
        assert!((near.x - 27.0 * 32.0).abs() < 1.0, "{near:?}");
        let cell = play_here_cell(&s.game, near).expect("a spot");
        let p = crate::map::cell_to_world(cell.0, cell.1);
        assert!(!(cell.0 > 23 && cell.0 < 31 && cell.1 > 3 && cell.1 < 14), "not inside the pen: {cell:?}");
        assert!(s.game.water().depth_at(p) != crate::ground::Depth::Deep, "not in the lake: {cell:?}");
        assert!(could_start_on(&s.game, cell), "{cell:?}");
        // And looking at the lake's middle: the shore, never the deep.
        s.builder.frame_camera(crate::math::Vec2::new(17.0 * 32.0, 8.5 * 32.0), 3.0);
        let near = s.builder.camera().center(&vp);
        let cell = play_here_cell(&s.game, near).expect("a spot");
        assert!(could_start_on(&s.game, cell), "{cell:?}");
        assert!(s.game.water().depth_at(crate::map::cell_to_world(cell.0, cell.1)) != crate::ground::Depth::Deep);
        s.play_here();
        assert_eq!(s.game.start_override, Some(cell));
    }

    /// On a couch, player 2 starts beside player 1's spot, not on the
    /// map's `start2` by the map's start; and a win from the spot is no
    /// level won.
    #[test]
    fn play_here_seats_player_two_beside_player_one_and_wins_no_level() {
        let mut map = open_field();
        map.set_cell(3, 11, CellObject::Start2);
        let mut s = build_session(map);
        s.game.players = PlayerCount::TWO;
        s.play_here();
        let one = s.game.seat_pose(0).unwrap().position;
        let two = s.game.seat_pose(1).unwrap().position;
        // Beside: the nearest open cell a clear tank's width (two of its
        // size) off player 1.
        assert!(one.distance_to(two) < 6.0 * 32.0, "beside player 1: {one:?} and {two:?}");
        assert!(two.distance_to(crate::map::cell_to_world(3, 11)) > 4.0 * 32.0, "not on the map's start2: {two:?}");

        let mut s = level_session(two_levels(), 0);
        s.driver = Driver::Build;
        s.play_here();
        let spot = s.game.start_override.expect("a spot");
        assert!(!s.game.hold_end_screen, "a test's end screen restarts on its own");
        finish(&mut s, true);
        assert_eq!(s.take_progress(), None, "a test from a spot wins no level");
        let view = s.play_chrome().result.expect("the end screen");
        assert_eq!(view.buttons, None, "free play's end screen: no way on to the next level");
        assert!(!s.next_level(), "and nothing leads there");
        let frames = end_frames(&s);
        to_verdict(&mut s);
        assert!(!s.enter_result());
        for _ in 0..frames + 60 {
            step(&mut s);
        }
        assert_eq!((s.level(), s.game.outcome(), s.game.start_override), (Some(0), Outcome::Playing, Some(spot)), "the same level again, from the spot");
        assert_eq!(s.builder.map().name.as_deref(), Some("lotus-lagoon"), "the builder still holds the level");

        // PLAY from the builder is the level again, its end screen a
        // level's.
        s.press_build();
        s.answer_dialog(true);
        s.play();
        assert_eq!(s.game.start_override, None);
        assert!(s.game.hold_end_screen);
    }

    /// A builder session on the open field with one enemy by the map's
    /// own `tanks` - no override - and the round clock running from the
    /// first frame, so a win from PLAY is a clear.
    fn clear_session() -> Session {
        let mut map = open_field();
        map.tanks = Some(1);
        let mut game = Game::default();
        game.seed_override = Some(7);
        game.show_intro = false;
        game.map = map;
        let (w, h) = game.map.field_size();
        game.init(w, h);
        let mut s = Session::new(game);
        s.driver = Driver::Build;
        s
    }

    fn tenth(seconds: f32) -> f64 {
        (seconds as f64 * 10.0).round() / 10.0
    }

    /// The clear check: a win in the round PLAY started, played as the map
    /// is authored, clears the canvas's revision with the round clock as
    /// its par; the round started again is still that attempt, a quicker
    /// win lowers the par and a slower one leaves it.
    #[test]
    fn a_win_from_plain_play_clears_the_revision_with_its_par() {
        let mut s = clear_session();
        let revision = s.builder.revision();
        assert_eq!(s.builder.par(), None);
        s.play();
        assert_eq!(s.clear_attempt, Some(revision));
        for _ in 0..90 {
            step(&mut s);
        }
        finish(&mut s, true);
        let first = s.game.round_stats().seconds;
        assert!(first > 1.0, "{first}");
        assert_eq!(s.builder.par(), Some(tenth(first)), "cleared, the clock its par");
        for _ in 0..30 {
            step(&mut s);
        }
        assert_eq!(s.builder.par(), Some(tenth(first)), "the end screen notes it once");
        s.play_again();
        for _ in 0..30 {
            step(&mut s);
        }
        finish(&mut s, true);
        let quicker = s.game.round_stats().seconds;
        assert!(quicker < first);
        assert_eq!(s.builder.par(), Some(tenth(quicker)), "the best win is the par");
        s.play_again();
        for _ in 0..150 {
            step(&mut s);
        }
        finish(&mut s, true);
        assert_eq!(s.builder.par(), Some(tenth(quicker)), "a slower win leaves it");
        // BUILD shows it, and the map SAVE would write carries it.
        s.press_build();
        assert_eq!(s.driver, Driver::Build);
        assert_eq!(s.builder.map_to_save().cleared_par(), Some(tenth(quicker)));
    }

    /// What clears nothing: a win from PLAY HERE, with a second seat, with
    /// an enemy count from the command line, in a round PLAY did not start
    /// or after a loss; and an edit since PLAY - a win on the round's
    /// revision clears the revision that was won, never the edited canvas,
    /// which an undo back finds cleared.
    #[test]
    fn only_a_revision_won_as_authored_is_cleared() {
        let mut s = clear_session();
        s.play_here();
        assert_eq!(s.clear_attempt, None);
        finish(&mut s, true);
        assert_eq!(s.builder.par(), None, "a test from a spot");

        let mut s = clear_session();
        s.game.players = PlayerCount::TWO;
        s.play();
        finish(&mut s, true);
        assert_eq!(s.builder.par(), None, "two seats");

        let mut s = clear_session();
        s.game.enemy_count_override = Some(1);
        s.play();
        finish(&mut s, true);
        assert_eq!(s.builder.par(), None, "an enemy count from the command line");

        // Another sky than the map's, as `--weather` would put in.
        let mut s = clear_session();
        s.play();
        s.game.weather = crate::map::Weather::Night;
        finish(&mut s, true);
        assert_eq!(s.builder.par(), None, "another sky");

        // A chassis the `player_tank` knob picked, as the dev panel's would.
        let mut s = clear_session();
        s.play();
        assert!(played_as_authored(&s.game, &crate::tuning::Tuning::DEFAULT));
        let picked = crate::tuning::Tuning { player_tank: 3, ..crate::tuning::Tuning::DEFAULT };
        assert!(!played_as_authored(&s.game, &picked), "a chassis from the knob");

        // Night falling on a map that asks for it is the map's own sky.
        let mut s = clear_session();
        s.play();
        s.game.map.nightfall = Some(0.0);
        s.game.tick_nightfall();
        assert_eq!(s.game.weather(), crate::map::Weather::Night);
        assert!(played_as_authored(&s.game, &crate::tuning::Tuning::DEFAULT), "nightfall is the map's");

        let mut s = clear_session();
        s.driver = Driver::Play;
        finish(&mut s, true);
        assert_eq!(s.builder.par(), None, "a round PLAY did not start");

        let mut s = clear_session();
        s.play();
        finish(&mut s, false);
        assert_eq!(s.builder.par(), None, "a loss");

        let mut s = clear_session();
        let won = s.builder.revision();
        s.play();
        s.press_build();
        s.answer_dialog(true);
        s.builder.stroke(&[(20, 5)], false);
        assert_ne!(s.builder.revision(), won);
        // A tool's restart: the round PLAY started, on its own map.
        s.driver = Driver::Play;
        let (w, h) = s.game.map.field_size();
        s.game.init(w, h);
        finish(&mut s, true);
        assert_eq!(s.builder.par(), None, "the edited canvas is not what was won");
        s.builder.undo();
        assert_eq!(s.builder.revision(), won);
        assert!(s.builder.par().is_some(), "the revision that was won is cleared");
    }

    #[test]
    fn replace_map_sets_both_the_game_and_the_builder() {
        let mut s = session();
        let mut map = MapFile::new();
        map.set_cell(3, 3, CellObject::Gate);
        s.replace_map(map);
        assert_eq!(s.game.map.cell(3, 3), Some(&CellObject::Gate));
        assert_eq!(s.builder.map().cell(3, 3), Some(&CellObject::Gate));
        assert!(!s.builder.dirty());
    }

    /// Two band levels, so a win on the second is the last level's.
    fn two_levels() -> crate::levels::Levels {
        crate::levels::Levels::parse(
            "[[level]]\nmap = \"lotus-lagoon\"\ntitle = \"Lotus Lagoon\"\n[[level]]\nmap = \"glasshouses\"\ntitle = \"Glasshouse Gardens\"\n",
        )
        .expect("the test's levels parse")
    }

    /// A session on `levels`, on level `i`, one enemy a round.
    fn level_session(levels: crate::levels::Levels, i: usize) -> Session {
        let campaign = Campaign::new(levels, None);
        let mut game = Game::default();
        game.enemy_count_override = Some(1);
        game.seed_override = Some(7);
        game.map = campaign.map(i).expect("the level opens");
        let (w, h) = game.map.field_size();
        game.init(w, h);
        let mut s = Session::new(game);
        s.set_campaign(campaign);
        s
    }

    /// Boot Camp is played alone: its corners offer no players button and
    /// the dialog does not open over it.
    #[test]
    fn a_training_round_offers_no_second_seat() {
        let mut s = level_session(crate::levels::Levels::shipped(), 0);
        assert!(s.game.map.training.is_some(), "level 0 is the training stage");
        assert!(!s.play_chrome().players_button);
        assert!(!s.press_players(), "the players dialog stays shut");
    }

    /// One frame as `app.rs` runs it: a step while the round is live,
    /// then the session's reading of it.
    fn step(s: &mut Session) {
        if s.playing() {
            let (w, h) = s.game.map.field_size();
            s.game.update(crate::simulation::Input::default(), crate::PHYSICS_FIXED_DT, w, h);
        }
        s.note_outcome();
        s.follow_countdown();
        s.tick_curtain(crate::PHYSICS_FIXED_DT);
    }

    /// A decided round's whole end screen, in frames: its finale, its
    /// fade and its countdown (`EndBeats::total`).
    fn end_frames(s: &Session) -> usize {
        (s.game.end_beats().total() * 60.0).ceil() as usize
    }

    /// Skip the finale and step until the end screen takes presses.
    fn to_verdict(s: &mut Session) {
        assert!(s.game.in_finale(), "the round was just decided");
        s.game.skip_finale();
        for _ in 0..(crate::tuning::tuning().round_verdict_fade_seconds * 60.0).ceil() as usize + 1 {
            step(s);
        }
    }

    /// Step until the end screen takes presses, the finale already skipped.
    fn to_verdict_from_skip(s: &mut Session) {
        for _ in 0..(crate::tuning::tuning().round_verdict_fade_seconds * 60.0).ceil() as usize + 1 {
            step(s);
        }
    }

    /// Step through the fade out a press on the end screen runs before it
    /// takes its way.
    fn through_fade(s: &mut Session) {
        let t = crate::tuning::tuning();
        for _ in 0..(t.level_fade_seconds.max(t.retry_fade_seconds) * 60.0).ceil() as usize + 2 {
            step(s);
        }
    }

    /// Wreck the round's enemy (`win`) or player 1 (`!win`) and step
    /// once, so the round ends.
    fn finish(s: &mut Session, win: bool) {
        let slot = if win {
            s.game.world.query::<&crate::tank::Tank>().with::<&crate::ai::Ai>().iter().map(|t| t.owner_slot()).min().expect("an enemy")
        } else {
            0
        };
        s.game.debug_kill(slot).expect("the tank exists");
        step(s);
        assert_eq!(s.game.outcome(), if win { Outcome::Won } else { Outcome::Lost });
    }

    fn centre(r: crate::math::Rectangle) -> crate::math::Vec2 {
        crate::math::Vec2::new(r.x + r.width / 2.0, r.y + r.height / 2.0)
    }

    /// The chrome's area on a window of the default map's bitmap, a point
    /// a pixel: where the end screen, the level select and the lobby lay
    /// themselves out (`hud::UiFrame`).
    fn area() -> Rect {
        crate::hud::UiFrame::plain((1088.0, 576.0)).area
    }

    /// The end screen's buttons are play's: the builder over a decided
    /// round draws none of them, so it reports none either.
    #[test]
    fn the_end_screens_buttons_are_reported_only_in_play() {
        let mut s = level_session(two_levels(), 0);
        finish(&mut s, false);
        let ui = crate::hud::UiFrame::plain((1088.0, 576.0));
        let names = |s: &Session| s.screen_buttons(&ui).into_iter().map(|(name, _)| name).collect::<Vec<_>>();
        assert!(names(&s).iter().any(|n| n == "again"), "{:?}", names(&s));
        s.press_build();
        assert_eq!(s.mode(), Driver::Build, "a decided round asks nothing on the way out");
        assert!(!names(&s).iter().any(|n| n == "again" || n == "levels" || n == "next"), "{:?}", names(&s));
    }

    fn result_rects(s: &Session) -> Option<crate::hud::ResultRects> {
        let view = s.play_chrome().result?;
        crate::hud::result_layout(area(), &view).buttons
    }

    /// A level opens with its number and title, and a win is progress the
    /// frame it happens. The end screen counts `restart_delay` down to
    /// `NEXT LEVEL` and takes it by itself - Enter or the button take it
    /// at once - while the last level's win counts nothing down and waits
    /// for a button, whose way on is the first level again.
    #[test]
    fn a_won_level_counts_down_to_the_next_and_the_last_waits_for_a_button() {
        let mut s = level_session(two_levels(), 0);
        assert_eq!(s.level(), Some(0));
        assert!(s.game.hold_end_screen);
        let banner = s.play_chrome().level.expect("a level's banner");
        assert_eq!((banner.number, banner.count, banner.title.as_str()), (1, 2, "Lotus Lagoon"));
        assert!(s.play_chrome().result.is_none(), "no end screen mid-round");

        finish(&mut s, true);
        assert_eq!(s.take_progress().as_deref(), Some("glasshouses"), "the next level is reached");
        assert_eq!(s.take_progress(), None, "once");
        let view = s.play_chrome().result.expect("the end screen");
        let seconds = crate::tuning::tuning().restart_delay.ceil() as u32;
        assert_eq!(view.buttons, Some(ResultButtons { next: Some(NextLevel::Next), countdown: Some(seconds) }));
        assert_eq!((view.stats.destroyed, view.stats.enemies), (1, 1));
        assert!(s.play_chrome().result.is_some() && s.game.in_finale(), "the finale plays before the end screen");
        for _ in 0..end_frames(&s) - 30 {
            step(&mut s);
        }
        assert_eq!((s.level(), s.game.outcome()), (Some(0), Outcome::Won), "still counting");
        assert_eq!(s.play_chrome().result.and_then(|v| v.buttons).and_then(|b| b.countdown), Some(1));
        for _ in 0..40 {
            step(&mut s);
        }
        assert_eq!((s.level(), s.game.outcome()), (Some(1), Outcome::Playing), "on to the next level by itself");
        assert_eq!(s.builder.map().name.as_deref(), Some("glasshouses"), "the builder holds the level on the field");
        assert_eq!(s.builder.history().undo_depth(), 0, "and undo cannot walk back into the last one");
        assert_eq!(s.play_chrome().level.expect("a level").number, 2);

        // The last level: every level complete, and no countdown round
        // to the first.
        finish(&mut s, true);
        assert_eq!(s.take_progress(), None, "the last level opens nothing further");
        let buttons = s.play_chrome().result.and_then(|v| v.buttons).expect("the buttons");
        assert_eq!(buttons, ResultButtons { next: Some(NextLevel::FirstAgain { levels: 2 }), countdown: None });
        for _ in 0..end_frames(&s) + 60 {
            step(&mut s);
        }
        assert_eq!((s.level(), s.game.outcome()), (Some(1), Outcome::Won), "the campaign's end waits");
        let rects = result_rects(&s).expect("the buttons");
        assert!(s.press_result(centre(rects.next.expect("the way on")), area()));
        assert!(s.curtain() < 1.0, "the fade out runs first");
        through_fade(&mut s);
        assert_eq!(s.level(), Some(0), "round to the first");

        // A press in the finale goes to the verdict; then Enter does not
        // wait for the countdown.
        finish(&mut s, true);
        assert!(s.enter_result(), "Enter skips the finale");
        assert!(!s.game.in_finale());
        to_verdict_from_skip(&mut s);
        assert!(s.enter_result());
        through_fade(&mut s);
        assert_eq!((s.level(), s.game.outcome()), (Some(1), Outcome::Playing));
    }

    /// A lost level counts down to `PLAY AGAIN` - the same level, a fresh
    /// round, nothing gained - and takes it by itself, but not while the
    /// level select stands over it; the button and Enter take it at once,
    /// and a press off the buttons is not theirs.
    #[test]
    fn a_lost_level_counts_down_to_the_same_level_again() {
        let mut s = level_session(two_levels(), 0);
        finish(&mut s, false);
        assert_eq!(s.take_progress(), None);
        let view = s.play_chrome().result.expect("the end screen");
        let seconds = crate::tuning::tuning().level_loss_retry_seconds.ceil() as u32;
        assert_eq!(view.buttons, Some(ResultButtons { next: None, countdown: Some(seconds) }));
        assert!(!s.next_level(), "no way on after a loss");
        let frames = end_frames(&s);
        to_verdict(&mut s);
        let rects = result_rects(&s).expect("the button");
        assert!(rects.next.is_none());
        assert!(!s.press_result(crate::math::Vec2::new(4.0, 4.0), area()), "a press off the buttons");
        assert_eq!(s.game.outcome(), Outcome::Lost);

        for _ in 0..60 {
            step(&mut s);
        }
        assert!(s.press_levels());
        for _ in 0..frames + 60 {
            step(&mut s);
        }
        assert_eq!((s.level(), s.game.outcome()), (Some(0), Outcome::Lost), "frozen behind the level select");
        assert!(!s.press_levels());
        for _ in 0..frames {
            step(&mut s);
            if s.game.outcome() == Outcome::Playing {
                break;
            }
        }
        assert_eq!((s.level(), s.game.outcome()), (Some(0), Outcome::Playing), "the same level again, by itself");
        assert!(s.game.frame() < 90, "a fresh round");

        finish(&mut s, false);
        to_verdict(&mut s);
        assert!(s.press_result(centre(result_rects(&s).expect("the button").again), area()));
        through_fade(&mut s);
        assert_eq!((s.level(), s.game.outcome()), (Some(0), Outcome::Playing));
        finish(&mut s, false);
        to_verdict(&mut s);
        assert!(s.enter_result());
        through_fade(&mut s);
        assert_eq!((s.level(), s.game.outcome()), (Some(0), Outcome::Playing));
    }

    /// A level edited in the builder is played as edited, and still is
    /// when the campaign comes back round to it; the dialogs and the
    /// builder keep the end screen's buttons out of reach.
    #[test]
    fn a_levels_edit_is_played_every_time_it_comes_round() {
        let mut s = level_session(two_levels(), 0);
        let mut edited = s.builder.map().clone();
        edited.set_cell(5, 5, CellObject::Gate);
        s.press_build();
        s.answer_dialog(true);
        s.builder.load(edited.clone());
        s.play();
        assert_eq!(s.level(), Some(0), "the canvas is still that level by name");
        assert_eq!(s.game.map.cell(5, 5), Some(&CellObject::Gate));
        finish(&mut s, true);
        s.next_level();
        finish(&mut s, true);
        s.next_level();
        assert_eq!(s.level(), Some(0));
        assert_eq!(s.game.map.cell(5, 5), Some(&CellObject::Gate), "played as edited");
        assert_eq!(s.builder.map().cell(5, 5), Some(&CellObject::Gate), "and the builder has the edit");
        finish(&mut s, true);
        s.press_build();
        assert_eq!(s.mode(), Driver::Build, "the end screen switches at once");
        assert!(!s.press_result(centre(result_rects(&s).expect("buttons").again), area()), "not while the builder is up");
    }

    /// The level select stands over a frozen round and starts only a
    /// level already reached; closing it leaves the round exactly where
    /// it stood, and a replay - won or lost - moves nothing. The end
    /// screen's `LEVELS` opens it, as the HUD's level button does.
    #[test]
    fn the_level_select_replays_a_reached_level_over_a_frozen_round() {
        use crate::level_select::{back_rect, tile_rect, SelectInput, TileState};
        assert!(!session().press_levels(), "a session with no levels has no level select");

        let mut s = level_session(two_levels(), 0);
        let field = area();
        let press = |p| SelectInput { pointer: Some(p), pressed: true, ..SelectInput::default() };
        assert_eq!(s.level_button(), Some(1));
        step(&mut s);
        let frame = s.game.frame();
        if crate::TWO_PLAYERS_AVAILABLE {
            s.press_players();
        }
        assert!(s.press_levels());
        assert!(!s.players_dialog, "one question at a time");
        assert!(!s.playing(), "the round stands still behind it");
        let view = s.play_chrome().levels.expect("the screen");
        assert_eq!((view.tiles[0].state, view.tiles[1].state), (TileState::Next, TileState::Locked));
        assert!(!s.update_level_select(&press(centre(tile_rect(field, 1))), field));
        assert!(s.level_select.is_some(), "a locked level is no button");
        assert!(!s.update_level_select(&press(centre(back_rect(field))), field));
        assert!(s.level_select.is_none() && s.playing());
        assert_eq!((s.game.frame(), s.level()), (frame, Some(0)), "the same round, where it stood");

        // Won: level 2 is reached, and the end screen opens the screen.
        finish(&mut s, true);
        assert_eq!(s.take_progress().as_deref(), Some("glasshouses"));
        to_verdict(&mut s);
        assert!(s.press_result(centre(result_rects(&s).expect("the buttons").levels), area()));
        assert!(s.level_select.is_some());
        assert!(s.update_level_select(&press(centre(tile_rect(field, 0))), field), "level 1 again");
        assert_eq!((s.level(), s.game.outcome()), (Some(0), Outcome::Playing));
        assert!(s.level_select.is_none() && s.playing());
        finish(&mut s, false);
        assert_eq!(s.take_progress(), None, "a lost replay moves nothing");
        assert_eq!(s.campaign.as_ref().map(Campaign::reached), Some(1), "level 2 is still the furthest reached");

        // The keys: right onto level 2, Enter starts it.
        s.play_again();
        s.press_levels();
        let key = |f: fn(&mut SelectInput)| {
            let mut input = SelectInput::default();
            f(&mut input);
            input
        };
        assert!(!s.update_level_select(&key(|i| i.right = true), field));
        assert!(s.update_level_select(&key(|i| i.enter = true), field));
        assert_eq!(s.level(), Some(1));
        assert_eq!(s.level_button(), Some(2));
    }

    /// Any map that is not a level is free play: its end screen shows the
    /// numbers and counts down to a restart of its own, with no buttons.
    #[test]
    fn a_map_that_is_no_level_is_free_play() {
        let mut s = session();
        s.set_campaign(Campaign::new(two_levels(), None));
        assert_eq!(s.level(), None);
        assert!(!s.game.hold_end_screen);
        assert!(s.play_chrome().level.is_none());
        assert_eq!(s.level_button(), None, "the HUD keeps its mission word");
        assert!(s.press_levels(), "and the level select is the way back to the levels");
        assert!(!s.press_levels());
        // The default map is a wave round, so its enemies are still out
        // of the field: the round ends on player 1's wreck.
        s.game.debug_kill(0).unwrap();
        s.game.update(crate::simulation::Input::default(), crate::PHYSICS_FIXED_DT, W, H);
        s.note_outcome();
        let view = s.play_chrome().result.expect("the numbers still show");
        assert_eq!(view.buttons, None);
        assert_eq!(s.take_progress(), None);
        to_verdict(&mut s);
        assert!(!s.enter_result() && !s.next_level());
        // A level's map by name makes it that level, however it arrives.
        s.replace_map(crate::map::open_map("glasshouses").unwrap());
        assert_eq!(s.level(), Some(1));
        assert!(s.game.hold_end_screen);
    }
}
