//! Host shortcuts are intercepted before page input; Super means Command on macOS.
use winit::keyboard::{Key, ModifiersState, NamedKey};

fn command(modifiers: ModifiersState, macos: bool) -> bool {
    !modifiers.alt_key()
        && ((modifiers.control_key() && !modifiers.super_key())
            || (macos && modifiers.super_key() && !modifiers.control_key()))
}

pub(super) fn palette(key: &Key, modifiers: ModifiersState, macos: bool) -> bool {
    command(modifiers, macos)
        && modifiers.shift_key()
        && matches!(key, Key::Character(s) if s.eq_ignore_ascii_case("p"))
}

pub(super) fn clipboard(key: &Key, modifiers: ModifiersState, macos: bool) -> Option<&'static str> {
    if command(modifiers, macos) && !modifiers.shift_key() {
        match key {
            Key::Character(s) if s.eq_ignore_ascii_case("c") => Some("c"),
            Key::Character(s) if s.eq_ignore_ascii_case("x") => Some("x"),
            Key::Character(s) if s.eq_ignore_ascii_case("v") => Some("v"),
            _ => None,
        }
    } else if modifiers == ModifiersState::SHIFT && *key == Key::Named(NamedKey::Insert) {
        Some("v")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_clipboard_is_mac_only_and_control_still_works() {
        for (character, action) in [("c", "c"), ("X", "x"), ("v", "v")] {
            let key = Key::Character(character.into());
            assert_eq!(clipboard(&key, ModifiersState::SUPER, true), Some(action));
            assert_eq!(clipboard(&key, ModifiersState::SUPER, false), None);
            for macos in [false, true] {
                assert_eq!(
                    clipboard(&key, ModifiersState::CONTROL, macos),
                    Some(action)
                );
                for extra in [
                    ModifiersState::ALT,
                    ModifiersState::SHIFT,
                    ModifiersState::SUPER,
                ] {
                    assert_eq!(
                        clipboard(&key, ModifiersState::CONTROL | extra, macos),
                        None
                    );
                }
                assert_eq!(clipboard(&key, ModifiersState::empty(), macos), None);
            }
            for extra in [
                ModifiersState::ALT,
                ModifiersState::SHIFT,
                ModifiersState::CONTROL,
            ] {
                assert_eq!(clipboard(&key, ModifiersState::SUPER | extra, true), None);
            }
        }
    }

    #[test]
    fn palette_accepts_command_shift_p_on_mac_without_grabbing_other_chords() {
        let key = Key::Character("P".into());
        let command_shift = ModifiersState::SUPER | ModifiersState::SHIFT;
        assert!(palette(&key, command_shift, true));
        assert!(!palette(&key, command_shift, false));
        for macos in [true, false] {
            assert!(palette(
                &key,
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                macos
            ));
            assert!(!palette(&key, ModifiersState::CONTROL, macos));
            assert!(!palette(&key, command_shift | ModifiersState::ALT, macos));
            let insert = Key::Named(NamedKey::Insert);
            assert_eq!(clipboard(&insert, ModifiersState::SHIFT, macos), Some("v"));
            assert_eq!(clipboard(&insert, command_shift, macos), None);
        }
    }
}
