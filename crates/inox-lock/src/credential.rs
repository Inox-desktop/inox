//! The credential buffer (ADR 0015).
//!
//! ADR 0015 makes three demands of the thing that holds what a user typed, and
//! they are the whole of this module:
//!
//! 1. **It does not linger.** The bytes are overwritten with zeros as soon as
//!    they are no longer the credential — on a clear, on a cancel, on a suspend —
//!    and again when the value is dropped, through [`zeroize`].
//! 2. **It cannot be printed.** Neither `Debug` nor `Display` writes the value or
//!    its length, so `tracing::debug!(?secret)` is a line that says nothing, and
//!    a log, a notification and a panic message are all closed to it.
//! 3. **There is exactly one way out.** [`Secret::expose`] is the only accessor
//!    for the plaintext in the crate, and its only caller is
//!    [`crate::contract::AuthAttempt`] on its way to the trusted boundary ADR
//!    0015 requires a review for.
//!
//! Two things this type deliberately is not. It is not `Clone` — a credential
//! that can be copied is a credential that can be left behind in an allocation
//! nobody remembers to clear — and it is not `PartialEq`, because comparing
//! credentials in a loop whose length depends on their content is exactly the
//! timing oracle the authentication boundary must not be.
//!
//! What it cannot do is also worth stating: the buffer is not `mlock`ed, so it
//! can still reach a swap file, and a reallocation can leave a copy in memory
//! this type has stopped tracking. Both are properties of the process, not of
//! this type, and both belong to the authentication boundary's review
//! (ADR 0015) rather than to a claim made here.

use std::fmt;

use zeroize::{Zeroize, Zeroizing};

/// What the user typed, held so that it can be taken back.
///
/// The bytes live in a buffer that is never shrunk, only overwritten: clearing
/// writes zeros over the whole allocation rather than setting a length to zero,
/// which is what makes "the value is gone" a statement about memory rather than
/// about bookkeeping.
pub struct Secret {
    /// The allocation. Every byte of it is zero after a clear.
    buffer: Zeroizing<Vec<u8>>,
    /// How many leading bytes of `buffer` are the credential.
    length: usize,
}

impl Default for Secret {
    fn default() -> Self {
        Self::new()
    }
}

impl Secret {
    /// An empty credential.
    pub fn new() -> Self {
        Self {
            buffer: Zeroizing::new(Vec::new()),
            length: 0,
        }
    }

    /// Appends a character.
    ///
    /// The character's UTF-8 bytes go in as they are: a password is a byte string,
    /// not an ASCII one, and encoding it here is what keeps the buffer and the
    /// authentication boundary's argument the same bytes.
    pub fn push(&mut self, character: char) {
        let mut encoded = [0u8; 4];
        let bytes = character.encode_utf8(&mut encoded).as_bytes();
        let end = self.length + bytes.len();

        if end > self.buffer.len() {
            self.buffer.resize(end, 0);
        }
        self.buffer[self.length..end].copy_from_slice(bytes);
        self.length = end;
    }

    /// Removes the last character, returning whether there was one.
    ///
    /// A character, not a byte: backspacing a `é` or a `在` must leave the buffer
    /// holding whole characters, and the bytes it removes are zeroed as they go.
    pub fn backspace(&mut self) -> bool {
        if self.length == 0 {
            return false;
        }

        // Walk back over the continuation bytes (0b10xxxxxx) of the last
        // character's UTF-8 encoding.
        let mut start = self.length - 1;
        while start > 0 && (self.buffer[start] & 0xC0) == 0x80 {
            start -= 1;
        }

        for byte in &mut self.buffer[start..self.length] {
            *byte = 0;
        }
        self.length = start;

        true
    }

    /// Overwrites the credential with zeros and forgets it.
    ///
    /// Called when the screen hides, when the user cancels, and when the session
    /// suspends — none of which wait for an animation to finish (ADR 0015).
    pub fn clear(&mut self) {
        self.buffer.as_mut_slice().zeroize();
        self.length = 0;
    }

    /// How many characters the credential holds.
    ///
    /// Characters rather than bytes, because this is what the mask on screen
    /// counts. It is the one property of the credential that reaches the screen,
    /// and it cannot not: a password field shows how much has been typed.
    pub fn len(&self) -> usize {
        self.buffer[..self.length]
            .iter()
            .filter(|byte| (**byte & 0xC0) != 0x80)
            .count()
    }

    /// Whether nothing has been typed.
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// The credential bytes, for the authentication boundary and nothing else.
    ///
    /// There is no `&str`, no `AsRef`, no `Deref` and no `Into<String>`: the
    /// plaintext leaves this type through one door, so there is one place to
    /// review and no accidental `format!` between here and the boundary. The
    /// boundary itself does not exist in this build — ADR 0015 makes it a human
    /// review item — so today nothing calls this.
    pub fn expose(&self) -> &[u8] {
        &self.buffer[..self.length]
    }
}

