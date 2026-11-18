# Android port

This document tracks the Android port separately from the desktop and web builds. The
Android build is not yet supported; this is the implementation plan and the results of
the initial portability audit.

## Baseline audit

The existing application is a Rust 2024 workspace using egui/eframe 0.36. The engine
crates below `src/` are intentionally independent of egui and windowing, which gives
the port a reusable document, animation, rendering and command layer.

The current desktop executable is not an Android executable yet:

- `src/main.rs` selects the desktop startup path for every target except wasm32. Android
  needs its own `android_main` entry point and must provide the Android activity to
  winit's event loop before the native eframe runner starts.
- The shared eframe dependency does not enable either Android activity backend.
  The eframe 0.36 feature names are `android-game-activity` and
  `android-native-activity`. The initial implementation will target GameActivity.
- The current non-wasm dependency group includes desktop-only pieces such as native
  file dialogs, clipboard integration, desktop update/CLI services and OS-specific
  helpers. These need to be split by target rather than pulled into Android.
- The platform layer currently treats every non-wasm target as a desktop OS in several
  places. Android needs explicit platform behavior for storage, file access, dialogs,
  clipboard, lifecycle and system appearance.
- The default `sound-output` feature enables `cpal`; audio output needs a separately
  validated Android backend and must not block the first graphics/input milestone.
- The engine already imports Animate `.fla` files through XFL. Exporting a native
  binary `.fla` file is not currently an established capability and is not part of
  the first Android milestone.

## Porting sequence

### A0 — Build and platform boundary

- Keep the desktop and web configurations unchanged.
- Add an Android-specific application entry point using eframe's GameActivity backend.
- Move desktop-only dependencies and startup code behind desktop target conditions.
- Add Android packaging metadata and a reproducible ARM64 build command.
- Add Android CI compile checks once the entry point and packaging scaffold are in place.

**Exit criteria:** the ARM64 Android native library builds from a clean checkout; desktop
and web checks continue to pass.

### A1 — First interactive Android build

- Launch the existing editor shell on a phone and tablet.
- Implement Android app-private storage and document open/save through Android's
  document picker / Storage Access Framework.
- Handle pause, resume, activity recreation and process death without losing the
  current document.
- Validate touch, multi-touch, mouse/trackpad and hardware keyboard input.

**Exit criteria:** create, edit, save, close and reopen a native `.anicraft` document
on both a phone and a tablet.

### A2 — Drawing and stylus

- Verify stylus events end-to-end on real Android hardware.
- Map pressure to brush width/opacity where the tool supports it.
- Handle stylus hover, eraser, palm rejection, pointer identity and touch gestures.
- Add adaptive phone/tablet layouts and larger touch targets without removing the
  desktop-oriented precision workflow.

**Exit criteria:** a documented device test matrix covering pressure-sensitive drawing,
eraser, palm rejection, multi-touch navigation and external input devices.

### A3 — Import/export and media

- Verify XFL-based `.fla` import through Android document URIs.
- Implement Android-safe export destinations and sharing.
- Validate image, GIF, SVG, Lottie and video export paths on-device.
- Evaluate binary `.fla` export separately; do not imply compatibility until it exists
  and passes round-trip tests.

### A4 — Performance and release

- Profile CPU/GPU rendering, memory use and large timelines on representative devices.
- Test thermal behavior, backgrounding, low-memory recovery and long sessions.
- Produce signed release artifacts and publish installation/update instructions.

## Device coverage

The minimum validation set is:

- ARM64 phone, compact display.
- ARM64 phone, high-density display.
- Android tablet, landscape and portrait.
- Tablet with an active stylus supporting pressure and eraser.
- Optional mouse/keyboard and external display.

A successful cross-compile alone does not establish runtime, stylus or tablet support.


## Work started on `android-port`

### Android backend probe and Gradle host (ARM64 probe verified; APK fix queued)

An isolated crate now lives in `android/probe/`. It is intentionally outside the
root workspace so it cannot enable Android-only features in the desktop/web
dependency graph. It is pinned to the versions already resolved by the main
repository lockfile:

- eframe 0.36.2 with `android-game-activity`, `accesskit`, `default_fonts`
  and `wgpu`.
- winit 0.30.13 with the matching `android-game-activity` backend.
- A `cdylib` named `main`, with an `android_main(AndroidApp)` entry point
  passing the activity to `eframe::NativeOptions::android_app`.

Build instructions are in `android/probe/README.md`. The expected ARM64
artifact is `android/probe/target/aarch64-linux-android/release/libmain.so`.

**Current status:** the isolated Rust probe remains a backend test, not the editor.
A first Android Gradle application scaffold now lives in `android/`: an AndroidX
GameActivity host, manifest, ARM64 native-library staging task, and instructions in
`android/README.md`. Gradle invokes `cargo-ndk` to build the probe and packages
`libmain.so` into a debug APK. The Rust ARM64 probe has now compiled successfully in GitHub Actions and its `libmain.so` artifact was uploaded. The first APK packaging attempt exposed a missing explicit AndroidX Core dependency required by GameActivity's Java API; `androidx.core:core:1.16.0` has now been added. The corrected APK build is awaiting CI. No runtime or physical-device validation has happened.

The GitHub Actions workflow `.github/workflows/android-probe.yml` installs the
Android SDK tools and a pinned NDK, cross-compiles the probe, and has a second job
to package a debug APK with Gradle. It also triggers for changes anywhere under
`android/`. The initial workflow attempts failed during SDK/NDK installation. After excluding the retired SDK `tools` package and installing NDK r27d explicitly, run 6 completed the ARM64 Rust build successfully (`Finished release profile`) and uploaded the native probe artifact. Its Gradle job then failed at Java compilation because `androidx.core.view.OnApplyWindowInsetsListener` was not on the compile classpath. The missing explicit AndroidX Core dependency was added in commit `8044045`; the next CI run will verify the fix and APK packaging.

The Gradle host currently packages only the probe. It does not yet link the real
AniCraft app modules or implement Android document picking, persistence, lifecycle
recovery, touch/stylus handling or export destinations.


### Full editor integration: platform boundary audit

A fresh source pass confirms that the Android target cannot simply reuse the
desktop native entry point. `src/main.rs` currently treats every non-WASM target
as a desktop process: it parses CLI arguments, runs update-helper logic, starts
the desktop recovery loop, and uses desktop startup/backend selection. Android
needs a separate `android_main(AndroidApp)` path that creates the same
`app::App` around an Android-aware `NativeOptions`.

The platform layer is the main integration boundary. `src/platform.rs` currently
uses `rfd::FileDialog` for native open/save dialogs and `arboard` for system
clipboard access; folder reveal launches Explorer/Finder/xdg-open. These must be
replaced or isolated for Android. Document access should use Android's Storage
Access Framework (persistable URI permissions and content resolver streams),
not pretend that picker results are ordinary durable filesystem paths.
The desktop updater, remote-control listener and desktop-only helper processes
also need to be excluded from Android while preserving the shared editor,
engine, rendering, autosave and export logic where their APIs permit.

The first safe integration sequence is therefore:
1. keep desktop startup and its dependencies unchanged;
2. add an Android-specific native entry point and target-specific dependency set;
3. introduce Android document handles/stream I/O behind the platform boundary;
4. adapt app open/save/export call sites to that abstraction;
5. then enable the editor modules on Android and validate touch, stylus and lifecycle.

Do not enable the whole root binary for Android until these seams compile cleanly;
the separate probe remains the guard for eframe/winit/GameActivity compatibility.
