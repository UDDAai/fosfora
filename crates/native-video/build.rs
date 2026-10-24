fn main() {
    println!("cargo:rerun-if-changed=src/player.h");
    println!("cargo:rerun-if-changed=src/player.m");
    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target != "macos" {
        return;
    }
    cc::Build::new()
        .file("src/player.m")
        .flag("-fobjc-arc")
        .flag("-Wall")
        .flag("-Wextra")
        .compile("fosfora_native_player");
    for framework in [
        "Foundation",
        "AVFoundation",
        "CoreVideo",
        "CoreMedia",
        "CoreFoundation",
        "QuartzCore",
        "Metal",
        "VideoToolbox",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
