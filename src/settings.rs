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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

/// Defaults: Ctrl+Alt+H (open; Ctrl+Shift+V stays free for "paste without formatting"), Ctrl+D (favorite), Ctrl+Delete (delete), Tab (next tab).
pub const DEFAULT_KEYS: [Bind; 4] = [
    Bind { mods: M_CTRL | M_ALT, vk: 0x48 },
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
    /// Accent color: 0 = the theme's own, 1.. = `ACCENTS[n - 1]`
    pub accent: usize,
    /// Delete items not used for `cleanup_days` days (favorites are kept). Off by default.
    pub cleanup: bool,
    pub cleanup_days: u32,
    /// Remappable keys: [open (global), favorite, delete, next tab]
    pub keys: [Bind; 4],
}

/// Choices for the automatic cleanup, in days.
pub const CLEAN_DAYS: [u32; 4] = [7, 30, 90, 365];

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
            accent: 0,
            cleanup: false,
            cleanup_days: 30,
            keys: DEFAULT_KEYS,
        }
    }
}

impl Settings {
    pub fn parse(txt: &str) -> Self {
        let mut s = Settings::default();
        for line in txt.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "theme" => s.theme = v.parse().unwrap_or(s.theme).min(THEMES.len() - 1),
                "opacity" => s.opacity = v.parse().unwrap_or(s.opacity).clamp(40, 100),
                "pack" => s.pack = v.parse().unwrap_or(s.pack),
                "volume" => s.volume = v.parse().unwrap_or(s.volume).min(100),
                "muted" => s.muted = v == "1",
                "blur" => s.blur = v == "1",
                "anim" => s.anim = v == "1",
                "win_v" => s.win_v = v == "1",
                "pos" => s.pos = v.parse().unwrap_or(s.pos).min(3),
                "radius" => s.radius = v.parse().unwrap_or(s.radius).min(24),
                "paste_on_select" => s.paste_on_select = v != "0",
                "paste_sound" => s.paste_sound = v != "0",
                "compact" => s.compact = v == "1",
                "single_click" => s.single_click = v == "1",
                "lang" => s.lang = v.parse().unwrap_or(s.lang).min(4),
                "accent" => s.accent = v.parse().unwrap_or(s.accent).min(ACCENTS.len()),
                "cleanup" => s.cleanup = v == "1",
                "cleanup_days" => {
                    let d = v.parse().unwrap_or(s.cleanup_days);
                    s.cleanup_days = if CLEAN_DAYS.contains(&d) { d } else { s.cleanup_days };
                }
                "key_open" => s.keys[0] = Bind::parse(v).unwrap_or(DEFAULT_KEYS[0]),
                "key_fav" => s.keys[1] = Bind::parse(v).unwrap_or(DEFAULT_KEYS[1]),
                "key_del" => s.keys[2] = Bind::parse(v).unwrap_or(DEFAULT_KEYS[2]),
                "key_tab" => s.keys[3] = Bind::parse(v).unwrap_or(DEFAULT_KEYS[3]),
                "anim_style" => s.anim_style = v.parse().unwrap_or(s.anim_style).min(2),
                "anim_ms" => s.anim_ms = v.parse().unwrap_or(s.anim_ms).clamp(80, 500),
                _ => {}
            }
        }
        s
    }

    pub fn format(&self) -> String {
        format!(
            "theme={}\nopacity={}\npack={}\nvolume={}\nmuted={}\nblur={}\nanim={}\nanim_style={}\nanim_ms={}\nwin_v={}\npos={}\nradius={}\npaste_on_select={}\npaste_sound={}\ncompact={}\nsingle_click={}\nlang={}\naccent={}\ncleanup={}\ncleanup_days={}\nkey_open={}\nkey_fav={}\nkey_del={}\nkey_tab={}\n",
            self.theme, self.opacity, self.pack, self.volume, self.muted as u8, self.blur as u8,
            self.anim as u8, self.anim_style, self.anim_ms, self.win_v as u8, self.pos, self.radius,
            self.paste_on_select as u8, self.paste_sound as u8, self.compact as u8, self.single_click as u8, self.lang,
            self.accent, self.cleanup as u8, self.cleanup_days,
            self.keys[0].ser(), self.keys[1].ser(), self.keys[2].ser(), self.keys[3].ser()
        )
    }

    pub fn load() -> Self {
        match fs::read_to_string(data_dir().join("settings.ini")) {
            Ok(txt) => Self::parse(&txt),
            Err(_) => Settings::default(),
        }
    }

    pub fn save(&self) {
        let _ = fs::create_dir_all(data_dir());
        let _ = fs::write(data_dir().join("settings.ini"), self.format());
    }
}

pub fn parse_favs(txt: &str) -> HashSet<u64> {
    txt.lines().filter_map(|l| u64::from_str_radix(l.trim(), 16).ok()).collect()
}

pub fn format_favs(f: &HashSet<u64>) -> String {
    f.iter().map(|h| format!("{h:016x}\n")).collect()
}

pub fn load_favs() -> HashSet<u64> {
    parse_favs(&fs::read_to_string(data_dir().join("favorites.txt")).unwrap_or_default())
}

pub fn save_favs(f: &HashSet<u64>) {
    let _ = fs::create_dir_all(data_dir());
    let _ = fs::write(data_dir().join("favorites.txt"), format_favs(f));
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
    /// Slider and switch knobs
    pub knob: u32,
    /// Light theme (a few fixed colors, like error text, need other shades on it)
    pub light: bool,
}

