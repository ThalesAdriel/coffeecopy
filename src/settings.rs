//! Settings (settings.ini), favorites (favorites.txt) and themes.

use std::collections::HashSet;
use std::fs;

use crate::history::data_dir;

/// Modifier bits of a key binding.
pub const M_CTRL: u8 = 1;
pub const M_SHIFT: u8 = 2;
pub const M_ALT: u8 = 4;
pub const M_WIN: u8 = 8;

/// A key combination: modifier bits + virtual-key code.
#[derive(Clone, Copy, PartialEq)]
pub struct Bind {
    pub mods: u8,
    pub vk: u16,
}

impl Bind {
    pub fn ser(&self) -> String {
        format!("{},{}", self.mods, self.vk)
    }
    pub fn parse(s: &str) -> Option<Bind> {
        let (m, v) = s.split_once(',')?;
        Some(Bind { mods: m.trim().parse().ok()?, vk: v.trim().parse().ok()? })
    }
}

/// Defaults: Ctrl+Shift+V (open), Ctrl+D (favorite), Ctrl+Delete (delete), Tab (next tab).
pub const DEFAULT_KEYS: [Bind; 4] = [
    Bind { mods: M_CTRL | M_SHIFT, vk: 0x56 },
    Bind { mods: M_CTRL, vk: 0x44 },
    Bind { mods: M_CTRL, vk: 0x2E },
    Bind { mods: 0, vk: 0x09 },
];

#[derive(Clone, Copy)]
pub struct Settings {
    pub theme: usize,
    /// 40..=100 (%)
    pub opacity: u8,
    pub pack: usize,
    /// 0..=100 (%)
    pub volume: u8,
    pub muted: bool,
    /// Blur (acrylic) behind the window
    pub blur: bool,
    /// Opening and closing animations
    pub anim: bool,
    /// 0 = fade, 1 = slide up, 2 = slide down
    pub anim_style: usize,
    /// Duration in ms (80..=500)
    pub anim_ms: u32,
    /// Use Win+V to open CoffeeCopy instead of the Windows clipboard history
    pub win_v: bool,
    /// Where the window opens: 0 center, 1 near the cursor, 2 top, 3 bottom-right corner
    pub pos: usize,
    /// Corner radius (logical px, 0..=24); 0 (square corners) is the default
    pub radius: u32,
    /// When choosing an item (Enter / double click): true = paste right away; false = copy only
    pub paste_on_select: bool,
    /// Play a sound when choosing/pasting a history item
    pub paste_sound: bool,
    /// Compact mode: narrow window, list only (no preview), smaller rows
    pub compact: bool,
    /// Confirm the item with a single click (false = double click, the default behavior)
    pub single_click: bool,
    /// Interface language: 0 English, 1 Português, 2 Русский, 3 中文, 4 日本語
    pub lang: usize,
    /// Remappable keys: [open (global), favorite, delete, next tab]
    pub keys: [Bind; 4],
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: 0,
            opacity: 77,
            pack: 0,
            volume: 80,
            muted: false,
            blur: true,
            anim: true,
            anim_style: 1,
            anim_ms: 180,
            win_v: false,
            pos: 0,
            radius: 0,
            paste_on_select: true,
            paste_sound: true,
            compact: false,
            single_click: false,
            lang: 0,
            keys: DEFAULT_KEYS,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let mut s = Settings::default();
        if let Ok(txt) = fs::read_to_string(data_dir().join("settings.ini")) {
            for line in txt.lines() {
                let Some((k, v)) = line.split_once('=') else { continue };
                let v = v.trim();
                match k.trim() {
                    "theme" => s.theme = v.parse().unwrap_or(0).min(THEMES.len() - 1),
                    "opacity" => s.opacity = v.parse().unwrap_or(100u8).clamp(40, 100),
                    "pack" => s.pack = v.parse().unwrap_or(0),
                    "volume" => s.volume = v.parse().unwrap_or(80u8).min(100),
                    "muted" => s.muted = v == "1",
                    "blur" => s.blur = v == "1",
                    "anim" => s.anim = v == "1",
                    "win_v" => s.win_v = v == "1",
                    "pos" => s.pos = v.parse().unwrap_or(0).min(3),
                    "radius" => s.radius = v.parse().unwrap_or(12u32).min(24),
                    "paste_on_select" => s.paste_on_select = v != "0",
                    "paste_sound" => s.paste_sound = v != "0",
                    "compact" => s.compact = v == "1",
                    "single_click" => s.single_click = v == "1",
                    "lang" => s.lang = v.parse().unwrap_or(0usize).min(4),
                    "key_open" => s.keys[0] = Bind::parse(v).unwrap_or(DEFAULT_KEYS[0]),
                    "key_fav" => s.keys[1] = Bind::parse(v).unwrap_or(DEFAULT_KEYS[1]),
                    "key_del" => s.keys[2] = Bind::parse(v).unwrap_or(DEFAULT_KEYS[2]),
                    "key_tab" => s.keys[3] = Bind::parse(v).unwrap_or(DEFAULT_KEYS[3]),
                    "anim_style" => s.anim_style = v.parse().unwrap_or(1).min(2),
                    "anim_ms" => s.anim_ms = v.parse().unwrap_or(180u32).clamp(80, 500),
                    _ => {}
                }
            }
        }
        s
    }

    pub fn save(&self) {
        let txt = format!(
            "theme={}\nopacity={}\npack={}\nvolume={}\nmuted={}\nblur={}\nanim={}\nanim_style={}\nanim_ms={}\nwin_v={}\npos={}\nradius={}\npaste_on_select={}\npaste_sound={}\ncompact={}\nsingle_click={}\nlang={}\nkey_open={}\nkey_fav={}\nkey_del={}\nkey_tab={}\n",
            self.theme, self.opacity, self.pack, self.volume, self.muted as u8, self.blur as u8,
            self.anim as u8, self.anim_style, self.anim_ms, self.win_v as u8, self.pos, self.radius,
            self.paste_on_select as u8, self.paste_sound as u8, self.compact as u8, self.single_click as u8, self.lang,
            self.keys[0].ser(), self.keys[1].ser(), self.keys[2].ser(), self.keys[3].ser()
        );
        let _ = fs::create_dir_all(data_dir());
        let _ = fs::write(data_dir().join("settings.ini"), txt);
    }
}

