//! Fuzzy search (subsequence matching with bonuses), fzf style.

pub fn lower(s: &str) -> Vec<char> {
    s.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect()
}

/// `hay` and `needle` already lowercased. `None` = no match.
pub fn score(hay: &[char], needle: &[char]) -> Option<i32> {
    if needle.is_empty() {
        return Some(0);
    }
    // 1) forward pass: find the end of the first match
    let mut qi = 0;
    let mut end = 0;
    for (i, &c) in hay.iter().enumerate() {
        if c == needle[qi] {
            qi += 1;
            if qi == needle.len() {
                end = i;
                break;
            }
        }
    }
    if qi < needle.len() {
        return None;
    }
    // 2) backward pass: tighten the match to the shortest span
    let mut pos = vec![0usize; needle.len()];
    let mut qj = needle.len();
    let mut i = end + 1;
    while qj > 0 {
        i -= 1;
        if hay[i] == needle[qj - 1] {
            qj -= 1;
            pos[qj] = i;
        }
    }
    // 3) scoring
    let mut s = 0i32;
    for (k, &p) in pos.iter().enumerate() {
        s += 16;
        if p == 0 || !hay[p - 1].is_alphanumeric() {
            s += 12; // start of a word
        }
        if k > 0 {
            if p == pos[k - 1] + 1 {
                s += 20; // consecutivo
            } else {
                s -= ((p - pos[k - 1] - 1) as i32).min(10);
            }
        }
    }
    s -= (pos[0] as i32).min(30) / 2; // late start
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc(hay: &str, needle: &str) -> Option<i32> {
        score(&lower(hay), &lower(needle))
    }

    #[test]
    fn empty_needle_matches_with_zero() {
        assert_eq!(sc("anything", ""), Some(0));
        assert_eq!(sc("", ""), Some(0));
    }

    #[test]
    fn non_match_returns_none() {
        assert_eq!(sc("hello world", "xyz"), None);
        assert_eq!(sc("abc", "abcd"), None);
        assert_eq!(sc("abc", "cba"), None);
    }

    #[test]
    fn case_insensitive_and_unicode() {
        assert!(sc("CoffeeCopy Clipboard", "cc").is_some());
        assert!(sc("Água Café", "água").is_some());
        assert_eq!(lower("ÁBC"), vec!['á', 'b', 'c']);
    }

    #[test]
    fn consecutive_beats_scattered() {
        let tight = sc("hello world", "world").unwrap();
        let spread = sc("w o r l d", "world").unwrap();
        assert!(tight > spread, "expected {tight} > {spread}");
    }

    #[test]
    fn word_boundary_beats_mid_word() {
        let boundary = sc("foo bar", "bar").unwrap();
        let mid = sc("foobar", "bar").unwrap();
        assert!(boundary > mid, "expected {boundary} > {mid}");
    }

    #[test]
    fn backward_pass_tightens_span() {
        // 'a' appears early and again right before "bc"; backward pass should pick the tight "abc"
        let s = sc("a________abc", "abc").unwrap();
        let scattered = sc("a________xbc", "abc").unwrap();
        assert!(s > scattered, "expected {s} > {scattered}");
    }
}
