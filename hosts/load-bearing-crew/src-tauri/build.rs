fn main() {
    // Publisher-owned desktop application identity; never bundle a user's OAuth JSON or tokens.
    println!("cargo:rerun-if-env-changed=NASTI_APP_YOUTUBE_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=NASTI_APP_YOUTUBE_CLIENT_SECRET");
    let configured = |name| std::env::var(name).is_ok_and(|value| !value.trim().is_empty());
    if configured("NASTI_APP_YOUTUBE_CLIENT_ID") && !configured("NASTI_APP_YOUTUBE_CLIENT_SECRET") {
        panic!("Nasti publisher builds require NASTI_APP_YOUTUBE_CLIENT_SECRET with the client ID");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        for file in [
            "native/playback-outputs.h",
            "native/playback-outputs.m",
            "native/playback.h",
            "native/playback.m",
            "native/spectrum.c",
            "native/spectrum.h",
        ] {
            println!("cargo:rerun-if-changed={file}");
        }
        for framework in [
            "Accelerate",
            "AppKit",
            "AVFoundation",
            "CoreAudio",
            "CoreMedia",
            "MediaPlayer",
            "MediaToolbox",
            "QuartzCore",
            "WebKit",
        ] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
        cc::Build::new()
            .file("native/playback-outputs.m")
            .file("native/playback.m")
            .file("native/spectrum.c")
            .std("gnu17")
            .flag("-fobjc-arc")
            .flag("-mmacosx-version-min=12.0")
            .compile("nasti_playback");
    }
    tauri_build::build();
}
