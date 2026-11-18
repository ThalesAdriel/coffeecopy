<p align="center">
  <img src="assets/coffeecopy.svg" width="120" alt="CoffeeCopy icon">
</p>

<h1 align="center">CoffeeCopy</h1>

<p align="center">A fast, local, searchable clipboard history for Windows.</p>

<p align="center"><b>English</b> · <a href="README.pt-BR.md">Português (Brasil)</a></p>

![CoffeeCopy demo](docs/demo.gif)

> The GIF and every screenshot in this README use **mock data**: fake items made up for the demo, not a real clipboard.

## Why this exists

The built-in Windows clipboard history (`Win+V`) is garbage.

I have to use Windows for work, and I got tired of the bullshit that is the native clipboard. Per Microsoft's own [documentation](https://support.microsoft.com/en-us/windows/apps/using-the-clipboard), it keeps only the **last 25 items**, **wipes everything on restart** (except pinned items), caps each item at **4 MB**, and handles only text, HTML and bitmaps. There is no search.

So I used Claude Sonnet 5.5's help to build something relatively better. That's CoffeeCopy.

## Why the name?

Because I think it sounds cool, even though it doesn't make that much sense.

## Screenshots

| History | Fuzzy search |
| --- | --- |
| ![History](docs/screenshots/history.png) | ![Search](docs/screenshots/search.png) |

| Favorites | Image preview |
| --- | --- |
| ![Favorites](docs/screenshots/favorites.png) | ![Image](docs/screenshots/image.png) |

| Settings: appearance | Settings: sounds |
| --- | --- |
| ![Appearance](docs/screenshots/settings-appearance.png) | ![Sounds](docs/screenshots/settings-sounds.png) |

| Settings: keybinds | Compact mode |
| --- | --- |
| ![Keybinds](docs/screenshots/settings-keybinds.png) | ![Compact](docs/screenshots/compact.png) |

| Crimson theme | OLED black theme |
| --- | --- |
| ![Crimson](docs/screenshots/history-crimson.png) | ![OLED](docs/screenshots/theme-oled.png) |

| Light theme with another accent color | Settings: data (automatic cleanup) |
| --- | --- |
| ![Light](docs/screenshots/theme-light.png) | ![Data](docs/screenshots/settings-data.png) |

| Japanese | Russian | Chinese |
| --- | --- | --- |
| ![Japanese](docs/screenshots/lang-ja-history.png) | ![Russian](docs/screenshots/lang-ru-history.png) | ![Chinese](docs/screenshots/lang-zh-history.png) |

## Features

**History and search**
- Keeps everything you copy (text and images), with no fixed item limit (the list shows up to 5,000 results at a time), and keeps it across restarts.
- Fuzzy search: type a few letters, or several words, and the best matches come first.
- Search filters: text only, images only, or only what you copied today.
- Favorites: star an item and find it in its own tab. Favorites survive "Clear history".
- Preview panel for text and images. Delete single items or clear the whole history.
- Optional automatic cleanup (off by default): delete items you have not used for a week, a month, three months or a year. Favorites are never deleted.

**Pasting**
- Pick an item with `Enter` or a click and it is pasted into the app you were using.
- Optional "copy only" mode: put the item on the clipboard and paste it yourself.
- Confirm with a double click (default) or a single click. Hovering an item selects it.

**Sound feedback**
- A different sound for a *new* copy, a *repeat* of the current item, something *copied before*, and a *paste*.
- Six sound packs (Camera / cards, Kenney, Paper / pages, Corporate, 8-bit / Arcade, Hi-Tech), a volume slider and a mute switch.

**Look and feel**
- Four themes (Dark, OLED black, Crimson, Light), an accent color of your choice, window transparency, a blur-behind effect and an adjustable corner radius.
- Compact mode: a narrow window with just the list.
- Out of the box the app starts with blur on, 77% transparency and square corners.
- Smooth animations (fade or slide), crisp on high-DPI screens, with anti-aliased shapes.
- Opens where you want: screen center, next to the cursor, top, or bottom-right corner, on the monitor you are using.

**Keyboard**
- A global shortcut to open it (default `Ctrl+Alt+H`).
- Every shortcut can be remapped in Settings → Keybinds.
- Optional: take over `Win+V` from Windows, and turn off the Windows clipboard history.
- If another app already owns the shortcut, CoffeeCopy tells you (a tray notification and a message in Settings → Keybinds) instead of failing silently.

**Windows integration**
- Tray icon (it adapts to the light or dark taskbar), start with Windows, per-user installer with an uninstaller.

**Languages**
- English (native), Português (Brasil), Русский, 中文 and 日本語. Switch in Settings → General.

## Keyboard shortcuts

| Action | Default | Remappable |
| --- | --- | --- |
| Open / close CoffeeCopy (global) | `Ctrl+Alt+H` | yes |
| Paste or copy the selected item | `Enter` | no |
| Favorite the selected item | `Ctrl+D` | yes |
| Delete the selected item | `Ctrl+Delete` | yes |
| Next tab (`Shift` + key: previous tab) | `Tab` | yes |
| Move the selection | `↑` `↓` `PgUp` `PgDn` | no |
| Close | `Esc` | no |
| Select all in the search box | `Ctrl+A` | no |

## Install

1. Download `CoffeeCopy-Setup.exe`
2. Run it. It installs for the current user in `%LOCALAPPDATA%\Programs\CoffeeCopy`. No administrator rights needed.
3. Press `Ctrl+Alt+H`.

The installer is not code-signed, so Windows SmartScreen may warn you the first time: click **More info → Run anyway**.

Silent install: `CoffeeCopy-Setup.exe --silent`. Uninstall from Windows Settings → Apps, or run `uninstall.exe` in the install folder.

## Build from source

You need:
- Rust (stable) with the `x86_64-pc-windows-gnu` target.
- MinGW's `windres` on your `PATH` (it embeds the icon; without it the build still works, just with no icon).

```
cargo build --release
```

The installer embeds the release build, so build the app first:

```
cd installer
cargo build --release
```

Run the translation tests with `cargo test`.

## Demo mode

`coffeecopy.exe --demo <folder>` fills a throw-away data folder with mock clipboard items, walks through the whole UI and saves screenshots and GIF frames (BMP) into `<folder>`. It never touches your real history, settings or registry. That is how the media in this README was made.

You can also point the app at another data folder with the `COFFEECOPY_DATA` environment variable.

## Your data

Everything stays on your machine, in `%APPDATA%\CoffeeCopy` (history, favorites, settings, images). Nothing is sent anywhere.

Heads up: the history is stored **unencrypted**, and a clipboard can hold passwords. Clear it from Settings → Data whenever you need to.

## Credits

- Sounds are from [OpenGameArt](https://opengameart.org): Kenney, Brandon Morris, PWL, starninjas, Cockatrice, Little Robot, and the 8-bit and Hi-Tech packs, all CC0. The paper-crumple sound is by Nicole Marie T, [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
- Icons are from [Font Awesome Free 7](https://fontawesome.com) (CC BY 4.0).
- UI translations follow Microsoft's own Windows terminology (clipboard history, settings, paste, delete) in Russian, Chinese and Japanese.
- Built with Claude Sonnet 5.5's help.
