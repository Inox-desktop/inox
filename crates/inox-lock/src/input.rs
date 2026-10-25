//! What a key means, without a toolkit.
//!
//! `wl_keyboard` hands a client a keymap and then numbers: the keymap says what
//! every number means on the layout the compositor is actually configured for, and
//! this module is the only place in the crate that reads it. It produces
//! [`crate::app::Key`] values and nothing else — it cannot submit anything, cannot see a
//! credential, and does not decide whether the screen is currently taking input
//! ([`crate::view_model::LockViewModel::accepts_input`] is that question, and it
//! belongs to the projection).
//!
//! Four decisions are worth reading before changing anything here.
//!
//! - **The keymap is compiled, not guessed.** A hand-written US table would take
//!   far less code and would silently produce the *wrong character* on every other
//!   layout, in every Shift and AltGr combination. A wrong password is worse than
//!   no password prompt, so the layout comes from the same `xkbcommon` the
//!   compositor links.
//! - **The keymap text, not the fd.** `xkbcommon`'s `new_from_fd` is an `unsafe fn`
//!   and this crate's hand-written Rust contains no `unsafe`; the keymap is read
//!   out of the fd as text instead. The read is bounded, and a keymap with an
//!   interior NUL is refused rather than handed to a `CString::new(..).unwrap()`
//!   inside the crate, where it would panic.
//! - **No keymap, no input.** A format this build does not understand leaves the
//!   keyboard without a meaning for any keycode, and every key is then ignored. The
//!   session stays covered, which is the fail-closed direction: the alternative is
//!   interpreting keycodes as if they were a US layout again.
//! - **Nothing here logs a character.** [`crate::app::Key`] and
//!   [`crate::app::Message`] derive
//!   `Debug` and carry the typed character; a `?key` in a tracing field would put a
//!   fragment of a password in the log. Log a count, a keycode or nothing.
//!
//! What this does not do, and cannot until the stack grows the protocols for it:
//! compose/dead keys (`key_get_utf8` returns nothing for them, so `é` typed with a
//! dead key is dropped), input methods (the lock surface has no `text-input`, so
//! CJK passwords cannot be typed), and key repeat (nothing in this session
//! synthesises repeats, so holding Backspace deletes one character).

use std::fs::File;
use std::io::Read as _;
use std::os::fd::OwnedFd;

use xkbcommon::xkb;

use crate::app::{Key, Named};

/// The largest keymap this module will read.
///
/// A keymap is a few hundred kilobytes of text at most; the bound exists so a
/// compositor cannot make the lock screen allocate without limit before the lock is
/// even on screen.
const MAX_KEYMAP_BYTES: u64 = 1 << 20;

/// A keyboard: the layout it declared, and which keys are down under it.
pub(crate) struct Keyboard {
    context: xkb::Context,
    keymap: xkb::Keymap,
    state: xkb::State,
}

impl Keyboard {
    /// Compiles `text` as an XKB keymap, or refuses it.
    ///
    /// Refusing is not an error path with a recovery: a keyboard whose keymap this
    /// build cannot compile types nothing, and the screen keeps saying that nobody
    /// can unlock it.
    pub(crate) fn from_keymap_text(text: String) -> Option<Self> {
        let bytes = text.trim_end_matches('\0');
        if bytes.contains('\0') {
            // `xkbcommon` builds a `CString` out of this string and unwraps it.
            return None;
        }

        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_string(
            &context,
            bytes.to_owned(),
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )?;
        let state = xkb::State::new(&keymap);

        Some(Self {
            context,
            keymap,
            state,
        })
    }

    /// Reads and compiles the keymap the compositor sent on `fd`.
    pub(crate) fn from_keymap_fd(fd: OwnedFd) -> Option<Self> {
        let mut text = String::new();
        let read = File::from(fd)
            .take(MAX_KEYMAP_BYTES)
            .read_to_string(&mut text)
            .ok()?;

        if read == 0 {
            return None;
        }

        Self::from_keymap_text(text)
    }

    /// Applies the modifier state the compositor last reported.
    pub(crate) fn update_modifiers(
        &mut self,
        depressed: u32,
        latched: u32,
        locked: u32,
        group: u32,
    ) {
        self.state
            .update_mask(depressed, latched, locked, 0, 0, group);
    }

    /// A key going down, as the key it types.
    ///
    /// `None` is every key that types nothing here: modifiers, dead keys, keys this
    /// screen has no use for, and control characters (`Ctrl+A` is not the letter
    /// `a` typed into a password field).
    pub(crate) fn press(&mut self, keycode: u32) -> Option<Key> {
        let code = self.code(keycode);
        self.state.update_key(code, xkb::KeyDirection::Down);

        match self.state.key_get_one_sym(code).raw() {
            xkb::keysyms::KEY_Return | xkb::keysyms::KEY_KP_Enter => Some(Key::Named(Named::Enter)),
            xkb::keysyms::KEY_BackSpace => Some(Key::Named(Named::Backspace)),
            xkb::keysyms::KEY_Escape => Some(Key::Named(Named::Escape)),
            xkb::keysyms::KEY_Tab => Some(Key::Named(Named::Tab)),
            _ => {
                let typed = self.state.key_get_utf8(code);
                let mut characters = typed.chars();

                match (characters.next(), characters.next()) {
                    // One character, and nothing a text control would refuse: a
                    // control character is a chord (`Ctrl+…`), not something
                    // anybody types into a password.
                    (Some(character), None) if !character.is_control() => {
                        Some(Key::Character(typed))
                    }
                    _ => None,
                }
            }
        }
    }

