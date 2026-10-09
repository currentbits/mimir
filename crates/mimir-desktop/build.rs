fn main() {
    // ScreenCaptureKit's Swift bridge needs the system Swift runtime (/usr/lib/swift, part of macOS since 12).
    // Its own build script sets this rpath, but a dependency's link args don't reach our binary, so without it
    // the app fails at launch with "Library not loaded: @rpath/libswift_Concurrency.dylib".
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,/usr/lib/swift");
    }
    // Windows: Tauri's app manifest (Common Controls v6) goes into the app only, and a test binary without it can't
    // find TaskDialogIndirect, so Windows shows an error dialog no one can close and `cargo test` hangs. The same
    // manifest, embedded into every binary of this crate instead (Tauri's documented fix).
    let mut windows = tauri_build::WindowsAttributes::new();
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        windows = tauri_build::WindowsAttributes::new_without_app_manifest();
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows)).expect("failed to run tauri-build");
}
