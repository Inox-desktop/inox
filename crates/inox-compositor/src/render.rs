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
//! # Glass (backdrop blur and refraction)
//!
//! Smithay 0.7 has no blur either, so [`HeadlessRenderer::render_elements`]
//! implements both halves of the `inox-glass-v1` effect itself (the protocol is
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
//! for a reason the renderer itself does not already reject, and both shaders
//! take the same three `#define` variants Smithay's own texture program does.
//!
//! ## The refraction pass (version 2)
//!
//! A pane that carries a [`LensPass`] is drawn by one extra fragment pass
//! instead of the copy-back of step 2 above:
//!
//! 1. The source rectangle is the pane region expanded by
//!    `ceil(radius) + ceil(amount)` and clipped to the target
//!    (`expanded_source_region`, crate-private and therefore deliberately not
//!    linked from here: a public module must not link to private items,
//!    `rustdoc::private_intra_doc_links`). The expansion is what gives refraction
//!    the pixels just outside the outline it must bend, and the blur a real
//!    neighbourhood at the pane border.
//! 2. That rectangle — not just the pane — is blurred into the scratch pair with
//!    the version 1 kernel, or identity-copied with `step = 0` when the radius is
//!    zero, so a refraction-only pane costs one copy and no blur.
//! 3. A pane-region-sized pass (`LENS_SHADER`) samples the scratch: a rounded-rect
//!    SDF with per-corner radii masks the shape (alpha 0 outside it, ±1 px of
//!    coverage at the outline), and inside a band of `height` pixels the sample
//!    coordinate is displaced along the SDF gradient by
//!    `(1 - sqrt(1 - t²)) * amount` towards the interior, so the backdrop appears
//!    pushed outwards at the outline and untouched beyond the band. The optional
//!    depth bit adds a radial component towards the pane centre, the optional
//!    dispersion bit splits the sample into seven chromatic taps.
//! 4. That result is drawn back over the pane region, and the client's own
//!    translucent surface is composited over it as usual.
//!
//! Both passes read their own scratch texture and write another one, so the only
//! texture ever sampled is one that is not attached to the framebuffer being
//! rendered into. Panes without a lens keep exactly the version 1 path: the same
//! calls, in the same order, on the same textures.
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

use crate::glass::{GlassPane, LensPass};

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

/// The fragment shader of the refraction pass.
///
/// It draws one pane-sized quad while sampling the blurred scratch texture, so
/// every constraint [`BLUR_SHADER`] carries applies here too: `#version 100`
/// first, the literal `//_DEFINES_` marker, one sampler, `alpha`, every tap
/// clamped into the half-texel inset. On top of that:
///
/// - the quad's texture coordinates span exactly the pane's sub-rectangle of the
///   scratch, so `v_coords * source_size - origin` is the pane-local pixel
///   coordinate with `y` running down from the pane's top-left corner — the same
///   orientation the protocol, `set_corner_radii` and the readbacks use.
/// - `centre` and `half` describe the pane's *shape*: the shape may start outside
///   the drawable region when the pane was clipped by the output edge, and the
///   rounded outline has to stay where the client drew its corners.
/// - the coverage mask, not the sampled alpha, decides the fragment's alpha: the
///   sampled pixels are always opaque by construction (the target is cleared to
///   an opaque background before any pane is drawn), so the `NO_ALPHA` variant
///   deliberately writes the same premultiplied value as the plain one. The shape
///   has to fade out at its outline in either variant, and nothing else in this
///   shader depends on the texture's own alpha.
/// - the corner radii are selected by quadrant, so the four values keep the wire
///   order (top-left, top-right, bottom-right, bottom-left) all the way to the
///   fragment.
/// - the chromatic taps are the reference's seven: red is displaced furthest,
///   blue least, which is what turns the outline into a colour fringe; with
///   dispersion off the pass samples once and the edge stays achromatic.
const LENS_SHADER: &str = r#"#version 100

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
uniform vec2 inset;
// The shape's centre and half extent, in pixels, relative to the drawn pane.
uniform vec2 centre;
uniform vec2 half_size;
// Where the drawn pane starts inside the sampled texture, in pixels.
uniform vec2 origin;
// The sampled texture's size, in pixels.
uniform vec2 source_size;
// Corner radii in pixels, in wire order.
uniform vec4 corners;
uniform float height;
uniform float amount;
// (depth, dispersion) as 0.0 / 1.0 switches.
uniform vec2 components;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

