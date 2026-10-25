//! Compositor-level input interpretation.
//!
//! The heavy lifting — keymap, modifier tracking and focus delivery — is done by
//! Smithay's seat implementation. This module holds the small, pure decision
//! layer that turns a key press into a compositor [`Action`], which keeps key
//! bindings independent of the event loop and unit testable.

use std::collections::HashSet;

use smithay::{
    // Smithay keeps `KeyState` next to the backend events it comes from; the
    // keyboard module imports it from there too.
    backend::input::KeyState,
    input::keyboard::{Keycode, Keysym, ModifiersState, keysyms},
};

/// A compositor action triggered by a key binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Move the keyboard focus to the next window in layout order.
    FocusNext,
    /// Move the keyboard focus to the previous window in layout order.
    FocusPrev,
    /// Ask the focused window to close.
    ///
    /// This is `xdg_toplevel.close`, not a kill: the client decides whether to
    /// save, prompt or refuse, which is the only thing a compositor may do to an
    /// application that owns data.
    CloseWindow,
    /// Ask the compositor to shut down cleanly.
    Quit,
    /// Switch to a virtual terminal through the runtime's libseat session.
    SwitchVt(u8),
}

/// Resolve a key press into a compositor [`Action`].
///
/// Bindings are matched on the *modified* keysym, so `Alt+Tab` and
/// `Alt+Shift+Tab` are distinguished by the modifier state rather than by
/// different key symbols. Returns `None` when the key is not bound, in which
/// case the event is forwarded to the focused client unchanged.
pub fn action_for_key(sym: Keysym, modifiers: &ModifiersState) -> Option<Action> {
    let raw = sym.raw();
    if (keysyms::KEY_XF86Switch_VT_1..=keysyms::KEY_XF86Switch_VT_12).contains(&raw) {
        return Some(Action::SwitchVt(
            (raw - keysyms::KEY_XF86Switch_VT_1 + 1) as u8,
        ));
    }
    if modifiers.ctrl && modifiers.alt && (keysyms::KEY_F1..=keysyms::KEY_F12).contains(&raw) {
        return Some(Action::SwitchVt((raw - keysyms::KEY_F1 + 1) as u8));
    }
    if modifiers.alt && is_tab(sym) {
        return Some(if modifiers.shift {
            Action::FocusPrev
        } else {
            Action::FocusNext
        });
    }

    if modifiers.logo && is_q(sym) {
        // `Super+Shift+Q` closes the focused window; `Super+Q` on its own ends the
        // session. Both levels of the key count: `Shift` moves `Q` to its second
        // level, so the seat reports `Q` and not `q` for the close chord.
        return Some(if modifiers.shift {
            Action::CloseWindow
        } else {
            Action::Quit
        });
    }

    None
}

/// Whether `sym` is `Q` at either of its levels.
///
/// The same second-level rule as [`is_tab`]: with `Shift` held the seat reports
/// the uppercase symbol, so matching only the lowercase one would leave
/// `Super+Shift+Q` unreachable.
fn is_q(sym: Keysym) -> bool {
    sym == Keysym::new(keysyms::KEY_q) || sym == Keysym::new(keysyms::KEY_Q)
}

/// Whether `sym` is `Tab` at either of its levels.
///
/// `Shift` selects a key's second level, and the `evdev` keymaps give `Tab`'s second
/// level a symbol of its own: `ISO_Left_Tab`. Matching `KEY_Tab` alone would leave
/// `Alt+Shift+Tab` — and with it [`Action::FocusPrev`] — unreachable.
fn is_tab(sym: Keysym) -> bool {
    sym == Keysym::new(keysyms::KEY_Tab) || sym == Keysym::new(keysyms::KEY_ISO_Left_Tab)
}

/// Remembers which key presses this compositor kept from the client, so that every
/// release is answered the way its press was.
///
/// The seat hands the key filter a release exactly as it does a press, and it resolves
/// the binding from the modifiers of *that moment*. Deciding each half on its own gets
/// it wrong both ways: letting the release run the binding too fires it twice, and
/// letting the modifier state decide the release can swallow one half of a chord.
/// Press `Tab` with nothing held and it goes to the client; hold `Alt` and release
/// `Tab` and the release now matches `Alt+Tab`, so the client would be left holding a
/// key down — which is also what makes Smithay report that phantom key to whichever
/// client takes the focus next.
#[derive(Debug, Default)]
pub struct SwallowedPresses {
    keys: HashSet<Keycode>,
}

