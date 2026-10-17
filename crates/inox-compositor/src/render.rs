//! Headless rendering: a surfaceless EGL display, a GLES renderer and one
//! offscreen render target per output.
//!
//! Smithay 0.7 has no headless backend (see ADR 0005), so Inox builds this path
//! from Smithay's EGL and GLES abstractions: a surfaceless `EGLDisplay`
//! (`EGL_MESA_platform_surfaceless`), a configless `EGLContext`, a `GlesRenderer`
//! and a renderbuffer sized to the output mode.
//!
//! Since M2 the render target also composites the client windows that the
//! compositor manages: [`HeadlessRenderer::render_elements`] clears to the
//! background colour and then draws the render elements produced by the desktop
//! `Space` (see [`crate::state`]).
//!
//! Two readbacks make the drawn pixels observable:
//! [`HeadlessRenderer::read_center_pixel`] for the integration tests and
//! [`HeadlessRenderer::read_frame_rgba`] for the whole frame (the test-input
//! channel's `dump-frame` command).
//!
//! # Unsafe code
//!
//! This module holds the crate's `unsafe` allowance, which is localised to
//! [`HeadlessRenderer::new`]: `EGLDisplay` and `GlesRenderer` can only be
//! constructed through `unsafe` constructors whose contracts are tied to
//! process-wide EGL/GL state. Both calls are wrapped into a safe API and carry
//! `// SAFETY:` justifications.

use std::collections::HashMap;

use smithay::{
    backend::{
        allocator::Fourcc,
        egl::{EGLContext, EGLDisplay, Error as EglError, native::EGLSurfacelessDisplay},
        renderer::{
            Bind, ExportMem, Frame, Offscreen, Renderer,
            element::RenderElement,
            element::{
                memory::MemoryRenderBufferRenderElement, surface::WaylandSurfaceRenderElement,
            },
            gles::{GlesError, GlesRenderbuffer, GlesRenderer},
            utils::draw_render_elements,
        },
    },
    utils::{Buffer as BufferCoord, Physical, Rectangle, Size, Transform},
};

/// Background colour of the headless output, as 8-bit RGBA.
pub const CLEAR_COLOR_RGBA8: [u8; 4] = [17, 34, 51, 255];

/// [`CLEAR_COLOR_RGBA8`] in the normalised floats GL expects.
pub const CLEAR_COLOR: [f32; 4] = [
    CLEAR_COLOR_RGBA8[0] as f32 / 255.0,
    CLEAR_COLOR_RGBA8[1] as f32 / 255.0,
    CLEAR_COLOR_RGBA8[2] as f32 / 255.0,
    CLEAR_COLOR_RGBA8[3] as f32 / 255.0,
];

/// Bytes per pixel of the render target, which uses [`Fourcc::Abgr8888`].
const BYTES_PER_PIXEL: usize = 4;

smithay::backend::renderer::element::render_elements! {
    pub(crate) SceneElement<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Cursor=MemoryRenderBufferRenderElement<GlesRenderer>,
}

