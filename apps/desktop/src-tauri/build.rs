use std::path::Path;

fn main() {
    ensure_sidecar_placeholder();
    tauri_build::build()
}

/// `tauri-build` refuses to run when a `bundle.externalBin` file is missing,
/// which would break `cargo check`/`clippy`/`test` on a fresh checkout. The
/// real `rocket` CLI is produced by `scripts/prepare-sidecar.mjs` (the
/// `beforeBuildCommand`/`beforeDevCommand` hook, which always overwrites it),
/// so an empty stand-in is enough for everything that does not bundle.
fn ensure_sidecar_placeholder() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET");
    let exe = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    let path = Path::new("binaries").join(format!("rocket-{target}{exe}"));
    if path.exists() {
        return;
    }
    std::fs::create_dir_all("binaries").expect("create binaries/");
    std::fs::write(&path, b"").expect("write the sidecar placeholder");
}
