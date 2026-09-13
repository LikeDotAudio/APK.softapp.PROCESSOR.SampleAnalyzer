//! Repeated phrases in the slice sequence, and how it is quoted back.

use std::collections::HashMap;

use super::tunables::MAXIMUM_MOTIF_LENGTH;

/// The longest phrase that occurs more than once, and how many times. Searched
/// from long to short so the first hit is the answer; capped at
/// `MAXIMUM_MOTIF_LENGTH` because a "motif" the length of the song is just the
/// song. Occurrences may overlap — a stutter (AAAA) is a real repetition of AA.
pub(super) fn longest_repeated_motif(sequence: &str) -> (String, usize) {
    let letters: Vec<char> = sequence.chars().collect();
    let n = letters.len();
    let longest = MAXIMUM_MOTIF_LENGTH.min(n / 2);
    for length in (2..=longest).rev() {
        let mut seen: HashMap<&[char], usize> = HashMap::new();
        for start in 0..=(n - length) {
            *seen.entry(&letters[start..start + length]).or_insert(0) += 1;
        }
        if let Some((phrase, count)) = seen
            .iter()
            .filter(|(_, &c)| c > 1)
            .max_by(|x, y| x.1.cmp(y.1).then(y.0.cmp(x.0)))
        {
            return (phrase.iter().collect(), *count);
        }
    }
    (String::new(), 0)
}

/// The sequence is written in full into its own field; the reason line only
/// needs enough of it to recognize the shape.
pub(super) fn truncate_sequence(sequence: &str) -> String {
    const SHOWN: usize = 32;
    if sequence.chars().count() <= SHOWN {
        sequence.to_string()
    } else {
        format!("{}…", sequence.chars().take(SHOWN).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{analyze, sequence_of};
    use super::*;

    #[test]
    fn motif_search_finds_the_longest_repeat() {
        assert_eq!(longest_repeated_motif("ABCABC"), ("ABC".to_string(), 2));
        assert_eq!(longest_repeated_motif("ABCD"), (String::new(), 0));
        let (phrase, count) = longest_repeated_motif("AAAA");
        assert_eq!(count, 3, "overlapping repeats of {} count", phrase);
    }

    #[test]
    fn an_alternating_song_has_a_motif() {
        let r = sequence_of(&[0, 1, 0, 1, 0, 1], 0.2, 0.3);
        let s = analyze(&r, 3.5);
        assert!(s.dominant_motif_occurrences > 1, "no repeated motif in {}", s.sequence);
        assert!(s.dominant_motif.len() >= 2);
    }
}