vec4 tap(vec2 uv) {
    return texture2D(tex, clamp(uv, inset, vec2(1.0) - inset));
}

// Signed distance to a rounded rectangle with per-corner radii, plus the
// outward unit gradient at `p` (which points outside the shape for the pixels a
// refraction can reach: sd <= 0 there).
float rounded_box(vec2 p, vec2 half_extent, vec4 radii, out vec2 gradient) {
    vec2 q = abs(p);
    float radius = p.x > 0.0
        ? (p.y > 0.0 ? radii.z : radii.y)
        : (p.y > 0.0 ? radii.w : radii.x);
    radius = min(radius, min(half_extent.x, half_extent.y));
    vec2 d = q - (half_extent - vec2(radius));

    vec2 direction = vec2(p.x >= 0.0 ? 1.0 : -1.0, p.y >= 0.0 ? 1.0 : -1.0);
    if (d.x > 0.0 && d.y > 0.0) {
        gradient = direction * normalize(d);
    } else if (d.x > d.y) {
        gradient = vec2(direction.x, 0.0);
    } else {
        gradient = vec2(0.0, direction.y);
    }

    return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0) - radius;
}

void main() {
    // Pane-local pixel coordinates, y down from the drawn pane's top-left corner.
    vec2 local = v_coords * source_size - origin;
    vec2 p = local - centre;

    vec2 gradient;
    float sd = rounded_box(p, half_size, corners, gradient);

    // One pixel of antialiasing at the outline, nothing outside it. Outside the
    // shape the backdrop must stay exactly as the scene drew it.
    float coverage = clamp(0.5 - sd, 0.0, 1.0);
    if (coverage <= 0.0) {
        gl_FragColor = vec4(0.0);
        return;
    }

    // The band: 1 on the outline, 0 where it reaches the interior.
    float t = clamp(1.0 + sd / max(height, 0.001), 0.0, 1.0);
    float bend = (1.0 - sqrt(max(1.0 - t * t, 0.0))) * amount;

    // Sampling towards the interior makes the backdrop near the outline appear
    // pushed outwards; that is the lens reading the reference produces.
    vec2 offset = -gradient * bend;

    if (components.x > 0.5) {
        // The optional depth component: a spherical term that pulls the sample
        // towards the pane centre, as if the pane were a dome rather than an
        // edge-only lens. It fades out with the band, so the interior stays
        // byte-identical to the plain blur.
        vec2 radial = centre - local;
        float reach = length(radial);
        radial = reach > 0.0 ? radial / reach : vec2(0.0, -1.0);
        offset += radial * (t * amount * 0.5);
    }

    vec2 uv = (local + origin + offset) / source_size;

    vec3 color;
    if (components.y > 0.5) {
        // Seven taps: red displaced furthest, blue least.
        vec2 shift = offset / source_size;
        color.r = 0.5 * (tap(uv + shift).r + tap(uv + shift * 0.75).r);
        color.g = 0.5 * (tap(uv + shift * 0.5).g + tap(uv + shift * 0.25).g);
        color.b = 0.5 * tap(uv).b
            + 0.25 * (tap(uv - shift * 0.25).b + tap(uv - shift * 0.5).b);
    } else {
        color = tap(uv).rgb;
    }

    // Premultiplied, with the shape's coverage as the alpha. The sampled pixels
    // are always opaque (the target is cleared to an opaque background before any
    // pane is drawn) and only their colour is read, so the NO_ALPHA variant needs
    // no separate branch: this mask is the shape's, not the texture's.
    gl_FragColor = vec4(color * coverage, coverage) * alpha;

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        gl_FragColor = vec4(0.0, 0.2, 0.0, 0.2) + gl_FragColor * 0.8;
#endif
}
"#;

