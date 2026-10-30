//! The `cxx` bridge to the native shell surface (`src/native/host.h`).
//!
//! It is a plain `cxx` bridge rather than a `cxx_qt` one: nothing here is a
//! `QObject`, and the host is a value whose lifetime the Rust side owns. That is
//! deliberate — the role objects QML and Qt create are Qt's, and the only thing
//! Rust needs is a handle that lives exactly as long as the panel's loop.
//!
//! Four things cross, and nothing else does:
//!
//! - the QML document's URL, so the one place that knows where the panel's QML
//!   module lives stays [`crate::offscreen::QML_URL`]'s sibling constant in
//!   [`crate::native_surface`];
//! - the projection, as one JSON string (contract §3);
//! - the placement, as the protocol's own fields — integers, so a C++ struct
//!   with a different memory layout can never be read as a Rust one;
//! - the events and actions coming back, as integers and one JSON string.
//!
//! Every fallible call takes `error: &mut QString` and answers with a `bool`:
//! the message is the host's own, in words an operator can act on, and a
//! `std::exception` crossing the boundary would be a panic in a translation unit
//! that must not throw.
//!
//! This module is compiled only where the build host's Qt can carry the private
//! ABI (`cfg(native_surface)`): without it the C++ it refers to is not built at
//! all (ADR 0046 §10).

#[cxx::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("native/host.h");
        include!("cxx-qt-lib/qbytearray.h");
        include!("cxx-qt-lib/qstring.h");

        /// The native host (`src/native/host.cpp`).
        type Host;
        /// Qt's byte array: the action QML produced.
        type QByteArray = cxx_qt_lib::QByteArray;
        /// Qt's string: the URL, the projection and every error message.
        type QString = cxx_qt_lib::QString;

        /// Creates the host, and with it the process's one `QGuiApplication`.
        fn new_host() -> UniquePtr<Host>;

        /// Loads `url`, installs `role` on its window and shows it.
        fn start(self: Pin<&mut Host>, url: &QString, role: u32, error: &mut QString) -> bool;

        /// Replaces the projection the QML root parses.
        fn set_model(self: Pin<&mut Host>, json: &QString, error: &mut QString) -> bool;

        /// Sends one `set_placement` request.
        #[allow(clippy::too_many_arguments)]
        fn place(
            self: Pin<&mut Host>,
            layer: u32,
            anchor: u32,
            exclusive_zone: i32,
            margin_top: i32,
            margin_right: i32,
            margin_bottom: i32,
            margin_left: i32,
            width: u32,
            height: u32,
            error: &mut QString,
        ) -> bool;

        /// Sends one `set_transition` request.
        fn transition(
            self: Pin<&mut Host>,
            enter: u32,
            exit: u32,
            duration: u32,
            curve: u32,
            origin: u32,
            error: &mut QString,
        ) -> bool;

        /// Sends one `set_keyboard` request.
        fn keyboard(self: Pin<&mut Host>, mode: u32, error: &mut QString) -> bool;

        /// Opens a popup of this surface, positioned by relation to it.
        #[allow(clippy::too_many_arguments)]
        fn open_popup(
            self: Pin<&mut Host>,
            parent_anchor: u32,
            popup_anchor: u32,
            x: i32,
            y: i32,
            width: u32,
            height: u32,
            dismissable: bool,
            error: &mut QString,
        ) -> u32;

        /// Asks the compositor to close a popup.
        fn dismiss_popup(self: Pin<&mut Host>, popup: u32) -> bool;

        /// Creates the surface's glass object, if the compositor offers one.
        fn start_glass(self: Pin<&mut Host>, error: &mut QString) -> bool;

        /// Sends one `inox-glass-v1` parameter set, in the protocol's order.
        ///
        /// One argument per protocol field, like `place`: a struct would have to
        /// be a shared type, and the values are the protocol's own numbers.
        #[allow(clippy::too_many_arguments)]
        fn glass(
            self: Pin<&mut Host>,
            radius: u32,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            top_left: u32,
            top_right: u32,
            bottom_right: u32,
            bottom_left: u32,
            refraction: bool,
            band_height: u32,
            band_amount: u32,
            flags: u32,
            error: &mut QString,
        ) -> bool;

        /// The glass object's version: 0 for none, 1 or 2 otherwise.
        fn glass_version(self: &Host) -> u32;

        /// Unmaps the surface without destroying it.
        fn hide(self: Pin<&mut Host>);

        /// Drains Qt's event dispatch once.
        fn pump(self: Pin<&mut Host>);

        /// Pops the next shell event, as a kind and four integers.
        fn take_event(
            self: Pin<&mut Host>,
            a: &mut i32,
            b: &mut i32,
            c: &mut i32,
            d: &mut i32,
        ) -> u32;

        /// Pops the next action QML produced, or an empty array.
        fn take_action(self: Pin<&mut Host>) -> QByteArray;

        /// The configured width, logical pixels; 0 before the first configure.
        fn width(self: &Host) -> i32;
        /// The configured height, logical pixels; 0 before the first configure.
        fn height(self: &Host) -> i32;
        /// Whether the compositor has configured the surface.
        fn configured(self: &Host) -> bool;
        /// The host's own words for the last failure.
        fn last_error(self: &Host) -> QString;
    }
}
