//! The cxx-qt bridge: one JSON projection into QML, and the offscreen host.
//!
//! ADR 0038 decides how this component carries QML: the lock surface stays a
//! real `ext_session_lock_v1` surface created by [`crate::session`], and QML is a
//! pure renderer whose frames are grabbed offscreen and written into the same
//! `wl_shm` buffer the crate's own renderer used to fill. This module is the
//! whole Rust <-> C++/QML boundary of that arrangement:
//!
//! - [`qobject::LockScreen`] is the QObject QML instantiates. It carries exactly
//!   one property, `modelJson`, which is the projection [`crate::view`] builds
//!   from [`crate::view_model::LockViewModel`]. There is no property per field,
//!   because a per-field bridge would be a second definition of the interface.
//! - [`qobject::Offscreen`] is the C++ host from `src/offscreen.cpp`: one
//!   `QQmlApplicationEngine` + `QQuickWindow` on the offscreen platform, one per
//!   lock surface, so several outputs can be rendered at their own sizes.
//!
//! # What may cross this boundary
//!
//! Only what a person can read: the headline, the lines under it, the field's
//! *mask* (one glyph per character typed), the control labels, and the colours
//! and sizes derived from [`inox_config::Theme`]. The credential itself never
//! enters the JSON, a QML property or a QML variable; it lives in
//! [`crate::credential::Secret`] and leaves the state only as
//! [`crate::app::OutgoingCommand::Authenticate`]. `view::screen` is
//! given the projection, which carries the mask's *length*, so there is no path
//! by which the plaintext could reach this file's property.
//!
//! []:

use cxx_qt_lib::QString;

/// The Rust backing type of the QML element `LockScreen`.
///
/// `model_json` is the whole projection: [`crate::render::Interface`] rebuilds it
/// whenever the state has moved and hands it to the QML object before grabbing a
/// frame. It starts empty, which QML reads as "nothing to draw yet".
#[derive(Default)]
pub struct LockScreenRust {
    /// The JSON document QML renders, and the only data QML receives.
    pub model_json: QString,
}

#[cxx_qt::bridge]
pub mod qobject {
    // Doc comments are not allowed on items inside a bridge (the cxx parser
    // rejects `///` there); the documentation is on [`super::LockScreenRust`] and
    // on the module above.
    #[allow(missing_docs)]
    unsafe extern "C++" {
        include!("offscreen.h");
        include!("cxx-qt-lib/qbytearray.h");
        include!("cxx-qt-lib/qstring.h");

        type Offscreen;
        type QByteArray = cxx_qt_lib::QByteArray;
        type QString = cxx_qt_lib::QString;

        #[cxx_name = "new_offscreen"]
        fn new_offscreen() -> UniquePtr<Offscreen>;

        #[cxx_name = "load"]
        fn load(self: Pin<&mut Offscreen>, url: &QString, width: i32, height: i32) -> bool;

        #[cxx_name = "setModel"]
        fn set_model(self: Pin<&mut Offscreen>, json: &QString);

        #[cxx_name = "render"]
        fn render(self: Pin<&mut Offscreen>, width: i32, height: i32) -> QByteArray;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, model_json, cxx_name = "modelJson")]
        type LockScreen = super::LockScreenRust;
    }
}
