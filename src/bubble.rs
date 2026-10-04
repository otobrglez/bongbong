//! The frog's voice in a training round (docs/training-stage.md), headless
//! like `indicators.rs`: what the frog says and when (`FrogVoice`), and the
//! speech bubble it says it in, laid out in the bitmap's pixels (`layout`);
//! `render/bubble.rs` paints it.
//!
//! The round never reads a word (nothing under `simulation/` does): the
//! script names each beat's lines by message key, and the events the round
//! already sends say when the frog has something to add. `FrogVoice` is
//! owned by `app.rs` the way `fx::Fx` is - it reads the round after every
//! step and never writes it, and keeps its queue and clock on the time the
//! round advanced, so a frozen round says nothing more.
//!
//! A line that names a control has a `-touch` twin; which is shown follows
//! the input last used (`hud::Hints`). A control in a line is a token the
//! bubble draws as a key: `<ARROWS>`, `<SPACE>`, `<STICK>`, `<TAP>`.

use std::collections::{BTreeSet, VecDeque};

use crate::frog::Side;
use crate::math::{Rectangle, Vec2};
use crate::pickup::PickupKind;
use crate::simulation::{Event, Game, HitTarget, Outcome};
use crate::tuning::Tuning;

/// What the frog says and when: the lines waiting, the one showing, how
/// long nothing has been said, and the lines it says only once a round.
#[derive(Debug, Default)]
pub struct FrogVoice {
    queue: VecDeque<String>,
    line: Option<Showing>,
    quiet: f32,
    once: BTreeSet<&'static str>,
    /// The beat whose lines were queued last (1-based), `None` before the
    /// first.
    beat: Option<usize>,
    finished: bool,
    last_frame: u64,
    seed: u64,
}

/// The line on screen and how long it has been up, in seconds of round
/// time.
#[derive(Clone, Debug, PartialEq)]
pub struct Showing {
    pub key: String,
    pub age: f32,
}

impl FrogVoice {
    /// The line up, if any.
    pub fn showing(&self) -> Option<&Showing> {
        self.line.as_ref()
    }

    /// Read the step `game` just ran: a new round forgets everything, a
    /// new beat queues its lines, and the frame's events queue what they
    /// call for. Once per simulated frame, like `fx::Fx::observe_events`;
    /// a frame already read adds nothing.
    pub fn observe(&mut self, game: &Game) {
        if game.frame() == self.last_frame && game.round_seed() == self.seed {
            return;
        }
        if game.frame() < self.last_frame || game.round_seed() != self.seed {
            *self = FrogVoice { seed: game.round_seed(), ..FrogVoice::default() };
        }
        self.last_frame = game.frame();
        let (Some(status), Some(script)) = (game.training_status(), game.map.training.as_ref()) else {
            self.queue.clear();
            self.line = None;
            return;
        };
        if status.finished {
            if !self.finished {
                self.finished = true;
                self.queue.clear();
                self.say(crate::text::keys::FROG_READY.0);
            }
            return;
        }
        if self.beat != Some(status.beat) {
            // A new lesson starts fresh: whatever the last beat still had
            // to say is old news.
            self.beat = Some(status.beat);
            self.queue.clear();
            self.line = None;
            if let Some(beat) = script.beat(status.beat - 1) {
                for key in &beat.say {
                    self.say(key);
                }
            }
        }
        let seats = game.players.count();
        for event in game.events() {
            match *event {
                Event::Hit { target: HitTarget::Frog { side: Side::Player }, .. } if status.id == "frog" => {
                    self.say_once(crate::text::keys::FROG_OW.0);
                    self.say_once(crate::text::keys::FROG_KIT.0);
                }
                Event::Hit { target: HitTarget::Enemy { .. }, .. } if status.id == "enemy" => self.say_once(crate::text::keys::FROG_ONLY_ME.0),
                Event::FrogHealed { side: Side::Player, .. } => self.say_once(crate::text::keys::FROG_HEALED.0),
                Event::FrogBite { side: Side::Player, .. } => self.say_once(crate::text::keys::FROG_CHOMP.0),
                Event::PickupCollected { slot, kind: PickupKind::Minigun, .. } if slot < seats => self.say_once(crate::text::keys::FROG_MINIGUN.0),
                Event::Wreck { slot, .. } if slot < seats => {
                    if !self.queue.iter().any(|k| k == crate::text::keys::FROG_DOWN.0) {
                        self.say(crate::text::keys::FROG_DOWN.0);
                    }
                }
                _ => {}
            }
        }
    }

