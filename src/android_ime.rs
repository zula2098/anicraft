//! Android software-keyboard bridge for egui.
//!
//! GameActivity owns the Android input connection, but winit 0.30 does not yet
//! forward its text snapshots to egui. This small InputConnection supplies the
//! missing IME path while leaving pointer and hardware-key input on winit.

use android_ime::{AndroidImeConnection, AndroidImeConnectionHandler, AndroidImeContext};
use egui::{Event, ImeEvent, Key, Modifiers};
use jni::{JavaVM, objects::JObject};
use std::{
    borrow::Cow,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use winit::platform::android::activity::AndroidApp;

type PendingEvents = Arc<Mutex<Vec<Event>>>;

struct Handler {
    events: PendingEvents,
    ctx: egui::Context,
    close_requested: Arc<AtomicBool>,
}

impl Handler {
    fn push(&self, event: Event) {
        if let Ok(mut events) = self.events.lock() {
            events.push(event);
        }
        self.ctx.request_repaint();
    }

    fn key(&self, key: Key) {
        self.push(Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        self.push(Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
    }
}

impl AndroidImeConnectionHandler for Handler {
    fn connection_closed(&self) {
        self.close_requested.store(true, Ordering::Release);
        self.ctx.request_repaint();
    }

    fn send_key_event(&self, key_code: i32) -> bool {
        let key = match key_code {
            19 => Key::ArrowUp,
            20 => Key::ArrowDown,
            21 => Key::ArrowLeft,
            22 => Key::ArrowRight,
            61 => Key::Tab,
            66 => Key::Enter,
            67 => Key::Backspace,
            112 => Key::Delete,
            _ => return false,
        };
        self.key(key);
        true
    }

    fn perform_context_menu_action(&self, _action: i32) -> bool {
        false
    }

    fn perform_editor_action(&self, _action: i32) -> bool {
        self.key(Key::Enter);
        self.close_requested.store(true, Ordering::Release);
        true
    }

    fn begin_batch_edit(&self) -> bool {
        true
    }

    fn end_batch_edit(&self) -> bool {
        true
    }

    fn commit_text(&self, text: &str, _new_cursor_position: isize) -> bool {
        self.push(Event::Ime(ImeEvent::Commit(text.to_owned())));
        true
    }

    fn delete_surrounding_text(&self, before: usize, after: usize) -> bool {
        // Android supplies UTF-16 lengths here. Most keyboards use the code-point
        // variant for supplementary characters; egui expects character counts.
        self.push(Event::Ime(ImeEvent::DeleteSurrounding {
            before_chars: before,
            after_chars: after,
        }));
        true
    }

    fn delete_surrounding_text_in_code_points(&self, before: usize, after: usize) -> bool {
        self.push(Event::Ime(ImeEvent::DeleteSurrounding {
            before_chars: before,
            after_chars: after,
        }));
        true
    }

    fn set_selection(&self, _start: usize, _end: usize) -> bool {
        true
    }

    fn set_composing_region(&self, _start: usize, _end: usize) -> bool {
        true
    }

    fn set_composing_text(&self, text: &str, _new_cursor_position: isize) -> bool {
        self.push(Event::Ime(ImeEvent::Preedit {
            text: text.to_owned(),
            active_range_chars: Some(0..text.chars().count()),
        }));
        true
    }

    fn finish_composing_text(&self) -> bool {
        self.push(Event::Ime(ImeEvent::Preedit {
            text: String::new(),
            active_range_chars: None,
        }));
        true
    }

    fn get_selected_text(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(""))
    }

    fn get_text_after_cursor(&self, _count: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(""))
    }

    fn get_text_before_cursor(&self, _count: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(""))
    }

    fn get_cursor_caps_mode(&self, _req_modes: i32) -> i32 {
        0
    }

    fn request_cursor_updates(&self, _cursor_update_mode: i32) -> bool {
        false
    }
}

pub(crate) struct Bridge {
    context: AndroidImeContext,
    connection: Option<AndroidImeConnection<Handler>>,
    events: PendingEvents,
    close_requested: Arc<AtomicBool>,
    focused: Option<egui::Id>,
    dismissed: bool,
}

impl Bridge {
    #[allow(unsafe_code)]
    pub(crate) fn from_android_app(app: &AndroidApp) -> Result<Self, String> {
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }
            .map_err(|error| format!("Android Java VM: {error}"))?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
        let mut env = vm
            .attach_current_thread()
            .map_err(|error| format!("Attach Android UI thread: {error}"))?;
        let context = AndroidImeContext::new(&mut env, &activity)
            .map_err(|error| format!("Create Android IME: {error:#}"))?;

        Ok(Self {
            context,
            connection: None,
            events: Arc::new(Mutex::new(Vec::new())),
            close_requested: Arc::new(AtomicBool::new(false)),
            focused: None,
            dismissed: false,
        })
    }

    pub(crate) fn update(&mut self, ctx: &egui::Context) {
        let pending = self
            .events
            .lock()
            .map(|mut events| std::mem::take(&mut *events))
            .unwrap_or_default();
        if !pending.is_empty() {
            ctx.input_mut(|input| input.events.extend(pending));
            ctx.request_repaint();
        }

        let focused = ctx.memory(|memory| memory.focused());
        if focused != self.focused {
            self.focused = focused;
            self.dismissed = false;
        }
        if self.close_requested.swap(false, Ordering::AcqRel) {
            self.connection = None;
            self.dismissed = true;
        }

        if !ctx.egui_wants_keyboard_input() {
            self.connection = None;
            self.dismissed = false;
        } else if !self.dismissed && self.connection.is_none() {
            let handler = Handler {
                events: self.events.clone(),
                ctx: ctx.clone(),
                close_requested: self.close_requested.clone(),
            };
            match AndroidImeConnection::open(&self.context, handler) {
                Ok(connection) => self.connection = Some(connection),
                Err(error) => {
                    self.dismissed = true;
                    let _ = error;
                }
            }
        }
    }
}