/// Errors produced by the headless renderer.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// An offscreen target has invalid or unsupported dimensions.
    #[error("invalid render target size {width}x{height}")]
    InvalidSize {
        /// Requested width.
        width: i32,
        /// Requested height.
        height: i32,
    },
    /// A readback or headless render was requested on a physical output.
    #[error("this operation requires a headless render target")]
    NotHeadless,
    /// The EGL display or context could not be created.
    #[error("failed to create the EGL display or context")]
    Egl(#[from] EglError),

    /// A GLES call failed.
    #[error("GLES rendering failed")]
    Gles(#[from] GlesError),

    /// The render target was read back as fewer bytes than its geometry requires.
    #[error("short readback: expected {expected} bytes, got {actual}")]
    ShortReadback {
        /// Number of bytes the readback needed to contain.
        expected: usize,
        /// Number of bytes the readback produced.
        actual: usize,
    },
    /// No target is allocated for the named output.
    ///
    /// Only reachable when a caller reads back an output the headless backend has
    /// never rendered, which the compositor does not do: every read goes through
    /// the output it just drew.
    #[error("no headless render target for output `{name}`")]
    UnknownOutput {
        /// The output name that has no target.
        name: String,
    },
}

/// One output's offscreen render target.
struct Target {
    buffer: GlesRenderbuffer,
    size: Size<i32, Physical>,
}

/// An offscreen GLES render target per headless output.
///
/// Targets are created on first use and resized when an output's mode changes, so
/// an output that disappears releases its buffer and one that comes back gets a
/// fresh one. All of them share the single EGL display and GLES renderer: a
/// process may only hold one EGL display (see the `SAFETY` note in
/// [`HeadlessRenderer::new`]), and composites every output with it.
pub struct HeadlessRenderer {
    renderer: GlesRenderer,
    targets: HashMap<String, Target>,
}

impl HeadlessRenderer {
    /// Create the EGL/GLES renderer.
    ///
    /// No render target is allocated here: each output's target appears with the
    /// first frame it renders.
    ///
    /// # Errors
    ///
    /// Returns a [`RenderError`] when no usable GL driver is available.
    #[allow(unsafe_code)]
    pub fn new() -> Result<Self, RenderError> {
        // SAFETY: `EGLDisplay::new` takes ownership of the platform display and calls
        // `eglTerminate` when it is dropped. The compositor obtains its EGL display
        // through this single path, so there is no other owner to invalidate.
        let display = unsafe { EGLDisplay::new(EGLSurfacelessDisplay) }?;
        let context = EGLContext::new(&display)?;

        // SAFETY: `GlesRenderer::new` requires the context not to be current on any
        // other thread. The compositor is single-threaded, and this context was just
        // created here, so no other thread can be using it.
        let renderer = unsafe { GlesRenderer::new(context) }?;

        Ok(Self {
            renderer,
            targets: HashMap::new(),
        })
    }

    /// The underlying GLES renderer.
    ///
    /// The compositor needs it to import client buffers and produce the render
    /// elements for the desktop `Space` before calling
    /// [`HeadlessRenderer::render_elements`].
    pub fn renderer_mut(&mut self) -> &mut GlesRenderer {
        &mut self.renderer
    }

    /// Render `elements` for `output` over the background colour.
    ///
    /// The target is allocated on first use and reallocated when the output's
    /// mode changes. `scale` is that output's scale factor: a render element
    /// carries its position in physical pixels but derives its extent from the
    /// logical size times this scale, so drawing an output at the wrong scale
    /// would place correctly and size incorrectly (ADR 0020).
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::Gles`] when any GLES call fails.
    pub fn render_elements<E>(
        &mut self,
        output: &str,
        size: (i32, i32),
        scale: f64,
        background: [f32; 4],
        elements: &[E],
    ) -> Result<(), RenderError>
    where
        E: RenderElement<GlesRenderer>,
    {
        // Destructured so the renderer and the target table are borrowed
        // separately: binding a framebuffer needs both at once.
        let Self { renderer, targets } = self;
        ensure_target(renderer, targets, output, size)?;
        let target = targets
            .get_mut(output)
            .ok_or_else(|| RenderError::UnknownOutput {
                name: output.to_owned(),
            })?;
        let size = target.size;
        let damage = [Rectangle::from_size(size)];

        let mut bound = renderer.bind(&mut target.buffer)?;

        {
            let mut frame = renderer.render(&mut bound, size, Transform::Normal)?;
            frame.clear(background.into(), &damage)?;

            // Smithay handles front-to-back ordering and element-local damage.
            // This target always redraws fully, so no retained damage is needed.
            drop(draw_render_elements::<GlesRenderer, _, _>(
                &mut frame, scale, elements, &damage,
            )?);

            // The readback performed by callers blocks until the frame is complete, so
            // the sync point is deliberately dropped rather than waited on.
            drop(frame.finish()?);
        }

        Ok(())
    }

    /// Read back the pixel at the centre of `output`'s target, as RGBA bytes.
    ///
    /// Integration tests use this as evidence that a frame really reached the render
    /// target. The colour is returned with the raw 8-bit precision of the target, so
    /// callers should compare with a one-step tolerance.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::Gles`] when the readback fails,
    /// [`RenderError::UnknownOutput`] when nothing was rendered for `output`, and
    /// [`RenderError::ShortReadback`] when the mapping is smaller than expected.
    pub fn read_center_pixel(&mut self, output: &str) -> Result<[u8; 4], RenderError> {
        let Self { renderer, targets } = self;
        let target = targets
            .get_mut(output)
            .ok_or_else(|| RenderError::UnknownOutput {
                name: output.to_owned(),
            })?;
        let size = target.size;

        let region = Rectangle::from_size(Size::<i32, BufferCoord>::from((size.w, size.h)));

        let bound = renderer.bind(&mut target.buffer)?;
        let mapping = renderer.copy_framebuffer(&bound, region, Fourcc::Abgr8888)?;
        let data = renderer.map_texture(&mapping)?;

        // `Abgr8888` readbacks are tightly packed, so the stride is the width in bytes.
        let stride = size.w as usize * BYTES_PER_PIXEL;
        let offset = (size.h as usize / 2) * stride + (size.w as usize / 2) * BYTES_PER_PIXEL;

        data.get(offset..offset + BYTES_PER_PIXEL)
            .and_then(|bytes| <[u8; BYTES_PER_PIXEL]>::try_from(bytes).ok())
            .ok_or(RenderError::ShortReadback {
                expected: offset + BYTES_PER_PIXEL,
                actual: data.len(),
            })
    }

    /// Read back all of `output`'s target as tightly packed RGBA8 bytes.
    ///
    /// Row-major, top row first, four bytes per pixel with no padding — the layout
    /// the PPM writer of the test-input channel (`crate::test_input`) consumes.
    /// The colour order matches [`HeadlessRenderer::read_center_pixel`], so callers
    /// compare with a one-step tolerance for the same reason.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::Gles`] when the readback fails,
    /// [`RenderError::UnknownOutput`] when nothing was rendered for `output`, and
    /// [`RenderError::ShortReadback`] when the mapping is smaller than the frame.
    pub fn read_frame_rgba(&mut self, output: &str) -> Result<Vec<u8>, RenderError> {
        let Self { renderer, targets } = self;
        let target = targets
            .get_mut(output)
            .ok_or_else(|| RenderError::UnknownOutput {
                name: output.to_owned(),
            })?;
        let size = target.size;
        let region = Rectangle::from_size(Size::<i32, BufferCoord>::from((size.w, size.h)));

        let bound = renderer.bind(&mut target.buffer)?;
        let mapping = renderer.copy_framebuffer(&bound, region, Fourcc::Abgr8888)?;
        let data = renderer.map_texture(&mapping)?;

        // `Abgr8888` readbacks are tightly packed, so the frame is exactly this long.
        let expected = size.w as usize * size.h as usize * BYTES_PER_PIXEL;
        if data.len() < expected {
            return Err(RenderError::ShortReadback {
                expected,
                actual: data.len(),
            });
        }

        Ok(data[..expected].to_vec())
    }
}

