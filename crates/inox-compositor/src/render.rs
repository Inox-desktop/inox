//! Headless rendering: a surfaceless EGL display, a GLES renderer and one
//! offscreen render target per output.
//!
//! Smithay 0.7 has no headless backend, so Inox builds this path
//! from Smithay's EGL and GLES abstractions: a surfaceless `EGLDisplay`
//! (`EGL_MESA_platform_surfaceless`), a configless `EGLContext`, a `GlesRenderer`
//! and a texture sized to the output mode.
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
//! # Frosted glass (backdrop blur)
//!
//! Smithay 0.7 has no blur either, so [`HeadlessRenderer::render_elements`]
//! implements the `inox-glass-v1` effect itself (the protocol is
//! [`crate::glass`]). The render target is a [`GlesTexture`] rather than a
//! renderbuffer precisely because a renderbuffer cannot be sampled and the
//! backdrop has to be read back as a texture to be blurred.
//!
//! The pass structure is driven by [`GlassPane`], whose `at` is the index of the
//! pane's own back-most render element in the front-to-back element list:
//!
//! 1. one frame is opened on the target, cleared to the background and the part
//!    of the scene behind the back-most pane is drawn into it;
//! 2. the frame is finished and the target unbound, then two region-sized
//!    separable passes run — one horizontal into a scratch texture, one vertical
//!    into a second scratch texture, both sampling what the previous step wrote.
//!    Sampling the target is legal exactly here, because the binding from step 1
//!    has been dropped: GL forbids reading a texture that is currently attached
//!    to the framebuffer being rendered into;
//! 3. the target is bound again (without clearing) and the vertical result is
//!    drawn back, clipped to the pane's region by the damage slice;
//! 4. steps 1–3 repeat for the next pane, back-most first, each one including
//!    the previous pane's pixels in its backdrop, and the frame finally draws
//!    the front-most part of the scene (including the panes' own pixels).
//!
//! Everything the panes need is derived from the protocol parameters by the pure
//! [`crate::glass::collect_panes`]; this module only draws. The blur kernel is a
//! 9-tap Gaussian with a step of `radius / 4`, which reads as frosted glass and
//! stays cheap enough for llvmpipe in CI. The shader is compiled once and cached
//! (`GlassCache`), as are the two scratch textures, which are reallocated when
//! a pane's region has a different size than the previous one.
//!
//! A glass pass that fails reports [`RenderError::Gles`] and the frame is not
//! presented, exactly like any other GLES failure on this path: there is no
//! fall-back to the sharp scene yet. Compilation is the one step that could fail
//! for a reason the renderer itself does not already reject, and this shader
//! takes the same three `#define` variants Smithay's own texture program does.
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
            Bind, ExportMem, Frame, Offscreen, Renderer, Texture,
            element::{
                Kind, RenderElement,
                memory::MemoryRenderBufferRenderElement,
                surface::WaylandSurfaceRenderElement,
                texture::{TextureBuffer, TextureRenderElement},
            },
            gles::{
                GlesError, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName,
                UniformType, UniformValue, element::TextureShaderElement,
            },
            utils::draw_render_elements,
        },
    },
    utils::{Buffer as BufferCoord, Logical, Physical, Point, Rectangle, Size, Transform},
};

use crate::glass::GlassPane;

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

/// How far apart the taps of the blur kernel are, as a fraction of the radius.
///
/// A 9-tap Gaussian with a step of `radius / 4` spans `±radius`, which is the
/// look the protocol's "frosted glass" asks for: enough blur to hide detail
/// behind a translucent pane without turning the backdrop into a flat wash.
const KERNEL_STEP_DIVISOR: f32 = 4.0;

