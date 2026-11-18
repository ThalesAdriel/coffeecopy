//! Persistent history: an append-only file (`history.bin`) with the items
//! and an `img/` folder with the copied bitmaps (DIB).

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Text = 0,
    Image = 1,
}

#[derive(Clone)]
pub struct Entry {
    pub kind: Kind,
    pub ts: u64,
    pub hash: u64,
    /// Copied text; for images, a label like "[Image 800x600]".
    pub text: String,
}

pub struct History {
    /// Oldest first, most recently used last.
    pub entries: Vec<Entry>,
    known: HashSet<u64>,
}

/// FNV-1a 64-bit: stable across runs and Rust versions.
pub fn fnv(tag: u32, bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325 ^ tag as u64;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub const CF_UNICODETEXT: u32 = 13;

pub fn text_hash(s: &str) -> u64 {
    let bytes: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    fnv(CF_UNICODETEXT, &bytes)
}

fn appdata() -> PathBuf {
    PathBuf::from(std::env::var_os("APPDATA").unwrap_or_else(|| ".".into()))
}

pub fn data_dir() -> PathBuf {
    // COFFEECOPY_DATA overrides the folder (used by the demo mode so it never touches real data)
    if let Some(d) = std::env::var_os("COFFEECOPY_DATA") {
        return PathBuf::from(d);
    }
    appdata().join("CoffeeCopy")
}

/// The app used to be called CopySound: brings the old data over to the new folder.
pub fn migrate_old_data() {
    let old = appdata().join("CopySound");
    if old.exists() && !data_dir().exists() {
        let _ = fs::rename(old, data_dir());
    }
}

pub fn image_path(hash: u64) -> PathBuf {
    data_dir().join("img").join(format!("{hash:016x}.dib"))
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn encode(e: &Entry) -> Vec<u8> {
    let t = e.text.as_bytes();
    let mut v = Vec::with_capacity(21 + t.len());
    v.push(e.kind as u8);
    v.extend_from_slice(&e.ts.to_le_bytes());
    v.extend_from_slice(&e.hash.to_le_bytes());
    v.extend_from_slice(&(t.len() as u32).to_le_bytes());
    v.extend_from_slice(t);
    v
}

impl History {
    pub fn load() -> Self {
        let dir = data_dir();
        let _ = fs::create_dir_all(dir.join("img"));
        let path = dir.join("history.bin");
        let data = fs::read(&path).unwrap_or_default();

        let mut all: Vec<Entry> = Vec::new();
        let mut p = 0usize;
        while p + 21 <= data.len() {
            let kind = if data[p] == 1 { Kind::Image } else { Kind::Text };
            let ts = u64::from_le_bytes(data[p + 1..p + 9].try_into().unwrap());
            let hash = u64::from_le_bytes(data[p + 9..p + 17].try_into().unwrap());
            let len = u32::from_le_bytes(data[p + 17..p + 21].try_into().unwrap()) as usize;
            if p + 21 + len > data.len() {
                break; // truncated record
            }
            let text = String::from_utf8_lossy(&data[p + 21..p + 21 + len]).into_owned();
            all.push(Entry { kind, ts, hash, text });
            p += 21 + len;
        }
        let total = all.len();

        // Keep only the last occurrence of each hash (the most recent one).
        let mut seen = HashSet::new();
        let mut entries: Vec<Entry> = Vec::new();
        for e in all.into_iter().rev() {
            if seen.insert(e.hash) {
                entries.push(e);
            }
        }
        entries.reverse();

        let h = History { entries, known: seen };
        if total > h.entries.len() || p < data.len() {
            h.rewrite();
        }
        h
    }

    fn rewrite(&self) {
        let dir = data_dir();
        let tmp = dir.join("history.tmp");
        let mut buf = Vec::new();
        for e in &self.entries {
            buf.extend_from_slice(&encode(e));
        }
        if fs::write(&tmp, buf).is_ok() {
            let _ = fs::rename(&tmp, dir.join("history.bin"));
        }
    }

    fn append(&self, e: &Entry) {
        if let Ok(mut f) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(data_dir().join("history.bin"))
        {
            let _ = f.write_all(&encode(e));
        }
    }

    /// Records a copy. Returns `true` if the content was already in the history.
    pub fn add(&mut self, hash: u64, kind: Kind, text: String, image: Option<&[u8]>) -> bool {
        if self.known.contains(&hash) {
            self.touch(hash);
            return true;
        }
        if let Some(bytes) = image {
            let _ = fs::write(image_path(hash), bytes);
        }
        let e = Entry { kind, ts: now(), hash, text };
        self.append(&e);
        self.known.insert(hash);
        self.entries.push(e);
        false
    }

    /// Deletes an item from the history (and its image on disk, if any).
    pub fn remove(&mut self, hash: u64) {
        if let Some(pos) = self.entries.iter().position(|e| e.hash == hash) {
            let e = self.entries.remove(pos);
            self.known.remove(&hash);
            if e.kind == Kind::Image {
                let _ = fs::remove_file(image_path(hash));
            }
            self.rewrite();
        }
    }

    /// Deletes everything except the hashes in `keep`. Returns how many items were removed.
    pub fn clear_except(&mut self, keep: &HashSet<u64>) -> usize {
        let before = self.entries.len();
        let (kept, gone): (Vec<Entry>, Vec<Entry>) =
            std::mem::take(&mut self.entries).into_iter().partition(|e| keep.contains(&e.hash));
        for e in &gone {
            if e.kind == Kind::Image {
                let _ = fs::remove_file(image_path(e.hash));
            }
        }
        self.entries = kept;
        self.known = self.entries.iter().map(|e| e.hash).collect();
        self.rewrite();
        before - self.entries.len()
    }

    /// How many items were last used before `cutoff` (unix seconds) and are not in `keep`.
    pub fn count_older(&self, cutoff: u64, keep: &HashSet<u64>) -> usize {
        self.entries.iter().filter(|e| e.ts < cutoff && !keep.contains(&e.hash)).count()
    }

    /// Deletes the items last used before `cutoff` that are not in `keep`. Returns how many went.
    pub fn remove_older(&mut self, cutoff: u64, keep: &HashSet<u64>) -> usize {
        if self.count_older(cutoff, keep) == 0 {
            return 0;
        }
        let stay: HashSet<u64> =
            self.entries.iter().filter(|e| e.ts >= cutoff || keep.contains(&e.hash)).map(|e| e.hash).collect();
        self.clear_except(&stay)
    }

    /// Moves an existing item to the top (used on repeated copies and on paste).
    pub fn touch(&mut self, hash: u64) {
        if let Some(pos) = self.entries.iter().position(|e| e.hash == hash) {
            let mut e = self.entries.remove(pos);
            e.ts = now();
            self.append(&e);
            self.entries.push(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_removes_only_old_items_that_are_not_favorites() {
        let dir = std::env::temp_dir().join(format!("coffeecopy-test-{}", std::process::id()));
        std::env::set_var("COFFEECOPY_DATA", &dir);
        let mut h = History::load();
        for t in ["old", "old but favorite", "recent"] {
            h.add(text_hash(t), Kind::Text, t.to_string(), None);
        }
        h.entries[0].ts = 100;
        h.entries[1].ts = 100;
        h.entries[2].ts = 5_000;
        let keep: HashSet<u64> = [text_hash("old but favorite")].into_iter().collect();
        assert_eq!(h.count_older(1_000, &keep), 1);
        assert_eq!(h.remove_older(1_000, &keep), 1);
        assert_eq!(h.entries.len(), 2);
        assert_eq!(h.remove_older(1_000, &keep), 0); // nothing left to remove
        assert_eq!(History::load().entries.len(), 2); // and the file was rewritten
        let _ = fs::remove_dir_all(&dir);
    }
}
