//! A seat's steering keys: which of its held direction keys the tank
//! drives by. Headless and pure - `app.rs` reads raylib's held keys and
//! hands them here, one `Steer` per keyboard seat (the arrows, WASD).
//!
//! The newest key still held wins: rolling from one arrow onto the next
//! without lifting the first turns the tank the moment the second goes
//! down, and letting the second go hands the tank back to the first. Keys
//! that go down on the same frame stand in `Dir::ALL`'s order - Up, Down,
//! Left, Right - Up the newest of them.

use crate::tank::Dir;

/// One seat's held direction keys in the order they went down, the
/// newest last.
#[derive(Debug, Clone, Default)]
pub struct Steer {
    held: Vec<Dir>,
}

impl Steer {
    /// Brings the order up to this frame and returns the direction it
    /// steers. `down[d.index()]` is whether `d`'s key is held now: a key
    /// let go leaves the order, a held key not in it yet joins at the
    /// newest end.
    pub fn update(&mut self, down: [bool; 4]) -> Option<Dir> {
        self.held.retain(|d| down[d.index()]);
        for d in Dir::ALL.into_iter().rev() {
            if down[d.index()] && !self.held.contains(&d) {
                self.held.push(d);
            }
        }
        self.held.last().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(held: &[Dir]) -> [bool; 4] {
        let mut down = [false; 4];
        for d in held {
            down[d.index()] = true;
        }
        down
    }

    #[test]
    fn a_key_pressed_while_another_is_held_takes_over() {
        let mut steer = Steer::default();
        assert_eq!(steer.update(keys(&[Dir::Up])), Some(Dir::Up));
        assert_eq!(steer.update(keys(&[Dir::Up, Dir::Left])), Some(Dir::Left));
        assert_eq!(steer.update(keys(&[Dir::Up, Dir::Left, Dir::Down])), Some(Dir::Down));
        assert_eq!(steer.update(keys(&[Dir::Up, Dir::Left, Dir::Down])), Some(Dir::Down), "held, it stays");
    }

    #[test]
    fn letting_the_newest_go_hands_back_to_the_one_before() {
        let mut steer = Steer::default();
        steer.update(keys(&[Dir::Right]));
        steer.update(keys(&[Dir::Right, Dir::Up]));
        steer.update(keys(&[Dir::Right, Dir::Up, Dir::Left]));
        assert_eq!(steer.update(keys(&[Dir::Right, Dir::Up])), Some(Dir::Up));
        assert_eq!(steer.update(keys(&[Dir::Right])), Some(Dir::Right));
        assert_eq!(steer.update(keys(&[])), None);
    }

    #[test]
    fn letting_an_older_key_go_keeps_the_newest() {
        let mut steer = Steer::default();
        steer.update(keys(&[Dir::Down]));
        steer.update(keys(&[Dir::Down, Dir::Right]));
        assert_eq!(steer.update(keys(&[Dir::Right])), Some(Dir::Right));
    }

    #[test]
    fn a_key_pressed_again_is_the_newest_again() {
        let mut steer = Steer::default();
        steer.update(keys(&[Dir::Left]));
        steer.update(keys(&[Dir::Left, Dir::Up]));
        steer.update(keys(&[Dir::Up]));
        assert_eq!(steer.update(keys(&[Dir::Up, Dir::Left])), Some(Dir::Left));
    }

    #[test]
    fn keys_down_on_one_frame_stand_up_down_left_right() {
        let all = [Dir::Up, Dir::Down, Dir::Left, Dir::Right];
        assert_eq!(Steer::default().update(keys(&all)), Some(Dir::Up));
        assert_eq!(Steer::default().update(keys(&[Dir::Down, Dir::Right])), Some(Dir::Down));
        assert_eq!(Steer::default().update(keys(&[Dir::Left, Dir::Right])), Some(Dir::Left));
        // Up gone, the rest hand on in the same order.
        let mut steer = Steer::default();
        steer.update(keys(&all));
        assert_eq!(steer.update(keys(&[Dir::Down, Dir::Left, Dir::Right])), Some(Dir::Down));
        assert_eq!(steer.update(keys(&[Dir::Left, Dir::Right])), Some(Dir::Left));
    }
}
