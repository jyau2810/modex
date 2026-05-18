fn main() {
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rerun-if-changed=src/macos_notification.m");
        cc::Build::new()
            .file("src/macos_notification.m")
            .flag("-fobjc-arc")
            .flag("-Wno-deprecated-declarations")
            .flag("-Wno-unguarded-availability-new")
            .compile("modex_macos_notification");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=UserNotifications");
    }

    tauri_build::build()
}
