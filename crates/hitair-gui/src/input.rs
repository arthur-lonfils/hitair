//! Keyboard shortcuts. Text entry is handled by real egui text widgets (which
//! own focus, selection, IME and paste); this only routes navigation and
//! shortcut keys — and only when no text field is focused, so the two don't
//! fight over the same keystrokes.

use egui::{Event, Key as EKey};
use hitair_core::session::{Key, Session};

pub fn feed(ctx: &egui::Context, session: &mut Session) {
    // A `TextEdit` (or similar) currently holds keyboard focus.
    let typing = ctx.memory(|m| m.focused().is_some());
    let events = ctx.input(|i| i.events.clone());
    for event in events {
        let Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = event
        else {
            continue;
        };
        let Some(k) = to_key(key, modifiers.ctrl || modifiers.command) else {
            continue;
        };
        // While typing, let the field keep Enter/arrows/Backspace; still honour
        // Escape (back out) and the volume shortcuts.
        if typing && !matches!(k, Key::Esc | Key::CtrlUp | Key::CtrlDown) {
            continue;
        }
        session.handle_key(k);
    }
}

fn to_key(key: EKey, ctrl: bool) -> Option<Key> {
    Some(match key {
        EKey::ArrowUp if ctrl => Key::CtrlUp,
        EKey::ArrowDown if ctrl => Key::CtrlDown,
        EKey::ArrowUp => Key::Up,
        EKey::ArrowDown => Key::Down,
        EKey::ArrowLeft => Key::Left,
        EKey::ArrowRight => Key::Right,
        EKey::Enter => Key::Enter,
        EKey::Escape => Key::Esc,
        EKey::Tab => Key::Tab,
        // Ctrl + letter (e.g. Ctrl+O opens Challenge, Ctrl+R replays).
        other if ctrl => {
            let name = other.name();
            let c = name.chars().next()?;
            if name.len() == 1 && c.is_ascii_alphabetic() {
                Key::Ctrl(c.to_ascii_lowercase())
            } else {
                return None;
            }
        }
        _ => return None,
    })
}
