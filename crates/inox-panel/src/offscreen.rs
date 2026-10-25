//! The safe Rust side of the offscreen QML host.
//!
//! [`Host`] wraps the opaque C++ handle in `src/offscreen.cpp` (through the
//! bridge in [`crate::cxxqt_object`]) and does the two things the protocol leg
//! needs from it: hand over the projection and get a frame back.
//!
//! It is a **renderer**, not a window. The platform is `offscreen` and the
//! buffer is a QQuickWindow: no Qt Wayland connection is opened, no
//! `xdg_toplevel` exists, and the panel's real surface stays the
//! `wlr-layer-shell` one [`crate::layer`] owns. A host that cannot
//! initialize is a readable error and a non-zero exit — never a fallback to a
//! window.
//!
//! The bytes `frame` returns are RGBA8, premultiplied, in the order
//! `QImage::Format_RGBA8888_Premultiplied` uses. [`crate::layer`] reorders them
//! into the `Argb8888` `wl_shm` wants; nothing here unpacks or blends.

use cxx_qt_lib::QString;

use crate::cxxqt_object::qobject::{Offscreen, new_offscreen};

/// The QML module window this crate ships.
///
/// The resource path is `qrc:/qt/qml/` + the module URI with dots turned into
/// directories + the QML file's own relative path; `build.rs` registers both
/// halves (contract §2.2).
pub const QML_URL: &str = "qrc:/qt/qml/com/inox/panel/qml/qml/Main.qml";

/// Why the offscreen renderer could not draw a frame.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// The QML module window could not be loaded, or the host produced no
    /// image. The message is the C++ host's, in words an operator can act on.
    #[error("the panel's QML renderer is not available: {0}")]
    Host(String),
    /// A frame came back the wrong length for the surface it was drawn for.
    #[error("the QML renderer returned {actual} bytes for a {expected}-byte frame")]
    FrameLength {
        /// `width * height * 4` for the configured size.
        expected: usize,
        /// What the renderer actually produced.
        actual: usize,
    },
    /// The projection could not be serialised. This cannot happen for the
    /// values [`crate::view`] builds — there are no non-finite numbers in it —
    /// but it is an error rather than a panic because the alternative would be
    /// a process that dies mid-frame.
    #[error("the panel's view could not be serialised: {0}")]
    Projection(String),
}

/// The panel's QML renderer: one per surface.
pub struct Host {
    inner: cxx::UniquePtr<Offscreen>,
}

impl Host {
    /// Loads the QML module window and returns the renderer.
    ///
    /// # Errors
    ///
    /// [`RenderError::Host`] when the QML window cannot be loaded — a missing
    /// QML import, a syntax error, a root object that is not a window, or no
    /// offscreen platform plugin. There is no fallback to an ordinary window:
    /// a panel that cannot render is a panel that reports the failure and exits
    /// non-zero.
    pub fn new() -> Result<Self, RenderError> {
        let mut inner = new_offscreen();

        let loaded = inner
            .as_mut()
            .map(|host| host.load(&QString::from(QML_URL)))
            .unwrap_or(false);

        if !loaded {
            return Err(RenderError::Host(error_of(&inner)));
        }

        Ok(Self { inner })
    }

    /// Hands the QML root the projection it should draw.
    ///
    /// # Errors
    ///
    /// [`RenderError::Projection`] when the value cannot be serialised.
    pub fn set_model(&mut self, model: &serde_json::Value) -> Result<(), RenderError> {
        let json = serde_json::to_string(model)
            .map_err(|error| RenderError::Projection(error.to_string()))?;

        let Some(host) = self.inner.as_mut() else {
            return Err(RenderError::Host("the QML host went away".to_owned()));
        };

        host.set_model(&QString::from(json.as_str()));

        Ok(())
    }

    /// Renders one `width` × `height` frame as RGBA8 premultiplied pixels.
    ///
    /// # Errors
    ///
    /// [`RenderError::Host`] when the renderer produced no image or an image of
    /// another size, and [`RenderError::FrameLength`] when the byte count is not
    /// the surface's.
    pub fn frame(&mut self, width: u32, height: u32) -> Result<Vec<u8>, RenderError> {
        let expected = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);

        let Some(host) = self.inner.as_mut() else {
            return Err(RenderError::Host("the QML host went away".to_owned()));
        };

        let (Ok(width), Ok(height)) = (i32::try_from(width), i32::try_from(height)) else {
            return Err(RenderError::Host(format!(
                "a {width}x{height} frame is not one this renderer can draw"
            )));
        };

        let bytes = host.render(width, height);

        if bytes.is_empty() {
            return Err(RenderError::Host(error_of(&self.inner)));
        }

        let bytes = bytes.as_slice().to_vec();

        if bytes.len() != expected {
            return Err(RenderError::FrameLength {
                expected,
                actual: bytes.len(),
            });
        }

        Ok(bytes)
    }
}

/// The C++ host's own message for the last failure, or a plain one.
fn error_of(host: &cxx::UniquePtr<Offscreen>) -> String {
    host.as_ref()
        .map(|host| host.last_error().to_string())
        .filter(|error| !error.is_empty())
        .unwrap_or_else(|| "the offscreen QML host did not start".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_module_url_is_the_shape_build_rs_registers() {
        // `build.rs` registers the `com.inox.panel.qml` module with
        // `qml/Main.qml`; the resource path is the URI with dots to slashes,
        // the file's own relative path appended.
        assert_eq!(
            QML_URL, "qrc:/qt/qml/com/inox/panel/qml/qml/Main.qml",
            "the URL and the module in build.rs have to agree"
        );
        assert!(
            std::path::Path::new("qml/Main.qml").exists(),
            "the file the module registers is in the crate"
        );
    }
}
