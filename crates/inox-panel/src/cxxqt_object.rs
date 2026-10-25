//! The one QML ↔ Rust boundary of the panel: an opaque handle on the C++
//! offscreen host.
//!
//! The panel's QML is a **renderer only**: input, focus, placement
//! and state stay in Rust (`src/layer.rs`, `src/app.rs`, `src/placement.rs`).
//! So this bridge is deliberately narrow — no `#[qproperty]` per field, no
//! QObject with setters — and carries three things:
//!
//! - `load(url)`: the QML module window is loaded once, offscreen;
//! - `setModel(json)`: the whole Qt-free projection (`src/view.rs`) is handed
//!   over as one JSON string, replaced wholesale whenever the state changes
//!   (contract §3);
//! - `render(w, h)`: the current frame comes back as RGBA8 premultiplied bytes
//!   for the `wl_shm` buffer.
//!
//! `#[cxx_qt::bridge]` generates `unsafe extern "C++"` glue, which is why this
//! crate allows `unsafe_code` at its root.
//! Nothing else in this crate is `unsafe`.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("offscreen.h");
        include!("cxx-qt-lib/qbytearray.h");
        include!("cxx-qt-lib/qstring.h");

        /// The C++ offscreen QML host (`src/offscreen.cpp`).
        type Offscreen;
        /// Qt's byte array: the frame.
        type QByteArray = cxx_qt_lib::QByteArray;
        /// Qt's string: the URL and the projection.
        type QString = cxx_qt_lib::QString;

        /// Creates the host. The one `QGuiApplication` is created here, with
        /// the offscreen platform forced before it exists.
        fn new_offscreen() -> UniquePtr<Offscreen>;
        /// Loads the QML window at `url`.
        fn load(self: Pin<&mut Offscreen>, url: &QString) -> bool;
        /// Replaces the projection the QML root parses.
        #[cxx_name = "setModel"]
        fn set_model(self: Pin<&mut Offscreen>, model: &QString);
        /// Renders one frame of RGBA8 premultiplied pixels.
        fn render(self: Pin<&mut Offscreen>, width: i32, height: i32) -> QByteArray;
        /// The reason the last call failed, for a readable error and a
        /// non-zero exit.
        #[cxx_name = "lastError"]
        fn last_error(self: &Offscreen) -> QString;
        /// Whether the frame's alpha had to be reconstructed from two opaque
        /// grabs, because this Qt's offscreen backing store has no alpha
        /// channel. A diagnostic: it names the pixel path the last frame came
        /// from, which is what lets a test prove both paths run.
        #[cxx_name = "alphaReconstructed"]
        fn alpha_reconstructed(self: &Offscreen) -> bool;
    }
}
