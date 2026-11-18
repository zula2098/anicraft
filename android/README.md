# Android app scaffold

This directory contains the first Android packaging host for the isolated Rust
backend probe. It is **not yet the full AniCraft editor**.

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

The Gradle `preBuild` task cross-compiles `android/probe` for `arm64-v8a`
and stages `libmain.so` into the APK's native-library directory. The resulting
debug APK is:

```
app/build/outputs/apk/debug/app-debug.apk
```

Install on a connected ARM64 device with:

```sh
gradle :app:installDebug
```

## Current scope and limitations

- The Java activity extends AndroidX GameActivity 4.4.2.
- The native library is currently the small eframe/wgpu backend probe from
  `probe/`, not the AniCraft editor.
- Only ARM64 is wired into this first scaffold.
- This is a packaging scaffold; a successful Gradle build and on-device launch
  still need to be verified in CI and on physical hardware.
- The editor's desktop startup, file dialogs, storage access, lifecycle recovery,
  touch/stylus behavior and export destinations have not yet been ported.
