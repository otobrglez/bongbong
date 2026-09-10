//! The game's two modes and the switch between them
//! (docs/game-editor-fusion.md, section 5): `Session` owns the `Game`, the
//! `MapEditor` and which of the two is live, plus the "leave this round?"
//! question in between. Both the window (`main.rs`) and the dev server go
//! through the methods here, so a tool and a click are the same path. No
//! `RaylibHandle` anywhere in this file: drawing stays in `game.rs`,
//! `hud.rs` and `editor.rs`.

use crate::editor::{BuilderInput, EditorAction, MapEditor};
use crate::hud::PlayChrome;
use crate::map::MapFile;
use crate::simulation::{Game, Outcome};
use crate::Layout;

/// Which mode the window is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Driver {
    Play,
    Build,
}

impl Driver {
    pub fn name(self) -> &'static str {
        match self {
            Driver::Play => "play",
            Driver::Build => "build",
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
    pub fn new(game: Game, width: f32, height: f32) -> Self {
        let builder = MapEditor::new(game.map.clone(), width, height);
        Session { driver: Driver::Play, game, builder, dialog: false }
    }

    pub fn mode(&self) -> Driver {
        self.driver
    }

    /// Whether `Game::update` should run this frame: play mode with no
    /// question pending.
    pub fn playing(&self) -> bool {
        self.driver == Driver::Play && !self.dialog
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
            if self.dialog {
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
        self.driver = Driver::Build;
    }

    /// `PLAY` from the builder: the edited map becomes the round's map and
    /// a fresh round starts (a `--seed` stays pinned, the mission banner
    /// shows as on any restart). A no-op in play mode.
    pub fn play(&mut self, width: f32, height: f32) -> Driver {
        if self.driver == Driver::Build {
            // A menu left open would still be there on the next BUILD.
            self.builder.close_popup();
            self.game.map = self.builder.map().clone();
            self.game.init(width, height);
            self.driver = Driver::Play;
            self.dialog = false;
        }
        self.driver
    }

    /// The `Tab` key: `press_build` in play mode, `play` in build mode -
    /// except while the builder's Save prompt is taking text, when a key
    /// is a character and not a command.
    pub fn toggle(&mut self, width: f32, height: f32) -> Driver {
        match self.driver {
            Driver::Play => self.press_build(),
            Driver::Build if self.builder.text_entry_open() => self.driver,
            Driver::Build => self.play(width, height),
        }
    }

    /// Replace the session's map wholesale - the dev server's `restart`
    /// with a `map`/`map_toml`: the game's map for the round it is about
    /// to start, and the builder's canvas and baseline.
    pub fn replace_map(&mut self, map: MapFile, width: f32, height: f32) {
        self.game.map = map.clone();
        self.builder.load(map, width, height);
    }

    /// One frame of the builder, in build mode: `PLAY` starts the round.
    pub fn update_builder(&mut self, input: &BuilderInput, layout: &Layout) {
        if self.driver != Driver::Build {
            return;
        }
        if let EditorAction::Play = self.builder.update(input, layout) {
            self.play(layout.field.w, layout.field.h);
        }
    }

    /// What `Game::render` should draw around the field this frame.
    pub fn play_chrome(&self) -> PlayChrome {
        PlayChrome { build_button: true, leave_dialog: self.dialog }
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
        Session::new(game, W, H)
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
        s.play(W, H);
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
        s.builder.stroke(&[cell], false, W, H);
        assert!(s.builder.dirty());
        assert_eq!(s.play(W, H), Driver::Play);
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
        let map_button = sola_raylib::prelude::Vector2::new(layout.panel.x + 980.0, layout.panel.y + 16.0);
        let press = BuilderInput { pointer: Some(map_button), pressed: true, held: true, ..Default::default() };
        s.update_builder(&press, &layout);
        assert_eq!(s.builder.open_menu(), Some("map"));
        assert_eq!(s.toggle(W, H), Driver::Play);
        s.press_build();
        s.answer_dialog(true);
        assert_eq!(s.builder.open_menu(), None, "the settings panel stayed open across PLAY");

        // The Save-as prompt (FILE > SAVE AS...) takes text: Tab is a
        // character there, not PLAY.
        let file_button = sola_raylib::prelude::Vector2::new(layout.panel.x + 900.0, layout.panel.y + 16.0);
        let press = BuilderInput { pointer: Some(file_button), pressed: true, held: true, ..Default::default() };
        s.update_builder(&press, &layout);
        assert_eq!(s.builder.open_menu(), Some("file"));
        if crate::map::saving_available() {
            // The third row of the menu is SAVE AS.
            let save_as = sola_raylib::prelude::Vector2::new(layout.panel.x + 900.0, layout.panel.y + 32.0 + 2.5 * 48.0);
            let press = BuilderInput { pointer: Some(save_as), pressed: true, held: true, ..Default::default() };
            s.update_builder(&press, &layout);
            assert_eq!(s.builder.open_menu(), Some("save"));
            assert_eq!(s.toggle(W, H), Driver::Build, "Tab in the Save prompt started a round");
            s.update_builder(&BuilderInput { escape: true, ..Default::default() }, &layout);
            assert_eq!(s.builder.open_menu(), None);
            assert_eq!(s.toggle(W, H), Driver::Play);
        }
    }

    #[test]
    fn replace_map_sets_both_the_game_and_the_builder() {
        let mut s = session();
        let mut map = MapFile::new();
        map.set_cell(3, 3, CellObject::Gate);
        s.replace_map(map, W, H);
        assert_eq!(s.game.map.cell(3, 3), Some(&CellObject::Gate));
        assert_eq!(s.builder.map().cell(3, 3), Some(&CellObject::Gate));
        assert!(!s.builder.dirty());
    }
}
