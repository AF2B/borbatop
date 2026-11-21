//! Translation of terminal events into the program's own key type.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::input::Key;

/// Whether `event` is Ctrl+C, the universal way out.
///
/// # Arguments
///
/// * `event` - key event read from the terminal
#[must_use]
pub fn is_interrupt(event: &KeyEvent) -> bool {
    event.modifiers.contains(KeyModifiers::CONTROL) && event.code == KeyCode::Char('c')
}

/// The program's [`Key`] for a terminal key event, if it means anything.
///
/// Key releases and combinations with Ctrl or Alt are ignored.
///
/// # Arguments
///
/// * `event` - key event read from the terminal
#[must_use]
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "crossterm has many keys (function keys, media keys) the program ignores"
)]
pub fn key_from_event(event: KeyEvent) -> Option<Key> {
    if event.kind == KeyEventKind::Release {
        return None;
    }
    if event
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }

    match event.code {
        KeyCode::Char(character) => Some(Key::Char(character)),
        KeyCode::Up => Some(Key::Up),
        KeyCode::Down => Some(Key::Down),
        KeyCode::PageUp => Some(Key::PageUp),
        KeyCode::PageDown => Some(Key::PageDown),
        KeyCode::Home => Some(Key::Home),
        KeyCode::End => Some(Key::End),
        KeyCode::Enter => Some(Key::Enter),
        KeyCode::Esc => Some(Key::Escape),
        KeyCode::Backspace => Some(Key::Backspace),
        KeyCode::Tab => Some(Key::Tab),
        KeyCode::BackTab => Some(Key::BackTab),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn plain_keys_map_one_to_one() {
        assert_eq!(
            key_from_event(press(KeyCode::Char('q'), KeyModifiers::NONE)),
            Some(Key::Char('q'))
        );
        assert_eq!(
            key_from_event(press(KeyCode::Char('K'), KeyModifiers::SHIFT)),
            Some(Key::Char('K'))
        );
        assert_eq!(
            key_from_event(press(KeyCode::Esc, KeyModifiers::NONE)),
            Some(Key::Escape)
        );
        assert_eq!(
            key_from_event(press(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Some(Key::BackTab)
        );
        assert_eq!(
            key_from_event(press(KeyCode::Down, KeyModifiers::NONE)),
            Some(Key::Down)
        );
    }

    #[test]
    fn unknown_keys_modifiers_and_releases_are_ignored() {
        assert_eq!(
            key_from_event(press(KeyCode::F(5), KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            key_from_event(press(KeyCode::Char('q'), KeyModifiers::CONTROL)),
            None
        );
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        assert_eq!(key_from_event(release), None);
    }

    #[test]
    fn ctrl_c_is_an_interrupt() {
        assert!(is_interrupt(&press(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )));
        assert!(!is_interrupt(&press(
            KeyCode::Char('c'),
            KeyModifiers::NONE
        )));
    }
}
