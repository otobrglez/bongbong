//! Where a sound is heard from (docs/sound-system-prd.md section 4.3): the
//! listeners, and what distance, walls and motion do to a sound on its way
//! to them.
//!
//! Each view on the window has a listener floating above the field at a
//! height of `audio_listener_height` times half the view's width, its ears
//! between the camera's centre and the seat's own tank. A sound is heard by
//! the listener that hears it loudest. Distance takes level off past
//! `audio_near_px` and closes a high-cut toward the class's far end; every
//! wall cell on the straight line between takes its `sound_transmission`
//! off and closes the high-cut further; a far sound waits for its distance
//! over `audio_sound_speed_px`; a sound moving toward or away from the ears
//! is pitched by doppler.

use crate::math::Vec2;
use crate::obstacle::Obstacle;
use crate::simulation::Game;
use crate::tuning::Tuning;

use super::Class;
use super::dsp::db_to_gain;

/// One view on the window, as the app hands it over each frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// The middle of what it shows, in field pixels.
    pub center: Vec2,
    /// Half its width, in field pixels.
    pub half_width: f32,
    /// The seat it follows, if any: where its tank is.
    pub seat: Option<Vec2>,
}

/// A listener, worked out from a view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ear {
    pub at: Vec2,
    pub height: f32,
    pub velocity: Vec2,
}

/// What a sound sounds like at the ears.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Heard {
    /// Linear, distance and walls together.
    pub gain: f32,
    pub pan: f32,
    pub cutoff: f32,
    /// Seconds it waits for its distance.
    pub delay: f32,
    pub doppler: f32,
    /// Across the field, px.
    pub distance: f32,
    /// The share of it sent to the room: more the farther it is, so a far
    /// sound is heard by its room as much as by itself.
    pub send: f32,
}

impl Heard {
    /// A sound with no place: the UI, a stinger, the player's own voice.
    pub const CENTRE: Heard = Heard { gain: 1.0, pan: 0.0, cutoff: 20_000.0, delay: 0.0, doppler: 1.0, distance: 0.0, send: 0.0 };
}

/// The field's cells, each with the dB a sound loses crossing it.
#[derive(Default)]
pub struct Occlusion {
    min: (i32, i32),
    cols: i32,
    rows: i32,
    loss: Vec<f32>,
}

impl Occlusion {
    /// From the tiles standing now.
    pub fn of(game: &Game) -> Occlusion {
        let cells: Vec<((i32, i32), f32)> = game
            .world
            .query::<&Obstacle>()
            .iter()
            .filter(|o| !o.destroyed)
            .map(|o| (o.cell(), o.material.sound_transmission()))
            .filter(|&(_, loss)| loss < 0.0)
            .collect();
        Occlusion::from_cells(&cells)
    }

    pub fn from_cells(cells: &[((i32, i32), f32)]) -> Occlusion {
        if cells.is_empty() {
            return Occlusion::default();
        }
        let (mut lo, mut hi) = (cells[0].0, cells[0].0);
        for &((c, r), _) in cells {
            lo = (lo.0.min(c), lo.1.min(r));
            hi = (hi.0.max(c), hi.1.max(r));
        }
        let (cols, rows) = (hi.0 - lo.0 + 1, hi.1 - lo.1 + 1);
        let mut loss = vec![0.0f32; (cols * rows) as usize];
        for &((c, r), l) in cells {
            let i = ((r - lo.1) * cols + (c - lo.0)) as usize;
            loss[i] = loss[i].min(l);
        }
        Occlusion { min: lo, cols, rows, loss }
    }

    fn at(&self, cell: (i32, i32)) -> f32 {
        let (c, r) = (cell.0 - self.min.0, cell.1 - self.min.1);
        if c < 0 || r < 0 || c >= self.cols || r >= self.rows {
            return 0.0;
        }
        self.loss[(r * self.cols + c) as usize]
    }

