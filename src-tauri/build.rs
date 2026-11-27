fn main() {
    // tauri-build normally embeds the Windows application manifest (which
    // requests Common Controls v6) as a resource of the app binary only.
    // Test binaries that start Tauri's mock runtime need it too, or they
    // fail to load with STATUS_ENTRYPOINT_NOT_FOUND. So embed it through
    // the linker for every target instead (tauri-apps/tauri#13419).
    let mut attributes = tauri_build::Attributes::new();
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
    tauri_build::try_build(attributes).expect("tauri-build failed");
}
