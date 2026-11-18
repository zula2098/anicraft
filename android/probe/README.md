# Android backend smoke test

This small standalone crate validates the Android platform backend independently
from AniCraft's desktop shell. It is **not** the Android editor and does not yet
provide document I/O, stylus handling, or APK packaging.

## Why it is separate

The current AniCraft executable is a desktop binary with command-line startup,
desktop dialogs and other host integrations. Enabling Android features on that
binary would not create a valid Android application. This probe isolates the
required Rust/eframe/winit entry point without changing the desktop or web
dependency graph.

## Build the ARM64 native library

Requirements:

- Rust toolchain 1.95 or newer.
- Android NDK installed.
- Android target installed: `rustup target add aarch64-linux-android`.
- `cargo-ndk`: `cargo install cargo-ndk`.

Set `ANDROID_NDK_HOME` to the installed NDK directory, then run from the
repository root:

```sh
cargo ndk -t arm64-v8a build --release --manifest-path android/probe/Cargo.toml
```

The expected native library is:

```
target/aarch64-linux-android/release/libmain.so
```

A successful cross-build proves that the selected eframe GameActivity backend
and the Rust Android entry point compile for ARM64. It does **not** prove that
the library can be packaged or launched: the GameActivity Java/Kotlin host,
manifest, Gradle project and device runtime checks are subsequent tasks.

## Backend choice

The probe uses eframe's `android-game-activity` feature and winit's matching
GameActivity feature. GameActivity requires an Android host Activity and its
AndroidX/AGDK integration; that packaging layer will be added after the native
cross-build is validated.