/// Accent colors to choose from (a subset of the Windows accent palette). Index 0 in the
/// settings means "the theme's own accent"; 1.. picks one of these.
pub const ACCENTS: [u32; 8] = [
    rgb(88, 101, 242),
    rgb(0, 153, 188),
    rgb(16, 137, 62),
    rgb(255, 185, 0),
    rgb(247, 99, 12),
    rgb(232, 17, 35),
    rgb(227, 0, 140),
    rgb(135, 100, 184),
];

pub const THEMES: [Theme; 4] = [
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
        knob: rgb(245, 245, 245),
        light: false,
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
        knob: rgb(245, 245, 245),
        light: false,
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
        knob: rgb(245, 245, 245),
        light: false,
    },
    Theme {
        name: "Light",
        bg: rgb(243, 243, 245),
        ctrl: rgb(255, 255, 255),
        prev: rgb(250, 250, 251),
        text: rgb(28, 28, 30),
        dim: rgb(104, 108, 118),
        accent: rgb(0, 103, 192),
        on_accent: rgb(255, 255, 255),
        sel: rgb(221, 229, 243),
        track: rgb(190, 194, 202),
        icon_img: rgb(0, 103, 192),
        knob: rgb(255, 255, 255),
        light: true,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_roundtrip_and_invalid() {
        let b = Bind { mods: M_CTRL | M_ALT, vk: 0x48 };
        assert_eq!(Bind::parse(&b.ser()), Some(b));
        assert_eq!(Bind::parse(" 5 , 72 "), Some(b));
        assert_eq!(Bind::parse(""), None);
        assert_eq!(Bind::parse("only_one"), None);
        assert_eq!(Bind::parse("abc,72"), None);
        assert_eq!(Bind::parse("1,99999"), None);
    }

    #[test]
    fn settings_roundtrip() {
        let orig = Settings {
            theme: 2,
            opacity: 65,
            pack: 4,
            volume: 42,
            muted: true,
            blur: false,
            anim: false,
            anim_style: 2,
            anim_ms: 320,
            win_v: true,
            pos: 3,
            radius: 14,
            paste_on_select: false,
            paste_sound: false,
            compact: true,
            single_click: true,
            lang: 1,
            accent: 5,
            cleanup: true,
            cleanup_days: 90,
            keys: [
                Bind { mods: M_CTRL | M_SHIFT, vk: 0x56 },
                Bind { mods: M_ALT, vk: 0x46 },
                Bind { mods: M_SHIFT, vk: 0x2E },
                Bind { mods: M_CTRL, vk: 0x09 },
            ],
        };
        let parsed = Settings::parse(&orig.format());
        assert_eq!(parsed.theme, orig.theme);
        assert_eq!(parsed.opacity, orig.opacity);
        assert_eq!(parsed.pack, orig.pack);
        assert_eq!(parsed.volume, orig.volume);
        assert_eq!(parsed.muted, orig.muted);
        assert_eq!(parsed.blur, orig.blur);
        assert_eq!(parsed.anim, orig.anim);
        assert_eq!(parsed.anim_style, orig.anim_style);
        assert_eq!(parsed.anim_ms, orig.anim_ms);
        assert_eq!(parsed.win_v, orig.win_v);
        assert_eq!(parsed.pos, orig.pos);
        assert_eq!(parsed.radius, orig.radius);
        assert_eq!(parsed.paste_on_select, orig.paste_on_select);
        assert_eq!(parsed.paste_sound, orig.paste_sound);
        assert_eq!(parsed.compact, orig.compact);
        assert_eq!(parsed.single_click, orig.single_click);
        assert_eq!(parsed.lang, orig.lang);
        assert_eq!(parsed.accent, orig.accent);
        assert_eq!(parsed.cleanup, orig.cleanup);
        assert_eq!(parsed.cleanup_days, orig.cleanup_days);
        assert_eq!(parsed.keys, orig.keys);
    }

    #[test]
    fn settings_invalid_and_out_of_range_values() {
        let def = Settings::default();
        let bad = "opacity=not_a_number\nradius=bad\ntheme=99\nvolume=250\nanim_ms=10\ncleanup_days=45\nkey_open=broken\n";
        let s = Settings::parse(bad);
        assert_eq!(s.opacity, def.opacity); // 77, not 100
        assert_eq!(s.radius, def.radius); // 0, not 12
        assert_eq!(s.theme, THEMES.len() - 1);
        assert_eq!(s.volume, 100);
        assert_eq!(s.anim_ms, 80);
        assert_eq!(s.cleanup_days, def.cleanup_days);
        assert_eq!(s.keys[0], DEFAULT_KEYS[0]);

        let clamped = Settings::parse("opacity=5\nradius=99\nanim_ms=9999\nlang=99\naccent=99\npos=99\n");
        assert_eq!(clamped.opacity, 40);
        assert_eq!(clamped.radius, 24);
        assert_eq!(clamped.anim_ms, 500);
        assert_eq!(clamped.lang, 4);
        assert_eq!(clamped.accent, ACCENTS.len());
        assert_eq!(clamped.pos, 3);
    }

    #[test]
    fn favs_roundtrip() {
        let orig: HashSet<u64> = [0x0123_4567_89ab_cdef, 0, u64::MAX].into_iter().collect();
        let txt = format!("{}not-hex\n\n", format_favs(&orig));
        assert_eq!(parse_favs(&txt), orig);
    }
}
