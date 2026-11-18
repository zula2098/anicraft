# Android app

This directory contains the AndroidX GameActivity host and Gradle packaging
project for the native AniCraft editor.

## Requirements

- JDK 17
- Android SDK with platform 36 and NDK 27.3.13750724
- Gradle 8.13
- Rust 1.95+ and `cargo-ndk`

Install the Rust helper once:

```sh
cargo install cargo-ndk --locked
```

Set `ANDROID_HOME` / `ANDROID_SDK_ROOT` and ensure the NDK is installed. Then
from this directory run:

```sh
gradle :app:assembleDebug
```

The Gradle `preBuild` task builds the root AniCraft crate as an Android
GameActivity `cdylib` (`--no-default-features --features android`) for
`arm64-v8a` and stages `libmain.so` into the APK's native-library directory.
The Java activity loads that library through AndroidX GameActivity.

The resulting debug APK is:

```
app/build/outputs/apk/debug/app-debug.apk
```

Install on a connected ARM64 device with:

```sh
gradle :app:installDebug
```

## Current scope and limitations

- The native entry point now starts the actual AniCraft `App` and sample
  document, not the isolated eframe smoke-test screen.
- Only ARM64 is wired into this first native build.
- Android-specific document selection, persisted URI access, save/export
  destinations, lifecycle recovery, touch/stylus refinements and audio
  behavior still need dedicated implementation and device testing.
- CI compilation and APK packaging do not prove that the editor launches or
  behaves correctly on physical Android hardware. Treat runtime support as
  unverified until a device test is recorded.
