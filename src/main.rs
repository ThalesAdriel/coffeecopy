#![windows_subsystem = "windows"]

mod fuzzy;
mod history;
mod i18n;
mod settings;

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicUsize, Ordering::Relaxed};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::{DwmFlush, DwmSetWindowAttribute};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Storage::Xps::PrintWindow;
use windows_sys::Win32::Graphics::GdiPlus::{
    FillModeAlternate, GdipAddPathArc, GdipClosePathFigure, GdipCreateFromHDC, GdipCreatePath,
    GdipCreateSolidFill, GdipDeleteBrush, GdipDeleteGraphics, GdipDeletePath, GdipFillEllipse,
    GdipFillPath, GdipFillRectangle, GdipSetSmoothingMode, GdiplusStartup, GdiplusStartupInput, GpBrush, GpGraphics,
    GpPath, GpSolidFill, SmoothingModeAntiAlias,
};
use windows_sys::Win32::Media::Audio::*;
use windows_sys::Win32::System::DataExchange::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::*;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::System::Threading::{AttachThreadInput, CreateMutexW, GetCurrentThreadId};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use history::{fnv, text_hash, History, Kind};
use settings::{load_favs, save_favs, Bind, Settings, Theme, DEFAULT_KEYS, M_ALT, M_CTRL, M_SHIFT, M_WIN, THEMES};

// ───────────── Sound packs (all embedded) ─────────────
// Camera: "Camera" (CC0) + "Equipment Click" (CC0) — opengameart.org
// Kenney: Interface Sounds (CC0) — kenney.nl
// Paper: "Paper Crumple Sfx" by Nicole Marie T (CC BY 4.0)
// Corporate: "Completion sound" by Brandon Morris (CC0) + "Bell dings" by PWL (CC0)
/// Which sound to play:
/// - `New`: content never copied before;
/// - `Repeat`: the same as what is already on the clipboard (you copied the current item again);
/// - `Seen`: copied before, but it is not the current item.
#[derive(Clone, Copy, PartialEq)]
enum Snd {
    New,
    Repeat,
    Seen,
    /// A history item was chosen (pasted or copied back)
    Paste,
}
struct Pack {
    name: &'static str,
    s_new: &'static [u8],
    s_repeat: &'static [u8],
    s_seen: &'static [u8],
    s_paste: &'static [u8],
}
macro_rules! pack {
    ($name:expr, $dir:literal) => {
        Pack {
            name: $name,
            s_new: include_bytes!(concat!("../sounds/", $dir, "/new.wav")),
            s_repeat: include_bytes!(concat!("../sounds/", $dir, "/repeat.wav")),
            s_seen: include_bytes!(concat!("../sounds/", $dir, "/seen.wav")),
            s_paste: include_bytes!(concat!("../sounds/", $dir, "/paste.wav")),
        }
    };
}
static PACKS: [Pack; 6] = [
    pack!("Camera / cards", "camera"),
    pack!("Kenney", "kenney"),
    pack!("Paper / pages", "paper"),
    pack!("Corporate", "corporate"),
    pack!("8-bit / Arcade", "8bit"),
    pack!("Hi-Tech", "hitech"),
];

// Font Awesome Free 7 (Solid): OFL font, CC BY 4.0 icons (fontawesome.com)
static FA_FONT: &[u8] = include_bytes!("../assets/fa-solid-900.ttf");
const IC_SEARCH: &str = "\u{f002}";
const IC_TEXT: &str = "\u{f15c}";
const IC_IMAGE: &str = "\u{f03e}";
const IC_STAR: &str = "\u{f005}";
const IC_TRASH: &str = "\u{f2ed}";
const IC_HISTORY: &str = "\u{f1da}";
const IC_GEAR: &str = "\u{f013}";
const IC_PLAY: &str = "\u{f04b}";

const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;
const CF_DIB: u32 = 8;

// Global shortcut: remappable, see register_hotkey().
const WM_TRAY: u32 = WM_APP + 1;
const EM_SETSEL: u32 = 0x00B1;
const ID_OPEN: usize = 4;
const ID_SETTINGS: usize = 5;
const ID_MUTE: usize = 1;
const ID_AUTOSTART: usize = 2;
const ID_EXIT: usize = 3;
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const APP_NAME: &str = "CoffeeCopy";
const OLD_APP_NAME: &str = "CopySound";
/// App mark (tab bar and the tray fallback icon): the clipboard, not the mug,
/// so it does not look like a "keep awake" app.
const IC_MUG: &str = "\u{f328}";

// Layout (logical pixels). The sizes depend on compact mode.
const LIST_X: i32 = 12;
const LIST_Y: i32 = 100;
static COMPACT: AtomicBool = AtomicBool::new(false);
/// Demo mode (`--demo`): scripted tour over a generated wallpaper, on the primary monitor.
static DEMO: AtomicBool = AtomicBool::new(false);
fn compact() -> bool {
    COMPACT.load(Relaxed)
}
fn ui_w_fn() -> i32 {
    if compact() { 520 } else { 960 }
}
fn ui_h_fn() -> i32 {
    if compact() { 560 } else { 660 }
}
fn list_w_fn() -> i32 {
    if compact() { ui_w_fn() - 24 } else { 440 }
}
fn list_h_fn() -> i32 {
    ui_h_fn() - 100 - 44
}
fn row_h_fn() -> i32 {
    if compact() { 30 } else { 36 }
}
fn list_r_fn() -> i32 {
    LIST_X + list_w_fn()
}
fn star_x0_fn() -> i32 {
    list_r_fn() - 78
}
fn star_x1_fn() -> i32 {
    list_r_fn() - 44
}
fn trash_x1_fn() -> i32 {
    list_r_fn() - 10
}
fn sb_x0_fn() -> i32 {
    list_r_fn() - 16
}
fn snap_w_fn() -> i32 {
    ui_w_fn() / 6
}
fn snap_h_fn() -> i32 {
    ui_h_fn() / 6
}
const MAX_RESULTS: usize = 5000;
const SEARCH_CHARS: usize = 1000;
const ID_EDIT: usize = 101;
const WM_WINV: u32 = WM_APP + 2;
const TAB_ANIM_MS: f32 = 200.0;
const TAB_NAMES: [&str; 3] = ["History", "Favorites", "Settings"];

struct App {
    last: Option<(u64, Instant)>,
    /// Hash we have just put on the clipboard when pasting from the history.
    ignore: Option<(u64, Instant)>,
    /// Copied files (not stored in the history, but they still play a sound).
    seen_other: HashSet<u64>,
}

#[derive(Clone, Copy, PartialEq)]
enum Act {
    Tab(usize),
    Theme(usize),
    Pack(usize),
    TestNew,
    TestDup,
    TestOld,
    TestPaste,
    PasteSel,
    PasteSound,
    Autostart,
    Mute,
    Blur,
    WinV,
    WinHist,
    Sub(usize),
    Compact,
    ClickMode(usize),
    Lang(usize),
    Key(usize),
    KeysReset,
    Pos(usize),
    Anim,
    AnimStyle(usize),
    Clear,
    Opacity,
    Volume,
    Duration,
    Radius,
}

thread_local! {
    static APP: RefCell<App> = RefCell::new(App { last: None, ignore: None, seen_other: HashSet::new() });
    static HIST: RefCell<History> = RefCell::new(History::load());
    static SET: RefCell<Settings> = RefCell::new(Settings::load());
    static FAVS: RefCell<HashSet<u64>> = RefCell::new(load_favs());
    /// List row -> index in HIST.entries
    static VIEW: RefCell<Vec<usize>> = RefCell::new(Vec::new());
    /// Clickable areas drawn by the last paint
    static HITS: RefCell<Vec<(RECT, Act)>> = RefCell::new(Vec::new());
    /// Buffer of the sound being played (async PlaySound needs it alive)
    static PLAYBUF: RefCell<Vec<u8>> = RefCell::new(Vec::new());
    /// State of the list we draw ourselves (selection, smooth scrolling)
    static LST: RefCell<ListSt> = RefCell::new(ListSt::new());
    /// Bytes of the image shown in the preview panel (avoids reading the disk on every frame)
    static IMG_CACHE: RefCell<Option<(u64, Vec<u8>)>> = RefCell::new(None);
    /// (DC, bitmap) of the drawing buffer, reused between frames
    static MEMBUF: std::cell::Cell<(isize, isize)> = std::cell::Cell::new((0, 0));
}

/// The list we draw ourselves: positions in logical pixels, inside the list area.
struct ListSt {
    sel: isize,
    scroll: f32,
    target: f32,
    /// position (y, in content coordinates) of the animated highlight bar
    hl: f32,
    last_tick: Option<Instant>,
    animating: bool,
}
impl ListSt {
    fn new() -> Self {
        ListSt { sel: -1, scroll: 0.0, target: 0.0, hl: 0.0, last_tick: None, animating: false }
    }
}

// ── Frame clock: a thread wakes up on every screen refresh (DwmFlush ≈ 60 Hz)
// and sends a WM_FRAME to the window while any animation is active.
const WM_FRAME: u32 = WM_APP + 3;
static ANIM_ACTIVE: AtomicBool = AtomicBool::new(false);
static FRAME_PENDING: AtomicBool = AtomicBool::new(false);

fn kick_frames() {
    ANIM_ACTIVE.store(true, Relaxed);
}

fn frame_thread() {
    loop {
        if ANIM_ACTIVE.load(Relaxed) {
            // wait for the next vblank; if the DWM does not answer, fall back to ~60 Hz by time
            let ok = unsafe { DwmFlush() } == 0;
            if !ok {
                std::thread::sleep(Duration::from_millis(16));
            }
            // do not pile up messages if the main thread is late
            if !FRAME_PENDING.swap(true, Relaxed) {
                unsafe {
                    PostMessageW(h(&UI), WM_FRAME, 0, 0);
                }
            }
        } else {
            std::thread::sleep(Duration::from_millis(8));
        }
    }
}

static MAIN: AtomicIsize = AtomicIsize::new(0);
static UI: AtomicIsize = AtomicIsize::new(0);
static EDIT: AtomicIsize = AtomicIsize::new(0);
static HOOK: AtomicIsize = AtomicIsize::new(0);
static WINV_SWALLOW: AtomicBool = AtomicBool::new(false);
static TAB_FROM: AtomicUsize = AtomicUsize::new(0);
thread_local! {
    /// Start of the tab transition (None = stopped)
    static TAB_ANIM: RefCell<Option<Instant>> = RefCell::new(None);
}
static PREV: AtomicIsize = AtomicIsize::new(0);
static FONT: AtomicIsize = AtomicIsize::new(0);
static FONT_S: AtomicIsize = AtomicIsize::new(0);
static FONT_B: AtomicIsize = AtomicIsize::new(0);
static FONT_ICON: AtomicIsize = AtomicIsize::new(0);
static FONT_CTL: AtomicIsize = AtomicIsize::new(0);
static BR_WIN: AtomicIsize = AtomicIsize::new(0);
static BR_CTRL: AtomicIsize = AtomicIsize::new(0);
static ORIG_EDIT: AtomicIsize = AtomicIsize::new(0);
static TAB: AtomicUsize = AtomicUsize::new(0);
static CONFIRM_CLEAR: AtomicBool = AtomicBool::new(false);
static DRAG: AtomicIsize = AtomicIsize::new(-1); // 0=opacity 1=volume 2=duration 3=list scrollbar 4=corner radius

