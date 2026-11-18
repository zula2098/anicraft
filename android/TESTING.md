# Android test build

This is an experimental build of the real AniCraft editor, not a production release.

## Build

Pushes to `android-port` that touch the Android project, Rust sources, or this workflow
run **Android backend probe** in GitHub Actions. The workflow:

1. Cross-compiles the root AniCraft Rust library for `arm64-v8a`.
2. Packages the Android debug APK with Gradle.
3. Uploads the artifact named `anicraft-android-debug-apk` (retained for 7 days).

To download it, open the repository's Actions tab, select the latest
**Android backend probe** run on `android-port`, wait for both jobs to pass,
then download `anicraft-android-debug-apk` under Artifacts.

## Install

The APK is a debug build and is intended for manual testing:

1. Download and extract the artifact ZIP.
2. Install `app-debug.apk` on an ARM64 Android device (Android may ask to allow
   installation from the app used to download it).
3. Launch AniCraft and check that the editor opens.

## What to test and report

- Does the app install and launch, or does Android show an error?
- Does the editor render and respond to touch?
- Rotate the device and background/restore the app. Does it remain usable?
- Try a hardware keyboard or stylus if available.
- Report the device model, Android version, and steps to reproduce any crash.

## Known limitations

This is an early port. Document open/save through Android's Storage Access Framework,
stylus pressure/eraser, lifecycle recovery, and several desktop-only platform services
are not yet verified on-device. Do not use it as the only copy of important work.