    /// The dB lost on the straight line from `from` to `to`, counting each
    /// cell once and neither end's own cell.
    pub fn between(&self, from: Vec2, to: Vec2) -> f32 {
        if self.loss.is_empty() {
            return 0.0;
        }
        let cell_of = |p: Vec2| crate::map::world_to_cell(p);
        let (start, end) = (cell_of(from), cell_of(to));
        if start == end {
            return 0.0;
        }
        let d = to - from;
        let len = d.length();
        let steps = ((len / (crate::OBSTACLE_GRID_SIZE * 0.5)).ceil() as usize).clamp(1, 256);
        let mut last = start;
        let mut total = 0.0;
        for i in 1..steps {
            let p = from + d * (i as f32 / steps as f32);
            let cell = cell_of(p);
            if cell != last && cell != end {
                total += self.at(cell);
                last = cell;
            }
        }
        total
    }
}

/// The listeners and what stands between them and the sounds.
#[derive(Default)]
pub struct Space {
    ears: Vec<Ear>,
    occlusion: Occlusion,
    dirty: bool,
}

impl Space {
    pub fn ears(&self) -> &[Ear] {
        &self.ears
    }

    /// The listeners for this frame's views; `dt` is the frame's time, for
    /// how fast the ears moved.
    pub fn set_views(&mut self, views: &[View], dt: f32, t: &Tuning) {
        let ears: Vec<Ear> = views
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let at = match v.seat {
                    Some(seat) => v.center + (seat - v.center) * t.audio_ear_bias,
                    None => v.center,
                };
                let velocity = match self.ears.get(i) {
                    Some(prev) if dt > 0.0 => (at - prev.at) * (1.0 / dt),
                    _ => Vec2::new(0.0, 0.0),
                };
                Ear { at, height: (v.half_width * t.audio_listener_height).max(1.0), velocity }
            })
            .collect();
        self.ears = ears;
    }

    /// The walls changed: rebuild before the next sound is placed.
    pub fn walls_changed(&mut self) {
        self.dirty = true;
    }

    pub fn refresh(&mut self, game: &Game) {
        if self.dirty {
            self.occlusion = Occlusion::of(game);
            self.dirty = false;
        }
    }

    /// A sound at `at`, moving at `velocity`, of `class`, as the loudest
    /// listener hears it. With no listener it is heard at the centre.
    pub fn hear(&self, at: Vec2, velocity: Vec2, class: Class, t: &Tuning) -> Heard {
        self.ears
            .iter()
            .map(|ear| hear_one(ear, at, velocity, class, &self.occlusion, t))
            .max_by(|a, b| a.gain.total_cmp(&b.gain))
            .unwrap_or(Heard::CENTRE)
    }
}

