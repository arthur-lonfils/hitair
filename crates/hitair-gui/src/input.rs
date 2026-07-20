//! Translate this frame's egui keyboard events into the session's `Key`s.
//!
//! Character input arrives as `Event::Text` (layout/IME-correct); the special
//! keys and Ctrl-combos arrive as `Event::Key`. We only take letters from
//! `Text` so a keystroke isn't counted twice.

use egui::{Event, Key as EKey};
use hitair_core::session::{Key, Session};

pub fn feed(ctx: &egui::Context, session: &mut Session) {
    let events = ctx.input(|i| i.events.clone());
    for event in events {
        match event {
            Event::Text(text) => {
                for c in text.chars() {
                    session.handle_key(Key::Char(c));
                }
            }
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                if let Some(k) = to_key(key, modifiers.ctrl || modifiers.command) {
                    session.handle_key(k);
                }
            }
            _ => {}
        }
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
        EKey::Backspace => Key::Backspace,
        // Ctrl + letter (e.g. Ctrl+O / Ctrl+R). Plain letters come via Event::Text.
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