impl SwallowedPresses {
    /// Record one key event and decide whether the focused client may not see it.
    ///
    /// `bound` is whether the compositor's own binding matched the event. It only
    /// matters for a press: the press is what decides, and this type carries that
    /// decision to the release.
    pub fn swallow(&mut self, keycode: Keycode, state: KeyState, bound: bool) -> bool {
        match state {
            KeyState::Pressed => {
                if bound {
                    self.keys.insert(keycode);
                } else {
                    // The previous press of this key may still be recorded if its
                    // release never came; this press goes to the client, so its
                    // release must too.
                    self.keys.remove(&keycode);
                }
                bound
            }
            KeyState::Released => self.keys.remove(&keycode),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_terminal_chords_are_intercepted_once() {
        let modifiers = ModifiersState {
            ctrl: true,
            alt: true,
            ..Default::default()
        };
        assert_eq!(
            action_for_key(Keysym::new(keysyms::KEY_F1), &modifiers),
            Some(Action::SwitchVt(1))
        );
        assert_eq!(
            action_for_key(
                Keysym::new(keysyms::KEY_XF86Switch_VT_12),
                &ModifiersState::default()
            ),
            Some(Action::SwitchVt(12))
        );
        assert_eq!(
            action_for_key(Keysym::new(keysyms::KEY_F1), &ModifiersState::default()),
            None
        );
    }

    fn tab() -> Keysym {
        Keysym::new(keysyms::KEY_Tab)
    }

    /// The symbol the seat reports for `Shift`+`Tab`, which is `Tab`'s second level.
    fn shifted_tab() -> Keysym {
        Keysym::new(keysyms::KEY_ISO_Left_Tab)
    }

    /// `Tab`'s key code, as the seat reports it.
    fn tab_code() -> Keycode {
        Keycode::new(23)
    }

    fn q() -> Keysym {
        Keysym::new(keysyms::KEY_q)
    }

    #[test]
    fn alt_tab_focuses_the_next_window() {
        let modifiers = ModifiersState {
            alt: true,
            ..ModifiersState::default()
        };

        assert_eq!(action_for_key(tab(), &modifiers), Some(Action::FocusNext));
    }

    #[test]
    fn alt_shift_tab_focuses_the_previous_window() {
        let modifiers = ModifiersState {
            alt: true,
            shift: true,
            ..ModifiersState::default()
        };

        // `Shift` moves `Tab` to its second level, so this is the symbol the seat
        // really delivers for `Alt+Shift+Tab`; the plain `Tab` symbol is accepted
        // too, for keymaps whose second level is not `ISO_Left_Tab`.
        assert_eq!(
            action_for_key(shifted_tab(), &modifiers),
            Some(Action::FocusPrev)
        );
        assert_eq!(action_for_key(tab(), &modifiers), Some(Action::FocusPrev));
    }

    #[test]
    fn shifted_tab_without_alt_is_not_bound() {
        let modifiers = ModifiersState {
            shift: true,
            ..ModifiersState::default()
        };

        assert_eq!(action_for_key(shifted_tab(), &modifiers), None);
    }

    #[test]
    fn super_q_quits() {
        let modifiers = ModifiersState {
            logo: true,
            ..ModifiersState::default()
        };

        assert_eq!(action_for_key(q(), &modifiers), Some(Action::Quit));
    }

    #[test]
    fn super_shift_q_closes_the_focused_window() {
        let close = ModifiersState {
            logo: true,
            shift: true,
            ..ModifiersState::default()
        };
        // `Shift` selects the second level of the key, which the seat reports as
        // the uppercase symbol — the close chord has to match that one.
        assert_eq!(
            action_for_key(Keysym::new(keysyms::KEY_Q), &close),
            Some(Action::CloseWindow)
        );

        // The two chords must not collapse into one: `Super+Q` still ends the
        // session, and `Shift+Q` alone is the client's own `Q`.
        let quit = ModifiersState {
            logo: true,
            ..ModifiersState::default()
        };
        assert_eq!(action_for_key(q(), &quit), Some(Action::Quit));
        let shift_only = ModifiersState {
            shift: true,
            ..ModifiersState::default()
        };
        assert_eq!(
            action_for_key(Keysym::new(keysyms::KEY_Q), &shift_only),
            None
        );
    }

    #[test]
    fn a_bound_chord_is_swallowed_on_both_halves() {
        let mut swallowed = SwallowedPresses::default();

        assert!(swallowed.swallow(tab_code(), KeyState::Pressed, true));
        assert!(swallowed.swallow(tab_code(), KeyState::Released, false));
    }

    #[test]
    fn a_forwarded_press_keeps_its_release_even_if_a_modifier_arrives() {
        let mut swallowed = SwallowedPresses::default();

        // Tab pressed on its own goes to the client, so the release must reach it
        // even though `Alt` is down by then and the event now matches `Alt+Tab`.
        assert!(!swallowed.swallow(tab_code(), KeyState::Pressed, false));
        assert!(!swallowed.swallow(tab_code(), KeyState::Released, true));
    }

    #[test]
    fn a_swallowed_press_keeps_its_release_even_if_the_modifier_leaves() {
        let mut swallowed = SwallowedPresses::default();

        // `Alt+Tab`: the press is the compositor's, and the release stays the
        // compositor's too even after `Alt` has been let go.
        assert!(swallowed.swallow(tab_code(), KeyState::Pressed, true));
        assert!(swallowed.swallow(tab_code(), KeyState::Released, false));
    }

    #[test]
    fn a_repeated_press_overrides_a_stale_earlier_one() {
        let mut swallowed = SwallowedPresses::default();

        // A press whose release never arrived must not make the next press's
        // release swallowable.
        assert!(swallowed.swallow(tab_code(), KeyState::Pressed, true));
        assert!(!swallowed.swallow(tab_code(), KeyState::Pressed, false));
        assert!(!swallowed.swallow(tab_code(), KeyState::Released, false));
    }

    #[test]
    fn unbound_keys_have_no_action() {
        assert_eq!(action_for_key(tab(), &ModifiersState::default()), None);
        assert_eq!(action_for_key(q(), &ModifiersState::default()), None);

        let shift_only = ModifiersState {
            shift: true,
            ..ModifiersState::default()
        };
        assert_eq!(
            action_for_key(Keysym::new(keysyms::KEY_a), &shift_only),
            None
        );
    }
}
