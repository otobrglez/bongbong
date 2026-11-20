//! The one motion switch (docs/large-maps-follow-camera.md section 6:
//! Apple's "make motion optional", Xbox accessibility guideline 117):
//! whether the camera moves only where the fight needs it to. Reduced
//! motion takes away the motion nobody asked for - the camera shake and a
//! blow's shove, the kill ripple that bends the whole screen, the edge
//! pulse, the health number's drop and the hold on a kill (`feedback.rs`),
//! a slam's hull flash (`fx.rs`), the establishing shot's zoom, which cuts
//! to the tank instead (`establish.rs`), a restart's glide, which cuts to
//! the new start, a shot's kick, the fight's pull and a couch's look-ahead
//! (`follow::FollowRules::reduced`) - and leaves
//! everything that shows the fight: the follow camera itself, the edge
//! arrows and every effect of the round's own. It also calms what flashes
//! (WCAG 2.3.1, no more than three flashes a second): a whole-screen
//! flash - a kill's, a lightning strike's - keeps `reduced_flash_scale` of
//! its strength (`flash`), and a light that strobes - a lit fuse - beats
//! slowly between two steps instead (`beat`).
//!
//! It is one tuning row, `reduce_motion`: follow the platform, off or on.
//! The platform's answer is read once at startup where it is one cheap
//! call - iOS's `UIAccessibilityIsReduceMotionEnabled`, the web page's
//! `prefers-reduced-motion` media query (`window.bbMotion`) - and kept
//! here (`set_platform`). Everywhere else - Android, which would need JNI,
//! and macOS, Linux and Windows - the platform says nothing and the switch
//! starts off. Presentation only, headless: nothing in `simulation/` reads
//! it, and a test resolves the switch with `reduced_by` rather than the
//! process-wide answer.

use std::sync::atomic::{AtomicU8, Ordering};

use crate::tuning::tuning;

/// The `reduce_motion` row: what the platform says.
pub const FOLLOW_PLATFORM: i32 = 0;

/// The `reduce_motion` row: full motion, whatever the platform says.
pub const OFF: i32 = 1;

/// The `reduce_motion` row: reduced motion, whatever the platform says.
pub const ON: i32 = 2;

/// The platform's answer: 0 unknown, 1 full motion, 2 reduced.
static PLATFORM: AtomicU8 = AtomicU8::new(0);

/// Keep the platform's answer - `None` where it gives none. `app::run`
/// calls it once at startup; nothing else writes it.
pub fn set_platform(reduce: Option<bool>) {
    PLATFORM.store(
        match reduce {
            None => 0,
            Some(false) => 1,
            Some(true) => 2,
        },
        Ordering::Relaxed,
    );
}

/// What the platform asked for at startup, `None` where it says nothing.
pub fn platform() -> Option<bool> {
    match PLATFORM.load(Ordering::Relaxed) {
        1 => Some(false),
        2 => Some(true),
        _ => None,
    }
}

/// What a web page's `window.bbMotion` says (`site/src/scripts/motion.ts`,
/// the `prefers-reduced-motion` media query): `reduce`, `no-preference`,
/// or nothing - a browser that cannot answer.
pub fn from_page(answer: &str) -> Option<bool> {
    match answer.trim() {
        "reduce" => Some(true),
        "no-preference" => Some(false),
        _ => None,
    }
}

/// Whether motion is reduced under the `reduce_motion` setting `setting`
/// on a platform that answered `platform`: the setting when it is on or
/// off, else the platform's answer, and full motion where it gave none.
/// An out-of-range setting follows the platform.
pub fn reduced_by(setting: i32, platform: Option<bool>) -> bool {
    match setting {
        OFF => false,
        ON => true,
        _ => platform.unwrap_or(false),
    }
}

/// Whether motion is reduced this frame: the tuning table's row and the
/// platform's answer.
pub fn reduced() -> bool {
    reduced_by(tuning().reduce_motion, platform())
}

/// How strong a whole-screen flash of `strength` is drawn: all of it at
/// full motion, `scale` (the `reduced_flash_scale` row) of it reduced.
pub fn flash(strength: f32, reduced: bool, scale: f32) -> f32 {
    if reduced { strength * scale.clamp(0.0, 1.0) } else { strength }
}

/// How often a strobing light beats under reduced motion, a second: well
/// under the three flashes a second WCAG 2.3.1 allows.
pub const REDUCED_BEAT_HZ: f32 = 1.0;