/// The additional uniforms [`LENS_SHADER`] declares.
///
/// One list, used both to compile the program and to pin the declarations in the
/// unit tests: a name Smithay cannot find is silently ignored (GL reports location
/// -1), which is exactly the kind of failure that would otherwise only show as a
/// wrong-looking pane.
const LENS_UNIFORMS: &[(&str, UniformType)] = &[
    ("inset", UniformType::_2f),
    ("centre", UniformType::_2f),
    ("half_size", UniformType::_2f),
    ("origin", UniformType::_2f),
    ("source_size", UniformType::_2f),
    ("corners", UniformType::_4f),
    ("height", UniformType::_1f),
    ("amount", UniformType::_1f),
    ("components", UniformType::_2f),
];

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
    /// Unreachable by construction: the glass passes allocate (and reallocate)
    /// the scratch pair and the refraction target before the passes that read
    /// them, and return early when the allocation fails. The variant exists so
    /// this invariant does not have to be expressed with a panic.
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
/// Reallocated when a pass needs a different size than the current pair, and
/// reused across panes and frames otherwise: a pane runs at least two passes, so
/// allocating per pass would hand the driver a fresh texture for every frame. For
/// a refraction pane the size is the *expanded* source rectangle, not the pane.
struct Scratch {
    horizontal: GlesTexture,
    vertical: GlesTexture,
    size: Size<i32, Physical>,
}

/// The glass programs and scratch textures, compiled and allocated on first use.
///
/// One blur program serves both separable passes: the axis is the `step` uniform,
/// and compiling the same source twice would only produce two identical programs.
/// The refraction pass is a second program because its work is different, and its
/// output gets its own pane-sized texture: it is sampled by the draw-back element
/// while the scratch pair is still the source of the next pane.
#[derive(Default)]
struct GlassCache {
    program: Option<GlesTexProgram>,
    scratch: Option<Scratch>,
    lens_program: Option<GlesTexProgram>,
    lens: Option<GlesTexture>,
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

    /// The compiled refraction program, compiling it on first use.
    fn lens_program(&mut self, renderer: &mut GlesRenderer) -> Result<GlesTexProgram, RenderError> {
        if let Some(program) = &self.lens_program {
            return Ok(program.clone());
        }
        let uniforms = LENS_UNIFORMS
            .iter()
            .map(|(name, type_)| UniformName::new(*name, *type_))
            .collect::<Vec<_>>();
        let program = renderer
            .compile_custom_texture_shader(LENS_SHADER, &uniforms)
            .map_err(RenderError::from)?;
        self.lens_program = Some(program.clone());
        Ok(program)
    }