/// Allocate `output`'s target, or reallocate it after a mode change.
fn ensure_target(
    renderer: &mut GlesRenderer,
    targets: &mut HashMap<String, Target>,
    output: &str,
    size: (i32, i32),
) -> Result<(), RenderError> {
    validate_size(size)?;
    let wanted: Size<i32, Physical> = size.into();

    let stale = match targets.get(output) {
        Some(target) => target.size != wanted,
        None => true,
    };
    if stale {
        // `Abgr8888` is the only format the GLES renderer can allocate as a
        // renderbuffer without extra capabilities.
        let buffer =
            renderer.create_buffer(Fourcc::Abgr8888, Size::<i32, BufferCoord>::from(size))?;
        targets.insert(
            output.to_owned(),
            Target {
                buffer,
                size: wanted,
            },
        );
    }

    Ok(())
}

fn validate_size((width, height): (i32, i32)) -> Result<(), RenderError> {
    if !(1..=crate::config::MAX_DIMENSION).contains(&width)
        || !(1..=crate::config::MAX_DIMENSION).contains(&height)
    {
        return Err(RenderError::InvalidSize { width, height });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_target_dimensions_are_rejected() {
        for size in [(-1, 1), (1, 0), (i32::MAX, 1), (0, 0)] {
            assert!(
                matches!(validate_size(size), Err(RenderError::InvalidSize { .. })),
                "expected {size:?} to be rejected"
            );
        }
        assert!(validate_size((1, 1)).is_ok());
        assert!(validate_size((16_384, 16_384)).is_ok());
    }
}