fn hear_one(ear: &Ear, at: Vec2, velocity: Vec2, class: Class, occlusion: &Occlusion, t: &Tuning) -> Heard {
    let d = at - ear.at;
    let flat = d.length();
    let far = t.audio_class_far_px[class.index()].max(t.audio_near_px + 1.0);
    let near = t.audio_near_px;
    let mut gain = if flat <= near { 1.0 } else { libm::powf(near / flat, t.audio_rolloff) };
    // The last quarter of the range fades to nothing, so a sound leaves the
    // mix at its edge rather than stopping on a step.
    let edge = 0.75 * far;
    if flat > edge {
        gain *= (1.0 - (flat - edge) / (far - edge)).clamp(0.0, 1.0);
    }
    let reach = ((flat - near) / (far - near)).clamp(0.0, 1.0);
    let mut cutoff = 20_000.0 * libm::powf(t.audio_far_cutoff_hz / 20_000.0, reach);
    let loss = occlusion.between(ear.at, at).max(t.audio_occlusion_max_db);
    if loss < 0.0 {
        gain *= db_to_gain(loss);
        let shut = (loss / t.audio_occlusion_max_db.min(-0.01)).clamp(0.0, 1.0);
        cutoff = cutoff.min(20_000.0 * libm::powf(t.audio_occlusion_cutoff_hz / 20_000.0, shut));
    }
    let pan = (d.x / libm::sqrtf(d.x * d.x + ear.height * ear.height)) * t.audio_pan_width;
    let delay = if t.audio_delay_max_s > 0.0 { (flat / t.audio_sound_speed_px).min(t.audio_delay_max_s) } else { 0.0 };
    let doppler = if flat > 1.0 {
        let toward = d * (1.0 / flat);
        let c = t.audio_doppler_c_px;
        let source = -(velocity.x * toward.x + velocity.y * toward.y);
        let listener = ear.velocity.x * toward.x + ear.velocity.y * toward.y;
        ((c + listener) / (c - source).max(1.0)).clamp(t.audio_doppler_min, t.audio_doppler_max)
    } else {
        1.0
    };
    let send = t.audio_reverb_send_near + (t.audio_reverb_send_far - t.audio_reverb_send_near) * reach;
    Heard { gain, pan, cutoff, delay, doppler, distance: flat, send }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ear() -> Ear {
        Ear { at: Vec2::new(0.0, 0.0), height: 300.0, velocity: Vec2::new(0.0, 0.0) }
    }

    fn t() -> Tuning {
        Tuning::DEFAULT
    }

    #[test]
    fn farther_is_quieter_and_duller() {
        let o = Occlusion::default();
        let near = hear_one(&ear(), Vec2::new(50.0, 0.0), Vec2::new(0.0, 0.0), Class::Medium, &o, &t());
        let far = hear_one(&ear(), Vec2::new(800.0, 0.0), Vec2::new(0.0, 0.0), Class::Medium, &o, &t());
        assert!(near.gain > far.gain && near.cutoff > far.cutoff);
        let gone = hear_one(&ear(), Vec2::new(5000.0, 0.0), Vec2::new(0.0, 0.0), Class::Medium, &o, &t());
        assert_eq!(gone.gain, 0.0);
    }

    #[test]
    fn a_sound_to_the_right_pans_right_and_one_below_stays_centred() {
        let o = Occlusion::default();
        let right = hear_one(&ear(), Vec2::new(400.0, 0.0), Vec2::new(0.0, 0.0), Class::Large, &o, &t());
        let below = hear_one(&ear(), Vec2::new(0.0, 400.0), Vec2::new(0.0, 0.0), Class::Large, &o, &t());
        assert!(right.pan > 0.5 && below.pan.abs() < 1e-6);
    }

    #[test]
    fn a_wall_between_muffles_a_sound() {
        let wall = Occlusion::from_cells(&[((5, 0), -12.0), ((5, 1), -12.0), ((5, -1), -12.0)]);
        let open = Occlusion::default();
        let at = Vec2::new(320.0, 0.0);
        let behind = hear_one(&ear(), at, Vec2::new(0.0, 0.0), Class::Large, &wall, &t());
        let clear = hear_one(&ear(), at, Vec2::new(0.0, 0.0), Class::Large, &open, &t());
        assert!((behind.gain / clear.gain - db_to_gain(-12.0)).abs() < 1e-3);
        assert!(behind.cutoff < clear.cutoff);
    }

    #[test]
    fn a_sound_coming_closer_is_higher() {
        let o = Occlusion::default();
        let at = Vec2::new(600.0, 0.0);
        let coming = hear_one(&ear(), at, Vec2::new(-300.0, 0.0), Class::Large, &o, &t());
        let going = hear_one(&ear(), at, Vec2::new(300.0, 0.0), Class::Large, &o, &t());
        assert!(coming.doppler > 1.0 && going.doppler < 1.0);
    }

    #[test]
    fn a_far_sound_waits_for_its_distance_up_to_the_cap() {
        let o = Occlusion::default();
        let t = t();
        let far = hear_one(&ear(), Vec2::new(2400.0, 0.0), Vec2::new(0.0, 0.0), Class::Huge, &o, &t);
        let close = hear_one(&ear(), Vec2::new(30.0, 0.0), Vec2::new(0.0, 0.0), Class::Huge, &o, &t);
        assert!((far.delay - t.audio_delay_max_s).abs() < 1e-6);
        assert!(close.delay < 0.02);
    }
}