/// The fragment shader of the separable blur.
///
/// Constraints, all of them load-bearing (see the module documentation and
/// Smithay's `compile_custom_texture_shader`, which injects nothing):
///
/// - `#version 100` first: the EGL context is GLES2.
/// - the literal `//_DEFINES_` marker: Smithay replaces it to build the plain,
///   `NO_ALPHA` and `EXTERNAL` variants plus a debug copy, so the source has to
///   compile under all of them. That is also why the sampler declaration and the
///   `tint` uniform are guarded the same way Smithay's own texture shader guards
///   them.
/// - no second sampler: Smithay binds `tex` to unit 0 and sets `alpha`; anything
///   else would have to be bound by hand, and that needs `unsafe`.
/// - `clamp` around every tap, to a half-texel inset: textures from
///   `create_buffer` get no `GL_CLAMP_TO_EDGE` wrap mode, so the default
///   `GL_REPEAT` applies, and GLES2 interpolates a coordinate of exactly `0.0` or
///   `1.0` between the outermost texel and the *wrapped-around* one. Clamping to
///   `0.0` alone would therefore pull the opposite edge of the texture into the
///   border of a frosted region; `inset` (half a texel of the sampled texture)
///   keeps every tap on a texel centre.
/// - the taps are unrolled because GLSL ES 1.00 has no array constructors, and
///   the loop bound has to be constant anyway.
///
/// `step` is one axis of the kernel in normalised texture coordinates, so the
/// same program serves both passes; `inset` is half a texel of whatever texture
/// the pass samples.
const BLUR_SHADER: &str = r#"#version 100

//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
uniform vec2 step;
uniform vec2 inset;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

// 9-tap Gaussian weights (sigma = 2 taps), normalised so they sum to one.
const float KERNEL_SUM = 0.2270270270 + 2.0 * (0.1945945946 + 0.1216216216 + 0.0540540541 + 0.0162162162);

vec4 tap(vec2 offset) {
    return texture2D(tex, clamp(v_coords + offset, inset, vec2(1.0) - inset));
}

void main() {
    vec4 color = tap(vec2(0.0, 0.0)) * 0.2270270270;
    color += tap(step) * 0.1945945946;
    color += tap(-step) * 0.1945945946;
    color += (tap(step * 2.0) + tap(step * -2.0)) * 0.1216216216;
    color += (tap(step * 3.0) + tap(step * -3.0)) * 0.0540540541;
    color += (tap(step * 4.0) + tap(step * -4.0)) * 0.0162162162;
    color /= KERNEL_SUM;

#if defined(NO_ALPHA)
    gl_FragColor = vec4(color.rgb, 1.0) * alpha;
#else
    gl_FragColor = color * alpha;
#endif

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        gl_FragColor = vec4(0.0, 0.2, 0.0, 0.2) + gl_FragColor * 0.8;
#endif
}
"#;

smithay::backend::renderer::element::render_elements! {
    pub(crate) SceneElement<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Cursor=MemoryRenderBufferRenderElement<GlesRenderer>,
}

/// Everything one output composites: the render elements plus the glass panes.
///
/// Kept together because they are two views of the same scene: a pane's [`GlassPane::at`]
/// is an index into `elements`, so separating them would let the two drift.
/// (`SceneElement` is not `Debug` — the `render_elements!` macro does not derive
/// it — so neither is this.)
#[derive(Default)]
pub(crate) struct Scene {
    /// Render elements, front to back, in the output's physical pixels.
    pub elements: Vec<SceneElement>,
    /// Frosted backdrops, back-most first (see [`crate::glass::collect_panes`]).
    pub glass: Vec<GlassPane>,
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
    /// A glass pane was drawn before its scratch texture was allocated.
    ///
    /// Unreachable by construction: `frost_region` allocates
    /// (and reallocates) the scratch pair before the passes that read it, and
    /// returns early when the allocation fails. The variant exists so this
    /// invariant does not have to be expressed with a panic.
    #[error("the glass scratch texture was never allocated")]
    MissingScratch,
}

/// One output's offscreen render target.
///
/// A texture, not a renderbuffer: the frosted-glass pass samples the backdrop,
/// and a [`GlesRenderbuffer`](smithay::backend::renderer::gles::GlesRenderbuffer)
/// cannot be sampled (it has no `Texture` implementation). Everything else — the
/// bind, the readbacks — behaves the same, because those go through the
/// framebuffer the texture is attached to.
struct Target {
    buffer: GlesTexture,
    size: Size<i32, Physical>,
}

/// The two region-sized scratch textures of the blur, and the size they hold.
///
/// Reallocated when a pane's region has a different size than the current pair,
/// and reused across panes and frames otherwise: the blur runs twice per pane, so
/// allocating per pass would hand the driver a fresh texture for every frame.
struct Scratch {
    horizontal: GlesTexture,
    vertical: GlesTexture,
    size: Size<i32, Physical>,
}

