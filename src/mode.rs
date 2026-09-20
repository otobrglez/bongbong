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

use crate::ai::Intent;
use crate::editor::{BuilderInput, EditorAction, MapEditor};
use crate::hud::PlayChrome;
use crate::lobby::{Lobby, LobbyAction, LobbyInput, RoomPhase, RoomView};
use crate::map::MapFile;
use crate::net::client::{RoomSetup, Target};
use crate::net::round::AnyRound;
use crate::net::rooms::{RoomCode, RoomsHost};
use crate::simulation::{Game, Outcome, PlayerCount};
use crate::{Layout, Rect};

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
    /// The name this player takes into a room (`--nick`).
    pub nick: String,
    /// The reconnect key this session takes into a room
    /// (`net::client::Identity::device_token`): the same token in a
    /// later join reclaims the same seat for the room's life, so it has
    /// to be this player's and nobody else's. A desktop build derives it
    /// from the nickname, which is what makes two `--nick`s on one
    /// machine two seats; the web page mints a random one and keeps it
    /// per tab (`site/src/scripts/room.ts`), which is what makes two
    /// tabs two players. `app.rs` sets it once at startup.
    pub token: String,
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
        Session {
            driver: Driver::Play,
            game,
            builder,
            dialog: false,
            players_dialog: false,
            online: None,
            lobby: None,
            rooms: RoomsHost::cluster(),
            nick: "player".into(),
            token: "bongbong-player".into(),
        }
    }

    pub fn mode(&self) -> Driver {
        self.driver
    }

    /// Whether `Game::update` should run this frame: play mode with no
    /// question pending.
    pub fn playing(&self) -> bool {
        self.driver == Driver::Play && !self.dialog && !self.players_dialog
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
            if self.players_dialog {
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

    fn enter_build(&mut self) {
        self.dialog = false;
        self.players_dialog = false;
        self.driver = Driver::Build;
    }

    /// The players button in the bar: open the players dialog, or close it
    /// if it is already up. Play mode only, and a no-op while the leave
    /// dialog is asking - one question at a time. Works on the end screen
    /// too (the restart countdown waits). Returns whether it is open. Never
    /// opens where two players are not offered (`TWO_PLAYERS_AVAILABLE`).
    pub fn press_players(&mut self) -> bool {
        if crate::TWO_PLAYERS_AVAILABLE && self.driver == Driver::Play && !self.dialog {
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
                let (width, height) = self.game.map.field_size();
                self.game.init(width, height);
            }
        }
        self.game.players
    }

    /// `PLAY` from the builder: the edited map becomes the round's map and
    /// a fresh round starts (a `--seed` stays pinned, the mission banner
    /// shows as on any restart). A no-op in play mode.
    pub fn play(&mut self) -> Driver {
        if self.driver == Driver::Build {
            // A menu left open would still be there on the next BUILD.
            self.builder.close_popup();
            self.game.map = self.builder.map().clone();
            let (width, height) = self.game.map.field_size();
            self.game.init(width, height);
            self.driver = Driver::Play;
            self.dialog = false;
            self.players_dialog = false;
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

    /// The bar's `ONLINE` button: open the lobby over the local round,
    /// which is left exactly where it stands (nothing here calls
    /// `Game::update`, and `playing()` is false from this frame on).
    /// A no-op where a build cannot reach a room, and while the builder
    /// is up - the lobby is the play bar's button. Returns the mode
    /// afterwards.
    pub fn press_online(&mut self) -> Driver {
        if !crate::ONLINE_AVAILABLE || self.driver != Driver::Play {
            return self.driver;
        }
        self.dialog = false;
        self.players_dialog = false;
        self.lobby = Some(Lobby::new(self.rooms.clone()));
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
        self.lobby = Some(Lobby::new(self.rooms.clone()));
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
        self.lobby = Some(Lobby::new(self.rooms.clone()));
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
    /// hit tests, then whatever it asked for. `app.rs` and the dev server
    /// both fill the same `LobbyInput`, so a tool's click lands on the
    /// hit test a finger does.
    ///
    /// The room's round is polled from here, which is what makes a seat
    /// fill up and a roster arrive while the screen is on; the seat sends
    /// no intent, since there is no round to steer yet. The frame the
    /// room starts the round the window hands over to `Driver::Online`
    /// and the replica is what is drawn. Returns the mode afterwards.
    pub fn update_lobby(&mut self, input: &LobbyInput, field: Rect, dt: f32) -> Driver {
        if self.driver != Driver::Lobby {
            return self.driver;
        }
        if let Some(round) = &mut self.online {
            round.frame(&Intent::default(), dt);
        }
        let room = self.online.as_ref().map(RoomView::of);
        let Some(lobby) = &mut self.lobby else { return self.driver };
        match lobby.update(input, field, room.as_ref()) {
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
        self.lobby = Some(Lobby::new(self.rooms.clone()));
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
        self.builder.load(map);
    }

    /// One frame of the builder, in build mode: `PLAY` starts the round.
    pub fn update_builder(&mut self, input: &BuilderInput, layout: &Layout) {
        if self.driver != Driver::Build {
            return;
        }
        if let EditorAction::Play = self.builder.update(input, layout) {
            self.play();
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

    /// What `Game::render` should draw around the field this frame. An
    /// online round shows none of the local buttons - the round is the
    /// room's to restart and the builder is not part of it - and carries
    /// a status line instead, until the lobby screen replaces it.
    pub fn play_chrome(&self) -> PlayChrome {
        match self.driver {
            Driver::Online => PlayChrome {
                status: self.online.as_ref().map(AnyRound::status),
                // A room's round does not restart where it stands: the
                // end screen counts down to the lobby it came from.
                countdown_label: Some("Back to the lobby in"),
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
                build_button: true,
                players_button: crate::TWO_PLAYERS_AVAILABLE,
                online_button: crate::ONLINE_AVAILABLE,
                restart_button: !crate::KEYBOARD_AVAILABLE,
                leave_dialog: self.dialog,
                players_dialog: self.players_dialog,
                status: None,
                lobby: None,
                countdown_label: None,
            },
        }
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
        use crate::editor::BuilderInput;
        let mut s = session();
        s.press_build();
        s.answer_dialog(true);
        let layout = Layout::for_field(W, H);
        // The MAP button, from the bar's fixed slots.
        let map_button = {
            let r = crate::editor::MapEditor::map_rect(&layout);
            crate::math::Vec2::new(r.x + r.width / 2.0, r.y + r.height / 2.0)
        };
        let press = BuilderInput { pointer: Some(map_button), pressed: true, held: true, ..Default::default() };
        s.update_builder(&press, &layout);
        assert_eq!(s.builder.open_menu(), Some("map"));
        assert_eq!(s.toggle(), Driver::Play);
        s.press_build();
        s.answer_dialog(true);
        assert_eq!(s.builder.open_menu(), None, "the settings panel stayed open across PLAY");

        // The Save-as prompt (FILE > SAVE AS...) takes text: Tab is a
        // character there, not PLAY.
        let file_rect = crate::editor::MapEditor::file_rect(&layout);
        let file_button = crate::math::Vec2::new(file_rect.x + file_rect.width / 2.0, file_rect.y + file_rect.height / 2.0);
        let press = BuilderInput { pointer: Some(file_button), pressed: true, held: true, ..Default::default() };
        s.update_builder(&press, &layout);
        assert_eq!(s.builder.open_menu(), Some("file"));
        if crate::map::saving_available() {
            // The third row of the menu is SAVE AS.
            let save_as = crate::math::Vec2::new(file_rect.x + 8.0, layout.panel.y + 32.0 + 2.5 * 48.0);
            let press = BuilderInput { pointer: Some(save_as), pressed: true, held: true, ..Default::default() };
            s.update_builder(&press, &layout);
            assert_eq!(s.builder.open_menu(), Some("save"));
            assert_eq!(s.toggle(), Driver::Build, "Tab in the Save prompt started a round");
            s.update_builder(&BuilderInput { escape: true, ..Default::default() }, &layout);
            assert_eq!(s.builder.open_menu(), None);
            assert_eq!(s.toggle(), Driver::Play);
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

        // The bar's own chrome is gone while the round belongs to a room.
        let chrome = s.play_chrome();
        assert!(!chrome.build_button && !chrome.players_button && !chrome.restart_button);
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
        // The bar's own buttons are gone; the lobby is what is drawn.
        let chrome = s.play_chrome();
        assert!(!chrome.build_button && !chrome.players_button && !chrome.online_button);
        let view = chrome.lobby.expect("the lobby is on screen");
        assert_eq!(view.stage, Stage::Start);
        assert_eq!(view.map, crate::map::SHIPPED_MAPS[0].0);

        // Frames of it change nothing about the round or the canvas, and
        // walking into the code entry and back out is all local.
        let field = crate::Rect::new(0.0, 32.0, W, H);
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
        let field = crate::Rect::new(0.0, 32.0, W, H);
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
        let field = crate::Rect::new(0.0, 32.0, W, H);
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
}