    /// The refraction output texture at `size`, allocating (or resizing) it.
    fn lens_target(
        &mut self,
        renderer: &mut GlesRenderer,
        size: Size<i32, Physical>,
    ) -> Result<GlesTexture, RenderError> {
        let wanted = Size::<i32, BufferCoord>::from((size.w, size.h));
        let stale = self.lens.as_ref().is_none_or(|lens| lens.size() != wanted);
        if stale {
            // `Abgr8888` like the target and the scratch pair: the refraction pass
            // writes premultiplied colour with a coverage alpha, so the texture
            // needs an alpha channel of its own.
            self.lens = Some(<GlesRenderer as Offscreen<GlesTexture>>::create_buffer(
                renderer,
                Fourcc::Abgr8888,
                wanted,
            )?);
        }
        self.lens.clone().ok_or(RenderError::MissingScratch)
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

            // The target is unbound now, so the glass passes may sample it. A pane
            // with a lens goes through the refraction pass; one without keeps the
            // version 1 blur path exactly as it was.
            let frost = match pane.lens {
                Some(lens) => lens_region(
                    renderer,
                    glass,
                    &target.buffer,
                    pane.region,
                    pane.radius,
                    &lens,
                )?,
                None => frost_region(renderer, glass, &target.buffer, pane.region, pane.radius)?,
            };

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

/// The source rectangle a pane's glass passes read: `region` expanded by the blur
/// radius and the refraction displacement, clipped to the target.
///
/// The expansion is what lets the passes behave at the pane's border: the blur
/// needs real neighbours outside the region it replaces, and the refraction
/// samples up to `amount` pixels away from where the outline was drawn. `radius`
/// and `amount` are in the source's pixels. The result always contains `region`:
/// clipping the expansion cannot cut into a region that already lies inside the
/// target.
#[must_use]
fn expanded_source_region(
    region: Rectangle<i32, Physical>,
    radius: f32,
    amount: f32,
    target: Rectangle<i32, Physical>,
) -> Rectangle<i32, Physical> {
    let pad = radius.max(0.0).ceil() as i32 + amount.max(0.0).ceil() as i32;
    let padded = Rectangle::new(
        Point::from((
            region.loc.x.saturating_sub(pad),
            region.loc.y.saturating_sub(pad),
        )),
        Size::from((
            region.size.w.saturating_add(pad.saturating_mul(2)),
            region.size.h.saturating_add(pad.saturating_mul(2)),
        )),
    );
    padded.intersection(target).unwrap_or(region)
}

/// Refract `region` of `source` and return the element that draws the result back
/// over it.
///
/// This is the version 2 pass of the module documentation: blur (or identity-copy)
/// the expanded source rectangle into the scratch pair, then run the refraction
/// shader over the pane region sampling that result. The caller must have unbound
/// `source`: this samples it.
fn lens_region(
    renderer: &mut GlesRenderer,
    glass: &mut GlassCache,
    source: &GlesTexture,
    region: Rectangle<i32, Physical>,
    radius: f32,
    lens: &LensPass,
) -> Result<TextureRenderElement<GlesTexture>, RenderError> {
    let program = glass.program(renderer)?;
    let source_size = source.size();
    let target = Rectangle::from_size(Size::<i32, Physical>::from((source_size.w, source_size.h)));
    let expanded = expanded_source_region(region, radius, lens.amount, target);

    glass.ensure_scratch(renderer, expanded.size)?;
    let (mut horizontal, mut vertical) = match glass.scratch.as_ref() {
        Some(scratch) => (scratch.horizontal.clone(), scratch.vertical.clone()),
        None => return Err(RenderError::MissingScratch),
    };

    let step = (radius / KERNEL_STEP_DIVISOR).max(0.0);
    // Same axis convention as `frost_region`: the kernel step is expressed
    // against the texture each pass samples.
    blur_pass(
        renderer,
        &mut horizontal,
        source,
        as_logical(expanded),
        (step / source_size.w.max(1) as f32, 0.0),
        program.clone(),
    )?;

    // `radius == 0` is the protocol's "no blur": one identity copy (`step = 0`
    // makes the kernel a no-op) and the refraction samples the backdrop exactly as
    // the scene drew it.
    let blurred = if step > 0.0 {
        let scratch_rect = Rectangle::new(
            Point::from((0.0, 0.0)),
            Size::from((f64::from(expanded.size.w), f64::from(expanded.size.h))),
        );
        blur_pass(
            renderer,
            &mut vertical,
            &horizontal,
            scratch_rect,
            (0.0, step / expanded.size.h.max(1) as f32),
            program,
        )?;
        vertical
    } else {
        horizontal
    };

    let lens_program = glass.lens_program(renderer)?;
    let mut lens_target = glass.lens_target(renderer, region.size)?;

    // Where the drawn pane starts inside the sampled texture, in pixels.
    let origin = (
        region.loc.x.saturating_sub(expanded.loc.x),
        region.loc.y.saturating_sub(expanded.loc.y),
    );
    let blurred_size = blurred.size();

    // The quad samples exactly the pane's own pixels of the blurred texture. That
    // is what makes `v_coords * source_size - origin` the pane-local pixel
    // coordinate in the shader.
    let pane_source = Rectangle::<f64, Logical>::new(
        Point::from((f64::from(origin.0), f64::from(origin.1))),
        Size::from((f64::from(region.size.w), f64::from(region.size.h))),
    );
    let buffer = TextureBuffer::from_texture(renderer, blurred, 1, Transform::Normal, None);
    let inner = TextureRenderElement::from_texture_buffer(
        (0.0, 0.0),
        &buffer,
        Some(1.0),
        Some(pane_source),
        None,
        Kind::Unspecified,
    );

    // The shape's centre relative to the drawn pane, in pixels, then its half
    // extent: a clipped pane keeps the outline of the rectangle its client drew.
    let centre = (
        (lens.shape.loc.x - region.loc.x) as f32 + lens.shape.size.w as f32 / 2.0,
        (lens.shape.loc.y - region.loc.y) as f32 + lens.shape.size.h as f32 / 2.0,
    );
    let half_size = (
        lens.shape.size.w as f32 / 2.0,
        lens.shape.size.h as f32 / 2.0,
    );
    let components = (
        if lens.depth { 1.0 } else { 0.0 },
        if lens.dispersion { 1.0 } else { 0.0 },
    );
    let inset = (
        0.5 / blurred_size.w.max(1) as f32,
        0.5 / blurred_size.h.max(1) as f32,
    );

    let element = TextureShaderElement::new(
        inner,
        lens_program,
        vec![
            Uniform::new("inset", UniformValue::_2f(inset.0, inset.1)),
            Uniform::new("centre", UniformValue::_2f(centre.0, centre.1)),
            Uniform::new("half_size", UniformValue::_2f(half_size.0, half_size.1)),
            Uniform::new(
                "origin",
                UniformValue::_2f(origin.0 as f32, origin.1 as f32),
            ),
            Uniform::new(
                "source_size",
                UniformValue::_2f(blurred_size.w as f32, blurred_size.h as f32),
            ),
            Uniform::new(
                "corners",
                UniformValue::_4f(
                    lens.corners[0],
                    lens.corners[1],
                    lens.corners[2],
                    lens.corners[3],
                ),
            ),
            Uniform::new("height", UniformValue::_1f(lens.height)),
            Uniform::new("amount", UniformValue::_1f(lens.amount)),
            Uniform::new("components", UniformValue::_2f(components.0, components.1)),
        ],
    );

    {
        let damage = [Rectangle::from_size(region.size)];
        let mut bound = renderer.bind(&mut lens_target)?;
        let mut frame = renderer.render(&mut bound, region.size, Transform::Normal)?;
        // The quad covers the whole target, but clearing keeps the result a
        // replace rather than an accumulation over a previous, differently sized
        // pane.
        frame.clear([0.0, 0.0, 0.0, 0.0].into(), &damage)?;
        drop(draw_render_elements::<GlesRenderer, _, _>(
            &mut frame,
            1.0,
            &[element],
            &damage,
        )?);
        drop(frame.finish()?);
    }

    // Drawn back 1:1 with the default texture program: this is a copy of the
    // refraction result, and the shader would only recompute it.
    let lens_rect = Rectangle::<f64, Logical>::new(
        Point::from((0.0, 0.0)),
        Size::from((f64::from(region.size.w), f64::from(region.size.h))),
    );
    let buffer = TextureBuffer::from_texture(renderer, lens_target, 1, Transform::Normal, None);
    Ok(TextureRenderElement::from_texture_buffer(
        region.loc.to_f64(),
        &buffer,
        Some(1.0),
        Some(lens_rect),
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

    /// The refraction shader has the same framework contract as the blur one, plus
    /// two of its own: every uniform the pass sets has to be declared somewhere
    /// Smithay can find it, and the taps have to be clamped like the blur's.
    #[test]
    fn the_lens_shader_is_a_gles2_texture_shader_smithay_can_variant() {
        assert!(
            LENS_SHADER.starts_with("#version 100\n"),
            "the source must begin with the version directive: Smithay injects nothing"
        );
        assert!(
            LENS_SHADER.contains("\n//_DEFINES_\n"),
            "Smithay replaces the `//_DEFINES_` line with the variant defines"
        );
        for define in ["EXTERNAL", "DEBUG_FLAGS", "GL_OES_EGL_image_external"] {
            assert!(
                LENS_SHADER.contains(define),
                "the source must handle the {define} case"
            );
        }
        assert!(
            LENS_SHADER.contains("uniform sampler2D tex;"),
            "the framework binds `tex` to unit 0"
        );
        assert!(
            LENS_SHADER.contains("uniform float alpha;"),
            "the framework sets `alpha`"
        );
        assert_eq!(
            LENS_SHADER.matches("sampler2D").count(),
            1,
            "only one sampler is bound: nothing else may be declared"
        );
        assert!(
            LENS_SHADER.contains("clamp(uv, inset, vec2(1.0) - inset)"),
            "`create_buffer` textures have no CLAMP_TO_EDGE wrap mode, so every \
             refraction tap has to stay a half texel inside the texture"
        );
        for (name, type_) in LENS_UNIFORMS {
            let declaration = match type_ {
                UniformType::_1f => format!("uniform float {name};"),
                UniformType::_2f => format!("uniform vec2 {name};"),
                UniformType::_4f => format!("uniform vec4 {name};"),
                other => panic!("{name} has an unexpected type {other:?}"),
            };
            assert!(
                LENS_SHADER.contains(&declaration),
                "the pass sets `{name}`, so the shader must declare `{declaration}`"
            );
        }
        // The shape and the displacement are the two things the pass is for: a
        // shader without them would compile and draw the plain blur again.
        assert!(
            LENS_SHADER.contains("rounded_box(") && LENS_SHADER.contains("+ sd /"),
            "the band has to be measured against the rounded-rect distance"
        );
        assert!(
            LENS_SHADER.contains("1.0 - sqrt(") && LENS_SHADER.contains("* amount"),
            "the reference displacement is (1 - sqrt(1 - t^2)) * amount"
        );
        for tap in [
            "tap(uv + shift)",
            "tap(uv + shift * 0.75)",
            "tap(uv + shift * 0.5)",
        ] {
            assert!(
                LENS_SHADER.contains(tap),
                "the dispersion branch loses the {tap} tap"
            );
        }
        assert_eq!(
            LENS_SHADER.matches("tap(uv").count(),
            8,
            "seven taps in the dispersion branch and the single one otherwise"
        );
    }

    #[test]
    fn the_source_region_is_expanded_by_the_blur_and_the_displacement() {
        let target = Rectangle::from_size(Size::from((320, 240)));
        let region = Rectangle::new(Point::from((100, 80)), Size::from((120, 60)));

        let expanded = expanded_source_region(region, 24.0, 16.0, target);
        assert_eq!(
            expanded,
            Rectangle::new(Point::from((60, 40)), Size::from((200, 140))),
            "ceil(blur) + ceil(amount) on every side, so refraction can sample \
             outside the outline and the blur has a real neighbourhood"
        );
        assert_eq!(
            expanded_source_region(region, 0.0, 0.0, target),
            region,
            "a pane that neither blurs nor bends reads nothing extra"
        );

        // Rounded up: a fractional radius must never read less than it needs.
        assert_eq!(
            expanded_source_region(region, 0.5, 0.5, target),
            Rectangle::new(Point::from((98, 78)), Size::from((124, 64)))
        );
    }

    #[test]
    fn the_source_region_never_leaves_the_target() {
        let target = Rectangle::from_size(Size::from((320, 240)));
        // The pane fills the target and asks for more than there is.
        let region = Rectangle::from_size(Size::from((320, 240)));
        assert_eq!(expanded_source_region(region, 64.0, 64.0, target), region);

        // A pane in the top-left corner keeps its inside, loses the outside: the
        // expansion is cut at the target's edge, so the padded rectangle is 52
        // wide starting at 0 rather than 64 starting at -12.
        let region = Rectangle::new(Point::from((0, 0)), Size::from((40, 40)));
        let expanded = expanded_source_region(region, 8.0, 4.0, target);
        assert_eq!(
            expanded,
            Rectangle::new(Point::from((0, 0)), Size::from((52, 52)))
        );
        assert_eq!(
            expanded.intersection(region),
            Some(region),
            "clipping the expansion may never cut into the pane itself"
        );
    }
}