    /// Run the voice's clock by `dt` seconds of round time: the line up
    /// ages and gives way to the next once it has been read
    /// (`training_line_seconds` plus `training_line_seconds_per_char` of
    /// its `chars`), and after `training_nudge_seconds` with nothing said
    /// the running beat's nudge is said again. `chars` is a line's length
    /// in the language on screen.
    pub fn update(&mut self, game: &Game, dt: f32, t: &Tuning, chars: impl Fn(&str) -> usize) {
        if let Some(line) = &mut self.line {
            line.age += dt;
            if line.age >= t.training_line_seconds + t.training_line_seconds_per_char * chars(&line.key) as f32 {
                self.line = None;
            }
        }
        if self.line.is_none()
            && let Some(key) = self.queue.pop_front()
        {
            self.line = Some(Showing { key, age: 0.0 });
            self.quiet = 0.0;
        }
        if self.line.is_some() || !self.queue.is_empty() || game.outcome() != Outcome::Playing {
            self.quiet = 0.0;
            return;
        }
        self.quiet += dt;
        if self.quiet < t.training_nudge_seconds {
            return;
        }
        self.quiet = 0.0;
        let nudge = game
            .training_status()
            .filter(|s| !s.finished)
            .and_then(|s| game.map.training.as_ref()?.beat(s.beat - 1)?.nudge.clone());
        if let Some(key) = nudge {
            self.say(&key);
        }
    }

    fn say(&mut self, key: &str) {
        self.queue.push_back(key.to_string());
        self.quiet = 0.0;
    }

    fn say_once(&mut self, key: &'static str) {
        if self.once.insert(key) {
            self.say(key);
        }
    }
}

/// A line's key on a touch screen: its `-touch` twin where the catalogue
/// has one (`has`), else itself.
pub fn key_for(key: &str, touch: bool, has: impl Fn(&str) -> bool) -> String {
    let twin = format!("{key}-touch");
    if touch && has(&twin) { twin } else { key.to_string() }
}

/// A control drawn as a key in a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    /// The four arrow keys.
    Arrows,
    /// The space bar.
    Space,
    /// A thumb on the stick.
    Stick,
    /// A tap.
    Tap,
}

impl Token {
    fn parse(word: &str) -> Option<Token> {
        match word {
            "<ARROWS>" => Some(Token::Arrows),
            "<SPACE>" => Some(Token::Space),
            "<STICK>" => Some(Token::Stick),
            "<TAP>" => Some(Token::Tap),
            _ => None,
        }
    }

    /// How wide the key draws at a text size of `size` px.
    pub fn width(self, size: f32) -> f32 {
        match self {
            Token::Arrows => size * 4.0 + 6.0,
            Token::Space => size * 4.4,
            Token::Stick => size * 1.4,
            Token::Tap => size * 2.8,
        }
    }
}

/// One word of a laid-out line: text, or a control drawn as a key, with
/// its left edge from the bubble's text column.
#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    Word { text: String, x: f32 },
    Key { token: Token, x: f32 },
}

