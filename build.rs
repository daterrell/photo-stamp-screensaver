//! Bakes the version (from the git tag, scripts/version.sh) into the program, and on Windows
//! builds compiles `assets/screensaver.rc` (name, icon, manifest, settings dialog) with mingw's
//! windres and links it in.

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=PSS_VERSION");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/tags");
    let version = Command::new("scripts/version.sh")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "0.0.0-dev".into());
    println!("cargo:rustc-env=PSS_VERSION={version}");

    for file in ["screensaver.rc", "screensaver.manifest", "photo-stamp.ico"] {
        println!("cargo:rerun-if-changed=assets/{file}");
    }
    println!("cargo:rerun-if-env-changed=WINDRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "x86_64".into());
    let windres = std::env::var("WINDRES").unwrap_or_else(|_| format!("{arch}-w64-mingw32-windres"));
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("screensaver-res.o");
    // The file properties' version: 0.7.1-dev.3+g1b37dbe → 0.7.1.
    let mut numbers = version.split(['.', '-', '+']).map(|n| n.parse::<u16>().unwrap_or(0));
    let defines =
        ["VER_MAJOR", "VER_MINOR", "VER_PATCH"].map(|d| format!("-D{d}={}", numbers.next().unwrap_or(0)));
    let status = Command::new(&windres)
        .current_dir("assets")
        .args(defines)
        .args(["--input", "screensaver.rc", "--output-format=coff", "--output"])
        .arg(&out)
        .status();
    match status {
        Ok(s) if s.success() => println!("cargo:rustc-link-arg-bins={}", out.display()),
        Ok(s) => panic!("{windres} failed ({s}) compiling assets/screensaver.rc"),
        // E.g. building natively with MSVC. The settings dialog is a resource, so this build can't
        // show it.
        Err(e) => println!("cargo:warning=no Windows resources (dialog, icon, manifest): {windres}: {e}"),
    }
}