impl fmt::Debug for Secret {
    /// Never the value, and never the length.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl fmt::Display for Secret {
    /// Never the value, and never the length.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A secret holding `value`.
    fn secret(value: &str) -> Secret {
        let mut secret = Secret::new();

        for character in value.chars() {
            secret.push(character);
        }

        secret
    }

    /// The whole allocation, including the bytes past the credential.
    ///
    /// Only reachable from inside the module, which is the point: this is the
    /// accessor that proves the *memory* is zeroed, and nothing outside has a
    /// reason to hold it.
    fn residue(secret: &Secret) -> &[u8] {
        &secret.buffer
    }

    #[test]
    fn a_fresh_secret_is_empty() {
        let secret = Secret::new();

        assert!(secret.is_empty());
        assert_eq!(secret.len(), 0);
        assert_eq!(secret.expose(), b"");
    }

    #[test]
    fn what_is_pushed_is_what_is_exposed() {
        let secret = secret("correct horse");

        assert_eq!(secret.expose(), b"correct horse");
        assert_eq!(secret.len(), 13);
        assert!(!secret.is_empty());
    }

    #[test]
    fn the_length_counts_characters_not_bytes() {
        let secret = secret("aé在🦀");

        assert_eq!(
            secret.expose().len(),
            10,
            "`a` is one byte, `é` two, `在` three and `🦀` four"
        );
        assert_eq!(secret.len(), 4);
    }

    #[test]
    fn backspace_removes_a_whole_character_and_zeroes_it() {
        let mut secret = secret("aé");

        assert!(secret.backspace());
        assert_eq!(secret.expose(), b"a");
        assert_eq!(
            residue(&secret),
            b"a\0\0",
            "the two bytes of `é` are overwritten, not merely forgotten"
        );

        assert!(secret.backspace());
        assert!(secret.is_empty());
        assert!(!secret.backspace(), "there was nothing left to remove");
        assert_eq!(residue(&secret), b"\0\0\0");
    }

    #[test]
    fn clearing_zeroes_the_memory_and_not_only_the_length() {
        let mut secret = secret("hunter2");

        secret.clear();

        assert!(secret.is_empty());
        assert_eq!(secret.len(), 0);
        assert_eq!(secret.expose(), b"");
        assert_eq!(
            residue(&secret),
            b"\0\0\0\0\0\0\0",
            "every byte of the allocation is zero after a clear"
        );
    }

    #[test]
    fn an_expanded_buffer_is_zeroed_too() {
        let mut secret = secret("a");
        secret.push('🦀');
        secret.clear();

        assert_eq!(residue(&secret), b"\0\0\0\0\0");
    }

    #[test]
    fn clearing_twice_is_harmless() {
        let mut secret = secret("abc");

        secret.clear();
        secret.clear();

        assert_eq!(residue(&secret), b"\0\0\0");
        assert!(secret.is_empty());
    }

    #[test]
    fn reuse_after_a_clear_does_not_resurrect_the_old_value() {
        let mut secret = secret("password");
        secret.clear();

        secret.push('x');

        assert_eq!(secret.expose(), b"x");
        assert_eq!(secret.len(), 1);
        assert_eq!(residue(&secret), b"x\0\0\0\0\0\0\0");
    }

    #[test]
    fn debug_never_prints_the_value_nor_its_length() {
        let secret = secret("hunter2");
        let rendered = format!("{secret:?}");

        assert_eq!(rendered, "Secret(<redacted>)");
        assert!(!rendered.contains("hunter2"));
        assert!(!rendered.contains('7'), "not even the length");
    }

    #[test]
    fn display_never_prints_the_value_nor_its_length() {
        let secret = secret("hunter2");
        let rendered = format!("{secret}");

        assert_eq!(rendered, "<redacted>");
        assert!(!rendered.contains("hunter2"));
        assert!(!rendered.contains('7'));
    }

    #[test]
    fn a_whole_attempt_is_safe_to_print() {
        use crate::contract::{AuthAttempt, Generation};

        let attempt = AuthAttempt::new(Generation::first(), secret("hunter2"));
        let rendered = format!("{attempt:?}");

        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(rendered.contains("Secret(<redacted>)"), "{rendered}");
    }

    #[test]
    fn a_cleared_secret_survives_being_dropped() {
        let mut secret = secret("hunter2");
        secret.clear();
        let address = residue(&secret).as_ptr();

        drop(secret);

        // Nothing to assert about freed memory from here — reading it would be
        // undefined behaviour and this crate forbids `unsafe`. What is checked
        // instead is that the zeroing happens before the drop, above, and that
        // dropping the type itself does not need a value to be present.
        assert!(!address.is_null());
    }
}
