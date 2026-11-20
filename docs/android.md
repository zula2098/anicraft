# Android build

The Android app uses Winit's AndroidX GameActivity backend. Gradle packages the Rust
`cdylib` together with the Java GameActivity host and AndroidX dependencies.

## Build locally

Install JDK 17, Android SDK platform 36, Android NDK 27.3.13750724, Rust stable,
and `cargo-ndk`. Add the Rust target and install cargo-ndk:

```sh
rustup target add aarch64-linux-android
cargo install cargo-ndk --locked
```

Set `ANDROID_NDK_HOME` to the installed NDK directory, then run this from the repository root:

```sh
gradle -p android --no-daemon :app:assembleDebug
```

The APK is written to
`android/app/build/outputs/apk/debug/app-debug.apk`. The Gradle `preBuild` task
builds the Rust library and stages it for packaging automatically.

The current APK targets 64-bit ARM Android devices. The Android feature selects
GameActivity and OpenGL for the UI; the stage uses its CPU renderer. The Android
build enables sound output and does not enable the desktop GPU-stage feature.
