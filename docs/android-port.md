# Android port

This document tracks the experimental Android port separately from the desktop and web builds.
The native ARM64 editor library now cross-compiles in CI; APK packaging and on-device behavior
remain under validation.

## Baseline audit

The existing application is a Rust 2024 workspace using egui/eframe 0.36. The engine
crates below `src/` are intentionally independent of egui and windowing, which gives
the port a reusable document, animation, rendering and command layer.

The current desktop executable is not an Android executable yet:

- `src/main.rs` now has a dedicated `android_main(AndroidApp)` entry point. It creates the
  shared `app::App` and starts eframe with `NativeOptions.android_app`; the desktop CLI and
  recovery loop remain on the non-WASM desktop `main` path.
- The `android` Cargo feature enables eframe's `android-game-activity` backend. The
  Android Gradle host uses AndroidX GameActivity 4.4.2.
- Some desktop-oriented dependencies remain in the non-WASM target group and currently
  compile for ARM64. Their runtime behavior is not assumed to be valid on Android; file
  dialogs, clipboard, update/CLI services and OS helper calls still need platform-specific
  behavior or explicit exclusion.
- The platform layer currently treats every non-wasm target as a desktop OS in several
  places. Android needs explicit platform behavior for storage, file access, dialogs,
  clipboard, lifecycle and system appearance.
- The default `sound-output` feature enables `cpal`; audio output needs a separately
  validated Android backend and must not block the first graphics/input milestone.
- The engine already imports Animate `.fla` files through XFL. Exporting a native
  binary `.fla` file is not currently an established capability and is not part of
  the first Android milestone.

## Porting sequence

### A0 — Native build and packaging

- [x] Add an Android-specific application entry point using eframe's GameActivity backend.
- [x] Add Android Gradle packaging metadata and a reproducible ARM64 build command.
- [x] Add CI cross-compilation of the real root AniCraft crate for ARM64.
- [ ] Complete debug APK packaging and artifact upload.
- [ ] Verify desktop and web checks remain unchanged.

**Progress:** the isolated backend probe has completed an ARM64 release cross-compile.
The workflow and Gradle task have since been switched to compile the real root crate;
the first integrated CI run is in progress. APK packaging previously exposed a missing
AppCompat dependency, now explicitly added as `androidx.appcompat:appcompat:1.7.1`.

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

### Android GameActivity host and native editor

The initial isolated backend probe remains under `android/probe/` as a small eframe/winit
compatibility reference. The actual Android app no longer packages that probe: the Gradle
`preBuild` task invokes `cargo-ndk` on the root `Cargo.toml` with
`--lib --no-default-features --features android`, then stages the resulting root
`libmain.so` into the APK.

The root crate now declares a `cdylib` named `main` and has an Android-specific
`android_main(AndroidApp)` entry point. That entry point starts the existing `app::App`
with a sample AniCraft document. The Android Java activity extends AndroidX GameActivity
4.4.2. Gradle uses SDK 36, NDK r27d (`27.3.13750724`), and Java 17.

**Verified in CI:** the isolated eframe/winit probe compiled for
`aarch64-linux-android` and uploaded successfully in workflow run 7. That run did
not yet compile the root editor crate. The root crate's Android build is now wired
into both the ARM64 workflow job and Gradle's native-library task; workflow run 11
is the first run testing this integrated path. The earlier APK attempt failed
because `androidx.appcompat.app.AppCompatActivity` was absent from the compile
classpath. AndroidX Core 1.16.0 and AppCompat 1.7.1 are now explicit dependencies.
Neither the integrated APK nor on-device runtime has been confirmed yet.

The APK build workflow is `.github/workflows/android-probe.yml`; despite its
historical filename, it now builds the root editor library and packages the
debug Android app. The isolated probe is not the app payload.

### Android runtime platform work still required

The full editor modules now compile into the ARM64 native library, but compile-time
success does not mean every desktop platform service works at runtime on Android.
The next boundary work is in `src/platform.rs` and the app's file workflows.

The platform layer currently uses `rfd::FileDialog` for native open/save dialogs
and `arboard` for system clipboard access; folder reveal launches
Explorer/Finder/xdg-open. Android needs document selection and saving through the
Storage Access Framework, including persistable URI permissions and content-resolver
streams. Picker results must not be treated as ordinary durable filesystem paths.
The desktop updater, remote-control listener and helper-process actions also need
to be disabled or replaced on Android where appropriate.

The implementation sequence from here is:
1. add Android platform capability checks and prevent desktop-only process launches;
2. implement open/save/export using Android document URIs and stream I/O;
3. validate autosave location and recovery across pause, resume and process death;
4. check clipboard, fonts, audio and appearance calls on real Android devices;
5. refine touch, multi-touch, stylus pressure/eraser and adaptive phone/tablet layout.

Until these are implemented and tested, the native build is an integration milestone,
not a claim of complete Android support. The current Android entry point starts the
real editor with the sample document; user document workflows and device behavior
remain unverified.
