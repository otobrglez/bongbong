//! The drawn shots followed from frame to frame: what the ledger and the
//! incoming-fire metric read.
//!
//! A room copy keeps the room's id for its life, so it is followed by id.
//! A provisional's id is stable too - `PROVISIONAL_ID_BASE` plus the
//! client's own number for the shot (`Live::id` within
//! `PROVISIONAL_ID_MASK`), handed out in launch order and wrapping every
//! 4096 shots - but a provisional is followed by continuity all the same,
//! because a jump is what the ledger measures: each one on a frame is the
//! one from the frame before that it could have become, no further on than
//! a shot of its kind flies in the time between the two frames and no
//! earlier in its own life (`ShotSample::stage`). A shot that moves further
//! than that has jumped - the client moved it to where the room had it:
//! its track ends and a new one begins, and the ledger puts the two back
//! together as one shot (`metrics::own_shots`).

use crate::sample::{FrameSample, ShotSample, flying_stage};

/// A point of a track: the frame it was drawn on and where.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackPoint {
    /// Index into the frames followed.
    pub frame: usize,
    pub x: f32,
    pub y: f32,
    pub flying: bool,
    pub impact: bool,
    /// The id it was drawn under on this frame (`ShotSample::id`).
    pub id: u32,
    /// Where it was in its own life (`ShotSample::stage`).
    pub stage: u8,
}

/// When a track was drawn at a place (`Track::time_at`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Passing {
    /// How far from the place its nearest drawing, or its flight carried
    /// on, passed.
    pub distance: f32,
    /// When it was there, on the frames' clock.
    pub t_ms: f64,
    /// It stood there in the same muzzle or impact frame the place was
    /// asked for, rather than flying past.
    pub same_stage: bool,
}

/// Where and how the picture stopped a shot (`Track::stop`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stop {
    /// Index into the frames followed.
    pub frame: usize,
    pub x: f32,
    pub y: f32,
    /// Taken off the picture in flight, rather than bursting in its
    /// impact frames.
    pub vanished: bool,
}

/// One drawn shot over the frames it was drawn on, unbroken.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub kind: u8,
    pub seat: Option<u8>,
    pub provisional: bool,
    /// The room's id, for a room copy.
    pub room_id: Option<u32>,
    pub points: Vec<TrackPoint>,
}

/// Slack on top of a frame's flight when following a provisional: the
/// wire's quarter pixels and a frame's rounding.
pub const FOLLOW_SLACK_PX: f32 = 2.0;

/// How much of a frame's full flight a provisional may cover and still be
/// the same shot: a shell leaving its muzzle frames covers a whole frame's
/// flight from standing, one meeting something stops part of the way.
pub const FOLLOW_FLIGHT: f32 = 1.25;

/// How far back along a track its way of flight is taken from
/// (`Track::flight`): most of a frame's flight, well past the wire's
/// quarter pixels and a picture's nudges.
pub const FLIGHT_BASE_PX: f32 = 6.0;

/// The fastest a shot of `kind` flies, pixels a second.
pub fn speed(kind: u8) -> f32 {
    let t = bongbong::tuning::tuning();
    match kind {
        0 => t.shell_speed,
        1 => t.minigun_bullet_speed,
        _ => t.plasma_speed,
    }
}

/// The half extent a shot of `kind` is hit-tested with.
pub fn half_extent(kind: u8) -> f32 {
    let t = bongbong::tuning::tuning();
    match kind {
        0 => t.shell_hit_half_extent,
        1 => t.minigun_bullet_hit_half_extent,
        _ => t.plasma_hit_half_extent,
    }
}

impl Track {
    pub fn first(&self) -> TrackPoint {
        self.points[0]
    }

    pub fn last(&self) -> TrackPoint {
        self.points[self.points.len() - 1]
    }

    /// Its drawing on frame `frame`, if it was drawn then.
    pub fn at(&self, frame: usize) -> Option<TrackPoint> {
        self.points.binary_search_by_key(&frame, |p| p.frame).ok().map(|i| self.points[i])
    }

