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
