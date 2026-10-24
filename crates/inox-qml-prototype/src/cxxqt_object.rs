//! cxx-qt bridge between Rust state and the QML UI.
//!
//! The `#[cxx_qt::bridge]` macro generates the C++ ABI glue that lets QML
//! instantiate a Rust-backed `QObject`. This module mirrors the KDAB
//! v0.10.0 `cargo_without_cmake` example: a small counter object proving that
//! properties and a `qinvokable` survive the Rust/C++/QML round trip.

use core::pin::Pin;
use cxx_qt_lib::QString;

/// The Rust backing type for the QML element `MyObject`.
#[derive(Default)]
pub struct MyObjectRust {
    /// The number shown in QML; incremented by [`qobject::MyObject::increment_number`].
    pub number: i32,
    /// A free-form string property demonstrating `QString` plumbing.
    pub string: QString,
}

/// Expose `MyObject` and its `make_unique` constructor to Rust and QML.
///
/// Note: doc comments (and other attributes such as `#[doc]`) are not placed
/// on items inside the bridge blocks below; the cxx parser rejects them there.
/// Documentation lives on the Rust `MyObjectRust` struct and the plain-Rust
/// `impl qobject::MyObject` block outside this macro.
#[cxx_qt::bridge]
pub mod qobject {
    // `type QString` below is a cxx alias with no Rust-side documentation; the
    // cxx parser rejects //! / `///` comments inside bridge items, so allow the
    // missing_docs lint for the whole bridge instead of annotating each item.
    #[allow(missing_docs)]
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");

        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(i32, number)]
        #[qproperty(QString, string)]
        #[namespace = "my_object"]
        type MyObject = super::MyObjectRust;

        #[qinvokable]
        #[cxx_name = "incrementNumber"]
        fn increment_number(self: Pin<&mut Self>);
    }
}

/// Implementation of the `MyObject` QObject behaviour.
impl qobject::MyObject {
    /// Increment `number` by one and log the new value.
    pub fn increment_number(self: Pin<&mut Self>) {
        let previous = *self.number();
        self.set_number(previous + 1);
        tracing::debug!(
            previous,
            next = previous + 1,
            "my_object.number incremented"
        );
    }
}