fn h(a: &AtomicIsize) -> HWND {
    a.load(Relaxed) as HWND
}
fn set(a: &AtomicIsize, p: *mut c_void) {
    a.store(p as isize, Relaxed);
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}
// ── Language ──
static LANG: AtomicUsize = AtomicUsize::new(0);
fn lang() -> usize {
    LANG.load(Relaxed)
}
/// Translates an English UI string into the current language (English is the fallback).
fn tr(key: &'static str) -> &'static str {
    i18n::tr(lang(), key)
}

fn theme() -> Theme {
    THEMES[SET.with(|s| s.borrow().theme).min(THEMES.len() - 1)]
}
fn settings() -> Settings {
    SET.with(|s| *s.borrow())
}
fn update_settings(f: impl FnOnce(&mut Settings)) {
    SET.with(|s| {
        let mut s = s.borrow_mut();
        f(&mut s);
        s.save();
    });
}
fn is_fav(hash: u64) -> bool {
    FAVS.with(|f| f.borrow().contains(&hash))
}

// ───────────────────────── Sound ─────────────────────────

/// Copies the 16-bit WAV applying the volume (quadratic curve).
fn scaled(src: &[u8], vol: u8) -> Vec<u8> {
    let mut b = src.to_vec();
    if vol >= 100 {
        return b;
    }
    let g = (vol as f32 / 100.0).powi(2);
    let mut p = 12;
    while p + 8 <= b.len() {
        let id = [b[p], b[p + 1], b[p + 2], b[p + 3]];
        let sz = u32::from_le_bytes([b[p + 4], b[p + 5], b[p + 6], b[p + 7]]) as usize;
        if &id == b"data" {
            let end = (p + 8 + sz).min(b.len());
            let mut i = p + 8;
            while i + 1 < end {
                let v = (i16::from_le_bytes([b[i], b[i + 1]]) as f32 * g) as i16;
                b[i..i + 2].copy_from_slice(&v.to_le_bytes());
                i += 2;
            }
            break;
        }
        p += 8 + sz + (sz & 1);
    }
    b
}

fn play_sound(kind: Snd) {
    let s = settings();
    let pack = &PACKS[s.pack.min(PACKS.len() - 1)];
    let src = match kind {
        Snd::New => pack.s_new,
        Snd::Repeat => pack.s_repeat,
        Snd::Seen => pack.s_seen,
        Snd::Paste => pack.s_paste,
    };
    let buf = scaled(src, s.volume);
    unsafe {
        PlaySoundW(null(), null_mut(), 0); // stop the previous sound before swapping the buffer
        PLAYBUF.with(|p| {
            *p.borrow_mut() = buf;
            let ptr = p.borrow().as_ptr();
            PlaySoundW(ptr as _, null_mut(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
        });
    }
}

// ───────────────────────── Clipboard ─────────────────────────

enum Clip {
    Text(String),
    Image(Vec<u8>),
    Other(u64),
}

unsafe fn grab(fmt: u32) -> Option<Vec<u8>> {
    let hd = GetClipboardData(fmt);
    if hd.is_null() {
        return None;
    }
    let size = GlobalSize(hd as _);
    let ptr = GlobalLock(hd as _) as *const u8;
    if ptr.is_null() {
        return None;
    }
    let v = std::slice::from_raw_parts(ptr, size).to_vec();
    GlobalUnlock(hd as _);
    Some(v)
}

unsafe fn read_clipboard(hwnd: HWND) -> Option<Clip> {
    let mut opened = false;
    for _ in 0..10 {
        if OpenClipboard(hwnd) != 0 {
            opened = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if !opened {
        return None;
    }
    let mut out = None;
    if IsClipboardFormatAvailable(CF_UNICODETEXT) != 0 {
        if let Some(b) = grab(CF_UNICODETEXT) {
            let units: Vec<u16> = b
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|&u| u != 0)
                .collect();
            if !units.is_empty() {
                out = Some(Clip::Text(String::from_utf16_lossy(&units)));
            }
        }
    }
    if out.is_none() && IsClipboardFormatAvailable(CF_DIB) != 0 {
        if let Some(b) = grab(CF_DIB) {
            if b.len() >= 40 {
                out = Some(Clip::Image(b));
            }
        }
    }
    if out.is_none() && IsClipboardFormatAvailable(CF_HDROP) != 0 {
        if let Some(b) = grab(CF_HDROP) {
            out = Some(Clip::Other(fnv(CF_HDROP, &b)));
        }
    }
    CloseClipboard();
    out
}

fn dib_size(b: &[u8]) -> (i32, i32) {
    let w = i32::from_le_bytes(b[4..8].try_into().unwrap());
    let hh = i32::from_le_bytes(b[8..12].try_into().unwrap());
    (w, hh.abs())
}

unsafe fn on_clipboard(hwnd: HWND) {
    let Some(clip) = read_clipboard(hwnd) else { return };
    let (hash, kind, text, image): (u64, Kind, String, Option<Vec<u8>>) = match clip {
        Clip::Text(s) => (text_hash(&s), Kind::Text, s, None),
        Clip::Image(b) => {
            let (w, hh) = dib_size(&b);
            (fnv(CF_DIB, &b), Kind::Image, format!("[Image {w}x{hh}]"), Some(b))
        }
        Clip::Other(hh) => (hh, Kind::Text, String::new(), None),
    };
    let now = Instant::now();

    // (ignore this event?, hash of the item that was on the clipboard until now)
    let (skip, current) = APP.with(|a| {
        let mut a = a.borrow_mut();
        if let Some((ih, t)) = a.ignore {
            if ih == hash && now.duration_since(t) < Duration::from_millis(1500) {
                return (true, None);
            }
        }
        // Some apps fire the event twice per copy: ignore the immediate echo.
        if let Some((lh, t)) = a.last {
            if lh == hash && now.duration_since(t) < Duration::from_millis(250) {
                return (true, None);
            }
        }
        let current = a.last.map(|(lh, _)| lh);
        a.last = Some((hash, now));
        (false, current)
    });
    if skip {
        return;
    }

    let known = if text.is_empty() && image.is_none() {
        APP.with(|a| !a.borrow_mut().seen_other.insert(hash))
    } else {
        HIST.with(|hs| hs.borrow_mut().add(hash, kind, text, image.as_deref()))
    };
    let snd = if !known {
        Snd::New
    } else if current == Some(hash) {
        Snd::Repeat
    } else {
        Snd::Seen
    };
    if !settings().muted {
        play_sound(snd);
    }
    if IsWindowVisible(h(&UI)) != 0 && TAB.load(Relaxed) != 2 {
        refilter();
    }
}

unsafe fn set_clipboard(fmt: u32, bytes: &[u8]) -> bool {
    let hg = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
    if hg.is_null() {
        return false;
    }
    let p = GlobalLock(hg) as *mut u8;
    if p.is_null() {
        GlobalFree(hg);
        return false;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
    GlobalUnlock(hg);
    let mut ok = false;
    for _ in 0..10 {
        if OpenClipboard(h(&MAIN)) != 0 {
            EmptyClipboard();
            ok = !SetClipboardData(fmt, hg as _).is_null();
            CloseClipboard();
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if !ok {
        GlobalFree(hg); // the clipboard only owns the memory when SetClipboardData succeeds
    }
    ok
}

// ───────────────────────── List, search and actions ─────────────────────────

fn display_line(e: &history::Entry) -> String {
    if e.kind == Kind::Image {
        // stored as "[Image WxH]" (older entries: "[Imagem WxH]"); shown in the current language
        let size = e.text.trim_matches(|c| c == '[' || c == ']').split_once(' ').map(|x| x.1).unwrap_or("");
        return tr("[Image {s}]").replace("{s}", size);
    }
    e.text
        .trim()
        .chars()
        .take(160)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

unsafe fn edit_text() -> String {
    let e = h(&EDIT);
    let n = GetWindowTextLengthW(e);
    if n <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; n as usize + 1];
    let got = GetWindowTextW(e, buf.as_mut_ptr(), n + 1);
    String::from_utf16_lossy(&buf[..got.max(0) as usize])
}

unsafe fn refilter() {
    let only_favs = TAB.load(Relaxed) == 1;
    let favs = FAVS.with(|f| f.borrow().clone());
    let terms: Vec<Vec<char>> = edit_text().split_whitespace().map(fuzzy::lower).collect();

    let view = HIST.with(|hs| {
        let hs = hs.borrow();
        let mut scored: Vec<(i32, usize)> = Vec::new();
        for i in (0..hs.entries.len()).rev() {
            let e = &hs.entries[i];
            if only_favs && !favs.contains(&e.hash) {
                continue;
            }
            if terms.is_empty() {
                scored.push((0, i));
                if scored.len() >= MAX_RESULTS {
                    break;
                }
                continue;
            }
            let hay = fuzzy::lower(&e.text.chars().take(SEARCH_CHARS).collect::<String>());
            let mut total = 0;
            let mut ok = true;
            for t in &terms {
                match fuzzy::score(&hay, t) {
                    Some(s) => total += s,
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                scored.push((total, i));
            }
        }
        // stable: ties stay in recency order (newest first)
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.truncate(MAX_RESULTS);
        scored.iter().map(|x| x.1).collect::<Vec<usize>>()
    });

    let n = view.len();
    VIEW.with(|v| *v.borrow_mut() = view);
    LST.with(|l| {
        let mut l = l.borrow_mut();
        l.sel = if n > 0 { 0 } else { -1 };
        l.scroll = 0.0;
        l.target = 0.0;
        l.hl = 0.0;
    });
    invalidate_list();
    invalidate_preview();
}

/// Refilters while keeping (roughly) the selected row.
unsafe fn refilter_keep() {
    let (cur, scroll) = LST.with(|l| {
        let l = l.borrow();
        (l.sel, l.scroll)
    });
    refilter();
    let n = row_count() as isize;
    if n > 0 && cur >= 0 {
        LST.with(|l| {
            let mut l = l.borrow_mut();
            l.sel = cur.min(n - 1);
            l.scroll = scroll;
            l.target = scroll;
            l.hl = l.sel as f32 * row_h_fn() as f32;
        });
        let ms = max_scroll();
        LST.with(|l| {
            let mut l = l.borrow_mut();
            l.scroll = l.scroll.min(ms);
            l.target = l.target.min(ms);
        });
        ensure_visible();
        invalidate_list();
        invalidate_preview();
    }
}

// ───────────── Own list: smooth scrolling, animated highlight ─────────────

fn row_count() -> usize {
    VIEW.with(|v| v.borrow().len())
}

fn max_scroll() -> f32 {
    (row_count() as f32 * row_h_fn() as f32 - list_h_fn() as f32).max(0.0)
}

unsafe fn invalidate_list() {
    let r = RECT { left: sc(LIST_X), top: sc(LIST_Y), right: sc(LIST_X + list_w_fn()), bottom: sc(LIST_Y + list_h_fn()) };
    InvalidateRect(h(&UI), &r, 0);
}

/// Sets the scroll target (with the least movement) so the selected row stays visible.
unsafe fn ensure_visible() {
    let ms = max_scroll();
    LST.with(|l| {
        let mut l = l.borrow_mut();
        if l.sel < 0 {
            return;
        }
        let top = l.sel as f32 * row_h_fn() as f32;
        let bottom = top + row_h_fn() as f32;
        if top < l.target {
            l.target = top;
        } else if bottom > l.target + list_h_fn() as f32 {
            l.target = bottom - list_h_fn() as f32;
        }
        l.target = l.target.clamp(0.0, ms);
    });
    start_list_anim();
}

/// Turns the list animation on: the frame clock (synced with the screen) calls
/// `list_frame` until scroll and highlight reach their target.
unsafe fn start_list_anim() {
    LST.with(|l| {
        let mut l = l.borrow_mut();
        l.animating = true;
        l.last_tick = Some(Instant::now());
    });
    kick_frames();
}

/// One list frame: moves scroll and highlight toward the target (light exponential smoothing).
/// Returns true while there is still movement.
unsafe fn list_frame() -> bool {
    if !LST.with(|l| l.borrow().animating) {
        return false;
    }
    let now = Instant::now();
    let done = LST.with(|l| {
        let mut l = l.borrow_mut();
        let dt = l.last_tick.map(|t| now.duration_since(t).as_secs_f32()).unwrap_or(0.016).min(0.05);
        l.last_tick = Some(now);
        let fs = 1.0 - (-dt * 26.0).exp(); // scroll: time constant ~40 ms
        let fh = 1.0 - (-dt * 34.0).exp(); // highlight: a bit snappier
        l.scroll += (l.target - l.scroll) * fs;
        let hl_target = l.sel.max(0) as f32 * row_h_fn() as f32;
        l.hl += (hl_target - l.hl) * fh;
        let a = (l.target - l.scroll).abs() < 0.3;
        let b = (hl_target - l.hl).abs() < 0.3;
        if a {
            l.scroll = l.target;
        }
        if b {
            l.hl = hl_target;
        }
        if a && b {
            l.animating = false;
        }
        a && b
    });
    invalidate_list();
    !done
}

unsafe fn set_sel(i: isize) {
    let n = row_count() as isize;
    if n == 0 {
        return;
    }
    LST.with(|l| l.borrow_mut().sel = i.clamp(0, n - 1));
    ensure_visible();
    invalidate_preview();
}

/// Mouse wheel: only scrolls the list (3 rows per "notch"), without changing the selection.
/// The target moves right away; the scroll follows it smoothly.
unsafe fn wheel(delta: i32) {
    if TAB.load(Relaxed) == 2 {
        return;
    }
    let ms = max_scroll();
    LST.with(|l| {
        let mut l = l.borrow_mut();
        let px = delta as f32 / 120.0 * 3.0 * row_h_fn() as f32;
        l.target = (l.target - px).clamp(0.0, ms);
    });
    start_list_anim();
}

fn preview_rect() -> RECT {
    if compact() {
        // no preview panel in compact mode
        return RECT { left: 0, top: 0, right: 0, bottom: 0 };
    }
    RECT { left: 464, top: LIST_Y, right: ui_w_fn() - 12, bottom: ui_h_fn() - 44 }
}

unsafe fn invalidate_preview() {
    let l = preview_rect();
    let r = RECT { left: sc(l.left), top: sc(l.top), right: sc(l.right), bottom: sc(l.bottom) };
    InvalidateRect(h(&UI), &r, 0);
}

unsafe fn entry_at(row: isize) -> Option<history::Entry> {
    if row < 0 {
        return None;
    }
    let idx = VIEW.with(|v| v.borrow().get(row as usize).copied())?;
    HIST.with(|hs| hs.borrow().entries.get(idx).cloned())
}

unsafe fn selected_entry() -> Option<history::Entry> {
    entry_at(LST.with(|l| l.borrow().sel))
}

unsafe fn move_sel(delta: isize) {
    let cur = LST.with(|l| l.borrow().sel);
    set_sel(cur + delta);
}

unsafe fn toggle_fav(hash: u64) {
    FAVS.with(|f| {
        let mut f = f.borrow_mut();
        if !f.remove(&hash) {
            f.insert(hash);
        }
        save_favs(&f);
    });
    if TAB.load(Relaxed) == 1 {
        refilter_keep();
    } else {
        invalidate_list();
    }
}

unsafe fn delete_entry(hash: u64) {
    HIST.with(|hs| hs.borrow_mut().remove(hash));
    FAVS.with(|f| {
        let mut f = f.borrow_mut();
        if f.remove(&hash) {
            save_favs(&f);
        }
    });
    refilter_keep();
}

// ───────────────────────── Remappable key bindings ─────────────────────────

/// Actions that can be remapped (index = position in `Settings::keys`).
const KEY_ACTIONS: [&str; 4] = ["Open CoffeeCopy (global)", "Favorite item", "Delete item", "Next tab"];
/// Index of the binding being recorded (-1 = none).
static RECORDING: AtomicIsize = AtomicIsize::new(-1);
thread_local! {
    /// Last message shown on the Keybinds page (conflicts, errors).
    static KEY_MSG: RefCell<Option<String>> = RefCell::new(None);
}

fn set_key_msg(m: Option<String>) {
    KEY_MSG.with(|k| *k.borrow_mut() = m);
}

/// Modifier keys currently held, as `M_*` bits.
fn current_mods() -> u8 {
    unsafe {
        let d = |vk: u16| GetKeyState(vk as i32) < 0;
        (if d(VK_CONTROL) { M_CTRL } else { 0 })
            | (if d(VK_SHIFT) { M_SHIFT } else { 0 })
            | (if d(VK_MENU) { M_ALT } else { 0 })
            | (if d(VK_LWIN) || d(VK_RWIN) { M_WIN } else { 0 })
    }
}

fn key_label(vk: u16) -> String {
    match vk {
        0x08 => "Backspace".into(),
        0x09 => "Tab".into(),
        0x0D => "Enter".into(),
        0x1B => "Esc".into(),
        0x20 => "Space".into(),
        0x21 => "PgUp".into(),
        0x22 => "PgDn".into(),
        0x23 => "End".into(),
        0x24 => "Home".into(),
        0x25 => "Left".into(),
        0x26 => "Up".into(),
        0x27 => "Right".into(),
        0x28 => "Down".into(),
        0x2D => "Insert".into(),
        0x2E => "Delete".into(),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        0x30..=0x39 | 0x41..=0x5A => (vk as u8 as char).to_string(),
        _ => unsafe {
            let c = MapVirtualKeyW(vk as u32, MAPVK_VK_TO_CHAR) & 0x7FFF_FFFF;
            match char::from_u32(c) {
                Some(ch) if c != 0 => ch.to_uppercase().to_string(),
                _ => format!("0x{vk:X}"),
            }
        },
    }
}

/// Human-readable name, e.g. "Ctrl+Shift+V".
fn bind_name(b: Bind) -> String {
    let mut s = String::new();
    if b.mods & M_CTRL != 0 {
        s += "Ctrl+";
    }
    if b.mods & M_SHIFT != 0 {
        s += "Shift+";
    }
    if b.mods & M_ALT != 0 {
        s += "Alt+";
    }
    if b.mods & M_WIN != 0 {
        s += "Win+";
    }
    s + &key_label(b.vk)
}

/// Removes the WM_CHAR / WM_SYSCHAR that TranslateMessage queued for a key we already handled
/// (otherwise Tab, Enter or Ctrl+letter would also insert a control character in the search box).
unsafe fn eat_char(hwnd: HWND) {
    let mut m: MSG = std::mem::zeroed();
    while PeekMessageW(&mut m, hwnd, WM_CHAR, WM_CHAR, PM_REMOVE) != 0 {}
    while PeekMessageW(&mut m, hwnd, WM_SYSCHAR, WM_SYSCHAR, PM_REMOVE) != 0 {}
}

/// +1 for the "next tab" binding, -1 for the same key with Shift toggled (previous tab).
fn tab_step(vk: u16, mods: u8) -> Option<i32> {
    let b = settings().keys[3];
    if vk != b.vk {
        return None;
    }
    if mods == b.mods {
        Some(1)
    } else if mods == b.mods ^ M_SHIFT {
        Some(-1)
    } else {
        None
    }
}

/// Handles a key press in the window. Returns true if it was consumed.
unsafe fn handle_key(vk: u16, mods: u8) -> bool {
    if RECORDING.load(Relaxed) >= 0 {
        record_key(vk, mods);
        return true;
    }
    if vk == VK_ESCAPE && mods == 0 {
        hide_ui();
        return true;
    }
    if let Some(d) = tab_step(vk, mods) {
        let c = TAB.load(Relaxed) as i32;
        switch_tab(((c + d + 3) % 3) as usize);
        return true;
    }
    if TAB.load(Relaxed) == 2 {
        return false;
    }
    if mods == 0 {
        let step = match vk {
            VK_DOWN => Some(1),
            VK_UP => Some(-1),
            VK_NEXT => Some(12),
            VK_PRIOR => Some(-12),
            _ => None,
        };
        if let Some(s) = step {
            move_sel(s);
            return true;
        }
        if vk == VK_RETURN {
            choose();
            return true;
        }
    }
    let k = settings().keys;
    if vk == k[1].vk && mods == k[1].mods {
        if let Some(e) = selected_entry() {
            toggle_fav(e.hash);
        }
        return true;
    }
    if vk == k[2].vk && mods == k[2].mods {
        if let Some(e) = selected_entry() {
            delete_entry(e.hash);
        }
        return true;
    }
    false
}

/// Registers the global "open" shortcut (replacing the previous one). False if the system refused.
unsafe fn register_hotkey() -> bool {
    let hwnd = h(&MAIN);
    UnregisterHotKey(hwnd, 1);
    let b = settings().keys[0];
    let mut m = MOD_NOREPEAT;
    if b.mods & M_CTRL != 0 {
        m |= MOD_CONTROL;
    }
    if b.mods & M_SHIFT != 0 {
        m |= MOD_SHIFT;
    }
    if b.mods & M_ALT != 0 {
        m |= MOD_ALT;
    }
    if b.mods & M_WIN != 0 {
        m |= MOD_WIN;
    }
    RegisterHotKey(hwnd, 1, m, b.vk as u32) != 0
}

/// A key press while recording a new binding.
unsafe fn record_key(vk: u16, mods: u8) {
    let i = RECORDING.load(Relaxed);
    if i < 0 {
        return;
    }
    let i = i as usize;
    // a lone modifier: keep waiting for the real key
    if matches!(vk, VK_CONTROL | VK_SHIFT | VK_MENU | VK_LWIN | VK_RWIN | 0xA0..=0xA5) {
        return;
    }
    set_key_msg(None);
    if vk == VK_ESCAPE && mods == 0 {
        RECORDING.store(-1, Relaxed);
        InvalidateRect(h(&UI), null(), 0);
        return;
    }
    let nb = Bind { mods, vk };
    let plain_ok = vk == VK_TAB || (0x70..=0x87).contains(&vk);
    let reserved = (mods == 0
        && matches!(vk, VK_RETURN | VK_UP | VK_DOWN | VK_LEFT | VK_RIGHT | VK_NEXT | VK_PRIOR | VK_HOME | VK_END))
        || (mods == M_CTRL && matches!(vk, 0x41 | 0x43 | 0x56 | 0x58 | 0x5A));
    let keys = settings().keys;
    let error: Option<String> = if reserved {
        Some(tr("Enter, Esc, the arrow keys and Ctrl+A are fixed.").to_string())
    } else if (mods == 0 && !plain_ok) || (i == 0 && mods == 0) {
        Some(tr("Use a modifier key (Ctrl, Shift, Alt or Win) with this key.").to_string())
    } else if let Some(j) = (0..4).find(|&j| j != i && keys[j] == nb) {
        Some(tr("Already used by “{a}”").replace("{a}", tr(KEY_ACTIONS[j])))
    } else {
        None
    };
    RECORDING.store(-1, Relaxed);
    if let Some(e) = error {
        set_key_msg(Some(e));
    } else {
        update_settings(|s| s.keys[i] = nb);
        if i == 0 && !register_hotkey() {
            update_settings(|s| s.keys[0] = keys[0]);
            register_hotkey();
            set_key_msg(Some(tr("Could not register this shortcut: another app is using it.").to_string()));
        }
        tray(h(&MAIN), NIM_MODIFY); // tooltip shows the open shortcut
    }
    InvalidateRect(h(&UI), null(), 0);
}

unsafe fn switch_tab(t: usize) {
    let prev = TAB.load(Relaxed);
    TAB.store(t, Relaxed);
    CONFIRM_CLEAR.store(false, Relaxed);
    let ui = h(&UI);
    let cfg = t == 2;
    ShowWindow(h(&EDIT), if cfg { SW_HIDE } else { SW_SHOW });
    if cfg {
        SetFocus(ui);
    } else {
        refilter();
        SetFocus(h(&EDIT));
    }
    // smooth transition: the tab indicator slides and the content fades in
    if prev != t && IsWindowVisible(ui) != 0 && settings().anim {
        TAB_FROM.store(prev, Relaxed);
        TAB_ANIM.with(|a| *a.borrow_mut() = Some(Instant::now()));
        kick_frames();
    } else {
        TAB_ANIM.with(|a| *a.borrow_mut() = None);
    }
    InvalidateRect(ui, null(), 0);
}

/// Progress (0..1, eased) of the tab transition; 1 = stopped.
fn tab_progress() -> f32 {
    TAB_ANIM.with(|a| match *a.borrow() {
        Some(t0) => ease_out((t0.elapsed().as_secs_f32() * 1000.0 / TAB_ANIM_MS).min(1.0)),
        None => 1.0,
    })
}

/// One frame of the tab transition. Returns true while it is still animating.
unsafe fn tab_frame() -> bool {
    let running = TAB_ANIM.with(|a| {
        let fin = matches!(*a.borrow(), Some(t0) if t0.elapsed().as_secs_f32() * 1000.0 >= TAB_ANIM_MS);
        if fin {
            *a.borrow_mut() = None;
        }
        a.borrow().is_some()
    });
    InvalidateRect(h(&UI), null(), 0);
    running
}

/// Brings the window to the front even when focus is in another app (hook / global shortcuts).
unsafe fn force_foreground(hwnd: HWND) {
    let fg = GetForegroundWindow();
    let ft = if fg.is_null() { 0 } else { GetWindowThreadProcessId(fg, null_mut()) };
    let me = GetCurrentThreadId();
    if ft != 0 && ft != me {
        AttachThreadInput(me, ft, 1);
        SetForegroundWindow(hwnd);
        AttachThreadInput(me, ft, 0);
    } else {
        SetForegroundWindow(hwnd);
    }
}

/// Works out where the window opens (on the monitor under the cursor), per the setting.
unsafe fn compute_pos() {
    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    let mon = if DEMO.load(Relaxed) {
        MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) // the demo always uses the primary monitor
    } else {
        MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST)
    };
    let mut mi: MONITORINFO = std::mem::zeroed();
    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(mon, &mut mi) == 0 {
        return;
    }
    let wa = mi.rcWork;
    let (w, hh) = (pw(), ph());
    let (cx, cy) = (wa.left + (wa.right - wa.left - w) / 2, wa.top + (wa.bottom - wa.top - hh) / 2);
    let (x, y) = match settings().pos {
        1 => (pt.x - w / 2, pt.y + sc(14)),                    // near the cursor
        2 => (cx, wa.top + sc(48)),                             // top, centered
        3 => (wa.right - w - sc(16), wa.bottom - hh - sc(16)),  // bottom-right corner
        _ => (cx, cy),                                          // center
    };
    let x = x.clamp(wa.left, (wa.right - w).max(wa.left));
    let y = y.clamp(wa.top, (wa.bottom - hh).max(wa.top));
    UI_X.store(x as isize, Relaxed);
    UI_Y.store(y as isize, Relaxed);
}

fn exiting() -> bool {
    ANIM.with(|a| matches!(*a.borrow(), Some((_, _, true))))
}

/// Opens the window on tab `tab`. With `toggle`, closes it if it is already open.
unsafe fn show_ui_tab(tab: usize, toggle: bool) {
    let ui = h(&UI);
    let visible = IsWindowVisible(ui) != 0;
    if visible && !exiting() {
        if toggle {
            hide_ui();
        } else {
            switch_tab(tab);
            force_foreground(ui);
        }
        return;
    }
    if !visible {
        set(&PREV, GetForegroundWindow());
        compute_pos();
        place_window(1.0);
        if settings().blur {
            capture_snapshot(); // window still hidden: captures only what is behind it
        }
        SetWindowTextW(h(&EDIT), wide("").as_ptr());
    }
    switch_tab(tab);
    let anim = settings().anim;
    if !visible {
        if anim {
            place_window(0.0);
        }
        ShowWindow(ui, SW_SHOW);
    }
    force_foreground(ui);
    SetFocus(if tab == 2 { ui } else { h(&EDIT) });
    if anim {
        start_anim(false);
    } else {
        ANIM.with(|a| *a.borrow_mut() = None);
        place_window(1.0);
    }
}

unsafe fn show_ui() {
    show_ui_tab(0, true);
}

/// Closes with an animation (if enabled).
unsafe fn hide_ui() {
    if IsWindowVisible(h(&UI)) == 0 || exiting() {
        return;
    }
    if settings().anim {
        start_anim(true);
    } else {
        hide_now();
    }
}

unsafe fn send_ctrl_v() {
    let key = |vk: u16, flags: u32| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    };
    let inputs = [
        key(VK_CONTROL, 0),
        key(VK_V, 0),
        key(VK_V, KEYEVENTF_KEYUP),
        key(VK_CONTROL, KEYEVENTF_KEYUP),
    ];
    SendInput(4, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
}

/// Enter: puts the item on the clipboard, closes the window and pastes into the previous app.
unsafe fn choose() {
    if TAB.load(Relaxed) == 2 {
        return;
    }
    let Some(e) = selected_entry() else { return };
    let ok = match e.kind {
        Kind::Text => {
            let units: Vec<u8> = e.text.encode_utf16().chain(std::iter::once(0)).flat_map(|u| u.to_le_bytes()).collect();
            set_clipboard(CF_UNICODETEXT, &units)
        }
        Kind::Image => match std::fs::read(history::image_path(e.hash)) {
            Ok(b) => set_clipboard(CF_DIB, &b),
            Err(_) => false,
        },
    };
    if !ok {
        return;
    }
    APP.with(|a| {
        let mut a = a.borrow_mut();
        a.ignore = Some((e.hash, Instant::now()));
        a.last = Some((e.hash, Instant::now())); // this is now the current clipboard item
    });
    HIST.with(|hs| hs.borrow_mut().touch(e.hash));
    hide_now(); // no animation: focus must go back to the previous app right away
    let s = settings();
    if s.paste_sound && !s.muted {
        play_sound(Snd::Paste);
    }
    let prev = h(&PREV);
    if !prev.is_null() {
        SetForegroundWindow(prev);
    }
    if s.paste_on_select {
        std::thread::sleep(Duration::from_millis(120));
        send_ctrl_v();
    }
    // with "paste on select" off, the item only goes to the clipboard (you paste with Ctrl+V)
}

// ───────────────────────── Theme and drawing ─────────────────────────

unsafe fn apply_theme() {
    let t = theme();
    let old = [h(&BR_WIN), h(&BR_CTRL)];
    set(&BR_WIN, CreateSolidBrush(t.bg));
    set(&BR_CTRL, CreateSolidBrush(t.ctrl));
    for b in old {
        if !b.is_null() {
            DeleteObject(b as _);
        }
    }
    if !h(&UI).is_null() {
        apply_opacity(); // the blur tint follows the theme color
    }
    InvalidateRect(h(&UI), null(), 1);
    InvalidateRect(h(&EDIT), null(), 1);
}

// ───────────── Transparency, own blur and animations ─────────────
//
// Without blur: the whole window is translucent (layered alpha) and shows the desktop through.
// With blur: on open, the app captures what is behind the window, blurs it and uses it as the
// background, mixed with the theme color. Opacity controls the mix (100% = solid background).
// This way blur and transparency work together, with no undocumented APIs.


thread_local! {
    /// Already-blurred capture (snap_w_fn() x snap_h_fn(), 0x00RRGGBB). Empty = not captured yet.
    static SNAP: RefCell<Vec<u32>> = RefCell::new(Vec::new());
    /// (start, duration ms, closing?)
    static ANIM: RefCell<Option<(Instant, u32, bool)>> = RefCell::new(None);
}
static BACK: AtomicIsize = AtomicIsize::new(0);
static UI_X: AtomicIsize = AtomicIsize::new(0);
static UI_Y: AtomicIsize = AtomicIsize::new(0);
const SLIDE_PX: f32 = 28.0;

fn blur_pass(src: &[u32], w: usize, hh: usize, r: i32, horizontal: bool) -> Vec<u32> {
    let mut out = vec![0u32; w * hh];
    for y in 0..hh {
        for x in 0..w {
            let (mut sr, mut sg, mut sb, mut n) = (0u32, 0u32, 0u32, 0u32);
            for d in -r..=r {
                let (xx, yy) = if horizontal {
                    ((x as i32 + d).clamp(0, w as i32 - 1) as usize, y)
                } else {
                    (x, (y as i32 + d).clamp(0, hh as i32 - 1) as usize)
                };
                let p = src[yy * w + xx];
                sb += p & 0xFF;
                sg += (p >> 8) & 0xFF;
                sr += (p >> 16) & 0xFF;
                n += 1;
            }
            out[y * w + x] = ((sr / n) << 16) | ((sg / n) << 8) | (sb / n);
        }
    }
    out
}

/// Captures the screen area where the window sits and keeps a heavily blurred copy.
unsafe fn capture_snapshot() {
    if DEMO.load(Relaxed) {
        set_snapshot(demo_backdrop_source()); // never the real screen
        return;
    }
    let ui = h(&UI);
    let mut wr: RECT = std::mem::zeroed();
    GetWindowRect(ui, &mut wr);
    let screen = GetDC(null_mut());
    let mem = CreateCompatibleDC(screen);
    let mut bi: BITMAPINFO = std::mem::zeroed();
    bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bi.bmiHeader.biWidth = snap_w_fn();
    bi.bmiHeader.biHeight = -snap_h_fn();
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    let mut bits: *mut c_void = null_mut();
    let dib = CreateDIBSection(screen, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    let old = SelectObject(mem, dib as _);
    SetStretchBltMode(mem, HALFTONE as _);
    StretchBlt(mem, 0, 0, snap_w_fn(), snap_h_fn(), screen, wr.left, wr.top, pw(), ph(), SRCCOPY | CAPTUREBLT);
    GdiFlush();
    let px: Vec<u32> = std::slice::from_raw_parts(bits as *const u32, (snap_w_fn() * snap_h_fn()) as usize)
        .iter()
        .map(|p| p & 0x00FF_FFFF)
        .collect();
    SelectObject(mem, old);
    DeleteObject(dib as _);
    DeleteDC(mem);
    ReleaseDC(null_mut(), screen);
    set_snapshot(px);
}

/// Blurs the captured pixels, keeps them and rebuilds the window background.
unsafe fn set_snapshot(mut px: Vec<u32>) {
    for _ in 0..3 {
        px = blur_pass(&px, snap_w_fn() as usize, snap_h_fn() as usize, 3, true);
        px = blur_pass(&px, snap_w_fn() as usize, snap_h_fn() as usize, 3, false);
    }
    SNAP.with(|s| *s.borrow_mut() = px);
    rebuild_backdrop();
}

/// Builds the background bitmap: blurred capture (interpolated) mixed with the theme color.
unsafe fn rebuild_backdrop() {
    let snap = SNAP.with(|s| s.borrow().clone());
    if snap.len() != (snap_w_fn() * snap_h_fn()) as usize {
        return;
    }
    let s = settings();
    let t = theme();
    let a = s.opacity as f32 / 100.0;
    let (tr, tg, tb) = ((t.bg & 0xFF) as f32, ((t.bg >> 8) & 0xFF) as f32, ((t.bg >> 16) & 0xFF) as f32);

    let screen = GetDC(null_mut());
    let mut bi: BITMAPINFO = std::mem::zeroed();
    bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    let (fw, fh) = (pw(), ph());
    bi.bmiHeader.biWidth = fw;
    bi.bmiHeader.biHeight = -fh;
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    let mut bits: *mut c_void = null_mut();
    let dib = CreateDIBSection(screen, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    ReleaseDC(null_mut(), screen);
    if dib.is_null() {
        return;
    }
    let out = std::slice::from_raw_parts_mut(bits as *mut u32, (fw * fh) as usize);
    let sx = (snap_w_fn() - 1) as f32 / (fw - 1) as f32;
    let sy = (snap_h_fn() - 1) as f32 / (fh - 1) as f32;
    for y in 0..fh {
        let fy = y as f32 * sy;
        let y0 = fy as usize;
        let y1 = (y0 + 1).min(snap_h_fn() as usize - 1);
        let wy = fy - y0 as f32;
        for x in 0..fw {
            let fx = x as f32 * sx;
            let x0 = fx as usize;
            let x1 = (x0 + 1).min(snap_w_fn() as usize - 1);
            let wx = fx - x0 as f32;
            let p = |xx: usize, yy: usize| snap[yy * snap_w_fn() as usize + xx];
            let (p00, p10, p01, p11) = (p(x0, y0), p(x1, y0), p(x0, y1), p(x1, y1));
            let ch = |shift: u32| {
                let g = |v: u32| ((v >> shift) & 0xFF) as f32;
                let top = g(p00) * (1.0 - wx) + g(p10) * wx;
                let bot = g(p01) * (1.0 - wx) + g(p11) * wx;
                top * (1.0 - wy) + bot * wy
            };
            // snap is 0x00RRGGBB; the theme is COLORREF (0x00BBGGRR)
            let (r, g, b) = (ch(16), ch(8), ch(0));
            let mix = |bg: f32, bl: f32| (bg * a + bl * (1.0 - a)).clamp(0.0, 255.0) as u32;
            out[(y * fw + x) as usize] = (mix(tr, r) << 16) | (mix(tg, g) << 8) | mix(tb, b);
        }
    }
    let old = BACK.swap(dib as isize, Relaxed);
    if old != 0 {
        DeleteObject(old as HBITMAP as _);
    }
}

fn base_alpha() -> u8 {
    let s = settings();
    if s.blur { 255 } else { (s.opacity as u32 * 255 / 100) as u8 }
}

/// Re-applies transparency/blur with the current settings.
unsafe fn apply_opacity() {
    let ui = h(&UI);
    let exiting_or_entering = ANIM.with(|a| a.borrow().is_some());
    if !exiting_or_entering {
        SetLayeredWindowAttributes(ui, 0, base_alpha(), LWA_ALPHA);
    }
    if settings().blur {
        rebuild_backdrop();
    }
    InvalidateRect(ui, null(), 0);
}

unsafe fn apply_window_effect() {
    apply_opacity();
}

/// (Re)creates the UI fonts for the current language: CJK languages need a CJK UI font.
unsafe fn make_fonts() {
    let face = wide(i18n::font_face(lang()));
    let mk = |height: i32, weight: i32| CreateFontW(height, 0, 0, 0, weight, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr());
    for (slot, f) in [(&FONT, mk(-18, 400)), (&FONT_S, mk(-14, 400)), (&FONT_B, mk(-16, 600)), (&FONT_CTL, mk(-sc(18), 400))] {
        let old = slot.swap(f as isize, Relaxed);
        if old != 0 {
            DeleteObject(old as _);
        }
    }
    let edit = h(&EDIT);
    if !edit.is_null() {
        SendMessageW(edit, WM_SETFONT, FONT_CTL.load(Relaxed) as usize, 1);
    }
}

/// Applies a language change: fonts, tray tooltip and a full repaint.
unsafe fn apply_language() {
    make_fonts();
    tray(h(&MAIN), NIM_MODIFY);
    InvalidateRect(h(&UI), null(), 1);
}

/// Makes the window invisible for a moment and captures what is behind it (for the blur).
unsafe fn recapture_backdrop() {
    SetLayeredWindowAttributes(h(&UI), 0, 0, LWA_ALPHA);
    std::thread::sleep(Duration::from_millis(80));
    capture_snapshot();
}

/// Switches between normal and compact mode: resizes the window, the search box and the buffers.
unsafe fn apply_layout() {
    let ui = h(&UI);
    COMPACT.store(settings().compact, Relaxed);
    // buffers and the blurred background depend on the size: discard them
    let (m, b) = MEMBUF.with(|c| c.replace((0, 0)));
    if m != 0 {
        DeleteDC(m as HDC);
        DeleteObject(b as _);
    }
    SNAP.with(|s| s.borrow_mut().clear());
    let old = BACK.swap(0, Relaxed);
    if old != 0 {
        DeleteObject(old as HBITMAP as _);
    }
    compute_pos();
    SetWindowPos(ui, null_mut(), UI_X.load(Relaxed) as i32, UI_Y.load(Relaxed) as i32, pw(), ph(), SWP_NOZORDER | SWP_NOACTIVATE);
    MoveWindow(h(&EDIT), sc(52), sc(59), sc(ui_w_fn() - 52 - 26), sc(26), 1);
    refilter();
    if settings().blur {
        recapture_backdrop();
    }
    apply_window_effect();
    InvalidateRect(ui, null(), 1);
}

/// The window's own corners (Windows 11): square, small or normal, depending on the radius.
/// Windows only offers these three levels; the inner panels follow the exact value.
unsafe fn apply_corner() {
    let ui = h(&UI);
    if ui.is_null() {
        return;
    }
    let r = settings().radius;
    let pref: i32 = if r == 0 { 1 } else if r <= 6 { 3 } else { 2 }; // DONOTROUND / ROUNDSMALL / ROUND
    DwmSetWindowAttribute(ui, 33, &pref as *const i32 as *const c_void, 4);
    InvalidateRect(ui, null(), 0);
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

unsafe fn place_window(v: f32) {
    // v: 0 = fully out (invisible), 1 = in place
    let ui = h(&UI);
    let s = settings();
    let off = match s.anim_style {
        1 => (1.0 - v) * SLIDE_PX * scale(),
        2 => -(1.0 - v) * SLIDE_PX * scale(),
        _ => 0.0,
    };
    SetLayeredWindowAttributes(ui, 0, (base_alpha() as f32 * v) as u8, LWA_ALPHA);
    SetWindowPos(ui, null_mut(), UI_X.load(Relaxed) as i32, UI_Y.load(Relaxed) as i32 + off as i32, 0, 0,
        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
}

/// One frame of the window open/close animation. Returns true while it is still animating.
unsafe fn anim_frame() -> bool {
    let Some((start, dur, out)) = ANIM.with(|a| *a.borrow()) else { return false };
    let e = (start.elapsed().as_secs_f32() * 1000.0 / dur as f32).min(1.0);
    let v = if out { 1.0 - e * e * e } else { ease_out(e) };
    place_window(v);
    if e >= 1.0 {
        ANIM.with(|a| *a.borrow_mut() = None);
        if out {
            hide_now();
        } else {
            place_window(1.0);
        }
        return false;
    }
    true
}

unsafe fn start_anim(out: bool) {
    let ms = settings().anim_ms;
    ANIM.with(|a| *a.borrow_mut() = Some((Instant::now(), ms, out)));
    kick_frames();
    anim_frame();
}

/// Hides right away, without animation, and leaves the window ready for the next opening.
unsafe fn hide_now() {
    let ui = h(&UI);
    ANIM.with(|a| *a.borrow_mut() = None);
    ShowWindow(ui, SW_HIDE);
    place_window(1.0);
}

unsafe fn fill(hdc: HDC, r: &RECT, color: u32) {
    let b = CreateSolidBrush(color);
    FillRect(hdc, r, b);
    DeleteObject(b as _);
}

// ── Scale (DPI) and anti-aliased shapes ──
// The app is DPI-aware: the window is created at its physical size and all of the drawing
// in `paint` uses logical coordinates with a scale transform on the DC.
// Rounded shapes are drawn with GDI+ (anti-aliasing).

static SCALE_BITS: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0f32
fn scale() -> f32 {
    f32::from_bits(SCALE_BITS.load(Relaxed))
}
fn sc(v: i32) -> i32 {
    (v as f32 * scale()).round() as i32
}
fn pw() -> i32 {
    sc(ui_w_fn())
}
fn ph() -> i32 {
    sc(ui_h_fn())
}
/// Physical (mouse) coordinate -> logical.
fn unsc(v: i32) -> i32 {
    (v as f32 / scale()).round() as i32
}

unsafe fn set_xform(hdc: HDC) {
    let s = scale();
    let xf = XFORM { eM11: s, eM12: 0.0, eM21: 0.0, eM22: s, eDx: 0.0, eDy: 0.0 };
    SetGraphicsMode(hdc, GM_ADVANCED);
    SetWorldTransform(hdc, &xf);
}

unsafe fn reset_xform(hdc: HDC) {
    ModifyWorldTransform(hdc, null(), MWT_IDENTITY);
}

/// Prepares a GDI+ Graphics with anti-aliasing, with the DC free of any GDI transform
/// (the scale is applied by the drawing itself), and puts the DC back as it was.
unsafe fn aa_fill(hdc: HDC, color: u32, draw: impl FnOnce(*mut GpGraphics, *mut GpBrush, f32)) {
    reset_xform(hdc);
    let mut g: *mut GpGraphics = null_mut();
    if GdipCreateFromHDC(hdc, &mut g) == 0 {
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        // COLORREF 0x00BBGGRR -> ARGB 0xFFRRGGBB
        let argb = 0xFF00_0000 | ((color & 0xFF) << 16) | (color & 0xFF00) | ((color >> 16) & 0xFF);
        let mut br: *mut GpSolidFill = null_mut();
        GdipCreateSolidFill(argb, &mut br);
        draw(g, br as *mut GpBrush, scale());
        GdipDeleteBrush(br as *mut GpBrush);
        GdipDeleteGraphics(g);
    }
    set_xform(hdc);
}

unsafe fn rrect(hdc: HDC, r: &RECT, rad: i32, color: u32) {
    // 10 and 12 are the "card corners" (panels, buttons, highlight): they follow the setting.
    // The others (switches, tracks, scrollbar) are pills and do not change.
    let rad = if rad == 10 || rad == 12 {
        (rad as f32 * settings().radius as f32 / 12.0).round() as i32
    } else {
        rad
    };
    aa_fill(hdc, color, |g, br, s| {
        let x = r.left as f32 * s;
        let y = r.top as f32 * s;
        let w = (r.right + 1 - r.left) as f32 * s;
        let hh = (r.bottom + 1 - r.top) as f32 * s;
        let d = (rad as f32 * s).min(w).min(hh);
        if d < 1.0 {
            // radius 0: a plain rectangle (zero-size arcs break the GDI+ path)
            GdipFillRectangle(g, br, x, y, w, hh);
            return;
        }
        let mut p: *mut GpPath = null_mut();
        GdipCreatePath(FillModeAlternate, &mut p);
        GdipAddPathArc(p, x, y, d, d, 180.0, 90.0);
        GdipAddPathArc(p, x + w - d, y, d, d, 270.0, 90.0);
        GdipAddPathArc(p, x + w - d, y + hh - d, d, d, 0.0, 90.0);
        GdipAddPathArc(p, x, y + hh - d, d, d, 90.0, 90.0);
        GdipClosePathFigure(p);
        GdipFillPath(g, br, p);
        GdipDeletePath(p);
    });
}

unsafe fn circle(hdc: HDC, cx: i32, cy: i32, rad: i32, color: u32) {
    aa_fill(hdc, color, |g, br, s| {
        GdipFillEllipse(
            g, br,
            (cx - rad) as f32 * s, (cy - rad) as f32 * s,
            (2 * rad + 1) as f32 * s, (2 * rad + 1) as f32 * s,
        );
    });
}

unsafe fn txt(hdc: HDC, s: &str, r: &RECT, flags: u32, font: &AtomicIsize, color: u32) {
    SelectObject(hdc, h(font) as _);
    SetTextColor(hdc, color);
    SetBkMode(hdc, 1);
    let mut rr = *r;
    DrawTextW(hdc, wide(s).as_ptr(), -1, &mut rr, flags);
}

const SL: u32 = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX;


/// Height and position of the scrollbar thumb (logical).
fn scrollbar_thumb() -> Option<(f32, f32)> {
    let total = row_count() as f32 * row_h_fn() as f32;
    if total <= list_h_fn() as f32 {
        return None;
    }
    let ms = total - list_h_fn() as f32;
    let thumb = (list_h_fn() as f32 * list_h_fn() as f32 / total).max(36.0);
    let scroll = LST.with(|l| l.borrow().scroll);
    let y0 = LIST_Y as f32 + 4.0 + (scroll / ms) * (list_h_fn() as f32 - 8.0 - thumb);
    Some((y0, thumb))
}

/// The list is drawn by us: pixel scrolling, with a highlight that slides to the selected item.
unsafe fn draw_list(hdc: HDC, t: &Theme) {
    rrect(hdc, &RECT { left: LIST_X, top: LIST_Y, right: list_r_fn() - 1, bottom: LIST_Y + list_h_fn() - 1 }, 12, t.ctrl);
    let n = row_count();
    if n == 0 {
        txt(hdc, tr("No items"), &RECT { left: LIST_X, top: LIST_Y, right: list_r_fn(), bottom: LIST_Y + 80 }, SL | DT_CENTER, &FONT, t.dim);
        return;
    }
    let (sel, scroll, hl) = LST.with(|l| {
        let l = l.borrow();
        (l.sel, l.scroll, l.hl)
    });
    IntersectClipRect(hdc, LIST_X, LIST_Y, list_r_fn(), LIST_Y + list_h_fn());
    if sel >= 0 {
        let y = (LIST_Y as f32 + hl - scroll).round() as i32;
        rrect(hdc, &RECT { left: LIST_X + 6, top: y + 2, right: list_r_fn() - 12, bottom: y + row_h_fn() - 3 }, 10, t.sel);
    }
    let rh = row_h_fn() as f32;
    let first = (scroll / rh).floor().max(0.0) as usize;
    let last = (((scroll + list_h_fn() as f32) / rh).ceil() as usize + 1).min(n);
    for i in first..last {
        let top = LIST_Y + (i as f32 * rh - scroll).round() as i32;
        let Some(e) = entry_at(i as isize) else { continue };
        let (glyph, col) = match e.kind {
            Kind::Text => (IC_TEXT, t.dim),
            Kind::Image => (IC_IMAGE, t.icon_img),
        };
        let bottom = top + row_h_fn();
        txt(hdc, glyph, &RECT { left: LIST_X + 16, top, right: LIST_X + 42, bottom }, SL | DT_LEFT, &FONT_ICON, col);
        txt(hdc, &display_line(&e), &RECT { left: LIST_X + 50, top, right: star_x0_fn() - 2, bottom }, SL | DT_LEFT | DT_END_ELLIPSIS, &FONT, t.text);
        let star = if is_fav(e.hash) { rgb(255, 193, 40) } else { t.track };
        txt(hdc, IC_STAR, &RECT { left: star_x0_fn(), top, right: star_x1_fn(), bottom }, SL | DT_CENTER, &FONT_ICON, star);
        txt(hdc, IC_TRASH, &RECT { left: star_x1_fn() + 2, top, right: trash_x1_fn(), bottom }, SL | DT_CENTER, &FONT_ICON, t.dim);
    }
    if let Some((y0, th)) = scrollbar_thumb() {
        let active = DRAG.load(Relaxed) == 3;
        rrect(hdc, &RECT { left: list_r_fn() - 9, top: y0.round() as i32, right: list_r_fn() - 5, bottom: (y0 + th).round() as i32 },
            4, if active { t.dim } else { t.track });
    }
    SelectClipRgn(hdc, null_mut());
}

/// Tab widths: with a name (normal) or just the icon (compact).
fn tab_w(i: usize) -> i32 {
    if compact() { 46 } else { [140, 140, 170][i] }
}

fn tab_rect(i: usize) -> RECT {
    let x: i32 = 12 + (0..i).map(|k| tab_w(k) + 6).sum::<i32>();
    RECT { left: x, top: 8, right: x + tab_w(i), bottom: 44 }
}

unsafe fn draw_tabs(hdc: HDC, t: &Theme, hits: &mut Vec<(RECT, Act)>) {
    let cur = TAB.load(Relaxed);
    let icons = [IC_HISTORY, IC_STAR, IC_GEAR];
    // the tab indicator slides from the previous tab to the current one
    let (a, b, p) = (tab_rect(TAB_FROM.load(Relaxed)), tab_rect(cur), tab_progress());
    let lerp = |x: i32, y: i32| (x as f32 + (y - x) as f32 * p).round() as i32;
    let pill = RECT { left: lerp(a.left, b.left), top: 8, right: lerp(a.right, b.right), bottom: 44 };
    rrect(hdc, &pill, 12, t.ctrl);
    fill(hdc, &RECT { left: pill.left + 14, top: pill.bottom - 3, right: pill.right - 14, bottom: pill.bottom }, t.accent);
    for i in 0..3 {
        let r = tab_rect(i);
        let col = if i == cur { t.text } else { t.dim };
        let icol = if i == cur { t.accent } else { t.dim };
        if compact() {
            txt(hdc, icons[i], &r, SL | DT_CENTER, &FONT_ICON, icol);
        } else {
            txt(hdc, icons[i], &RECT { left: r.left + 14, top: r.top, right: r.left + 38, bottom: r.bottom }, SL | DT_LEFT, &FONT_ICON, icol);
            txt(hdc, tr(TAB_NAMES[i]), &RECT { left: r.left + 42, top: r.top, right: r.right - 6, bottom: r.bottom }, SL | DT_LEFT, &FONT_B, col);
        }
        hits.push((r, Act::Tab(i)));
    }
    // app mark, on the right (just the clipboard icon in compact mode)
    if compact() {
        txt(hdc, IC_MUG, &RECT { left: ui_w_fn() - 44, top: 8, right: ui_w_fn() - 14, bottom: 44 }, SL | DT_CENTER, &FONT_ICON, t.accent);
    } else {
        txt(hdc, IC_MUG, &RECT { left: ui_w_fn() - 170, top: 8, right: ui_w_fn() - 140, bottom: 44 }, SL | DT_LEFT, &FONT_ICON, t.accent);
        txt(hdc, "CoffeeCopy", &RECT { left: ui_w_fn() - 136, top: 8, right: ui_w_fn() - 14, bottom: 44 }, SL | DT_LEFT, &FONT_B, t.dim);
    }
}

unsafe fn draw_preview(hdc: HDC, t: &Theme, bars: bool, content: bool) {
    if bars {
        // search bar
        rrect(hdc, &RECT { left: 12, top: 54, right: ui_w_fn() - 12, bottom: 90 }, 12, t.ctrl);
        txt(hdc, IC_SEARCH, &RECT { left: 26, top: 54, right: 50, bottom: 90 }, SL | DT_LEFT, &FONT_ICON, t.dim);
        // hint line
        let st = settings();
        let verb = if st.paste_on_select { tr("paste") } else { tr("copy") };
        let mut parts = vec![
            format!("Enter {verb}"),
            format!("{} {}", bind_name(st.keys[1]), tr("favorite")),
        ];
        if !compact() {
            parts.push(format!("{} {}", bind_name(st.keys[2]), tr("delete")));
            parts.push(format!("{} {}", bind_name(st.keys[3]), tr("switch tab")));
        }
        parts.push(format!("Esc {}", tr("close")));
        let hint = parts.join("  ·  ");
        txt(hdc, &hint, &RECT { left: 14, top: ui_h_fn() - 38, right: ui_w_fn() - 14, bottom: ui_h_fn() - 8 }, SL | DT_LEFT, &FONT_S, t.dim);
    }
    if !content || compact() {
        return; // while scrolling only the list is redrawn; there is no preview in compact mode
    }
    let mut r = preview_rect();
    rrect(hdc, &r, 12, t.prev);
    let Some(e) = selected_entry() else { return };
    match e.kind {
        Kind::Text => {
            SelectObject(hdc, h(&FONT) as _);
            SetTextColor(hdc, t.text);
            SetBkMode(hdc, 1);
            let s: String = e.text.chars().take(6000).collect();
            let mut tr = RECT { left: r.left + 14, top: r.top + 12, right: r.right - 14, bottom: r.bottom - 12 };
            DrawTextW(hdc, wide(&s).as_ptr(), -1, &mut tr, DT_WORDBREAK | DT_NOPREFIX | DT_EXPANDTABS);
        }
        Kind::Image => {
            // reads the image from disk only when the item changes (1-entry cache)
            IMG_CACHE.with(|c| {
                let mut c = c.borrow_mut();
                if c.as_ref().map(|x| x.0) != Some(e.hash) {
                    *c = std::fs::read(history::image_path(e.hash)).ok().map(|b| (e.hash, b));
                }
            });
            let cache = IMG_CACHE.with(|c| c.borrow().as_ref().map(|x| x.1.clone()));
            let Some(b) = cache else { return };
            if b.len() < 40 {
                return;
            }
            let bi_size = u32::from_le_bytes(b[0..4].try_into().unwrap()) as usize;
            let bpp = u16::from_le_bytes(b[14..16].try_into().unwrap()) as u32;
            let comp = u32::from_le_bytes(b[16..20].try_into().unwrap());
            let clr_used = u32::from_le_bytes(b[32..36].try_into().unwrap());
            let ncolors = if clr_used > 0 { clr_used } else if bpp <= 8 { 1 << bpp } else { 0 } as usize;
            let extra = if comp == 3 && bi_size == 40 { 12 } else { 0 };
            let off = bi_size + extra + ncolors * 4;
            let (w, hh) = dib_size(&b);
            if off >= b.len() || w <= 0 || hh <= 0 {
                return;
            }
            let aw = (r.right - r.left - 28) as f64;
            let ah = (r.bottom - r.top - 28) as f64;
            let sc = (aw / w as f64).min(ah / hh as f64).min(1.0);
            let dw = (w as f64 * sc) as i32;
            let dh = (hh as f64 * sc) as i32;
            r.left += 14;
            r.top += 14;
            SetStretchBltMode(hdc, HALFTONE as _);
            StretchDIBits(
                hdc, r.left, r.top, dw, dh, 0, 0, w, hh,
                b.as_ptr().add(off) as *const c_void,
                b.as_ptr() as *const BITMAPINFO,
                DIB_RGB_COLORS, SRCCOPY,
            );
        }
    }
}

unsafe fn slider(hdc: HDC, t: &Theme, track: &RECT, frac: f64, label: &str, hits: &mut Vec<(RECT, Act)>, act: Act) {
    let cy = (track.top + track.bottom) / 2;
    rrect(hdc, &RECT { left: track.left, top: cy - 3, right: track.right, bottom: cy + 3 }, 6, t.track);
    let kx = track.left + ((track.right - track.left) as f64 * frac) as i32;
    rrect(hdc, &RECT { left: track.left, top: cy - 3, right: kx, bottom: cy + 3 }, 6, t.accent);
    circle(hdc, kx, cy, 10, rgb(245, 245, 245));
    txt(hdc, label, &RECT { left: track.right + 16, top: track.top, right: track.right + 120, bottom: track.bottom }, SL | DT_LEFT, &FONT_B, t.text);
    hits.push((*track, act));
}

/// On/off switch in a column that starts at `x` and is `w` wide.
unsafe fn toggle(hdc: HDC, t: &Theme, x: i32, w: i32, y: i32, label: &str, on: bool, act: Act, hits: &mut Vec<(RECT, Act)>) {
    txt(hdc, label, &RECT { left: x, top: y, right: x + w - 64, bottom: y + 30 }, SL | DT_LEFT | DT_END_ELLIPSIS, &FONT, t.text);
    let r = RECT { left: x + w - 52, top: y + 3, right: x + w, bottom: y + 27 };
    rrect(hdc, &r, 24, if on { t.accent } else { t.track });
    circle(hdc, if on { r.right - 12 } else { r.left + 12 }, y + 15, 9, rgb(245, 245, 245));
    hits.push((r, act));
}

const IC_PALETTE: &str = "\u{f53f}";
const IC_WINDOW: &str = "\u{f2d0}";
const IC_VOLUME: &str = "\u{f028}";
const IC_BEHAVIOR: &str = "\u{f25a}";
const IC_KEYBOARD: &str = "\u{f11c}";
const IC_GENERAL: &str = "\u{f1de}";
const IC_DB: &str = "\u{f1c0}";
const SUB_NAMES: [&str; 7] = ["Appearance", "Window", "Sounds", "Behavior", "Keybinds", "General", "Data"];
static SUB: AtomicUsize = AtomicUsize::new(0);

/// Layout helper for the Settings tab: draws top to bottom with a moving "pen" (y).
struct Cfg<'a> {
    hdc: HDC,
    t: &'a Theme,
    hits: &'a mut Vec<(RECT, Act)>,
    x0: i32,
    cw: i32,
    y: i32,
}

impl<'a> Cfg<'a> {
    unsafe fn label(&mut self, s: &str) {
        let up = s.to_uppercase();
        txt(self.hdc, &up, &RECT { left: self.x0, top: self.y, right: self.x0 + self.cw, bottom: self.y + 22 }, SL | DT_LEFT | DT_END_ELLIPSIS, &FONT_S, self.t.dim);
        self.y += 28;
    }
    fn gap(&mut self, px: i32) {
        self.y += px;
    }
    unsafe fn toggle(&mut self, label: &str, on: bool, act: Act) {
        toggle(self.hdc, self.t, self.x0, self.cw, self.y, label, on, act, self.hits);
        self.y += 38;
    }
    unsafe fn slider(&mut self, frac: f64, label: &str, act: Act) {
        let track = RECT { left: self.x0 + 8, top: self.y, right: self.x0 + self.cw - 120, bottom: self.y + 28 };
        slider(self.hdc, self.t, &track, frac, label, self.hits, act);
        self.y += 38;
    }
    /// Grid of choice buttons. `dim`: drawn muted (option turned off).
    unsafe fn chips(&mut self, names: &[&str], per_row: usize, sel: Option<usize>, dim: bool, mk: fn(usize) -> Act) {
        let gap = 8;
        let w = (self.cw - gap * (per_row as i32 - 1)) / per_row as i32;
        for (i, n) in names.iter().enumerate() {
            let (c, r) = ((i % per_row) as i32, (i / per_row) as i32);
            let rect = RECT { left: self.x0 + c * (w + gap), top: self.y + r * 44, right: self.x0 + c * (w + gap) + w, bottom: self.y + r * 44 + 36 };
            let on = sel == Some(i) && !dim;
            rrect(self.hdc, &rect, 12, if on { self.t.accent } else { self.t.ctrl });
            let col = if dim { self.t.dim } else if on { self.t.on_accent } else { self.t.text };
            let inner = RECT { left: rect.left + 6, top: rect.top, right: rect.right - 6, bottom: rect.bottom };
            txt(self.hdc, n, &inner, SL | DT_CENTER | DT_END_ELLIPSIS, &FONT_B, col);
            self.hits.push((rect, mk(i)));
        }
        self.y += ((names.len() + per_row - 1) / per_row) as i32 * 44;
    }
    /// Action buttons with a "play" icon (sound previews).
    unsafe fn play_buttons(&mut self, items: &[(&str, Act)], per_row: usize) {
        let gap = 8;
        let w = (self.cw - gap * (per_row as i32 - 1)) / per_row as i32;
        for (i, (name, act)) in items.iter().enumerate() {
            let (c, r) = ((i % per_row) as i32, (i / per_row) as i32);
            let rect = RECT { left: self.x0 + c * (w + gap), top: self.y + r * 40, right: self.x0 + c * (w + gap) + w, bottom: self.y + r * 40 + 32 };
            rrect(self.hdc, &rect, 12, self.t.track);
            txt(self.hdc, IC_PLAY, &RECT { left: rect.left + 12, top: rect.top, right: rect.left + 30, bottom: rect.bottom }, SL | DT_LEFT, &FONT_ICON, self.t.accent);
            txt(self.hdc, name, &RECT { left: rect.left + 34, top: rect.top, right: rect.right - 4, bottom: rect.bottom }, SL | DT_LEFT | DT_END_ELLIPSIS, &FONT_S, self.t.text);
            self.hits.push((rect, *act));
        }
        self.y += ((items.len() + per_row - 1) / per_row) as i32 * 40;
    }
}

unsafe fn draw_config(hdc: HDC, t: &Theme, hits: &mut Vec<(RECT, Act)>) {
    let s = settings();
    let sub = SUB.load(Relaxed).min(SUB_NAMES.len() - 1);
    let comp = compact();
    let icons = [IC_PALETTE, IC_WINDOW, IC_VOLUME, IC_BEHAVIOR, IC_KEYBOARD, IC_GENERAL, IC_DB];
    let n_sub = SUB_NAMES.len() as i32;

    // ── navigation between categories ──
    let (x0, cw, y0);
    if !comp {
        // sidebar
        for i in 0..SUB_NAMES.len() {
            let r = RECT { left: 12, top: 62 + i as i32 * 46, right: 204, bottom: 62 + i as i32 * 46 + 40 };
            let on = i == sub;
            if on {
                rrect(hdc, &r, 12, t.ctrl);
                fill(hdc, &RECT { left: r.left + 2, top: r.top + 10, right: r.left + 5, bottom: r.bottom - 10 }, t.accent);
            }
            txt(hdc, icons[i], &RECT { left: r.left + 16, top: r.top, right: r.left + 42, bottom: r.bottom }, SL | DT_LEFT, &FONT_ICON, if on { t.accent } else { t.dim });
            txt(hdc, tr(SUB_NAMES[i]), &RECT { left: r.left + 48, top: r.top, right: r.right - 6, bottom: r.bottom }, SL | DT_LEFT | DT_END_ELLIPSIS, &FONT_B, if on { t.text } else { t.dim });
            hits.push((r, Act::Sub(i)));
        }
        x0 = 236;
        cw = (ui_w_fn() - 236 - 28).min(560);
        y0 = 66;
    } else {
        // icons on top + category title
        let gap = 6;
        let w = (ui_w_fn() - 48 - gap * (n_sub - 1)) / n_sub;
        for i in 0..SUB_NAMES.len() {
            let r = RECT { left: 24 + i as i32 * (w + gap), top: 60, right: 24 + i as i32 * (w + gap) + w, bottom: 96 };
            let on = i == sub;
            rrect(hdc, &r, 12, if on { t.accent } else { t.ctrl });
            txt(hdc, icons[i], &r, SL | DT_CENTER, &FONT_ICON, if on { t.on_accent } else { t.dim });
            hits.push((r, Act::Sub(i)));
        }
        txt(hdc, tr(SUB_NAMES[sub]), &RECT { left: 24, top: 106, right: ui_w_fn() - 24, bottom: 132 }, SL | DT_LEFT, &FONT_B, t.text);
        x0 = 24;
        cw = ui_w_fn() - 48;
        y0 = 140;
    }
    if !comp {
        txt(hdc, tr(SUB_NAMES[sub]), &RECT { left: x0, top: 62, right: x0 + cw, bottom: 90 }, SL | DT_LEFT, &FONT_B, t.text);
    }
    let mut c = Cfg { hdc, t, hits, x0, cw, y: if comp { y0 } else { y0 + 34 } };

    match sub {
        0 => {
            c.label(tr("Theme"));
            let names: Vec<&str> = THEMES.iter().map(|th| tr(th.name)).collect();
            c.chips(&names, 3, Some(s.theme), false, Act::Theme);
            c.gap(10);
            c.label(if s.blur { tr("Transparency (blends with blur)") } else { tr("Window transparency") });
            c.slider((s.opacity as f64 - 40.0) / 60.0, &format!("{}%", s.opacity), Act::Opacity);
            c.toggle(tr("Blur behind the window"), s.blur, Act::Blur);
            c.gap(10);
            c.label(tr("Corner radius"));
            c.slider(s.radius as f64 / 24.0, &format!("{} px", s.radius), Act::Radius);
            c.gap(10);
            c.toggle(tr("Compact mode (narrow window, no preview)"), s.compact, Act::Compact);
        }
        1 => {
            c.label(tr("Where the window opens"));
            c.chips(&[tr("Center"), tr("Cursor"), tr("Top"), tr("Corner")], 4, Some(s.pos), false, Act::Pos);
            c.gap(10);
            c.label(tr("Animation effects"));
            c.toggle(tr("Animate opening, closing and tabs"), s.anim, Act::Anim);
            c.chips(&[tr("Fade"), tr("Slide up"), tr("Slide down")], 3, Some(s.anim_style), !s.anim, Act::AnimStyle);
            c.gap(2);
            c.slider((s.anim_ms as f64 - 80.0) / 420.0, &format!("{} ms", s.anim_ms), Act::Duration);
        }
        2 => {
            c.label(tr("Sound pack"));
            let names: Vec<&str> = PACKS.iter().map(|p| tr(p.name)).collect();
            c.chips(&names, 3, Some(s.pack), false, Act::Pack);
            c.gap(8);
            c.label(tr("Listen"));
            c.play_buttons(&[(tr("New"), Act::TestNew), (tr("Repeat"), Act::TestDup), (tr("Seen before"), Act::TestOld), (tr("Paste"), Act::TestPaste)], if comp { 2 } else { 4 });
            c.gap(8);
            c.label(tr("Sound volume"));
            c.slider(s.volume as f64 / 100.0, &format!("{}%", s.volume), Act::Volume);
            c.toggle(tr("Mute (no sounds when copying)"), s.muted, Act::Mute);
            c.toggle(tr("Play a sound when pasting"), s.paste_sound, Act::PasteSound);
        }
        3 => {
            c.label(tr("When choosing a history item"));
            c.toggle(tr("Paste on select (off: copy only)"), s.paste_on_select, Act::PasteSel);
            c.gap(10);
            c.label(tr("How to confirm with the mouse"));
            c.chips(&[tr("Double click"), tr("Single click")], 2, Some(s.single_click as usize), false, Act::ClickMode);
            c.gap(10);
            c.label("Win+V");
            c.toggle(tr("Use instead of the Windows Win+V"), s.win_v, Act::WinV);
            c.toggle(tr("Turn off Windows clipboard history"), windows_clipboard_history_disabled(), Act::WinHist);
        }
        4 => {
            // ── key bindings ──
            txt(hdc, tr("Click a shortcut, then press the new key combination."), &RECT { left: x0, top: c.y, right: x0 + cw, bottom: c.y + 24 }, SL | DT_LEFT | DT_END_ELLIPSIS, &FONT_S, t.dim);
            c.gap(34);
            let rec = RECORDING.load(Relaxed);
            for i in 0..4usize {
                let chip_w = 230.min(cw / 2);
                let row_y = c.y;
                txt(hdc, tr(KEY_ACTIONS[i]), &RECT { left: x0, top: row_y, right: x0 + cw - chip_w - 12, bottom: row_y + 36 }, SL | DT_LEFT | DT_END_ELLIPSIS, &FONT, t.text);
                let r = RECT { left: x0 + cw - chip_w, top: row_y, right: x0 + cw, bottom: row_y + 36 };
                let recording = rec == i as isize;
                rrect(hdc, &r, 12, if recording { t.accent } else { t.ctrl });
                let label = if recording { tr("Press keys… (Esc cancels)").to_string() } else { bind_name(s.keys[i]) };
                let inner = RECT { left: r.left + 6, top: r.top, right: r.right - 6, bottom: r.bottom };
                txt(hdc, &label, &inner, SL | DT_CENTER | DT_END_ELLIPSIS, if recording { &FONT_S } else { &FONT_B }, if recording { t.on_accent } else { t.text });
                c.hits.push((r, Act::Key(i)));
                c.y += 46;
            }
            c.gap(6);
            let rr = RECT { left: x0, top: c.y, right: x0 + cw.min(300), bottom: c.y + 36 };
            rrect(hdc, &rr, 12, t.track);
            txt(hdc, tr("Reset to defaults"), &RECT { left: rr.left + 8, top: rr.top, right: rr.right - 8, bottom: rr.bottom }, SL | DT_CENTER | DT_END_ELLIPSIS, &FONT_B, t.text);
            c.hits.push((rr, Act::KeysReset));
            c.gap(52);
            if let Some(m) = KEY_MSG.with(|m| m.borrow().clone()) {
                txt(hdc, &m, &RECT { left: x0, top: c.y, right: x0 + cw, bottom: c.y + 44 }, DT_WORDBREAK | DT_NOPREFIX, &FONT_S, rgb(255, 120, 120));
                c.gap(48);
            }
            txt(hdc, tr("Enter, Esc, the arrow keys and Ctrl+A are fixed."), &RECT { left: x0, top: c.y, right: x0 + cw, bottom: c.y + 40 }, DT_WORDBREAK | DT_NOPREFIX, &FONT_S, t.dim);
        }
        5 => {
            c.label(tr("Startup"));
            c.toggle(tr("Start with Windows"), autostart_enabled(), Act::Autostart);
            c.gap(10);
            c.label(tr("Language"));
            c.chips(&i18n::LANG_NAMES, 3, Some(s.lang), false, Act::Lang);
        }
        _ => {
            c.label(tr("History"));
            let (n, f) = (HIST.with(|hs| hs.borrow().entries.len()), FAVS.with(|f| f.borrow().len()));
            let line = tr("{n} items in history  ·  {f} favorites").replace("{n}", &n.to_string()).replace("{f}", &f.to_string());
            txt(hdc, &line, &RECT { left: x0, top: c.y, right: x0 + cw, bottom: c.y + 26 }, SL | DT_LEFT | DT_END_ELLIPSIS, &FONT, t.text);
            c.gap(36);
            let confirm = CONFIRM_CLEAR.load(Relaxed);
            let r = RECT { left: x0, top: c.y, right: x0 + cw.min(420), bottom: c.y + 36 };
            rrect(hdc, &r, 12, if confirm { rgb(190, 30, 50) } else { t.track });
            txt(hdc, IC_TRASH, &RECT { left: r.left + 14, top: r.top, right: r.left + 38, bottom: r.bottom }, SL | DT_LEFT, &FONT_ICON, if confirm { rgb(255, 255, 255) } else { t.dim });
            txt(hdc, if confirm { tr("Click again to confirm") } else { tr("Clear history (keeps favorites)") },
                &RECT { left: r.left + 42, top: r.top, right: r.right - 8, bottom: r.bottom }, SL | DT_LEFT | DT_END_ELLIPSIS, &FONT_B, if confirm { rgb(255, 255, 255) } else { t.text });
            c.hits.push((r, Act::Clear));
            c.gap(56);
            c.label(tr("Credits"));
            let mut tr_rect = RECT { left: x0, top: c.y, right: x0 + cw, bottom: c.y + 150 };
            SelectObject(hdc, h(&FONT_S) as _);
            SetTextColor(hdc, t.dim);
            SetBkMode(hdc, 1);
            let credits = tr("Sounds (opengameart.org): Kenney, Brandon Morris, PWL, starninjas, Cockatrice, Little Robot / the 8-bit and Hi-Tech packs (CC0) and Nicole Marie T (CC BY 4.0).\nIcons: Font Awesome Free 7 (CC BY 4.0).");
            DrawTextW(hdc, wide(credits).as_ptr(), -1, &mut tr_rect, DT_WORDBREAK | DT_NOPREFIX);
        }
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let wdc = BeginPaint(hwnd, &mut ps);
    let (wp_, hp_) = (pw(), ph());
    // Drawing buffer reused between frames (creating a bitmap every frame is expensive).
    let (mut mem_i, mut bmp_i) = MEMBUF.with(|m| m.get());
    if mem_i == 0 {
        let m = CreateCompatibleDC(wdc);
        let b = CreateCompatibleBitmap(wdc, wp_, hp_);
        SelectObject(m, b as _);
        mem_i = m as isize;
        bmp_i = b as isize;
        MEMBUF.with(|c| c.set((mem_i, bmp_i)));
    }
    let _ = bmp_i;
    let mem = mem_i as HDC;

    // Only the dirty region is drawn: the rest is clipped and the parts that do not touch it
    // are not even computed. That is what keeps the animation at 60 frames.
    let dp = ps.rcPaint;
    let dirty = RECT {
        left: dp.left.max(0),
        top: dp.top.max(0),
        right: dp.right.min(wp_),
        bottom: dp.bottom.min(hp_),
    };
    if dirty.right <= dirty.left || dirty.bottom <= dirty.top {
        EndPaint(hwnd, &ps);
        return;
    }
    SelectClipRgn(mem, null_mut());
    IntersectClipRect(mem, dirty.left, dirty.top, dirty.right, dirty.bottom);
    let hit_rect = |l: i32, tp: i32, r: i32, b: i32| {
        dirty.right > sc(l) && dirty.left < sc(r) && dirty.bottom > sc(tp) && dirty.top < sc(b)
    };

    let t = theme();
    // Background (in physical pixels): with blur, the blurred capture already mixed with the theme;
    // otherwise, a solid color.
    let back = BACK.load(Relaxed);
    if settings().blur && back != 0 {
        let bdc = CreateCompatibleDC(wdc);
        let bold = SelectObject(bdc, back as HBITMAP as _);
        BitBlt(mem, dirty.left, dirty.top, dirty.right - dirty.left, dirty.bottom - dirty.top, bdc, dirty.left, dirty.top, SRCCOPY);
        SelectObject(bdc, bold);
        DeleteDC(bdc);
    } else {
        fill(mem, &dirty, t.bg);
    }
    // from here on, everything is drawn in logical coordinates and scaled
    set_xform(mem);
    let mut hits = Vec::new();
    if hit_rect(0, 0, ui_w_fn(), 96) || TAB.load(Relaxed) == 2 {
        draw_tabs(mem, &t, &mut hits);
    } else {
        for i in 0..3 {
            hits.push((tab_rect(i), Act::Tab(i))); // tab clicks keep working
        }
    }
    if TAB.load(Relaxed) == 2 {
        draw_config(mem, &t, &mut hits);
    } else {
        let pr = preview_rect();
        let content = hit_rect(pr.left, pr.top, pr.right, pr.bottom);
        let bars = hit_rect(0, 50, ui_w_fn(), 94) || hit_rect(0, ui_h_fn() - 44, ui_w_fn(), ui_h_fn());
        draw_preview(mem, &t, bars, content);
        if hit_rect(LIST_X, LIST_Y, list_r_fn(), LIST_Y + list_h_fn()) {
            draw_list(mem, &t);
        }
    }
    HITS.with(|hh| *hh.borrow_mut() = hits);
    reset_xform(mem);

    // tab transition: the new content fades in (an overlay of the background that fades out)
    let p = tab_progress();
    if p < 1.0 {
        let alpha = ((1.0 - p) * 255.0) as u8;
        let (ox, oy) = (dirty.left, dirty.top.max(sc(96)));
        let (ow, oh) = (dirty.right - ox, dirty.bottom - oy);
        if ow > 0 && oh > 0 {
            let bf = BLENDFUNCTION { BlendOp: 0, BlendFlags: 0, SourceConstantAlpha: alpha, AlphaFormat: 0 };
            let back = BACK.load(Relaxed);
            if settings().blur && back != 0 {
                let bdc = CreateCompatibleDC(wdc);
                let bold = SelectObject(bdc, back as HBITMAP as _);
                AlphaBlend(mem, ox, oy, ow, oh, bdc, ox, oy, ow, oh, bf);
                SelectObject(bdc, bold);
                DeleteDC(bdc);
            } else {
                let sdc = CreateCompatibleDC(wdc);
                let sbmp = CreateCompatibleBitmap(wdc, 1, 1);
                let sold = SelectObject(sdc, sbmp as _);
                SetPixel(sdc, 0, 0, t.bg);
                AlphaBlend(mem, ox, oy, ow, oh, sdc, 0, 0, 1, 1, bf);
                SelectObject(sdc, sold);
                DeleteObject(sbmp as _);
                DeleteDC(sdc);
            }
        }
    }

    BitBlt(wdc, dirty.left, dirty.top, dirty.right - dirty.left, dirty.bottom - dirty.top, mem, dirty.left, dirty.top, SRCCOPY);
    SelectClipRgn(mem, null_mut());
    EndPaint(hwnd, &ps);
}

// ───────────────────────── Window events ─────────────────────────

fn hit_at(x: i32, y: i32) -> Option<(RECT, Act)> {
    HITS.with(|hh| {
        hh.borrow()
            .iter()
            .rev()
            .find(|(r, _)| x >= r.left && x < r.right && y >= r.top - 4 && y < r.bottom + 4)
            .copied()
    })
}

unsafe fn drag_to(act: Act, rect: RECT, x: i32) {
    let frac = ((x - rect.left) as f64 / (rect.right - rect.left) as f64).clamp(0.0, 1.0);
    match act {
        Act::Opacity => {
            SET.with(|s| s.borrow_mut().opacity = (40.0 + frac * 60.0).round() as u8);
            apply_opacity();
        }
        Act::Volume => SET.with(|s| s.borrow_mut().volume = (frac * 100.0).round() as u8),
        Act::Duration => SET.with(|s| s.borrow_mut().anim_ms = ((80.0 + frac * 420.0) / 10.0).round() as u32 * 10),
        Act::Radius => {
            SET.with(|s| s.borrow_mut().radius = (frac * 24.0).round() as u32);
            apply_corner();
        }
        _ => {}
    }
    InvalidateRect(h(&UI), null(), 0);
}

unsafe fn do_act(act: Act, rect: RECT, x: i32) {
    if act != Act::Clear {
        CONFIRM_CLEAR.store(false, Relaxed);
    }
    if !matches!(act, Act::Key(_)) {
        RECORDING.store(-1, Relaxed); // any other click cancels a recording in progress
    }
    match act {
        Act::Tab(i) => switch_tab(i),
        Act::Theme(i) => {
            update_settings(|s| s.theme = i);
            apply_theme();
        }
        Act::Pack(i) => {
            update_settings(|s| s.pack = i);
            play_sound(Snd::New);
        }
        Act::TestNew => play_sound(Snd::New),
        Act::TestDup => play_sound(Snd::Repeat),
        Act::TestOld => play_sound(Snd::Seen),
        Act::TestPaste => play_sound(Snd::Paste),
        Act::PasteSel => update_settings(|s| s.paste_on_select = !s.paste_on_select),
        Act::PasteSound => update_settings(|s| s.paste_sound = !s.paste_sound),
        Act::Autostart => set_autostart(!autostart_enabled()),
        Act::Mute => update_settings(|s| s.muted = !s.muted),
        Act::Blur => {
            let on = !settings().blur;
            update_settings(|s| s.blur = on);
            if on {
                recapture_backdrop();
            }
            apply_window_effect();
        }
        Act::Sub(i) => SUB.store(i, Relaxed),
        Act::Lang(i) => {
            update_settings(|s| s.lang = i);
            LANG.store(i, Relaxed);
            apply_language();
        }
        Act::Key(i) => {
            set_key_msg(None);
            RECORDING.store(i as isize, Relaxed);
            SetFocus(h(&UI)); // key presses must reach the window
        }
        Act::KeysReset => {
            update_settings(|s| s.keys = DEFAULT_KEYS);
            set_key_msg(None);
            if !register_hotkey() {
                set_key_msg(Some(tr("Could not register this shortcut: another app is using it.").to_string()));
            }
            tray(h(&MAIN), NIM_MODIFY);
        }
        Act::ClickMode(i) => update_settings(|s| s.single_click = i == 1),
        Act::Compact => {
            update_settings(|s| s.compact = !s.compact);
            apply_layout();
        }
        Act::WinV => {
            update_settings(|s| s.win_v = !s.win_v);
            apply_winv();
        }
        Act::WinHist => set_windows_clipboard_history(!windows_clipboard_history_disabled()),
        Act::Pos(i) => update_settings(|s| s.pos = i),
        Act::Anim => update_settings(|s| s.anim = !s.anim),
        Act::AnimStyle(i) => update_settings(|s| s.anim_style = i),
        Act::Clear => {
            if CONFIRM_CLEAR.swap(true, Relaxed) {
                let keep = FAVS.with(|f| f.borrow().clone());
                HIST.with(|hs| hs.borrow_mut().clear_except(&keep));
                CONFIRM_CLEAR.store(false, Relaxed);
            }
        }
        Act::Opacity | Act::Volume | Act::Duration | Act::Radius => {
            DRAG.store(match act { Act::Opacity => 0, Act::Volume => 1, Act::Duration => 2, _ => 4 }, Relaxed);
            SetCapture(h(&UI));
            drag_to(act, rect, x);
        }
    }
    InvalidateRect(h(&UI), null(), 0);
}

/// Subclass of the search box: arrow keys, Enter, Esc and the remappable shortcuts.
unsafe extern "system" fn sub_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let (vk, mods) = (wp as u16, current_mods());
            if vk == VK_A && mods == M_CTRL && hwnd == h(&EDIT) {
                // a single-line EDIT does not handle Ctrl+A by itself
                SendMessageW(hwnd, EM_SETSEL, 0, -1);
                eat_char(hwnd);
                return 0;
            }
            if handle_key(vk, mods) {
                eat_char(hwnd);
                return 0;
            }
        }
        WM_MOUSEWHEEL => {
            // the wheel over the search box also scrolls the list
            wheel(((wp >> 16) & 0xFFFF) as i16 as i32);
            return 0;
        }
        _ => {}
    }
    let f: WNDPROC = std::mem::transmute(ORIG_EDIT.load(Relaxed));
    CallWindowProcW(f, hwnd, msg, wp, lp)
}
/// Click inside the list area (logical coordinates). Returns true if it handled the click.
unsafe fn list_click(x: i32, y: i32, double: bool) -> bool {
    if TAB.load(Relaxed) == 2 || x < LIST_X || x >= list_r_fn() || y < LIST_Y || y >= LIST_Y + list_h_fn() {
        return false;
    }
    SetFocus(h(&EDIT));
    // scrollbar: drag the thumb / jump to the clicked position
    if x >= sb_x0_fn() && scrollbar_thumb().is_some() {
        DRAG.store(3, Relaxed);
        SetCapture(h(&UI));
        scrollbar_drag_to(y);
        return true;
    }
    let scroll = LST.with(|l| l.borrow().scroll);
    let row = ((y - LIST_Y) as f32 + scroll) / row_h_fn() as f32;
    if row < 0.0 || row as usize >= row_count() {
        return true;
    }
    let row = row as isize;
    let Some(e) = entry_at(row) else { return true };
    if x >= star_x0_fn() && x < star_x1_fn() {
        toggle_fav(e.hash);
    } else if x >= star_x1_fn() && x < trash_x1_fn() {
        delete_entry(e.hash);
    } else {
        set_sel(row);
        // double click (default) or a single click, per the setting
        if double || settings().single_click {
            choose();
        }
    }
    true
}

/// true if (x, y) is over a list row or over the scrollbar.
fn list_clickable_at(x: i32, y: i32) -> bool {
    if TAB.load(Relaxed) == 2 || x < LIST_X || x >= list_r_fn() || y < LIST_Y || y >= LIST_Y + list_h_fn() {
        return false;
    }
    if x >= sb_x0_fn() && scrollbar_thumb().is_some() {
        return true;
    }
    let scroll = LST.with(|l| l.borrow().scroll);
    let row = ((y - LIST_Y) as f32 + scroll) / row_h_fn() as f32;
    row >= 0.0 && (row as usize) < row_count()
}

/// Hovering a row selects it (the highlight slides to it).
/// Only reached when the mouse actually moves; scrolling with the wheel does not trigger it.
unsafe fn hover_select(x: i32, y: i32) {
    if TAB.load(Relaxed) == 2 || x < LIST_X || x >= sb_x0_fn() || y < LIST_Y || y >= LIST_Y + list_h_fn() {
        return;
    }
    let scroll = LST.with(|l| l.borrow().scroll);
    let row = ((y - LIST_Y) as f32 + scroll) / row_h_fn() as f32;
    if row < 0.0 || row as usize >= row_count() {
        return;
    }
    let row = row as isize;
    let changed = LST.with(|l| {
        let mut l = l.borrow_mut();
        if l.sel != row {
            l.sel = row;
            true
        } else {
            false
        }
    });
    if changed {
        start_list_anim(); // only the highlight animates; the scroll does not move
        invalidate_preview();
    }
}

/// Thumb drag: converts the mouse position into scroll (no smoothing while dragging).
unsafe fn scrollbar_drag_to(y: i32) {
    let Some((_, thumb)) = scrollbar_thumb() else { return };
    let ms = max_scroll();
    let span = list_h_fn() as f32 - 8.0 - thumb;
    let frac = ((y as f32 - LIST_Y as f32 - 4.0 - thumb / 2.0) / span).clamp(0.0, 1.0);
    LST.with(|l| {
        let mut l = l.borrow_mut();
        l.scroll = frac * ms;
        l.target = l.scroll;
    });
    invalidate_list();
}

unsafe extern "system" fn ui_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let code = ((wp >> 16) & 0xFFFF) as u32;
            let id = wp & 0xFFFF;
            if id == ID_EDIT && code == EN_CHANGE {
                refilter();
            }
            0
        }
        WM_MOUSEWHEEL => {
            wheel(((wp >> 16) & 0xFFFF) as i16 as i32);
            0
        }
        WM_CTLCOLOREDIT => {
            let t = theme();
            SetTextColor(wp as HDC, t.text);
            SetBkColor(wp as HDC, t.ctrl);
            h(&BR_CTRL) as LRESULT
        }
        WM_ERASEBKGND => 1,
        WM_SETCURSOR => {
            // hand cursor over everything clickable (tabs, buttons, switches, sliders, list items)
            if (lp & 0xFFFF) as u32 == HTCLIENT {
                let mut pt = POINT { x: 0, y: 0 };
                GetCursorPos(&mut pt);
                ScreenToClient(hwnd, &mut pt);
                let (x, y) = (unsc(pt.x), unsc(pt.y));
                let clickable = hit_at(x, y).is_some() || list_clickable_at(x, y) || DRAG.load(Relaxed) >= 0;
                SetCursor(LoadCursorW(null_mut(), if clickable { IDC_HAND } else { IDC_ARROW }));
                return 1;
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_FRAME => {
            // one animation frame, at the screen's pace
            FRAME_PENDING.store(false, Relaxed);
            let a = anim_frame();
            let l = list_frame();
            let t = if TAB_ANIM.with(|x| x.borrow().is_some()) { tab_frame() } else { false };
            if !(a || l || t) {
                ANIM_ACTIVE.store(false, Relaxed);
            }
            // paint now, instead of waiting for WM_PAINT (low priority in the queue)
            UpdateWindow(hwnd);
            0
        }
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let x = unsc((lp & 0xFFFF) as i16 as i32);
            let y = unsc(((lp >> 16) & 0xFFFF) as i16 as i32);
            if list_click(x, y, msg == WM_LBUTTONDBLCLK) {
                // handled by the list
            } else if let Some((r, act)) = hit_at(x, y) {
                do_act(act, r, x);
            } else if CONFIRM_CLEAR.swap(false, Relaxed) {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_MOUSEMOVE => {
            let d = DRAG.load(Relaxed);
            if d < 0 {
                hover_select(unsc((lp & 0xFFFF) as i16 as i32), unsc(((lp >> 16) & 0xFFFF) as i16 as i32));
            }
            if d == 3 {
                scrollbar_drag_to(unsc(((lp >> 16) & 0xFFFF) as i16 as i32));
            } else if d >= 0 {
                let act = match d { 0 => Act::Opacity, 1 => Act::Volume, 2 => Act::Duration, _ => Act::Radius };
                if let Some((r, _)) = HITS.with(|hh| hh.borrow().iter().find(|(_, a)| *a == act).copied()) {
                    drag_to(act, r, unsc((lp & 0xFFFF) as i16 as i32));
                }
            }
            0
        }
        WM_LBUTTONUP => {
            let d = DRAG.swap(-1, Relaxed);
            if d >= 0 {
                ReleaseCapture();
                if d == 3 {
                    invalidate_list(); // thumb goes back to its normal color
                } else {
                    update_settings(|_| {}); // saves opacity/volume/duration
                }
            }
            0
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            if handle_key(wp as u16, current_mods()) {
                eat_char(hwnd);
                return 0;
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_ACTIVATE => {
            if (wp & 0xFFFF) == 0 {
                hide_ui();
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe fn create_ui(hinst: HINSTANCE) {
    make_fonts();
    let mut n = 0u32;
    AddFontMemResourceEx(FA_FONT.as_ptr() as *const c_void, FA_FONT.len() as u32, null(), &mut n);
    set(&FONT_ICON, CreateFontW(-17, 0, 0, 0, 900, 0, 0, 0, 1, 0, 0, 5, 0, wide("Font Awesome 7 Free Solid").as_ptr()));
    apply_theme_brushes_only();

    let class = wide("CoffeeCopyUI");
    let wc = WNDCLASSW {
        style: CS_DBLCLKS,
        lpfnWndProc: Some(ui_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinst,
        hIcon: null_mut(),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        hbrBackground: null_mut(),
        lpszMenuName: null(),
        lpszClassName: class.as_ptr(),
    };
    RegisterClassW(&wc);

    let x = (GetSystemMetrics(SM_CXSCREEN) - pw()) / 2;
    let y = (GetSystemMetrics(SM_CYSCREEN) - ph()) / 3;
    UI_X.store(x as isize, Relaxed);
    UI_Y.store(y as isize, Relaxed);
    let ui = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED, class.as_ptr(), wide("CoffeeCopy").as_ptr(),
        WS_POPUP | WS_CLIPCHILDREN, x, y, pw(), ph(),
        null_mut(), null_mut(), hinst, null(),
    );
    set(&UI, ui);
    apply_window_effect();
    apply_corner();

    let edit = CreateWindowExW(
        0, wide("EDIT").as_ptr(), null(),
        WS_CHILD | WS_VISIBLE | ES_AUTOHSCROLL as u32, sc(52), sc(59), sc(ui_w_fn() - 52 - 26), sc(26),
        ui, ID_EDIT as HMENU, hinst, null(),
    );
    set(&EDIT, edit);
    // the search box is a real control: it needs a font in physical pixels
    SendMessageW(edit, WM_SETFONT, FONT_CTL.load(Relaxed) as usize, 1);
    let sp = sub_proc as *const () as isize;
    ORIG_EDIT.store(SetWindowLongPtrW(edit, GWLP_WNDPROC, sp), Relaxed);
}

unsafe fn apply_theme_brushes_only() {
    let t = theme();
    set(&BR_WIN, CreateSolidBrush(t.bg));
    set(&BR_CTRL, CreateSolidBrush(t.ctrl));
}

// ───────────────────────── Win+V instead of the Windows history ─────────────────────────
//
// Windows reserves Win+V (RegisterHotKey does not work with it). With the option on, a
// low-level keyboard hook swallows the V while the Windows key is held and opens
// CoffeeCopy. An injected "empty" key keeps the Start menu from opening when Windows is released.

unsafe fn send_dummy_key() {
    let key = |flags: u32| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: 0xE8, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    };
    let inputs = [key(0), key(KEYEVENTF_KEYUP)];
    SendInput(2, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
}

/// Win+V was triggered during the current press of the Windows key.
static WINV_USED: AtomicBool = AtomicBool::new(false);
static WIN_DOWN: AtomicBool = AtomicBool::new(false);
static HOOK_TID: AtomicU32 = AtomicU32::new(0);

/// Injects, in order: an empty key (down/up) and the Windows key "release".
/// That way Windows always sees another key before the release and does not open the Start menu.
unsafe fn send_win_release(win_vk: u16) {
    let key = |vk: u16, flags: u32| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    };
    let inputs = [key(0xE8, 0), key(0xE8, KEYEVENTF_KEYUP), key(win_vk, KEYEVENTF_KEYUP)];
    SendInput(3, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
}

unsafe extern "system" fn ll_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == 0 {
        let k = &*(lp as *const KBDLLHOOKSTRUCT);
        let m = wp as u32;
        let is_down = m == WM_KEYDOWN || m == WM_SYSKEYDOWN;
        let is_up = m == WM_KEYUP || m == WM_SYSKEYUP;
        // tracks the Windows key through the hook's own events (more reliable than GetAsyncKeyState)
        if k.vkCode == VK_LWIN as u32 || k.vkCode == VK_RWIN as u32 {
            let injected = k.flags & LLKHF_INJECTED != 0;
            if is_down {
                WIN_DOWN.store(true, Relaxed);
            } else if is_up {
                WIN_DOWN.store(false, Relaxed);
                // If Win+V was used during this press, Windows would open the Start menu on release of the
                // Windows key (it only does not if it sees another key in between). To be deterministic:
                // swallow this "release" and re-inject [empty key, Windows release] in the right order.
                if !injected && WINV_USED.swap(false, Relaxed) {
                    send_win_release(k.vkCode as u16);
                    return 1;
                }
            }
        }
        if k.flags & LLKHF_INJECTED == 0 && k.vkCode == VK_V as u32 {
            if is_down {
                let down = |vk: u16| GetAsyncKeyState(vk as i32) < 0;
                let win = WIN_DOWN.load(Relaxed) || down(VK_LWIN) || down(VK_RWIN);
                let others = down(VK_CONTROL) || down(VK_SHIFT) || down(VK_MENU);
                if win && !others {
                    if !WINV_SWALLOW.swap(true, Relaxed) {
                        WINV_USED.store(true, Relaxed);
                        send_dummy_key();
                        PostMessageW(h(&MAIN), WM_WINV, 0, 0);
                    }
                    return 1; // swallow the V (including repeats)
                }
            } else if is_up && WINV_SWALLOW.swap(false, Relaxed) {
                return 1;
            }
        }
    }
    CallNextHookEx(null_mut(), code, wp, lp)
}

/// The hook runs on its own thread, with just the message loop: that way it answers right away,
/// even when the main thread is busy drawing. (If the hook is slow, Windows ignores it and
/// the original Win+V goes through, which is what made the Windows panel show up sometimes.)
fn hook_thread() {
    unsafe {
        HOOK_TID.store(GetCurrentThreadId(), Relaxed);
        let hk = SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll_proc), GetModuleHandleW(null()), 0);
        if hk.is_null() {
            HOOK_TID.store(0, Relaxed);
            HOOK.store(0, Relaxed);
            return;
        }
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {}
        UnhookWindowsHookEx(hk);
        // only clear the id if it is still ours: after a quick off/on, a newer hook thread owns it
        let me = GetCurrentThreadId();
        if HOOK_TID.compare_exchange(me, 0, Relaxed, Relaxed).is_ok() {
            WIN_DOWN.store(false, Relaxed);
            WINV_SWALLOW.store(false, Relaxed);
        }
    }
}

/// Turns the Win+V capture on or off according to the setting.
unsafe fn apply_winv() {
    let want = settings().win_v;
    let running = HOOK.load(Relaxed) != 0;
    if want && !running {
        HOOK.store(1, Relaxed);
        std::thread::spawn(hook_thread);
    } else if !want && running {
        let tid = HOOK_TID.load(Relaxed);
        if tid != 0 {
            PostThreadMessageW(tid, WM_QUIT, 0, 0);
        }
        HOOK.store(0, Relaxed);
    }
}

// ── The Windows own clipboard history (the Win+V panel) ──
const CLIP_KEY: &str = "Software\\Microsoft\\Clipboard";
const CLIP_VAL: &str = "EnableClipboardHistory";

/// true if the Windows history is turned off (value 0).
fn windows_clipboard_history_disabled() -> bool {
    unsafe {
        let key = wide(CLIP_KEY);
        let name = wide(CLIP_VAL);
        let mut hk: HKEY = null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, KEY_READ, &mut hk) != 0 {
            return false;
        }
        let mut val = 1u32;
        let mut sz = 4u32;
        let r = RegQueryValueExW(hk, name.as_ptr(), null(), null_mut(), &mut val as *mut u32 as *mut u8, &mut sz);
        RegCloseKey(hk);
        r == 0 && val == 0
    }
}

fn set_windows_clipboard_history(disable: bool) {
    unsafe {
        let key = wide(CLIP_KEY);
        let name = wide(CLIP_VAL);
        let mut hk: HKEY = null_mut();
        let r = RegCreateKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, null(), 0, KEY_WRITE, null(), &mut hk, null_mut());
        if r != 0 {
            return;
        }
        let v: u32 = if disable { 0 } else { 1 };
        RegSetValueExW(hk, name.as_ptr(), 0, REG_DWORD, &v as *const u32 as *const u8, 4);
        RegCloseKey(hk);
    }
}

// ───────────────────────── Tray, menu, autostart ─────────────────────────

fn autostart_enabled() -> bool {
    autostart_exists(APP_NAME)
}

fn remove_autostart(app: &str) {
    unsafe {
        let key = wide(RUN_KEY);
        let name = wide(app);
        let mut hk: HKEY = null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, KEY_WRITE, &mut hk) == 0 {
            RegDeleteValueW(hk, name.as_ptr());
            RegCloseKey(hk);
        }
    }
}

fn autostart_exists(app: &str) -> bool {
    unsafe {
        let key = wide(RUN_KEY);
        let name = wide(app);
        let mut hk: HKEY = null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, KEY_READ, &mut hk) != 0 {
            return false;
        }
        let r = RegQueryValueExW(hk, name.as_ptr(), null(), null_mut(), null_mut(), null_mut());
        RegCloseKey(hk);
        r == 0
    }
}

fn set_autostart(on: bool) {
    unsafe {
        let key = wide(RUN_KEY);
        let name = wide(APP_NAME);
        let mut hk: HKEY = null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, KEY_WRITE, &mut hk) != 0 {
            return;
        }
        if on {
            if let Ok(exe) = std::env::current_exe() {
                let v = wide(&format!("\"{}\"", exe.display()));
                RegSetValueExW(hk, name.as_ptr(), 0, REG_SZ, v.as_ptr() as *const u8, (v.len() * 2) as u32);
            }
        } else {
            RegDeleteValueW(hk, name.as_ptr());
        }
        RegCloseKey(hk);
    }
}

unsafe fn show_menu(hwnd: HWND) {
    let muted = settings().muted;
    let menu = CreatePopupMenu();
    let flag = |c: bool| if c { MF_STRING | MF_CHECKED } else { MF_STRING };
    AppendMenuW(menu, MF_STRING, ID_OPEN, wide(&format!("{}\t{}", tr("Open history"), bind_name(settings().keys[0]))).as_ptr());
    AppendMenuW(menu, MF_STRING, ID_SETTINGS, wide(tr("Settings")).as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    AppendMenuW(menu, flag(muted), ID_MUTE, wide(tr("Mute")).as_ptr());
    AppendMenuW(menu, flag(autostart_enabled()), ID_AUTOSTART, wide(tr("Start with Windows")).as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    AppendMenuW(menu, MF_STRING, ID_EXIT, wide(tr("Exit")).as_ptr());
    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    SetForegroundWindow(hwnd);
    // TPM_VERNEGANIMATION: the tray menu opens upward, with the system animation
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_VERNEGANIMATION | TPM_BOTTOMALIGN, pt.x, pt.y, 0, hwnd, null());
    PostMessageW(hwnd, WM_NULL, 0, 0);
    DestroyMenu(menu);
    match cmd as usize {
        ID_OPEN => show_ui_tab(0, false),
        ID_SETTINGS => show_ui_tab(2, false),
        ID_MUTE => update_settings(|s| s.muted = !s.muted),
        ID_AUTOSTART => set_autostart(!autostart_enabled()),
        ID_EXIT => {
            DestroyWindow(hwnd);
        }
        _ => {}
    }
}

/// Filled rounded rectangle (GDI+), in the icon's pixel coordinates.
unsafe fn gp_fill_rr(g: *mut GpGraphics, x: f32, y: f32, w: f32, hh: f32, r: f32, argb: u32) {
    let mut br: *mut GpSolidFill = null_mut();
    GdipCreateSolidFill(argb, &mut br);
    let d = (r * 2.0).min(w).min(hh);
    let mut p: *mut GpPath = null_mut();
    GdipCreatePath(FillModeAlternate, &mut p);
    GdipAddPathArc(p, x, y, d, d, 180.0, 90.0);
    GdipAddPathArc(p, x + w - d, y, d, d, 270.0, 90.0);
    GdipAddPathArc(p, x + w - d, y + hh - d, d, d, 0.0, 90.0);
    GdipAddPathArc(p, x, y + hh - d, d, d, 90.0, 90.0);
    GdipClosePathFigure(p);
    GdipFillPath(g, br as *mut GpBrush, p);
    GdipDeletePath(p);
    GdipDeleteBrush(br as *mut GpBrush);
}

/// Builds the tray icon: an outlined clipboard with the clip and text lines
/// (white on a dark taskbar, dark gray on a light one).
unsafe fn make_tray_icon() -> HICON {
    const S: i32 = 32;
    let screen = GetDC(null_mut());
    let mem = CreateCompatibleDC(screen);
    let mut bi: BITMAPINFO = std::mem::zeroed();
    bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bi.bmiHeader.biWidth = S;
    bi.bmiHeader.biHeight = -S;
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    let mut bits: *mut c_void = null_mut();
    let dib = CreateDIBSection(screen, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    let old = SelectObject(mem, dib as _);
    fill(mem, &RECT { left: 0, top: 0, right: S, bottom: S }, 0);
    // Outlined clipboard, with the clip and the text lines (white on black, anti-aliased)
    let mut g: *mut GpGraphics = null_mut();
    if GdipCreateFromHDC(mem, &mut g) == 0 {
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        const W: u32 = 0xFFFF_FFFF;
        const K: u32 = 0xFF00_0000;
        gp_fill_rr(g, 4.0, 5.5, 24.0, 26.0, 5.0, W); // white board, filled
        gp_fill_rr(g, 8.5, 0.5, 15.0, 9.0, 4.0, K); // gap around the clip
        gp_fill_rr(g, 10.0, 2.0, 12.0, 6.0, 3.0, W); // clip
        gp_fill_rr(g, 9.5, 13.0, 13.0, 2.6, 1.3, K); // cut-out text lines
        gp_fill_rr(g, 9.5, 18.0, 13.0, 2.6, 1.3, K);
        gp_fill_rr(g, 9.5, 23.0, 8.5, 2.6, 1.3, K);
        GdipDeleteGraphics(g);
    }
    GdiFlush();
    // white on black -> alpha (premultiplied)
    let px = std::slice::from_raw_parts_mut(bits as *mut u32, (S * S) as usize);
    // white on a dark taskbar; very dark gray on a light one (otherwise the icon would vanish)
    let (cr, cg, cb) = if taskbar_is_light() { (32u32, 32u32, 32u32) } else { (255u32, 255u32, 255u32) };
    for p in px.iter_mut() {
        let a = *p & 0xFF;
        *p = (a << 24) | ((cr * a / 255) << 16) | ((cg * a / 255) << 8) | (cb * a / 255);
    }
    let mask = CreateBitmap(S, S, 1, 1, null());
    let ii = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: dib };
    let icon = CreateIconIndirect(&ii);
    SelectObject(mem, old);
    DeleteObject(dib as _);
    DeleteObject(mask as _);
    DeleteDC(mem);
    ReleaseDC(null_mut(), screen);
    icon
}

/// true if the Windows taskbar is using the light theme.
fn taskbar_is_light() -> bool {
    unsafe {
        let key = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
        let name = wide("SystemUsesLightTheme");
        let mut hk: HKEY = null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, KEY_READ, &mut hk) != 0 {
            return false;
        }
        let mut val = 0u32;
        let mut sz = 4u32;
        let r = RegQueryValueExW(hk, name.as_ptr(), null(), null_mut(), &mut val as *mut u32 as *mut u8, &mut sz);
        RegCloseKey(hk);
        r == 0 && val == 1
    }
}

unsafe fn tray(hwnd: HWND, msg: u32) {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    // monochrome icon (clipboard) in the tray, also swapped when the Windows theme changes
    nid.hIcon = if msg == NIM_ADD || msg == NIM_MODIFY { make_tray_icon() } else { null_mut() };
    for (i, c) in wide(&format!("CoffeeCopy - {}", tr("{k} opens the history").replace("{k}", &bind_name(settings().keys[0])))).iter().take(127).enumerate() {
        nid.szTip[i] = *c;
    }
    Shell_NotifyIconW(msg, &nid);
    // the shell keeps its own copy of the icon: free ours (otherwise every refresh leaks one)
    if !nid.hIcon.is_null() {
        DestroyIcon(nid.hIcon);
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CLIPBOARDUPDATE => {
            // Wait for the source app to finish writing (and merge bursts of events).
            SetTimer(hwnd, 1, 40, None);
            0
        }
        WM_TIMER => {
            KillTimer(hwnd, 1);
            on_clipboard(hwnd);
            0
        }
        WM_SETTINGCHANGE => {
            // the Windows light/dark theme changed: rebuild the tray icon with the right color
            if lp != 0 {
                let p = lp as *const u16;
                let mut s = Vec::new();
                let mut i = 0;
                while i < 32 && *p.add(i) != 0 {
                    s.push(*p.add(i));
                    i += 1;
                }
                if String::from_utf16_lossy(&s) == "ImmersiveColorSet" {
                    tray(hwnd, NIM_MODIFY);
                }
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_HOTKEY | WM_WINV => {
            show_ui();
            0
        }
        WM_TRAY => {
            match lp as u32 {
                WM_LBUTTONUP => show_ui(),
                WM_RBUTTONUP => show_menu(hwnd),
                _ => {}
            }
            0
        }
        WM_DESTROY => {
            tray(hwnd, NIM_DELETE);
            RemoveClipboardFormatListener(hwnd);
            UnregisterHotKey(hwnd, 1);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// ───────────────────────── Demo mode (screenshots + GIF frames) ─────────────────────────
//
// `coffeecopy.exe --demo <folder>` fills a throw-away data folder with MOCK clipboard items,
// drives the UI through a scripted tour and saves screenshots / GIF frames (BMP) of the window.
// The GIF frames show the window over a generated wallpaper (also what the blur "sees"), so
// nothing from the real desktop ever ends up in the media.
// It never touches the real history, settings or registry. Used to make the README media.

fn mock_dib(w: i32, hh: i32) -> Vec<u8> {
    let mut b = Vec::with_capacity(40 + (w * hh * 4) as usize);
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&w.to_le_bytes());
    b.extend_from_slice(&hh.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&((w * hh * 4) as u32).to_le_bytes());
    b.extend_from_slice(&[0u8; 16]);
    for y in 0..hh {
        for x in 0..w {
            let fx = x as f32 / w as f32;
            let fy = y as f32 / hh as f32;
            let r = (60.0 + 190.0 * fx) as u8;
            let g = (90.0 + 120.0 * (1.0 - fy) * (0.6 + 0.4 * (fx * 9.0).sin().abs())) as u8;
            let bl = (150.0 + 90.0 * fy) as u8;
            b.extend_from_slice(&[bl, g, r, 255]);
        }
    }
    b
}

/// Margin (physical px) around the window in the GIF canvas.
const DEMO_MARGIN: i32 = 140;
/// Time between GIF frames (ms): 3 centiseconds, the finest delay GIF players honor reliably.
const DEMO_STEP: u32 = 30;

thread_local! {
    /// Demo wallpaper (canvas size, 0x00RRGGBB): the app is shown over it instead of the real desktop.
    static DEMO_WALL: RefCell<Vec<u32>> = RefCell::new(Vec::new());
}

/// Size of the GIF canvas: the normal (wide) window plus the margin on every side.
fn demo_canvas() -> (i32, i32) {
    (sc(960) + 2 * DEMO_MARGIN, sc(660) + 2 * DEMO_MARGIN)
}

/// Soft color blobs over a dark base, generated (never a capture of the real screen).
fn demo_wallpaper() -> Vec<u32> {
    let cached = DEMO_WALL.with(|w| w.borrow().clone());
    if !cached.is_empty() {
        return cached;
    }
    let (w, hh) = demo_canvas();
    let aspect = w as f32 / hh as f32;
    // (center x, center y, radius, color)
    let blobs: [(f32, f32, f32, [f32; 3]); 5] = [
        (0.12, 0.18, 0.50, [255.0, 122.0, 66.0]),
        (0.88, 0.12, 0.55, [168.0, 85.0, 230.0]),
        (0.84, 0.92, 0.60, [20.0, 126.0, 255.0]),
        (0.10, 0.96, 0.48, [255.0, 55.0, 110.0]),
        (0.50, 0.50, 0.35, [40.0, 60.0, 140.0]),
    ];
    let mut out = vec![0u32; (w * hh) as usize];
    for y in 0..hh {
        let v = y as f32 / hh as f32;
        for x in 0..w {
            let u = x as f32 / w as f32;
            let mut c = [20.0f32, 20.0, 38.0];
            for (bx, by, r, col) in blobs {
                let d2 = ((u - bx) * aspect).powi(2) + (v - by).powi(2);
                let k = (-d2 / (r * r)).exp() * 0.5; // kept soft: a GIF only has 256 colors
                for i in 0..3 {
                    c[i] += (col[i] - c[i]) * k;
                }
            }
            out[(y * w + x) as usize] = ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32;
        }
    }
    DEMO_WALL.with(|wl| *wl.borrow_mut() = out.clone());
    out
}

/// What is "behind the window" in the demo: the wallpaper under the centered window, at snapshot size.
fn demo_backdrop_source() -> Vec<u32> {
    let wall = demo_wallpaper();
    let (cw, ch) = demo_canvas();
    let (w, hh) = (pw(), ph());
    let (x0, y0) = ((cw - w) / 2, (ch - hh) / 2);
    let (sw, sh) = (snap_w_fn(), snap_h_fn());
    let mut px = Vec::with_capacity((sw * sh) as usize);
    for j in 0..sh {
        let y = (y0 + (j * hh + hh / 2) / sh).clamp(0, ch - 1);
        for i in 0..sw {
            let x = (x0 + (i * w + w / 2) / sw).clamp(0, cw - 1);
            px.push(wall[(y * cw + x) as usize]);
        }
    }
    px
}

/// Renders the window (with the search box) into pixels: (width, height, 0x00RRGGBB).
unsafe fn demo_grab() -> (i32, i32, Vec<u32>) {
    let ui = h(&UI);
    InvalidateRect(ui, null(), 0);
    UpdateWindow(ui);
    UpdateWindow(h(&EDIT));
    DwmFlush();
    let (w, hh) = (pw(), ph());
    let screen = GetDC(null_mut());
    let mem = CreateCompatibleDC(screen);
    let mut bi: BITMAPINFO = std::mem::zeroed();
    bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bi.bmiHeader.biWidth = w;
    bi.bmiHeader.biHeight = -hh;
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    let mut bits: *mut c_void = null_mut();
    let dib = CreateDIBSection(screen, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    let old = SelectObject(mem, dib as _);
    PrintWindow(ui, mem, 2); // PW_RENDERFULLCONTENT: includes the layered window and the search box
    GdiFlush();
    let px: Vec<u32> = std::slice::from_raw_parts(bits as *const u32, (w * hh) as usize)
        .iter()
        .map(|p| p & 0x00FF_FFFF)
        .collect();
    SelectObject(mem, old);
    DeleteObject(dib as _);
    DeleteDC(mem);
    ReleaseDC(null_mut(), screen);
    (w, hh, px)
}

/// Writes a top-down 32-bit BMP.
fn write_bmp(path: &Path, w: i32, hh: i32, px: &[u32]) {
    let mut f: Vec<u8> = Vec::with_capacity(54 + px.len() * 4);
    f.extend_from_slice(b"BM");
    f.extend_from_slice(&((54 + px.len() * 4) as u32).to_le_bytes());
    f.extend_from_slice(&[0, 0, 0, 0]);
    f.extend_from_slice(&54u32.to_le_bytes());
    f.extend_from_slice(&40u32.to_le_bytes());
    f.extend_from_slice(&w.to_le_bytes());
    f.extend_from_slice(&(-hh).to_le_bytes());
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&32u16.to_le_bytes());
    f.extend_from_slice(&[0u8; 24]);
    for p in px {
        f.extend_from_slice(&(p | 0xFF00_0000).to_le_bytes());
    }
    let _ = std::fs::write(path, f);
}

/// Jumps the list animation to its final state and stops the tab transition.
fn demo_settle() {
    LST.with(|l| {
        let mut l = l.borrow_mut();
        l.scroll = l.target;
        l.hl = l.sel.max(0) as f32 * row_h_fn() as f32;
        l.animating = false;
    });
    TAB_ANIM.with(|a| *a.borrow_mut() = None);
}

/// A still screenshot of the window alone.
unsafe fn demo_snap(path: &Path) {
    demo_settle();
    let (w, hh, px) = demo_grab();
    write_bmp(path, w, hh, &px);
}

/// One step of the list animation with a fixed time step (the demo has no frame clock).
fn demo_step_list(dt: f32) {
    LST.with(|l| {
        let mut l = l.borrow_mut();
        let fs = 1.0 - (-dt * 26.0).exp();
        let fh = 1.0 - (-dt * 34.0).exp();
        l.scroll += (l.target - l.scroll) * fs;
        let hl_target = l.sel.max(0) as f32 * row_h_fn() as f32;
        l.hl += (hl_target - l.hl) * fh;
        l.animating = false;
    });
}

fn mix_px(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| {
        let (x, y) = (((a >> s) & 0xFF) as f32, ((b >> s) & 0xFF) as f32);
        ((x + (y - x) * t).round() as u32) << s
    };
    ch(16) | ch(8) | ch(0)
}

/// Composes the window over the demo wallpaper, with a soft shadow.
/// `v`: 0 = closed (only the wallpaper), 1 = fully open; in between it fades and slides in.
unsafe fn demo_compose(v: f32) -> Vec<u32> {
    let (cw, ch) = demo_canvas();
    let mut out = demo_wallpaper();
    if v <= 0.0 {
        return out;
    }
    let (w, hh, win) = demo_grab();
    let x0 = (cw - w) / 2;
    let y0 = (ch - hh) / 2 + ((1.0 - v) * 44.0 * scale()) as i32;
    // shadow: falls off with the distance to the window, pushed down a little
    const SPREAD: f32 = 110.0;
    const DROP: i32 = 28;
    let (sx0, sy0, sx1, sy1) = (x0, y0 + DROP, x0 + w - 1, y0 + DROP + hh - 1);
    for y in (sy0 - SPREAD as i32).max(0)..(sy1 + SPREAD as i32 + 1).min(ch) {
        for x in (sx0 - SPREAD as i32).max(0)..(sx1 + SPREAD as i32 + 1).min(cw) {
            let dx = (sx0 - x).max(x - sx1).max(0) as f32;
            let dy = (sy0 - y).max(y - sy1).max(0) as f32;
            let d = (dx * dx + dy * dy).sqrt();
            if d < SPREAD {
                let s = 1.0 - d / SPREAD;
                let i = (y * cw + x) as usize;
                out[i] = mix_px(out[i], 0, 0.55 * s * s * v);
            }
        }
    }
    for y in 0..hh {
        let cy = y0 + y;
        if cy < 0 || cy >= ch {
            continue;
        }
        for x in 0..w {
            let i = (cy * cw + x0 + x) as usize;
            let p = win[(y * w + x) as usize];
            out[i] = if v >= 1.0 { p } else { mix_px(out[i], p, v) };
        }
    }
    out
}

/// Records the GIF frames: animated steps are written as images, pauses only as durations.
struct DemoRec {
    dir: PathBuf,
    /// Duration (ms) of each written frame
    durations: Vec<u32>,
    /// Last composed canvas (full size), the starting point of cross-fades
    last: Vec<u32>,
}

impl DemoRec {
    /// Writes one frame (the canvas halved: 2x2 pixels averaged, which also smooths the edges).
    fn emit(&mut self, canvas: Vec<u32>) {
        let (cw, ch) = demo_canvas();
        let (ow, oh) = (cw / 2, ch / 2);
        let mut out = Vec::with_capacity((ow * oh) as usize);
        for y in 0..oh {
            for x in 0..ow {
                let at = |dx: i32, dy: i32| canvas[((y * 2 + dy) * cw + x * 2 + dx) as usize];
                let q = [at(0, 0), at(1, 0), at(0, 1), at(1, 1)];
                let ch = |s: u32| (q.iter().map(|p| (p >> s) & 0xFF).sum::<u32>() / 4) << s;
                out.push(ch(16) | ch(8) | ch(0));
            }
        }
        write_bmp(&self.dir.join(format!("f{:04}.bmp", self.durations.len())), ow, oh, &out);
        self.durations.push(DEMO_STEP);
        self.last = canvas;
    }
    /// The window as it is now.
    unsafe fn frame(&mut self) {
        self.emit(demo_compose(1.0));
    }
    /// Keeps the last frame on screen for `ms` more.
    fn hold(&mut self, ms: u32) {
        if let Some(d) = self.durations.last_mut() {
            *d += ms;
        }
    }
    /// Lets the list animation (scroll + highlight) run for `ms`.
    unsafe fn play(&mut self, ms: u32) {
        for _ in 0..(ms / DEMO_STEP).max(1) {
            demo_step_list(DEMO_STEP as f32 / 1000.0);
            self.frame();
        }
        demo_settle();
    }
    /// Switches tab with the real transition (sliding indicator, content fading in).
    unsafe fn tab(&mut self, t: usize) {
        let prev = TAB.load(Relaxed);
        switch_tab(t);
        TAB_FROM.store(prev, Relaxed);
        let steps = (TAB_ANIM_MS as u32 / DEMO_STEP) + 1;
        for k in 1..=steps {
            let t0 = Instant::now().checked_sub(Duration::from_millis((k * DEMO_STEP) as u64));
            TAB_ANIM.with(|a| *a.borrow_mut() = t0);
            demo_step_list(DEMO_STEP as f32 / 1000.0);
            self.frame();
        }
        demo_settle();
        self.frame();
    }
    /// Cross-fades from the last frame to the window as it is now (for changes with no animation of their own).
    unsafe fn xfade(&mut self, ms: u32) {
        demo_settle();
        let from = std::mem::take(&mut self.last);
        let to = demo_compose(1.0);
        let steps = (ms / DEMO_STEP).max(1);
        for k in 1..steps {
            let t = k as f32 / steps as f32;
            let t = t * t * (3.0 - 2.0 * t);
            let mid: Vec<u32> = from.iter().zip(&to).map(|(a, b)| mix_px(*a, *b, t)).collect();
            self.emit(mid);
        }
        self.emit(to);
    }
    /// The window fades and slides in.
    unsafe fn open(&mut self, ms: u32) {
        demo_settle();
        let steps = (ms / DEMO_STEP).max(1);
        for k in 0..=steps {
            self.emit(demo_compose(ease_out(k as f32 / steps as f32)));
        }
    }
    /// The window fades and slides out.
    unsafe fn close(&mut self, ms: u32) {
        demo_settle();
        let steps = (ms / DEMO_STEP).max(1);
        for k in 1..=steps {
            let e = k as f32 / steps as f32;
            self.emit(demo_compose(1.0 - e * e * e));
        }
    }
    /// Writes the frame list for ffmpeg (concat demuxer).
    fn finish(&self) {
        let mut list = String::new();
        for (i, d) in self.durations.iter().enumerate() {
            list += &format!("file 'f{i:04}.bmp'\nduration {:.3}\n", *d as f32 / 1000.0);
        }
        if !self.durations.is_empty() {
            list += &format!("file 'f{:04}.bmp'\n", self.durations.len() - 1);
        }
        let _ = std::fs::write(self.dir.join("list.txt"), list);
    }
}

unsafe fn demo_search(q: &str) {
    SetWindowTextW(h(&EDIT), wide(q).as_ptr());
}

unsafe fn run_demo(out: &Path) {
    let frames = out.join("frames");
    let _ = std::fs::create_dir_all(&frames);
    let snap = |name: &str| {
        demo_snap(&out.join(format!("{name}.bmp")));
    };

    // ── mock data ──
    let items = [
        "The quick brown fox jumps over the lazy dog.",
        "docker run --rm -it -p 3000:3000 node:20-alpine sh",
        "https://developer.mozilla.org/en-US/docs/Web/API/Clipboard_API",
        "Remember to rotate the API keys before Friday",
        "{\"status\": \"ok\", \"items\": 128, \"page\": 1}",
        "ssh deploy@staging.example.com -p 2222",
        "192.168.1.42:8080",
        "Dear team, please find the quarterly report attached. Let me know if you have any questions.",
        "const total = items.reduce((sum, item) => sum + item.price * item.qty, 0);",
        "git commit -m \"fix: handle empty clipboard on startup\"",
        "Meeting notes: ship v2.1, review the API changes, update the docs",
        "SELECT id, name, email FROM users WHERE active = true ORDER BY created_at DESC LIMIT 20;",
        "npm install --save-dev typescript eslint prettier",
        "https://github.com/example/awesome-project",
    ];
    HIST.with(|hs| {
        let mut hs = hs.borrow_mut();
        for t in items {
            hs.add(text_hash(t), Kind::Text, t.to_string(), None);
        }
        let dib = mock_dib(640, 400);
        hs.add(fnv(CF_DIB, &dib), Kind::Image, "[Image 640x400]".to_string(), Some(&dib));
    });
    FAVS.with(|f| {
        let mut f = f.borrow_mut();
        for t in [items[2], items[9], items[12]] {
            f.insert(text_hash(t));
        }
        save_favs(&f);
    });
    // the media shows the app exactly as it comes out of the box
    update_settings(|s| *s = Settings::default());
    LANG.store(0, Relaxed);
    apply_language();
    apply_theme();
    apply_layout();
    apply_corner();
    compute_pos();
    place_window(1.0);
    ShowWindow(h(&UI), SW_SHOWNA);
    switch_tab(0);
    set_sel(0);
    demo_settle();

    // ── GIF: a calm tour of the app ──
    let mut rec = DemoRec { dir: frames, durations: Vec::new(), last: Vec::new() };
    rec.emit(demo_compose(0.0));
    rec.hold(300);
    rec.open(330);
    rec.hold(800);
    for i in 1..=4 {
        set_sel(i);
        rec.play(150);
        rec.hold(90);
    }
    rec.hold(500);
    // fuzzy search, typed one letter at a time
    for q in ["g", "gi", "git"] {
        demo_search(q);
        rec.xfade(120);
        rec.hold(200);
    }
    rec.hold(900);
    snap("search");
    demo_search("");
    rec.xfade(270);
    rec.hold(300);
    // star an item, then look at the favorites tab
    set_sel(3);
    rec.play(210);
    rec.hold(250);
    if let Some(e) = selected_entry() {
        toggle_fav(e.hash);
    }
    rec.xfade(180);
    rec.hold(600);
    rec.tab(1);
    rec.hold(1000);
    snap("favorites");
    rec.tab(0);
    set_sel(1);
    rec.play(180);
    rec.hold(400);
    snap("history");
    // an image item, with its preview
    demo_search("image");
    rec.xfade(270);
    rec.hold(1100);
    snap("image");
    demo_search("");
    set_sel(1);
    rec.xfade(270);
    rec.hold(300);

    // settings
    rec.tab(2);
    rec.hold(900);
    snap("settings-appearance");
    SUB.store(2, Relaxed);
    rec.xfade(270);
    rec.hold(900);
    snap("settings-sounds");
    SUB.store(4, Relaxed);
    rec.xfade(270);
    rec.hold(500);
    RECORDING.store(1, Relaxed); // shows the "press keys" state
    rec.xfade(180);
    rec.hold(900);
    snap("settings-keybinds");
    RECORDING.store(-1, Relaxed);
    // themes
    SUB.store(0, Relaxed);
    rec.xfade(270);
    rec.hold(400);
    update_settings(|s| s.theme = 2);
    apply_theme();
    rec.xfade(390);
    rec.hold(800);
    snap("theme-crimson");
    rec.tab(0);
    set_sel(1);
    rec.play(150);
    rec.hold(800);
    snap("history-crimson");
    update_settings(|s| s.theme = 1);
    apply_theme();
    rec.xfade(390);
    rec.hold(900);
    snap("theme-oled");
    update_settings(|s| s.theme = 0);
    apply_theme();
    rec.xfade(390);
    rec.hold(500);

    // compact mode
    update_settings(|s| s.compact = true);
    apply_layout();
    switch_tab(0);
    set_sel(1);
    rec.xfade(420);
    rec.hold(700);
    snap("compact");
    for i in 2..=5 {
        set_sel(i);
        rec.play(150);
        rec.hold(90);
    }
    rec.hold(600);
    switch_tab(2);
    SUB.store(0, Relaxed);
    snap("compact-settings");
    update_settings(|s| s.compact = false);
    apply_layout();
    switch_tab(0);
    set_sel(1);
    rec.xfade(420);
    rec.hold(900);
    rec.close(300);
    rec.hold(500);
    rec.finish();

    // ── stills that are not part of the GIF ──
    switch_tab(2);
    for (i, name) in [(1usize, "settings-window"), (3, "settings-behavior"), (5, "settings-general"), (6, "settings-data")] {
        SUB.store(i, Relaxed);
        snap(name);
    }
    SUB.store(0, Relaxed);
    for (code, l) in [("ru", 2usize), ("zh", 3), ("ja", 4), ("pt", 1)] {
        update_settings(|s| s.lang = l);
        LANG.store(l, Relaxed);
        apply_language();
        switch_tab(0);
        set_sel(1);
        snap(&format!("lang-{code}-history"));
        switch_tab(2);
        SUB.store(3, Relaxed);
        snap(&format!("lang-{code}-behavior"));
        SUB.store(0, Relaxed);
    }
    let _ = std::fs::write(out.join("DONE.txt"), format!("{} frames", rec.durations.len()));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let demo_dir: Option<PathBuf> = args
        .iter()
        .position(|a| a == "--demo")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);
    if let Some(d) = &demo_dir {
        // throw-away data folder: the demo must never touch the real history/settings
        let data = d.join("_data");
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::create_dir_all(data.join("img"));
        std::env::set_var("COFFEECOPY_DATA", &data);
    }
    let demo = demo_dir.is_some();
    DEMO.store(demo, Relaxed);
    unsafe {
        // Single instance (the demo has its own, so it can run next to the real app)
        let mutex_name = if demo { "Local\\CoffeeCopyDemoInstance" } else { "Local\\CoffeeCopySingleInstance" };
        let m = CreateMutexW(null(), 1, wide(mutex_name).as_ptr());
        if m.is_null() || GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        // Sharpness: the app handles its own DPI (otherwise Windows stretches the window and blurs it)
        SetProcessDPIAware();
        let screen = GetDC(null_mut());
        let dpi = GetDeviceCaps(screen, LOGPIXELSX as _);
        ReleaseDC(null_mut(), screen);
        // demo: a fixed 2x scale gives crisp, predictable screenshots
        SCALE_BITS.store(if demo { 2.0f32 } else { dpi.max(96) as f32 / 96.0 }.to_bits(), Relaxed);
        // GDI+ (anti-aliased shapes)
        let mut gdip_token = 0usize;
        let gdip_in = GdiplusStartupInput {
            GdiplusVersion: 1,
            DebugEventCallback: 0,
            SuppressBackgroundThread: 0,
            SuppressExternalCodecs: 0,
        };
        GdiplusStartup(&mut gdip_token, &gdip_in, null_mut());
        if !demo {
            // If "Start with Windows" is on, refresh the exe path (needed after moving or renaming the app).
            if autostart_exists(APP_NAME) {
                set_autostart(true);
            }
            // Migration from the old name (CopySound): data folder and startup entry
            history::migrate_old_data();
            if autostart_exists(OLD_APP_NAME) {
                remove_autostart(OLD_APP_NAME);
                set_autostart(true);
            }
        }
        let hinst = GetModuleHandleW(null());
        let class = wide("CoffeeCopyWnd");
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: null_mut(),
            hCursor: null_mut(),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0,
            null_mut(), null_mut(), hinst, null(),
        );
        if hwnd.is_null() {
            return;
        }
        set(&MAIN, hwnd);
        COMPACT.store(settings().compact, Relaxed);
        LANG.store(settings().lang, Relaxed);
        HIST.with(|_| {}); // loads (and compacts) the history at startup
        create_ui(hinst);
        if let Some(dir) = demo_dir {
            run_demo(&dir);
            return;
        }
        std::thread::spawn(frame_thread);
        register_hotkey();
        apply_winv();
        AddClipboardFormatListener(hwnd);
        tray(hwnd, NIM_ADD);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
