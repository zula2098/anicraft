//! Minimal Android entry point used to validate AniCraft's eframe/winit backend.
//!
//! This is deliberately a separate smoke-test library, not the full AniCraft editor.

use eframe::egui;

struct ProbeApp;

impl eframe::App for ProbeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("AniCraft Android backend");
            ui.label("GameActivity + eframe + wgpu started successfully.");
            ui.label("This is a backend smoke test, not the full editor.");
        });
    }
}

/// Android's native entry point, called by android-activity through winit.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    let options = eframe::NativeOptions {
        android_app: Some(app),
        ..Default::default()
    };

    // The activity owns the application lifetime. A startup error is logged by the
    // Android runtime rather than panicking across the native entry point.
    let _ = eframe::run_native(
        "AniCraft Android backend probe",
        options,
        Box::new(|_creation_context| Ok(Box::new(ProbeApp))),
    );
}
