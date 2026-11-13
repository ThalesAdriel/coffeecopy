//! Embeds the icon (assets/coffeecopy.ico) into the executable using MinGW's `windres`.
use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/coffeecopy.ico");
    println!("cargo:rerun-if-changed=app.rc");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let obj = PathBuf::from(env::var("OUT_DIR").unwrap()).join("app_res.o");
    let ok = Command::new("windres")
        .args(["--input", "app.rc", "--output-format=coff", "--output"])
        .arg(&obj)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        println!("cargo:rustc-link-arg-bins={}", obj.display());
    } else {
        println!("cargo:warning=windres not found: the icon was not embedded in the executable");
    }
}
