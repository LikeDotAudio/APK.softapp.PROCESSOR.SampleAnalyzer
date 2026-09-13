/// Normalize a file name into space-separated tokens for name matching:
/// lower-cased, every non-alphanumeric run becomes a space, and letter↔digit
/// boundaries are split (so "Tom2" → "tom 2", "OH_01" → "oh 01").
///
/// This form deliberately does NOT split camelCase. `categorize()` matches on
/// substrings and its rule table is written against welded names — "BassD" and
/// "BDrum" are its abbreviations for a bass drum. Use `normalize_name_words()` for
/// whole-token matching; see the note there.
pub fn normalize_name(name: &str) -> String {
    let lower = name.to_lowercase();
    let mut out = String::with_capacity(lower.len() + 8);
    let mut prev = 0u8; // 0 = sep, 1 = alpha, 2 = digit
    for c in lower.chars() {
        let kind = if c.is_ascii_alphabetic() { 1 } else if c.is_ascii_digit() { 2 } else { 0 };
        if kind == 0 {
            if !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            if prev != 0 && prev != kind {
                out.push(' ');
            }
            out.push(c);
        }
        prev = kind;
    }
    out
}

/// The same normalization, but with camelCase split into separate words first:
/// "KickDrum" → "kick drum", "BDKick" → "bd kick".
///
/// The UCS matcher compares WHOLE TOKENS against a synonym table, so a welded
/// "kickdrum" matches nothing in any of the 756 subcategories — every CamelCase drum
/// sample was invisible to the classifier while being obvious to `categorize()`, which
/// only needs the substring. Splitting is right for the matcher and wrong for the rule
/// table (it would turn "BassD" into "bass d" and call a bass drum a bass guitar), so
/// the two get different tokenizers rather than one compromised one.
pub fn normalize_name_words(name: &str) -> String {
    normalize_name(&split_camel_case(name))
}

/// Insert a space at every camelCase boundary: a lower-case letter or digit followed
/// by an upper-case one ("KickDrum" → "Kick Drum", "hiHat" → "hi Hat"), and the tail of
/// an acronym running into a word ("BDKick" → "BD Kick", "IRHall" → "IR Hall").
fn split_camel_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 8);
    for (i, &c) in chars.iter().enumerate() {
        if i > 0 && c.is_uppercase() {
            let prev = chars[i - 1];
            // lower→UPPER is a boundary. So is the tail of an acronym running into a
            // word ("BD|Kick") — but only when at least TWO capitals precede, or a plain
            // capitalized word gets shattered: "BDrum" (a bass drum) is B + Drum, not an
            // acronym, and splitting it there cost the Kick rule its "bd" token.
            let ends_acronym = prev.is_uppercase()
                && i >= 2
                && chars[i - 2].is_uppercase()
                && chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            if prev.is_lowercase() || prev.is_ascii_digit() || ends_acronym {
                out.push(' ');
            }
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_case_names_split_into_matchable_words() {
        assert_eq!(normalize_name_words("KickDrum.wav"), "kick drum wav");
        assert_eq!(normalize_name_words("hiHat"), "hi hat");
        assert_eq!(normalize_name_words("BDKick"), "bd kick");
        assert_eq!(normalize_name_words("OpenHat_02"), "open hat 02");
    }

    #[test]
    fn existing_separator_and_digit_splitting_still_holds() {
        assert_eq!(normalize_name("Tom2"), "tom 2");
        assert_eq!(normalize_name("OH_01"), "oh 01");
        assert_eq!(normalize_name("808op_hat"), "808 op hat");
    }

    // An acronym on its own must not be shattered into single letters.
    #[test]
    fn an_acronym_is_not_split_into_letters() {
        assert_eq!(normalize_name_words("IR"), "ir");
        assert_eq!(normalize_name_words("TR808"), "tr 808");
    }

    // "BDrum" is a bass drum — B + Drum, not the acronym "BD" + "rum". A single
    // leading capital is just a capitalized word, not an acronym tail.
    #[test]
    fn a_single_leading_capital_is_not_an_acronym() {
        assert_eq!(normalize_name_words("BDrum_7.wav"), "bdrum 7 wav");
    }

    // categorize()'s rule table is written against welded names; splitting them
    // renames a bass drum to a bass guitar. It must keep the unsplit form.
    #[test]
    fn the_rule_tables_tokenizer_leaves_welded_abbreviations_alone() {
        assert_eq!(normalize_name("BassD_2.wav"), "bassd 2 wav");
        assert_eq!(normalize_name("BDrum_7.wav"), "bdrum 7 wav");
    }
}
