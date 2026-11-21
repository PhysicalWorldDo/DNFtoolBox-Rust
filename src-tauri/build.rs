fn main() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // Unit-test images also link Tauri's native dialog path. They need the
        // same Common Controls v6 activation context as the desktop EXE.
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'");
        // The real binary already embeds Tauri's complete application manifest
        // through resource.lib; only harness images need linker generation.
        println!("cargo:rustc-link-arg-bin=PhysicalWorldToolbox=/MANIFEST:NO");
    }
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(&["toolbox_dispatch"]))
        .windows_attributes(
            tauri_build::WindowsAttributes::new().window_icon_path("icons/icon.ico"),
        );
    tauri_build::try_build(attributes).expect("Tauri host resource generation failed");
}