    /// A key coming back up, so the next press sees the right modifiers.
    pub(crate) fn release(&mut self, keycode: u32) {
        let code = self.code(keycode);
        self.state.update_key(code, xkb::KeyDirection::Up);
    }

    /// Forgets the modifiers, because the keyboard focus went elsewhere.
    ///
    /// A `Shift` held when focus left must not still be held when it comes back:
    /// the next character would be capitalised for a reason the person never
    /// pressed.
    pub(crate) fn reset(&mut self) {
        self.state = xkb::State::new(&self.keymap);
        let _ = &self.context;
    }

    /// The X11 keycode `xkbcommon` wants for a `wl_keyboard` keycode.
    ///
    /// The protocol carries evdev codes and XKB counts from 8; the compositor
    /// subtracts 8 when it writes the event, so the client adds it back.
    fn code(&self, keycode: u32) -> xkb::Keycode {
        xkb::Keycode::new(keycode.saturating_add(8))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A keymap the test can rely on: the same `us` layout a CI machine's
    /// `xkb-data` ships, spelled out so the test does not depend on the machine's
    /// XKB configuration.
    const US_KEYMAP: &str = "\
xkb_keymap {
    xkb_keycodes { include \"evdev+aliases(qwerty)\" };
    xkb_types { include \"complete\" };
    xkb_compat { include \"complete\" };
    xkb_symbols { include \"pc+us+inet(evdev)\" };
};
";

    fn keyboard() -> Keyboard {
        Keyboard::from_keymap_text(US_KEYMAP.to_owned()).expect("the test keymap compiles")
    }

    // Keycodes are evdev, exactly as `wl_keyboard.key` reports them.
    const KEY_A: u32 = 30;
    const KEY_SPACE: u32 = 57;
    const KEY_ESC: u32 = 1;
    const KEY_BACKSPACE: u32 = 14;
    const KEY_TAB: u32 = 15;
    const KEY_ENTER: u32 = 28;
    const KEY_LEFTSHIFT: u32 = 42;
    const KEY_LEFTCTRL: u32 = 29;

    #[test]
    fn a_letter_is_the_character_it_types() {
        let mut keyboard = keyboard();

        assert_eq!(keyboard.press(KEY_A), Some(Key::Character("a".to_owned())));
    }

    #[test]
    fn a_held_shift_capitalises_the_next_letter() {
        let mut keyboard = keyboard();

        assert_eq!(keyboard.press(KEY_LEFTSHIFT), None, "shift types nothing");
        assert_eq!(keyboard.press(KEY_A), Some(Key::Character("A".to_owned())));

        keyboard.release(KEY_LEFTSHIFT);
        assert_eq!(
            keyboard.press(KEY_A),
            Some(Key::Character("a".to_owned())),
            "releasing the modifier is what makes the next key lowercase again"
        );
    }

    #[test]
    fn the_named_keys_the_screen_uses_arrive_as_names() {
        let mut keyboard = keyboard();

        assert_eq!(keyboard.press(KEY_ENTER), Some(Key::Named(Named::Enter)));
        assert_eq!(
            keyboard.press(KEY_BACKSPACE),
            Some(Key::Named(Named::Backspace))
        );
        assert_eq!(keyboard.press(KEY_ESC), Some(Key::Named(Named::Escape)));
        assert_eq!(keyboard.press(KEY_TAB), Some(Key::Named(Named::Tab)));
    }

    #[test]
    fn a_space_is_a_character_because_a_password_may_contain_one() {
        let mut keyboard = keyboard();

        assert_eq!(
            keyboard.press(KEY_SPACE),
            Some(Key::Character(" ".to_owned()))
        );
    }

    #[test]
    fn a_chord_is_not_a_character() {
        let mut keyboard = keyboard();

        assert_eq!(keyboard.press(KEY_LEFTCTRL), None);
        assert_eq!(
            keyboard.press(KEY_A),
            None,
            "Ctrl+A is a control character, not the letter a typed into a password"
        );
    }

    #[test]
    fn leaving_resets_the_modifiers_instead_of_keeping_them_held() {
        let mut keyboard = keyboard();
        let _ = keyboard.press(KEY_LEFTSHIFT);

        keyboard.reset();

        assert_eq!(
            keyboard.press(KEY_A),
            Some(Key::Character("a".to_owned())),
            "a shift held when focus left must not still be held when it returns"
        );
    }

    #[test]
    fn a_keymap_this_build_cannot_read_types_nothing() {
        assert!(
            Keyboard::from_keymap_text(String::new()).is_none(),
            "an empty keymap is not a layout"
        );
        assert!(
            Keyboard::from_keymap_text("xkb_keymap { nonsense };".to_owned()).is_none(),
            "a keymap that does not compile leaves the keyboard without meanings"
        );
    }

    #[test]
    fn an_interior_nul_is_refused_rather_than_reaching_the_cstring() {
        assert!(
            Keyboard::from_keymap_text(format!("{US_KEYMAP}\u{0}trailing")).is_none(),
            "the crate builds a CString out of this text and unwraps it; an \
             interior NUL would panic inside it"
        );
    }

    #[test]
    fn a_trailing_nul_is_stripped_before_compiling() {
        assert!(
            Keyboard::from_keymap_text(format!("{US_KEYMAP}\u{0}\u{0}")).is_some(),
            "a keymap read out of a shared buffer ends with the buffer's padding"
        );
    }
}