/// The dim step of a strobing light's slow beat under reduced motion, as
/// a share of its bright step.
pub const REDUCED_BEAT_LOW: f32 = 0.6;

/// A strobing light's strength at clock `time`: its own `wave` (0 to 1,
/// whatever rate the painter runs it at) at full motion; reduced, a slow
/// beat of two held steps - full and `REDUCED_BEAT_LOW` - at
/// `REDUCED_BEAT_HZ`, so the light still says "live" without flashing.
pub fn beat(time: f32, wave: f32, reduced: bool) -> f32 {
    if !reduced {
        return wave;
    }
    let phase = (time * REDUCED_BEAT_HZ).rem_euclid(1.0);
    if phase < 0.5 { 1.0 } else { REDUCED_BEAT_LOW }
}

/// The spelling `status.camera.motion` uses for a setting.
pub fn setting_name(setting: i32) -> &'static str {
    match setting {
        OFF => "off",
        ON => "on",
        _ => "platform",
    }
}

#[cfg(test)]
mod motion_tests {
    use super::*;
    use crate::tuning::Tuning;

    #[test]
    fn the_setting_outranks_the_platform_and_follows_it_by_default() {
        for platform in [None, Some(false), Some(true)] {
            assert!(!reduced_by(OFF, platform), "off is off whatever the platform says: {platform:?}");
            assert!(reduced_by(ON, platform), "on is on whatever the platform says: {platform:?}");
        }
        assert!(!reduced_by(FOLLOW_PLATFORM, None), "a platform that says nothing keeps full motion");
        assert!(!reduced_by(FOLLOW_PLATFORM, Some(false)));
        assert!(reduced_by(FOLLOW_PLATFORM, Some(true)), "the platform's reduced motion is taken");
        assert!(reduced_by(7, Some(true)) && !reduced_by(-1, None), "out of range follows the platform");
    }

    #[test]
    fn a_pages_answer_is_the_media_querys() {
        assert_eq!(from_page("reduce"), Some(true));
        assert_eq!(from_page(" no-preference\n"), Some(false));
        assert_eq!(from_page(""), None, "a browser that cannot say");
        assert_eq!(from_page("REDUCE"), None);
        // Resolved through the row as the platform's answer.
        assert!(reduced_by(FOLLOW_PLATFORM, from_page("reduce")));
        assert!(!reduced_by(FOLLOW_PLATFORM, from_page("")));
        assert!(!reduced_by(OFF, from_page("reduce")));
    }

    #[test]
    fn reduced_motion_scales_a_screen_flash_and_slows_a_strobe() {
        assert_eq!(flash(0.12, false, 0.4), 0.12, "full motion keeps the whole flash");
        assert!((flash(0.12, true, 0.4) - 0.048).abs() < 1e-6, "reduced keeps the row's share");
        assert_eq!(flash(0.12, true, 0.0), 0.0, "and none at 0");
        assert_eq!(flash(0.12, true, 3.0), 0.12, "never more than the flash itself");
        // Full motion passes the painter's own wave through untouched.
        assert_eq!(beat(0.37, 0.25, false), 0.25);
        // Reduced: two held steps, and no more than REDUCED_BEAT_HZ changes
        // of direction a second whatever the wave was doing.
        let samples: Vec<f32> = (0..600).map(|i| beat(i as f32 / 60.0, ((i as f32) * 0.8).sin().abs(), true)).collect();
        assert!(samples.iter().all(|&v| v == 1.0 || v == REDUCED_BEAT_LOW), "only the two steps");
        let rises = samples.windows(2).filter(|w| w[1] > w[0]).count();
        assert!((9..=10).contains(&rises), "one flash a second over ten seconds, got {rises}");
        assert!(Tuning::meta("reduced_flash_scale").is_some_and(|m| m.group == "camera"));
    }

    #[test]
    fn the_row_follows_the_platform_by_default() {
        let meta = Tuning::meta("reduce_motion").expect("the row");
        assert_eq!(meta.group, "camera");
        assert_eq!((meta.min, meta.max), (0.0, 2.0));
        assert_eq!(Tuning::DEFAULT.reduce_motion, FOLLOW_PLATFORM);
        assert_eq!([setting_name(FOLLOW_PLATFORM), setting_name(OFF), setting_name(ON)], ["platform", "off", "on"]);
    }
}