    /// When it was drawn at `(x, y)`: standing there in its muzzle or
    /// impact frame `stage`, if it ever stood within `near` of it in that
    /// frame (the first such drawing); otherwise when its flight passed
    /// nearest - between two of its drawings, or carried on past its last
    /// drawing in flight at its kind's speed, along the way it was going.
    pub fn time_at(&self, frames: &[FrameSample], x: f32, y: f32, stage: u8, near: f32) -> Passing {
        let dist = |px: f32, py: f32| ((px - x).powi(2) + (py - y).powi(2)).sqrt();
        if stage != flying_stage(self.kind)
            && let Some(p) = self.points.iter().find(|p| p.stage == stage && dist(p.x, p.y) <= near)
        {
            return Passing { distance: dist(p.x, p.y), t_ms: frames[p.frame].t_ms, same_stage: true };
        }
        let mut best = Passing { distance: f32::INFINITY, t_ms: 0.0, same_stage: false };
        let mut consider = |distance: f32, t_ms: f64| {
            if distance < best.distance {
                best = Passing { distance, t_ms, same_stage: false };
            }
        };
        for p in &self.points {
            consider(dist(p.x, p.y), frames[p.frame].t_ms);
        }
        for w in self.points.windows(2).filter(|w| w[1].flying) {
            let (a, b) = (w[0], w[1]);
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let len2 = dx * dx + dy * dy;
            if len2 < 1e-6 {
                continue;
            }
            let u = (((x - a.x) * dx + (y - a.y) * dy) / len2).clamp(0.0, 1.0);
            let (ta, tb) = (frames[a.frame].t_ms, frames[b.frame].t_ms);
            consider(dist(a.x + u * dx, a.y + u * dy), ta + u as f64 * (tb - ta));
        }
        if let Some((p, (hx, hy))) = self.flight() {
            let along = (x - p.x) * hx + (y - p.y) * hy;
            if along > 0.0 {
                let ms = along / (speed(self.kind) / 1000.0);
                consider(dist(p.x + hx * along, p.y + hy * along), frames[p.frame].t_ms + ms as f64);
            }
        }
        best
    }

    /// Where it would be drawn at `t_ms` had it flown on from its last
    /// point, the way it was last seen moving (`flight`) at its kind's
    /// speed, carried at most `cap_ms`; a shot that was not flying stays
    /// where it was. Not its last drawn step: a picture that holds a shot
    /// for a frame and then catches it up draws steps of nothing and of
    /// several frames' flight.
    pub fn carried(&self, frames: &[FrameSample], t_ms: f64, cap_ms: f64) -> (f32, f32) {
        let last = self.last();
        if !last.flying {
            return (last.x, last.y);
        }
        let Some((_, (hx, hy))) = self.flight() else { return (last.x, last.y) };
        let ahead = (t_ms - frames[last.frame].t_ms).clamp(0.0, cap_ms) as f32 * speed(self.kind) / 1000.0;
        (last.x + hx * ahead, last.y + hy * ahead)
    }

    /// Where the picture stopped the shot: its first point in its impact
    /// frames (`vanished` false), a shot that burst before the picture ever
    /// drew it flying included, or, for one taken off the picture in flight,
    /// the frame after its last, at the point its flight had carried it to
    /// by then. `None` for a shot still flying when the frames ran out, or
    /// one that never left its muzzle.
    pub fn stop(&self, frames: &[FrameSample]) -> Option<Stop> {
        if let Some(p) = self.points.iter().find(|p| p.impact) {
            return Some(Stop { frame: p.frame, x: p.x, y: p.y, vanished: false });
        }
        if !self.last().flying {
            return None;
        }
        let gone = self.last().frame + 1;
        let at = frames.get(gone)?;
        let (x, y) = self.carried(frames, at.t_ms, at.t_ms - frames[self.last().frame].t_ms);
        Some(Stop { frame: gone, x, y, vanished: true })
    }

    /// Its last drawing in flight and the way it was flying there, as a
    /// unit vector - from the last drawing before it at least
    /// `FLIGHT_BASE_PX` away (the earliest drawing at another place, if none
    /// is that far), so neither a frame the picture held it still nor a
    /// pixel it was nudged back turns the way round: the line it was on when
    /// it burst or left the picture.
    pub fn flight(&self) -> Option<(TrackPoint, (f32, f32))> {
        let i = self.points.iter().rposition(|p| p.flying)?;
        let p = self.points[i];
        let dist = |q: &TrackPoint| ((q.x - p.x).powi(2) + (q.y - p.y).powi(2)).sqrt();
        let before = &self.points[..i];
        let from = before
            .iter()
            .rev()
            .find(|q| dist(q) >= FLIGHT_BASE_PX)
            .or_else(|| before.iter().find(|q| dist(q) > 1e-3))?;
        let (dx, dy) = (p.x - from.x, p.y - from.y);
        let len = (dx * dx + dy * dy).sqrt();
        Some((p, (dx / len, dy / len)))
    }

