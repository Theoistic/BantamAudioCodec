//! Puts the program's icon (`bac_player::icon`) on `bac.exe`; other targets build as they are.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../bac-player/src/icon.rs");
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    // the MSVC linker takes a .res file as it is
    if var("CARGO_CFG_TARGET_OS") == "windows" && var("CARGO_CFG_TARGET_ENV") == "msvc" {
        let res = std::path::Path::new(&var("OUT_DIR")).join("bac.res");
        std::fs::write(&res, bac_player::icon::res()).expect("the icon resource could not be written");
        println!("cargo:rustc-link-arg-bins={}", res.display());
    }
}