/// The bubble as the painter draws it, all in the bitmap's pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Bubble {
    /// The bubble's body.
    pub rect: Rectangle,
    /// Its lines, each a row of pieces from the text column's left edge.
    pub lines: Vec<Vec<Piece>>,
    /// The text column's top-left corner.
    pub text_at: Vec2,
    /// Text size and line height, px.
    pub size: f32,
    pub line_height: f32,
    /// Where the tail meets the body and where it points, or `None` when
    /// the frog is off the view and the bubble stands at its edge.
    pub tail: Option<(Vec2, Vec2)>,
    /// The bitmap's pixels per UI point, for the outline and the keys.
    pub px: f32,
}

/// The bubble's measures in UI points.
pub const TEXT_PT: f32 = 12.0;
pub const PAD_PT: f32 = 7.0;
pub const MAX_WIDTH_PT: f32 = 220.0;
/// How far above the frog's middle the tail's tip stands.
pub const ABOVE_PT: f32 = 22.0;
/// How far below the frog's middle a bubble that has no room above it
/// starts.
pub const BELOW_PT: f32 = 24.0;
/// How far the tail reaches out of the body.
pub const TAIL_PT: f32 = 8.0;
/// How far inside the view a bubble stays.
pub const INSET_PT: f32 = 6.0;

/// Lay out `text`, its first `shown` words revealed, as a bubble pointing
/// at `frog` (bitmap px) inside `view` (the field the screen shows, bitmap
/// px), at `px` bitmap pixels a point; `measure` is a word's width at a
/// text size (`text::width`'s answer). The body is sized for the whole
/// line, so it never grows as the words come in; it stands above the frog,
/// under it where there is no room above, and slid along the view's edge -
/// tail-less - while the frog is off the view.
pub fn layout(text: &str, shown: usize, frog: Vec2, view: Rectangle, keep_out: &[Rectangle], px: f32, measure: impl Fn(&str, f32) -> f32) -> Bubble {
    let size = (TEXT_PT * px).round().max(10.0);
    let line_height = (size * 1.25).round();
    let space = measure(" ", size).max(size * 0.3);
    let max_text = MAX_WIDTH_PT * px - 2.0 * PAD_PT * px;
    let words: Vec<&str> = text.split_whitespace().collect();
    let widths: Vec<f32> = words.iter().map(|w| Token::parse(w).map_or_else(|| measure(w, size), |t| t.width(size))).collect();
    // The rows the whole line wraps into, then the revealed words in them.
    let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
    let mut x = 0.0;
    for (i, w) in widths.iter().enumerate() {
        let row = rows.last_mut().expect("one row at least");
        if !row.is_empty() && x + space + w > max_text {
            rows.push(vec![i]);
            x = *w;
        } else {
            x += if row.is_empty() { *w } else { space + w };
            row.push(i);
        }
    }
    let text_width = rows
        .iter()
        .map(|row| row.iter().map(|&i| widths[i]).sum::<f32>() + space * row.len().saturating_sub(1) as f32)
        .fold(0.0, f32::max);
    let lines: Vec<Vec<Piece>> = rows
        .iter()
        .map(|row| {
            let mut x = 0.0;
            row.iter()
                .filter(|&&i| i < shown)
                .map(|&i| {
                    let piece = match Token::parse(words[i]) {
                        Some(token) => Piece::Key { token, x },
                        None => Piece::Word { text: words[i].to_string(), x },
                    };
                    x += widths[i] + space;
                    piece
                })
                .collect()
        })
        .collect();
    let (w, h) = (text_width + 2.0 * PAD_PT * px, rows.len() as f32 * line_height + 2.0 * PAD_PT * px - (line_height - size));
    let inset = INSET_PT * px;
    let (left, right) = (view.x + inset, view.x + view.width - inset - w);
    let (top, bottom) = (view.y + inset, view.y + view.height - inset - h);
    let on_view = view.contains(frog);
    let x = (frog.x - w / 2.0).clamp(left, right.max(left));
    let above = frog.y - ABOVE_PT * px - TAIL_PT * px - h;
    let overlaps = |y: f32| keep_out.iter().any(|k| overlap(Rectangle::new(x, y, w, h), *k));
    let (y, tail) = if !on_view {
        (frog.y.clamp(top, bottom.max(top)), None)
    } else if above >= top && !overlaps(above) {
        let tip = Vec2::new(frog.x.clamp(x + 2.0 * PAD_PT * px, x + w - 2.0 * PAD_PT * px), frog.y - ABOVE_PT * px);
        (above, Some((Vec2::new(tip.x, above + h), tip)))
    } else {
        let y = (frog.y + BELOW_PT * px + TAIL_PT * px).min(bottom.max(top));
        let tip = Vec2::new(frog.x.clamp(x + 2.0 * PAD_PT * px, x + w - 2.0 * PAD_PT * px), frog.y + BELOW_PT * px);
        (y, Some((Vec2::new(tip.x, y), tip)))
    };
    Bubble {
        rect: Rectangle::new(x.round(), y.round(), w.round(), h.round()),
        lines,
        text_at: Vec2::new((x + PAD_PT * px).round(), (y + PAD_PT * px).round()),
        size,
        line_height,
        tail,
        px,
    }
}

