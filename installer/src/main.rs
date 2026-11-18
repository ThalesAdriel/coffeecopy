//! CoffeeCopy installer / uninstaller (a single .exe, no dependencies).
//!
//! Usage:
//!   CoffeeCopy-Setup.exe                installs (asks first)
//!   CoffeeCopy-Setup.exe --silent       installs without asking
//!   uninstall.exe --uninstall           uninstalls (lives in the install folder)
//! Test options: --dir <folder>, --no-integration (does not touch shortcuts, registry or processes).
#![windows_subsystem = "windows"]

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use std::{env, fs};

use windows_sys::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDYES, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK, MB_YESNO,
};

/// The app, embedded in the installer (build CoffeeCopy in release first).
static APP_EXE: &[u8] = include_bytes!("../../target/release/coffeecopy.exe");

const NO_WINDOW: u32 = 0x0800_0000;
const APP: &str = "CoffeeCopy";
const EXE: &str = "coffeecopy.exe";
const UNINSTALL_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\CoffeeCopy";
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn msgbox(text: &str, flags: u32) -> i32 {
    unsafe { MessageBoxW(std::ptr::null_mut(), wide(text).as_ptr(), wide("CoffeeCopy").as_ptr(), flags) }
}

fn run(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .creation_flags(NO_WINDOW)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn reg_add(key: &str, name: &str, kind: &str, data: &str) {
    run("reg", &["add", key, "/v", name, "/t", kind, "/d", data, "/f"]);
}

fn start_menu_link() -> PathBuf {
    PathBuf::from(env::var_os("APPDATA").unwrap_or_default())
        .join(r"Microsoft\Windows\Start Menu\Programs")
        .join(format!("{APP}.lnk"))
}

fn kill_app() {
    run("taskkill", &["/F", "/IM", EXE]);
    std::thread::sleep(Duration::from_millis(700));
}

fn create_shortcut(link: &Path, target: &Path, dir: &Path) {
    let q = |p: &Path| p.display().to_string().replace('\'', "''");
    let script = format!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');$s.TargetPath='{}';\
         $s.WorkingDirectory='{}';$s.Description='CoffeeCopy - clipboard history';$s.Save()",
        q(link), q(target), q(dir)
    );
    run("powershell", &["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script]);
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn install(args: &[String]) {
    let silent = args.iter().any(|a| a == "--silent");
    let no_int = args.iter().any(|a| a == "--no-integration");
    let dir = arg_value(args, "--dir").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env::var_os("LOCALAPPDATA").unwrap_or_default()).join("Programs").join(APP)
    });

    if !silent
        && msgbox(
            &format!("Install CoffeeCopy to:\n\n{}\n\nYour data and settings are kept if they already exist.", dir.display()),
            MB_YESNO | MB_ICONQUESTION,
        ) != IDYES
    {
        return;
    }

    if !no_int {
        kill_app(); // update: the old exe must not be in use
    }
    if let Err(e) = fs::create_dir_all(&dir) {
        msgbox(&format!("Could not create the installation folder:\n{e}"), MB_OK | MB_ICONERROR);
        return;
    }
    let exe = dir.join(EXE);
    let mut written = Err(std::io::Error::other("not written"));
    for _ in 0..10 {
        written = fs::write(&exe, APP_EXE);
        if written.is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    if let Err(e) = written {
        msgbox(&format!("Could not write CoffeeCopy:\n{e}"), MB_OK | MB_ICONERROR);
        return;
    }
    // the installer itself becomes the uninstaller
    if let Ok(me) = env::current_exe() {
        let _ = fs::copy(me, dir.join("uninstall.exe"));
    }

    if !no_int {
        create_shortcut(&start_menu_link(), &exe, &dir);

        let version = env!("CARGO_PKG_VERSION");
        let unin = format!("\"{}\" --uninstall", dir.join("uninstall.exe").display());
        reg_add(UNINSTALL_KEY, "DisplayName", "REG_SZ", APP);
        reg_add(UNINSTALL_KEY, "DisplayVersion", "REG_SZ", version);
        reg_add(UNINSTALL_KEY, "Publisher", "REG_SZ", APP);
        reg_add(UNINSTALL_KEY, "InstallLocation", "REG_SZ", &dir.display().to_string());
        reg_add(UNINSTALL_KEY, "DisplayIcon", "REG_SZ", &exe.display().to_string());
        reg_add(UNINSTALL_KEY, "UninstallString", "REG_SZ", &unin);
        reg_add(UNINSTALL_KEY, "NoModify", "REG_DWORD", "1");
        reg_add(UNINSTALL_KEY, "NoRepair", "REG_DWORD", "1");
        reg_add(UNINSTALL_KEY, "EstimatedSize", "REG_DWORD", &(APP_EXE.len() / 1024 * 2).to_string());

        // if "Start with Windows" was already on, point it to the installed exe
        if run("reg", &["query", RUN_KEY, "/v", APP]) {
            reg_add(RUN_KEY, APP, "REG_SZ", &format!("\"{}\"", exe.display()));
        }
    }

    if silent {
        return;
    }
    if msgbox("CoffeeCopy installed!\n\nOpen it now? (Ctrl+Alt+H opens the history.)", MB_YESNO | MB_ICONINFORMATION) == IDYES
        && !no_int
    {
        let _ = Command::new(&exe).current_dir(&dir).spawn();
    }
}

fn uninstall(args: &[String]) {
    let silent = args.iter().any(|a| a == "--silent");
    let no_int = args.iter().any(|a| a == "--no-integration");
    let Ok(me) = env::current_exe() else { return };
    let Some(dir) = me.parent().map(Path::to_path_buf) else { return };
    // Safety: this deletes the whole folder it runs from, so it must be the installed
    // uninstall.exe sitting next to the app (never a Setup.exe started from e.g. Downloads).
    let is_uninstaller = me.file_name().map(|n| n.eq_ignore_ascii_case("uninstall.exe")).unwrap_or(false);
    if !is_uninstaller || !dir.join(EXE).exists() {
        if !silent {
            msgbox("Run uninstall.exe from the CoffeeCopy installation folder to remove it.", MB_OK | MB_ICONERROR);
        }
        return;
    }

    if !silent
        && msgbox(&format!("Remove CoffeeCopy from:\n\n{}?", dir.display()), MB_YESNO | MB_ICONQUESTION) != IDYES
    {
        return;
    }

    if !no_int {
        kill_app();
        let _ = fs::remove_file(start_menu_link());
        run("reg", &["delete", RUN_KEY, "/v", APP, "/f"]);
        run("reg", &["delete", UNINSTALL_KEY, "/f"]);
    }
    let data = PathBuf::from(env::var_os("APPDATA").unwrap_or_default()).join(APP);
    if !silent
        && data.exists()
        && msgbox(
            "Also remove the history, favorites and settings?\n\n(If you choose No, they are kept in case you reinstall.)",
            MB_YESNO | MB_ICONQUESTION,
        ) == IDYES
    {
        let _ = fs::remove_dir_all(&data);
    }

    let _ = fs::remove_file(dir.join(EXE));
    // this very file is in use: a separate task deletes the folder right after we exit
    // (raw_arg: cmd has its own quoting rules; the default Rust escaping would break the command)
    let cleanup = format!("/C ping 127.0.0.1 -n 4 > nul & rmdir /s /q \"{}\"", dir.display());
    let _ = Command::new("cmd").raw_arg(cleanup).creation_flags(NO_WINDOW | 0x0000_0008).spawn();

    if !silent {
        msgbox("CoffeeCopy was removed.", MB_OK | MB_ICONINFORMATION);
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|a| a == "--uninstall") {
        uninstall(&args);
    } else {
        install(&args);
    }
}