pub fn load_favs() -> HashSet<u64> {
    fs::read_to_string(data_dir().join("favorites.txt"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| u64::from_str_radix(l.trim(), 16).ok())
        .collect()
}

pub fn save_favs(f: &HashSet<u64>) {
    let txt: String = f.iter().map(|h| format!("{h:016x}\n")).collect();
    let _ = fs::create_dir_all(data_dir());
    let _ = fs::write(data_dir().join("favorites.txt"), txt);
}

const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    pub bg: u32,
    pub ctrl: u32,
    pub prev: u32,
    pub text: u32,
    pub dim: u32,
    pub accent: u32,
    pub on_accent: u32,
    pub sel: u32,
    pub track: u32,
    pub icon_img: u32,
}

pub const THEMES: [Theme; 3] = [
    Theme {
        name: "Dark",
        bg: rgb(30, 30, 30),
        ctrl: rgb(37, 37, 38),
        prev: rgb(24, 24, 24),
        text: rgb(232, 232, 232),
        dim: rgb(140, 146, 160),
        accent: rgb(0, 120, 212),
        on_accent: rgb(255, 255, 255),
        sel: rgb(58, 64, 82),
        track: rgb(62, 62, 68),
        icon_img: rgb(110, 175, 255),
    },
    Theme {
        name: "OLED black",
        bg: rgb(0, 0, 0),
        ctrl: rgb(14, 14, 15),
        prev: rgb(6, 6, 6),
        text: rgb(236, 236, 236),
        dim: rgb(128, 130, 138),
        accent: rgb(10, 132, 255),
        on_accent: rgb(255, 255, 255),
        sel: rgb(34, 36, 42),
        track: rgb(40, 40, 44),
        icon_img: rgb(110, 175, 255),
    },
    Theme {
        name: "Crimson",
        bg: rgb(27, 14, 18),
        ctrl: rgb(40, 21, 27),
        prev: rgb(21, 10, 14),
        text: rgb(242, 227, 230),
        dim: rgb(178, 140, 149),
        accent: rgb(210, 28, 60),
        on_accent: rgb(255, 255, 255),
        sel: rgb(94, 26, 46),
        track: rgb(76, 38, 49),
        icon_img: rgb(255, 128, 150),
    },
];