fn overlap(a: Rectangle, b: Rectangle) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

/// How many of a line's words are up after `age` seconds: they come in at
/// `training_line_words_per_second`, the first at once.
pub fn revealed(text: &str, age: f32, t: &Tuning) -> usize {
    let all = text.split_whitespace().count();
    (1 + (age * t.training_line_words_per_second) as usize).min(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(word: &str, size: f32) -> f32 {
        word.chars().count() as f32 * size * 0.6
    }

    const VIEW: Rectangle = Rectangle::new(0.0, 0.0, 800.0, 400.0);

    #[test]
    fn a_line_wraps_inside_the_bubble_and_keys_take_their_width() {
        let b = layout("<ARROWS> to drive. Ease off before every single turn you take.", 99, Vec2::new(400.0, 300.0), VIEW, &[], 1.0, measure);
        assert!(b.lines.len() >= 2, "{:?}", b.lines);
        assert!(b.rect.width <= MAX_WIDTH_PT + 0.5);
        assert!(matches!(b.lines[0][0], Piece::Key { token: Token::Arrows, x } if x == 0.0));
        for line in &b.lines {
            for piece in line {
                let x = match piece {
                    Piece::Word { x, .. } | Piece::Key { x, .. } => *x,
                };
                assert!(b.text_at.x + x < b.rect.x + b.rect.width, "inside the body");
            }
        }
    }

    #[test]
    fn the_body_is_sized_for_the_whole_line_while_its_words_come_in() {
        let all = layout("Roll over the three flags.", 99, Vec2::new(400.0, 300.0), VIEW, &[], 1.0, measure);
        let one = layout("Roll over the three flags.", 1, Vec2::new(400.0, 300.0), VIEW, &[], 1.0, measure);
        assert_eq!(all.rect, one.rect);
        assert_eq!(one.lines.iter().map(Vec::len).sum::<usize>(), 1);
    }

    #[test]
    fn the_bubble_stands_above_the_frog_and_under_it_near_the_top() {
        let high = layout("Hi! I'm your frog.", 99, Vec2::new(400.0, 300.0), VIEW, &[], 1.0, measure);
        let (_, tip) = high.tail.expect("a tail on view");
        assert!(high.rect.y + high.rect.height < 300.0 && tip.y < 300.0, "above");
        let low = layout("Hi! I'm your frog.", 99, Vec2::new(400.0, 20.0), VIEW, &[], 1.0, measure);
        assert!(low.rect.y > 20.0, "under the frog where there is no room above");
        let edge = layout("Hi! I'm your frog.", 99, Vec2::new(5.0, 200.0), VIEW, &[], 1.0, measure);
        assert!(edge.rect.x >= INSET_PT, "slid inside the view");
        let hud = [Rectangle::new(0.0, 0.0, 800.0, 280.0)];
        let clear = layout("Hi! I'm your frog.", 99, Vec2::new(400.0, 300.0), VIEW, &hud, 1.0, measure);
        assert!(clear.rect.y > 300.0, "under the frog rather than under a corner cluster");
    }

    #[test]
    fn a_frog_off_the_view_speaks_from_its_edge_with_no_tail() {
        let b = layout("Here it comes!", 99, Vec2::new(1400.0, 200.0), VIEW, &[], 1.0, measure);
        assert!(b.tail.is_none());
        assert!(b.rect.x + b.rect.width <= VIEW.width - INSET_PT + 0.5);
    }

    #[test]
    fn a_touch_screen_takes_a_lines_touch_twin_where_there_is_one() {
        let has = |k: &str| k == "frog-fire-touch";
        assert_eq!(key_for("frog-fire", true, has), "frog-fire-touch");
        assert_eq!(key_for("frog-fire", false, has), "frog-fire");
        assert_eq!(key_for("frog-hello", true, has), "frog-hello");
    }

    /// A course of one door and a flag, its two beats' lines and nudges.
    fn round() -> Game {
        let mut map = String::from(
            "version = 1\ntanks = 0\nsize = [16.0, 9.0]\ncells.\"2,4\" = { kind = \"start\" }\ncells.\"6,4\" = { kind = \"flag\" }\ncells.\"12,4\" = { kind = \"frog\" }\n",
        );
        map.push_str("\n[[training.beat]]\nsay = [\"frog-hello\", \"frog-flags\"]\nnudge = \"frog-flags-nudge\"\ndone = { flags = 1 }\n");
        map.push_str("\n[[training.beat]]\nsay = [\"frog-crates\"]\ndone = { past_col = 14 }\n");
        let mut game = Game::default();
        game.seed_override = Some(3);
        game.map = crate::map::MapFile::from_toml_str(&map).expect("parses");
        let (w, h) = game.map.field_size();
        game.init(w, h);
        game
    }

    fn run(game: &mut Game, voice: &mut FrogVoice, frames: usize, said: &mut Vec<String>) {
        let (w, h) = game.map.field_size();
        let t = crate::tuning::Tuning::DEFAULT;
        for _ in 0..frames {
            game.update(crate::simulation::Input::default(), 1.0 / 60.0, w, h);
            voice.observe(game);
            voice.update(game, 1.0 / 60.0, &t, |_| 20);
            if let Some(line) = voice.showing()
                && said.last() != Some(&line.key)
            {
                said.push(line.key.clone());
            }
        }
    }

    #[test]
    fn the_frog_says_each_beats_lines_in_turn_and_nudges_when_it_goes_quiet() {
        let mut game = round();
        let mut voice = FrogVoice::default();
        let mut said = Vec::new();
        let t = crate::tuning::Tuning::DEFAULT;
        let line = t.training_line_seconds + t.training_line_seconds_per_char * 20.0;
        run(&mut game, &mut voice, ((2.0 * line + t.training_nudge_seconds + 1.0) * 60.0) as usize, &mut said);
        assert_eq!(said, ["frog-hello", "frog-flags", "frog-flags-nudge"]);
        game.debug_teleport(0, crate::map::cell_to_world(6, 4), None).expect("seat 0");
        run(&mut game, &mut voice, 30, &mut said);
        assert_eq!(said.last().map(String::as_str), Some("frog-crates"), "the next beat's line: {said:?}");
    }

    #[test]
    fn every_shipped_script_line_is_a_message() {
        let english = crate::text::Catalogue::new("en");
        for (name, text) in crate::map::SHIPPED_MAPS {
            let map = crate::map::MapFile::from_toml_str(text).expect("shipped maps parse");
            let Some(script) = map.training else { continue };
            for beat in &script.beat {
                for key in beat.say.iter().chain(&beat.nudge) {
                    assert!(english.message(key, &[]).is_some(), "{name}: {key} is no message");
                    assert!(crate::text::keys::ALL.iter().any(|k| k.0 == key), "{name}: {key} is no key constant");
                }
            }
        }
    }
}