/// The blur program and scratch textures, compiled and allocated on first use.
///
/// One program serves both passes: the axis is the `step` uniform, and compiling
/// the same source twice would only produce two identical programs.
#[derive(Default)]
struct GlassCache {
    program: Option<GlesTexProgram>,
    scratch: Option<Scratch>,
}

impl GlassCache {
    /// The compiled blur program, compiling it on first use.
    fn program(&mut self, renderer: &mut GlesRenderer) -> Result<GlesTexProgram, RenderError> {
        if let Some(program) = &self.program {
            return Ok(program.clone());
        }
        let program = renderer
            .compile_custom_texture_shader(
                BLUR_SHADER,
                &[
                    UniformName::new("step", UniformType::_2f),
                    UniformName::new("inset", UniformType::_2f),
                ],
            )
            .map_err(RenderError::from)?;
        self.program = Some(program.clone());
        Ok(program)
    }

    /// Make sure both scratch textures exist at `size`.
    fn ensure_scratch(
        &mut self,
        renderer: &mut GlesRenderer,
        size: Size<i32, Physical>,
    ) -> Result<(), RenderError> {
        let stale = self
            .scratch
            .as_ref()
            .is_none_or(|scratch| scratch.size != size);
        if !stale {
            return Ok(());
        }

        let buffer_size = Size::<i32, BufferCoord>::from((size.w, size.h));
        // `Abgr8888` is what the target uses, and the two `Offscreen` impls of
        // `GlesRenderer` make a bare `create_buffer` call ambiguous, so name the
        // texture one.
        let horizontal = <GlesRenderer as Offscreen<GlesTexture>>::create_buffer(
            renderer,
            Fourcc::Abgr8888,
            buffer_size,
        )?;
        let vertical = <GlesRenderer as Offscreen<GlesTexture>>::create_buffer(
            renderer,
            Fourcc::Abgr8888,
            buffer_size,
        )?;
        self.scratch = Some(Scratch {
            horizontal,
            vertical,
            size,
        });
        Ok(())
    }
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
    glass: GlassCache,
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
            glass: GlassCache::default(),
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

