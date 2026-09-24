//! Room codes (docs/online-coop-prd.md §4.10): five letters, `CK7QX`. One
//! server holds every room, so a code names a room and nothing else - no
//! routing, no directory, no part of it reserved.

use rand::{Rng, RngExt};

/// The symbols a code is drawn from: 20 with no vowels (a code never
/// spells a word by accident) and no look-alikes (no 0/O, 1/I/L, 2/Z,
/// 5/S, 6/G, 8/B, 9/g), so a code survives being read aloud or typed
/// from a photo.
pub use bongbong::net::rooms::CODE_ALPHABET as ALPHABET;

/// How long a code is: 20^5 = 3.2 million rooms, far past `--max-rooms`,
/// so `mint`'s retry on a collision is a formality.
pub use bongbong::net::rooms::CODE_LETTERS as LETTERS;

/// A fresh code.
pub fn mint(rng: &mut impl Rng) -> String {
    (0..LETTERS).map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char).collect()
}

/// A code that is not `LETTERS` symbols from `ALPHABET` - the only way a
/// code can be wrong before the rooms are looked in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Malformed;

impl std::fmt::Display for Malformed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a room code is {LETTERS} letters from {}", std::str::from_utf8(ALPHABET).expect("ASCII"))
    }
}

/// `code` in canonical form (upper case, no surrounding space) if it is
/// well formed.
pub fn check(code: &str) -> Result<String, Malformed> {
    let code: String = code.trim().to_ascii_uppercase();
    let letters: Vec<char> = code.chars().collect();
    if letters.len() != LETTERS || !letters.iter().all(|&c| ALPHABET.contains(&(c as u8))) {
        return Err(Malformed);
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alphabet_has_twenty_distinct_symbols_without_vowels_or_look_alikes() {
        assert_eq!(ALPHABET.len(), 20);
        let mut sorted = ALPHABET.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 20, "every symbol once");
        for banned in b"AEIOU0O1IL2Z5S68B9" {
            assert!(!ALPHABET.contains(banned), "{} is a vowel or a look-alike", *banned as char);
        }
        assert!(ALPHABET.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()));
    }

    #[test]
    fn mint_makes_codes_check_passes_and_the_lobbys_grid_can_type() {
        let mut rng = rand::rng();
        for _ in 0..200 {
            let code = mint(&mut rng);
            assert_eq!(code.chars().count(), LETTERS);
            assert!(code.bytes().all(|b| ALPHABET.contains(&b)), "{code} has a key the grid does not offer");
            assert_eq!(check(&code), Ok(code.clone()));
        }
    }

    #[test]
    fn check_canonicalises_and_refuses_malformed_codes() {
        assert_eq!(check(" ck7qx "), Ok("CK7QX".into()));
        assert_eq!(check(""), Err(Malformed));
        assert_eq!(check("CK7Q"), Err(Malformed));
        assert_eq!(check("CK7QXX"), Err(Malformed));
        assert_eq!(check("CK7QO"), Err(Malformed), "O is not in the alphabet");
        assert_eq!(check("CK7Q1"), Err(Malformed), "1 is not in the alphabet");
        assert_eq!(check("AK7QX"), Err(Malformed), "nor is A");
        assert!(Malformed.to_string().contains("5 letters from CDFGHJKMNPQRTVWXY347"));
    }
}
