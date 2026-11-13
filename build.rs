use std::{env, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-changed=native/media_controls.m");
    println!("cargo:rerun-if-changed=native/video_player.m");
    println!("cargo:rerun-if-changed=native/smb_mount.m");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let object = out.join("media_controls.o");
    let video_object = out.join("video_player.o");
    let smb_object = out.join("smb_mount.o");
    assert!(
        Command::new("xcrun")
            .args([
                "clang",
                "-fobjc-arc",
                "-mmacosx-version-min=12.0",
                "-c",
                "native/smb_mount.m",
                "-o"
            ])
            .arg(&smb_object)
            .status()
            .unwrap()
            .success(),
        "Could not compile SMB support"
    );
    assert!(
        Command::new("xcrun")
            .args([
                "clang",
                "-fobjc-arc",
                "-fblocks",
                "-mmacosx-version-min=12.0",
                "-c",
                "native/video_player.m",
                "-o"
            ])
            .arg(&video_object)
            .status()
            .unwrap()
            .success(),
        "Could not compile video playback"
    );
    let status = Command::new("xcrun")
        .args([
            "clang",
            "-fobjc-arc",
            "-fblocks",
            "-mmacosx-version-min=12.0",
            "-c",
            "native/media_controls.m",
            "-o",
        ])
        .arg(&object)
        .status()
        .expect("macOS Command Line Tools are required");
    assert!(
        status.success(),
        "Could not compile macOS media integration"
    );
    assert!(
        Command::new("ar")
            .arg("crs")
            .arg(out.join("libkaleido_media.a"))
            .arg(object)
            .arg(video_object)
            .arg(smb_object)
            .status()
            .unwrap()
            .success()
    );
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=kaleido_media");
    for framework in [
        "Foundation",
        "MediaPlayer",
        "CoreAudio",
        "IOBluetooth",
        "Security",
        "AVFoundation",
        "CoreVideo",
        "CoreImage",
        "CoreMedia",
        "QuartzCore",
        "CoreGraphics",
        "NetFS",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
