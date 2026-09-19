//! Room codes (docs/online-coop-prd.md §4.10): the pod's letter followed
//! by four room letters, `AK7QX`. A join needs no directory: the first
//! letter names the pod, and a pod refuses a code that is not its own.

use rand::{Rng, RngExt};

/// The room letters: 20 symbols with no vowels (a code never spells a
/// word by accident) and no look-alikes (no 0/O, 1/I/L, 2/Z, 5/S, 6/G,
/// 8/B, 9/g), so a code survives being read aloud or typed from a photo.
pub const ALPHABET: &[u8; 20] = b"CDFGHJKMNPQRTVWXY347";

/// Letters after the pod's: 20^4 = 160 000 rooms per pod.
pub const ROOM_LETTERS: usize = 4;

/// A pod letter is any capital letter or digit: the operator picks it
/// (`--pod`), so it is not held to the alphabet.
pub fn pod_letter_valid(c: char) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit()
}

/// A fresh code for `pod`.
pub fn mint(pod: char, rng: &mut impl Rng) -> String {
    let mut code = String::with_capacity(1 + ROOM_LETTERS);
    code.push(pod);
    for _ in 0..ROOM_LETTERS {
        code.push(ALPHABET[rng.random_range(0..ALPHABET.len())] as char);
    }
    code
}

/// Why a code was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodeError {
    /// Not one pod letter plus `ROOM_LETTERS` room letters.
    Malformed,
    /// A well-formed code for another pod.
    OtherPod { code_pod: char, this_pod: char },
}

impl std::fmt::Display for CodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodeError::Malformed => write!(
                f,
                "a room code is one pod letter and {ROOM_LETTERS} letters from {}",
                std::str::from_utf8(ALPHABET).expect("ASCII")
            ),
            CodeError::OtherPod { code_pod, this_pod } => {
                write!(f, "code is for pod {code_pod}, this is pod {this_pod}")
            }
        }
    }
}

/// `code` in canonical form (upper case, no surrounding space) if it is
/// well formed and for `pod`.
pub fn check(code: &str, pod: char) -> Result<String, CodeError> {
    let code: String = code.trim().to_ascii_uppercase();
    let mut chars = code.chars();
    let code_pod = chars.next().ok_or(CodeError::Malformed)?;
    let room: Vec<char> = chars.collect();
    if !pod_letter_valid(code_pod) || room.len() != ROOM_LETTERS || !room.iter().all(|&c| ALPHABET.contains(&(c as u8))) {
        return Err(CodeError::Malformed);
    }
    if code_pod != pod {
        return Err(CodeError::OtherPod { code_pod, this_pod: pod });
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
    fn mint_makes_codes_for_the_pod_that_check_passes() {
        let mut rng = rand::rng();
        for _ in 0..200 {
            let code = mint('A', &mut rng);
            assert_eq!(code.len(), 1 + ROOM_LETTERS);
            assert!(code.starts_with('A'));
            assert_eq!(check(&code, 'A'), Ok(code.clone()));
            assert_eq!(
                check(&code, 'B'),
                Err(CodeError::OtherPod { code_pod: 'A', this_pod: 'B' }),
                "another pod refuses it by name"
            );
        }
    }

    #[test]
    fn check_canonicalises_and_refuses_malformed_codes() {
        assert_eq!(check(" ak7qx ", 'A'), Ok("AK7QX".into()));
        assert_eq!(check("", 'A'), Err(CodeError::Malformed));
        assert_eq!(check("AK7Q", 'A'), Err(CodeError::Malformed));
        assert_eq!(check("AK7QXX", 'A'), Err(CodeError::Malformed));
        assert_eq!(check("AK7QO", 'A'), Err(CodeError::Malformed), "O is not in the alphabet");
        assert_eq!(check("AK7Q1", 'A'), Err(CodeError::Malformed), "1 is not in the alphabet");
        assert_eq!(
            check("ak7qx", 'a'),
            Err(CodeError::OtherPod { code_pod: 'A', this_pod: 'a' }),
            "the code is read in capitals; the pod letter is the operator's"
        );
        assert!(CodeError::OtherPod { code_pod: 'B', this_pod: 'A' }.to_string().contains("pod B"));
    }
}