    /// Render `elements` for `output` over the background colour, frosting `panes`.
    ///
    /// The target is allocated on first use and reallocated when the output's
    /// mode changes. `scale` is that output's scale factor: a render element
    /// carries its position in physical pixels but derives its extent from the
    /// logical size times this scale, so drawing an output at the wrong scale
    /// would place correctly and size incorrectly.
    ///
    /// `panes` comes from [`crate::glass::collect_panes`] and must already be
    /// expressed in this output's physical pixels, back-most first. With an empty
    /// list this is the plain compositing path — clear and draw, nothing else —
    /// which is what keeps every non-glass frame byte-for-byte what it was.
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
        panes: &[GlassPane],
    ) -> Result<(), RenderError>
    where
        E: RenderElement<GlesRenderer>,
    {
        // Destructured so the renderer, the target table and the glass cache are
        // borrowed separately: none of the passes may alias the others.
        let Self {
            renderer,
            targets,
            glass,
        } = self;
        ensure_target(renderer, targets, output, size)?;
        let target = targets
            .get_mut(output)
            .ok_or_else(|| RenderError::UnknownOutput {
                name: output.to_owned(),
            })?;
        let target_size = target.size;
        let damage = [Rectangle::from_size(target_size)];

        if panes.is_empty() {
            let mut bound = renderer.bind(&mut target.buffer)?;

            {
                let mut frame = renderer.render(&mut bound, target_size, Transform::Normal)?;
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

            return Ok(());
        }

        let mut cursor = elements.len();
        // The target is cleared exactly once, in the frame that draws the
        // back-most pane's backdrop; every later frame continues painting. A flag
        // rather than a comparison against `cursor`, because a pane at the very
        // back of the scene leaves `cursor` unchanged.
        let mut cleared = false;
        for pane in panes {
            // Everything strictly behind this pane that has not been drawn yet.
            let start = pane.at.saturating_add(1).min(elements.len());
            let end = cursor.min(elements.len());

            {
                let mut bound = renderer.bind(&mut target.buffer)?;
                let mut frame = renderer.render(&mut bound, target_size, Transform::Normal)?;
                if !cleared {
                    frame.clear(background.into(), &damage)?;
                    cleared = true;
                }
                if start < end {
                    drop(draw_render_elements::<GlesRenderer, _, _>(
                        &mut frame,
                        scale,
                        &elements[start..end],
                        &damage,
                    )?);
                }
                drop(frame.finish()?);
            }

            // The target is unbound now, so the blur may sample it.
            let frost = frost_region(renderer, glass, &target.buffer, pane.region, pane.radius)?;

            {
                let mut bound = renderer.bind(&mut target.buffer)?;
                let mut frame = renderer.render(&mut bound, target_size, Transform::Normal)?;
                // The damage slice is what clips the replace: it is the pane's
                // region, the only place the frosted backdrop may land.
                drop(draw_render_elements::<GlesRenderer, _, _>(
                    &mut frame,
                    1.0,
                    &[frost],
                    &[pane.region],
                )?);
                drop(frame.finish()?);
            }

            // Everything drawn so far is strictly behind index `at`; the pane's
            // own elements are index `at` and are painted by the final frame (or
            // by the next iteration, for a pane that is not the front-most).
            cursor = pane.at.saturating_add(1);
        }

        // The front-most part of the scene, panes' own pixels included.
        let end = cursor.min(elements.len());
        let mut bound = renderer.bind(&mut target.buffer)?;
        let mut frame = renderer.render(&mut bound, target_size, Transform::Normal)?;
        if end > 0 {
            drop(draw_render_elements::<GlesRenderer, _, _>(
                &mut frame,
                scale,
                &elements[..end],
                &damage,
            )?);
        }
        drop(frame.finish()?);

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
        let Self {
            renderer, targets, ..
        } = self;
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
        let Self {
            renderer, targets, ..
        } = self;
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

/// Blur `region` of `source` into the two scratch textures and return the element
/// that draws the result back over `region`.
///
/// The caller must have unbound `source`: this samples it. `radius` is in the
/// source's pixels (the pane already carries a scale-resolved radius).
fn frost_region(
    renderer: &mut GlesRenderer,
    glass: &mut GlassCache,
    source: &GlesTexture,
    region: Rectangle<i32, Physical>,
    radius: f32,
) -> Result<TextureRenderElement<GlesTexture>, RenderError> {
    let program = glass.program(renderer)?;
    glass.ensure_scratch(renderer, region.size)?;
    let Some(scratch) = glass.scratch.as_mut() else {
        return Err(RenderError::MissingScratch);
    };

    let source_size = source.size();
    let step = (radius / KERNEL_STEP_DIVISOR).max(0.0);
    // One axis of the kernel per pass, in the units the sampled texture is
    // normalised in: `step` is expressed against the texture being read, so the
    // horizontal pass divides by the target's width and the vertical pass by the
    // region's height (the scratch texture's size).
    let horizontal = step / source_size.w.max(1) as f32;
    let vertical = step / region.size.h.max(1) as f32;
    let source_region = as_logical(region);

    blur_pass(
        renderer,
        &mut scratch.horizontal,
        source,
        source_region,
        (horizontal, 0.0),
        program.clone(),
    )?;

    let scratch_region = Rectangle::new(
        Point::from((0.0, 0.0)),
        Size::from((f64::from(scratch.size.w), f64::from(scratch.size.h))),
    );
    blur_pass(
        renderer,
        &mut scratch.vertical,
        &scratch.horizontal,
        scratch_region,
        (0.0, vertical),
        program,
    )?;

    // The result is drawn 1:1 with the default texture program, not the blur one:
    // this is a copy, and the blur kernel would only be a no-op identity.
    let buffer = TextureBuffer::from_texture(
        renderer,
        scratch.vertical.clone(),
        1,
        Transform::Normal,
        None,
    );
    Ok(TextureRenderElement::from_texture_buffer(
        region.loc.to_f64(),
        &buffer,
        Some(1.0),
        Some(scratch_region),
        None,
        Kind::Unspecified,
    ))
}

/// The glass passes all run at scale 1, so a physical rectangle is also its own
/// source rectangle in logical units — the only conversion the render element API
/// needs, and one that keeps the two coordinate kinds from being mixed up.
fn as_logical(rect: Rectangle<i32, Physical>) -> Rectangle<f64, Logical> {
    Rectangle::new(
        Point::from((f64::from(rect.loc.x), f64::from(rect.loc.y))),
        Size::from((f64::from(rect.size.w), f64::from(rect.size.h))),
    )
}

/// Run one separable blur pass from `source` into `target`.
fn blur_pass(
    renderer: &mut GlesRenderer,
    target: &mut GlesTexture,
    source: &GlesTexture,
    source_region: Rectangle<f64, Logical>,
    step: (f32, f32),
    program: GlesTexProgram,
) -> Result<(), RenderError> {
    let size = target.size();
    let target_size: Size<i32, Physical> = Size::from((size.w, size.h));
    let damage = [Rectangle::from_size(target_size)];

    // Half a texel of the texture this pass reads: the shader clamps its taps to
    // `[inset, 1 - inset]` so that GL's default `GL_REPEAT` wrap mode can never
    // interpolate the opposite edge of the source into a clamped tap.
    let source_size = source.size();
    let inset = (
        0.5 / source_size.w.max(1) as f32,
        0.5 / source_size.h.max(1) as f32,
    );

    let buffer = TextureBuffer::from_texture(renderer, source.clone(), 1, Transform::Normal, None);
    let inner = TextureRenderElement::from_texture_buffer(
        (0.0, 0.0),
        &buffer,
        Some(1.0),
        Some(source_region),
        None,
        Kind::Unspecified,
    );
    let element = TextureShaderElement::new(
        inner,
        program,
        vec![
            Uniform::new("step", UniformValue::_2f(step.0, step.1)),
            Uniform::new("inset", UniformValue::_2f(inset.0, inset.1)),
        ],
    );

    let mut bound = renderer.bind(target)?;
    let mut frame = renderer.render(&mut bound, target_size, Transform::Normal)?;
    // The pass writes every pixel of the scratch texture, but clearing keeps the
    // result independent of whatever a previous, differently sized pane left
    // behind, and makes the blend a replace rather than an accumulation.
    frame.clear([0.0, 0.0, 0.0, 0.0].into(), &damage)?;
    drop(draw_render_elements::<GlesRenderer, _, _>(
        &mut frame,
        1.0,
        &[element],
        &damage,
    )?);
    drop(frame.finish()?);

    Ok(())
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
        // `Abgr8888` is what the GLES renderer can allocate, and the texture form
        // is required so the glass pass can sample the composited backdrop.
        let buffer = <GlesRenderer as Offscreen<GlesTexture>>::create_buffer(
            renderer,
            Fourcc::Abgr8888,
            Size::<i32, BufferCoord>::from(size),
        )?;
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

    /// The blur shader is handed to Smithay verbatim — `compile_custom_texture_shader`
    /// injects nothing, unlike the pixel-shader entry point — so the two things the
    /// framework requires of it have to be checked here: the version directive first
    /// and the marker it substitutes the variants into.
    #[test]
    fn the_blur_shader_is_a_gles2_texture_shader_smithay_can_variant() {
        assert!(
            BLUR_SHADER.starts_with("#version 100\n"),
            "the source must begin with the version directive: Smithay injects nothing"
        );
        assert!(
            BLUR_SHADER.contains("\n//_DEFINES_\n"),
            "Smithay replaces the `//_DEFINES_` line with the variant defines"
        );
        for define in ["EXTERNAL", "NO_ALPHA", "DEBUG_FLAGS"] {
            assert!(
                BLUR_SHADER.contains(define),
                "the source must handle the {define} variant"
            );
        }
        assert!(
            BLUR_SHADER.contains("uniform sampler2D tex;"),
            "the framework binds `tex` to unit 0"
        );
        assert!(
            BLUR_SHADER.contains("uniform float alpha;"),
            "the framework sets `alpha`"
        );
        assert!(
            !BLUR_SHADER.contains("sampler2D shadow")
                && BLUR_SHADER.matches("sampler2D").count() == 1,
            "only one sampler is bound: nothing else may be declared"
        );
        assert!(
            BLUR_SHADER.contains("clamp(v_coords"),
            "`create_buffer` textures have no CLAMP_TO_EDGE wrap mode"
        );
        assert!(
            BLUR_SHADER.contains("uniform vec2 inset;")
                && BLUR_SHADER.contains("vec2(1.0) - inset"),
            "clamping to a bare 0.0/1.0 would sample the wrapped-around texel"
        );
    }
}