    /// How near `(x, y)` its drawn path passes: the segments between its
    /// points, the last one run on `extend` pixels past the end along the
    /// way it was going.
    pub fn path_distance(&self, x: f32, y: f32, extend: f32) -> f32 {
        let mut best = f32::INFINITY;
        let seg = |a: (f32, f32), b: (f32, f32)| {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len2 = dx * dx + dy * dy;
            let u = if len2 < 1e-6 { 0.0 } else { (((x - a.0) * dx + (y - a.1) * dy) / len2).clamp(0.0, 1.0) };
            ((x - a.0 - u * dx).powi(2) + (y - a.1 - u * dy).powi(2)).sqrt()
        };
        for w in self.points.windows(2) {
            best = best.min(seg((w[0].x, w[0].y), (w[1].x, w[1].y)));
        }
        let (first, last) = (self.first(), self.last());
        best = best.min(seg((first.x, first.y), (first.x, first.y)));
        let (dx, dy) = (last.x - first.x, last.y - first.y);
        let len = (dx * dx + dy * dy).sqrt();
        if len > 1e-3 {
            let beyond = (last.x + dx / len * extend, last.y + dy / len * extend);
            best = best.min(seg((last.x, last.y), beyond));
        }
        best
    }
}

/// Every shot `pick` accepts, followed over `frames`: room copies by id,
/// provisionals by continuity. A room copy taken off the picture and put
/// back (hidden, then shown) is two tracks, since the picture showed two
/// appearances. Tracks are in the order they began.
pub fn follow(frames: &[FrameSample], pick: impl Fn(&ShotSample) -> bool) -> Vec<Track> {
    let mut tracks: Vec<Track> = Vec::new();
    for (i, f) in frames.iter().enumerate() {
        let dt = if i == 0 { 0.0 } else { (f.t_ms - frames[i - 1].t_ms).max(0.0) as f32 / 1000.0 };
        let point = |s: &ShotSample| TrackPoint {
            frame: i,
            x: s.x,
            y: s.y,
            flying: s.flying,
            impact: s.impact,
            id: s.id,
            stage: s.stage,
        };
        let shots: Vec<&ShotSample> = f.shots.iter().filter(|s| pick(s)).collect();
        // The tracks drawn on the frame before, still open to this one.
        let open: Vec<usize> = (0..tracks.len()).filter(|&k| i > 0 && tracks[k].last().frame == i - 1).collect();
        let mut taken = vec![false; tracks.len()];
        let mut placed = vec![false; shots.len()];

        for (j, s) in shots.iter().enumerate().filter(|(_, s)| !s.provisional) {
            if let Some(&k) = open.iter().find(|&&k| !taken[k] && tracks[k].room_id == Some(s.id)) {
                taken[k] = true;
                placed[j] = true;
                tracks[k].points.push(point(s));
            }
        }

        // Provisionals: every pairing within reach and not back in the
        // shot's life, nearest first.
        let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
        for (j, s) in shots.iter().enumerate().filter(|(_, s)| s.provisional) {
            let reach = speed(s.kind) * dt * FOLLOW_FLIGHT + FOLLOW_SLACK_PX;
            for &k in open
                .iter()
                .filter(|&&k| tracks[k].provisional && tracks[k].kind == s.kind && tracks[k].last().stage <= s.stage)
            {
                let last = tracks[k].last();
                let (px, py) = tracks[k].carried(frames, f.t_ms, f64::INFINITY);
                let d = ((s.x - last.x).powi(2) + (s.y - last.y).powi(2))
                    .sqrt()
                    .min(((s.x - px).powi(2) + (s.y - py).powi(2)).sqrt());
                if d <= reach {
                    pairs.push((d, j, k));
                }
            }
        }
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, j, k) in pairs {
            if placed[j] || taken[k] {
                continue;
            }
            placed[j] = true;
            taken[k] = true;
            tracks[k].points.push(point(shots[j]));
        }

        for (j, s) in shots.iter().enumerate() {
            if !placed[j] {
                tracks.push(Track {
                    kind: s.kind,
                    seat: s.seat,
                    provisional: s.provisional,
                    room_id: (!s.provisional).then_some(s.id),
                    points: vec![point(s)],
                });
            }
        }
    }
    tracks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shot in flight, or - not flying - bursting (its first impact
    /// frame).
    fn shot(id: u32, x: f32, provisional: bool, flying: bool) -> ShotSample {
        let stage = if flying { flying_stage(0) } else { flying_stage(0) + 1 };
        ShotSample { id, kind: 0, x, y: 100.0, seat: Some(0), provisional, flying, impact: !flying, stage }
    }

    /// A shot standing in its first muzzle frame.
    fn muzzle(id: u32, x: f32) -> ShotSample {
        ShotSample { impact: false, stage: 0, ..shot(id, x, true, false) }
    }

    fn frames(shots: Vec<Vec<ShotSample>>) -> Vec<FrameSample> {
        shots
            .into_iter()
            .enumerate()
            .map(|(i, shots)| FrameSample { t_ms: i as f64 * 1000.0 / 60.0, shots, ..FrameSample::default() })
            .collect()
    }

    /// A track does not hang on a provisional's id: frames that draw one
    /// shot under another's id once that one has gone - a recorded run
    /// whose client numbered its shots by their place - still follow it as
    /// one track.
    #[test]
    fn provisionals_are_followed_through_their_ids_shifting() {
        let base = bongbong::net::predict::PROVISIONAL_ID_BASE;
        let step = speed(0) / 60.0;
        let f = frames(vec![
            vec![shot(base, 100.0, true, true), shot(base + 1, 300.0, true, true)],
            vec![shot(base, 100.0 + step, true, true), shot(base + 1, 300.0 + step, true, true)],
            // The first retires: the second is drawn under the first's id.
            vec![shot(base, 300.0 + 2.0 * step, true, true)],
            vec![shot(base, 300.0 + 3.0 * step, true, true)],
        ]);
        let tracks = follow(&f, |_| true);
        assert_eq!(tracks.len(), 2, "{tracks:?}");
        assert_eq!(tracks[0].points.len(), 2);
        assert_eq!(tracks[1].points.len(), 4, "the second shot is one track under two ids");
    }

    /// A provisional that jumps further than a frame of flight is a new
    /// track - the jump the hand-off gap reports.
    #[test]
    fn a_jump_breaks_a_provisional_track() {
        let base = bongbong::net::predict::PROVISIONAL_ID_BASE;
        let step = speed(0) / 60.0;
        let f = frames(vec![
            vec![shot(base, 400.0, true, true)],
            vec![shot(base, 400.0 + step, true, true)],
            vec![shot(base, 400.0 + step - 60.0, true, false)],
        ]);
        let tracks = follow(&f, |_| true);
        assert_eq!(tracks.len(), 2, "{tracks:?}");
        assert_eq!(tracks[1].first().x, 400.0 + step - 60.0);
    }

    /// A shell standing in its muzzle frames, then leaving at full speed,
    /// is still one shot.
    #[test]
    fn a_shell_leaving_its_muzzle_frames_is_the_same_shell() {
        let base = bongbong::net::predict::PROVISIONAL_ID_BASE;
        let step = speed(0) / 60.0;
        let f = frames(vec![
            vec![muzzle(base, 50.0)],
            vec![muzzle(base, 50.0)],
            vec![shot(base, 50.0 + step, true, true)],
            vec![shot(base, 50.0 + 2.0 * step, true, true)],
        ]);
        let tracks = follow(&f, |_| true);
        assert_eq!(tracks.len(), 1, "{tracks:?}");
    }

    /// A room copy is followed by its id, and hidden then shown is two
    /// appearances.
    #[test]
    fn a_room_copy_is_followed_by_id_and_a_gap_splits_it() {
        let f = frames(vec![
            vec![shot(7, 10.0, false, true), shot(9, 500.0, false, true)],
            vec![shot(7, 900.0, false, true), shot(9, 509.0, false, true)],
            vec![shot(9, 518.0, false, true)],
            vec![shot(7, 20.0, false, true)],
        ]);
        let tracks = follow(&f, |_| true);
        let sevens: Vec<&Track> = tracks.iter().filter(|t| t.room_id == Some(7)).collect();
        assert_eq!(sevens.len(), 2);
        assert_eq!(sevens[0].points.len(), 2, "an id is the same shot however far it moved");
        assert_eq!(tracks.iter().filter(|t| t.room_id == Some(9)).count(), 1);
    }

    /// Where the picture stopped a shot: its impact frames, or the frame
    /// it was taken off in flight, carried there.
    #[test]
    fn a_stop_is_the_impact_or_the_frame_it_vanished_on() {
        let f = frames(vec![
            vec![shot(3, 100.0, false, true)],
            vec![shot(3, 110.0, false, true)],
            vec![],
        ]);
        let tracks = follow(&f, |_| true);
        let stop = tracks[0].stop(&f).expect("a stop");
        assert_eq!((stop.frame, stop.vanished), (2, true));
        assert!((stop.x - (110.0 + speed(0) / 60.0)).abs() < 0.01, "a frame's flight on from 110: {stop:?}");
        let g = frames(vec![vec![shot(3, 100.0, false, true)], vec![shot(3, 104.0, false, false)], vec![shot(3, 104.0, false, false)]]);
        assert_eq!(follow(&g, |_| true)[0].stop(&g), Some(Stop { frame: 1, x: 104.0, y: 100.0, vanished: false }));
        let h = frames(vec![vec![shot(3, 100.0, false, true)], vec![shot(3, 110.0, false, true)]]);
        assert_eq!(follow(&h, |_| true)[0].stop(&h), None, "still flying when the frames ran out");
    }

    /// A burst never starts over: a provisional bursting where another
    /// finished its burst on the frame before is another shot, however
    /// near - one that jumped there - not the old one run back.
    #[test]
    fn a_provisional_never_runs_back_in_its_life() {
        let base = bongbong::net::predict::PROVISIONAL_ID_BASE;
        let last_frame = |x| ShotSample { stage: crate::sample::final_stage(0), ..shot(base, x, true, false) };
        let f = frames(vec![
            vec![last_frame(300.0), shot(base + 1, 40.0, true, true)],
            // The first retires; the second is snapped next to where it
            // burst, in its first impact frame.
            vec![shot(base, 302.0, true, false)],
        ]);
        let tracks = follow(&f, |_| true);
        assert_eq!(tracks.len(), 3, "{tracks:?}");
        assert_eq!(tracks[2].first().x, 302.0);
    }

    /// When a track was at a place: standing in the asked-for muzzle
    /// frame, between two drawings in flight, or carried on past its last
    /// drawing in flight at its kind's speed.
    #[test]
    fn a_track_is_timed_where_it_stood_or_where_its_flight_passed() {
        let base = bongbong::net::predict::PROVISIONAL_ID_BASE;
        let step = speed(0) / 60.0;
        let f = frames(vec![
            vec![muzzle(base, 50.0)],
            vec![ShotSample { stage: 1, ..muzzle(base, 50.0) }],
            vec![shot(base, 50.0 + step, true, true)],
            vec![shot(base, 50.0 + 2.0 * step, true, true)],
            vec![shot(base, 50.0 + 2.0 * step, true, false)],
        ]);
        let track = &follow(&f, |_| true)[0];
        let ms = |frame: f64| frame * 1000.0 / 60.0;
        let muzzle_frame = track.time_at(&f, 50.0, 100.0, 1, 12.0);
        assert!(muzzle_frame.same_stage);
        assert!((muzzle_frame.t_ms - ms(1.0)).abs() < 1e-6, "{muzzle_frame:?}");
        let between = track.time_at(&f, 50.0 + 1.5 * step, 100.0, flying_stage(0), 12.0);
        assert!(!between.same_stage && between.distance < 1e-3);
        assert!((between.t_ms - ms(2.5)).abs() < 1e-3, "{between:?}");
        let beyond = track.time_at(&f, 50.0 + 5.0 * step, 101.0, flying_stage(0), 12.0);
        assert!((beyond.distance - 1.0).abs() < 1e-3, "{beyond:?}");
        assert!((beyond.t_ms - ms(6.0)).abs() < 0.1, "three frames' flight past the last in flight: {beyond:?}");
    }
}
