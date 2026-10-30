//! The safe Rust side of the native shell surface.
//!
//! [`Surface`] wraps the C++ host in `src/native/host.cpp` (through the bridge in
//! [`crate::native_bridge`]) and does the four things the panel's loop needs from
//! it: start the window with a shell role, hand over the projection, send the
//! placement the configuration decided, and read back what the compositor said.
//!
//! # What this is, and what the offscreen host was
//!
//! The window here is a real Wayland surface created by Qt's wayland platform
//! plugin. The compositor sizes it, draws it, animates it in and out, and sends
//! it the work area — and Qt's own scene graph draws frames on a
//! `wl_surface.frame` clock. Nothing is grabbed, nothing is uploaded, and the
//! pixels never pass through this process's memory (ADR 0046 §1).
//!
//! That is why the events below exist at all. An offscreen renderer was told a
//! size by the protocol leg and drew at it; a real surface is *told* nothing
//! until the compositor answers, and the answers are these events.
//!
//! # The events are a queue, not callbacks
//!
//! `take_event` pops one `(kind, a, b, c, d)` tuple per call. Callbacks would run
//! inside Qt's event dispatch — that is, inside a compositor round trip — and a
//! callback that sent a new request or repainted would re-enter the connection
//! in the middle of dispatch. A queue keeps every decision on the panel's own
//! turn of the loop, which is also what makes [`Event::decode`] a pure function
//! with tests.
//!
//! This module is compiled only where the build host's Qt can carry the private
//! ABI (`cfg(native_surface)`, decided by `build.rs`); without it the C++ it
//! refers to is not built at all (ADR 0046 §10).

use std::fmt;

use cxx_qt_lib::QString;
use serde_json::Value;

use crate::native_bridge::ffi::{Host, new_host};

/// The QML document the panel draws.
///
/// The native host loads the same document the offscreen one did: what changed is
/// the window behind it, not the projection inside it. The constant lives with
/// the offscreen host because that is where the module's resource path is
/// derived from (`build.rs`), and two spellings of one URL is exactly the kind of
/// drift that only shows up as a blank panel.
pub use crate::offscreen::QML_URL;

/// The shell role a surface carries (`inox_shell_v1.surface_role`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A standing bar: taskbar, menu bar, status area.
    Panel,
    /// An application dock, usually floating above the work area.
    Dock,
    /// A full-output shell surface: launcher, switcher.
    Overlay,
    /// A shell-owned surface that belongs to another shell surface.
    Menu,
}

impl Role {
    /// The protocol's own value.
    pub const fn code(self) -> u32 {
        match self {
            Self::Panel => 0,
            Self::Dock => 1,
            Self::Overlay => 2,
            Self::Menu => 3,
        }
    }
}

/// `inox_shell_v1.capability`, as a bitfield.
///
/// A client checks the bits it depends on and falls back to an instant,
/// unanimated presentation for anything missing. The bits are the protocol's, so
/// an unknown bit is ignored rather than fatal: that is what lets the field grow
/// without a version bump.
pub mod capability {
    /// Enter and exit transitions are drawn by the compositor.
    pub const TRANSITIONS: u32 = 1;
    /// A popup can be granted the keyboard independently of its parent.
    pub const POPUP_KEYBOARD: u32 = 2;
    /// The `workarea` event is sent and kept up to date.
    pub const WORKAREA: u32 = 4;
    /// A popup may be the parent of another popup (submenus).
    pub const NESTED_POPUPS: u32 = 8;
}

/// `inox_shell_surface_v1.layer`: where the surface is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layer {
    /// Wallpaper-like content.
    Background,
    /// Below ordinary windows.
    Bottom,
    /// Above ordinary windows.
    #[default]
    Top,
    /// Above everything, including other shell surfaces.
    Overlay,
}

impl Layer {
    /// The protocol's own value.
    pub const fn code(self) -> u32 {
        match self {
            Self::Background => 0,
            Self::Bottom => 1,
            Self::Top => 2,
            Self::Overlay => 3,
        }
    }
}

/// `inox_shell_surface_v1.anchor`, as a bitfield.
pub mod anchor {
    /// The top edge.
    pub const TOP: u32 = 1;
    /// The bottom edge.
    pub const BOTTOM: u32 = 2;
    /// The left edge.
    pub const LEFT: u32 = 4;
    /// The right edge.
    pub const RIGHT: u32 = 8;
}

/// One `set_placement` request.
///
/// The fields are the protocol's, in the protocol's order, because the request is
/// one atomic unit: a compositor that saw half of it would reserve the wrong
/// strip for a frame (ADR 0046 §2). `exclusive_zone` follows layer-shell's
/// meaning — a positive thickness to reserve, 0 for none, -1 to extend to the
/// anchored edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Geometry {
    /// The shell layer.
    pub layer: Layer,
    /// The edges the surface is attached to.
    pub anchor: u32,
    /// The strip to reserve, 0 for none, -1 to extend to the edge.
    pub exclusive_zone: i32,
    /// Distance from the top anchor.
    pub margin_top: i32,
    /// Distance from the right anchor.
    pub margin_right: i32,
    /// Distance from the bottom anchor.
    pub margin_bottom: i32,
    /// Distance from the left anchor.
    pub margin_left: i32,
    /// Requested width, 0 to let the compositor decide.
    pub width: u32,
    /// Requested height, 0 to let the compositor decide.
    pub height: u32,
}

/// `inox_shell_surface_v1.keyboard_mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardMode {
    /// Never focused; pointer only. The panel's mode for ordinary presentation
    /// (D01).
    None,
    /// Ordinary focus semantics.
    OnDemand,
    /// Wants the keyboard whenever mapped on top.
    Exclusive,
}

impl KeyboardMode {
    /// The protocol's own value.
    pub const fn code(self) -> u32 {
        match self {
            Self::None => 0,
            Self::OnDemand => 1,
            Self::Exclusive => 2,
        }
    }
}

/// Why the compositor dismissed a popup (`dismiss_reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DismissReason {
    /// The pointer was pressed outside the popup.
    Outside,
    /// The user pressed Escape.
    Escape,
    /// The parent surface unmapped or was closed.
    ParentHidden,
    /// The compositor closed it for another reason.
    Policy,
}

impl DismissReason {
    /// The reason a protocol value names, when it is one this build knows.
    pub const fn from_code(code: i32) -> Option<Self> {
        match code {
            0 => Some(Self::Outside),
            1 => Some(Self::Escape),
            2 => Some(Self::ParentHidden),
            3 => Some(Self::Policy),
            _ => None,
        }
    }
}

/// What the compositor told the surface.
///
/// Every variant is one event of `inox-shell-v1`; there is no "transition
/// finished" and none is invented here. A client that waited for one would hang
/// on a compositor that never animated anything (ADR 0046 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The optional parts of the protocol this compositor implements, and the
    /// longest transition it will draw.
    Capabilities {
        /// The capability bitfield; see [`capability`].
        bits: u32,
        /// The longest enter or exit transition, milliseconds; 0 for none.
        max_transition_ms: u32,
    },
    /// The size the surface has been given. Authoritative: a surface that draws
    /// at another size is showing the wrong thing.
    Configure {
        /// Configured width, logical pixels.
        width: u32,
        /// Configured height, logical pixels.
        height: u32,
    },
    /// The rectangle the surface may lay itself out against: the output minus
    /// every exclusive zone on it. The event layer-shell does not have.
    Workarea {
        /// Work area origin, output-logical.
        x: i32,
        /// Work area origin, output-logical.
        y: i32,
        /// Work area width, output-logical.
        width: i32,
        /// Work area height, output-logical.
        height: i32,
    },
    /// Whether this surface currently receives key events.
    Keyboard {
        /// 1 when focused.
        focused: bool,
    },
    /// The compositor will not show this surface. After this the object is inert.
    Closed,
    /// A popup was dismissed, and why.
    PopupDismissed {
        /// The id [`Surface::open_popup`] returned.
        popup: u32,
        /// What the compositor said, or `None` for a reason this build does not
        /// know.
        reason: Option<DismissReason>,
    },
    /// A popup's size.
    PopupConfigure {
        /// The id [`Surface::open_popup`] returned.
        popup: u32,
        /// Configured width, logical pixels.
        width: u32,
        /// Configured height, logical pixels.
        height: u32,
    },
    /// Whether a popup currently receives key events.
    PopupKeyboard {
        /// The id [`Surface::open_popup`] returned.
        popup: u32,
        /// 1 when focused.
        focused: bool,
    },
    /// The version 1 glass answer: the largest blur radius this compositor will
    /// apply to this surface, 0 for none.
    GlassCapabilities {
        /// Largest accepted blur radius, logical pixels.
        max_radius: u32,
    },
    /// The version 2 glass answer, which is the authoritative one: it repeats
    /// the blur maximum and adds the refraction maxima and flags.
    LiquidCapabilities {
        /// Largest accepted blur radius, logical pixels.
        max_radius: u32,
        /// Largest accepted refraction band thickness.
        max_refraction_height: u32,
        /// Largest accepted refraction displacement.
        max_refraction_amount: u32,
        /// The `refraction_flag` bits this compositor applies.
        flags: u32,
    },
}

impl Event {
    /// The event a `(kind, a, b, c, d)` tuple names, or `None` for an empty queue
    /// slot or a kind this build does not know.
    ///
    /// The mapping lives here rather than in C++ so that it can be asserted
    /// without a compositor: a wrong index in this function is invisible on a
    /// headless run and obvious in a test.
    pub const fn decode(kind: u32, a: i32, b: i32, c: i32, d: i32) -> Option<Self> {
        match kind {
            1 => Some(Self::Capabilities {
                bits: a as u32,
                max_transition_ms: b as u32,
            }),
            2 => Some(Self::Configure {
                width: a as u32,
                height: b as u32,
            }),
            3 => Some(Self::Workarea {
                x: a,
                y: b,
                width: c,
                height: d,
            }),
            4 => Some(Self::Keyboard { focused: a != 0 }),
            5 => Some(Self::Closed),
            6 => Some(Self::PopupDismissed {
                popup: a as u32,
                reason: DismissReason::from_code(b),
            }),
            7 => Some(Self::PopupConfigure {
                popup: a as u32,
                width: b as u32,
                height: c as u32,
            }),
            8 => Some(Self::PopupKeyboard {
                popup: a as u32,
                focused: b != 0,
            }),
            9 => Some(Self::GlassCapabilities {
                max_radius: a as u32,
            }),
            10 => Some(Self::LiquidCapabilities {
                max_radius: a as u32,
                max_refraction_height: b as u32,
                max_refraction_amount: c as u32,
                flags: d as u32,
            }),
            _ => None,
        }
    }
}

/// Why the native surface could not start.
#[derive(Debug, thiserror::Error)]
pub enum SurfaceError {
    /// The host refused, in its own words: the platform is not Wayland, the
    /// compositor offers no `inox_shell_v1`, or the document produced no window.
    /// There is no fallback — a panel that cannot be placed must not pretend to
    /// be (ADR 0046 §9).
    #[error("{0}")]
    Host(String),
    /// The projection could not be serialised. It cannot happen for the values
    /// `crate::view` builds (there are no non-finite numbers in it), but it is an
    /// error rather than a panic because the alternative is a process that dies
    /// mid-frame.
    #[error("the panel's view could not be serialised: {0}")]
    Projection(String),
}

/// One decided glass request, flattened into what the protocol takes.
///
/// [`inox_glass::Request::apply`] is the one place that decides *which* requests
/// a parameter set turns into — the corner radii and the band travel together,
/// and a version 1 object gets neither — so this type implements
/// [`inox_glass::GlassTarget`] and lets that function do it, instead of a second
/// copy of the rule living here. What it does not do is call the compositor: it
/// only accumulates, so the mapping is a pure function with tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Batch {
    /// The object's version, which decides whether refraction may be sent.
    version: u32,
    /// `set_radius`.
    radius: u32,
    /// `set_region`.
    region: inox_glass::Region,
    /// `set_corner_radii`, when a band went with it.
    corners: inox_glass::Corners,
    /// `set_refraction`, when there is a band.
    refraction: Option<(u32, u32, u32)>,
}

impl Batch {
    /// The requests `request` becomes on an object of `version`.
    fn of(request: &inox_glass::Request, version: u32) -> Self {
        let mut batch = Self {
            version,
            ..Self::default()
        };

        request.apply(&mut batch);

        batch
    }
}

impl inox_glass::GlassTarget for Batch {
    fn set_radius(&mut self, radius: u32) {
        self.radius = radius;
    }

    fn set_region(&mut self, region: inox_glass::Region) {
        self.region = region;
    }

    fn set_corner_radii(&mut self, corners: inox_glass::Corners) {
        self.corners = corners;
    }

    fn set_refraction(&mut self, height: u32, amount: u32, flags: u32) {
        self.refraction = Some((height, amount, flags));
    }

    fn supports_refraction(&self) -> bool {
        self.version >= 2
    }
}

/// The panel's shell surface: one window, one role, one queue of answers.
pub struct Surface {
    host: cxx::UniquePtr<Host>,
}

impl fmt::Debug for Surface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Surface")
            .field("size", &self.size())
            .field("configured", &self.configured())
            .finish()
    }
}

impl Surface {
    /// Loads the panel's QML document, gives its window the `role` shell role
    /// and shows it.
    ///
    /// # Errors
    ///
    /// [`SurfaceError::Host`] with the host's own message — a non-Wayland
    /// platform, a compositor without `inox_shell_v1`, a QML document that
    /// produced no window. Every one of those is fatal to the panel: it has no
    /// surface to be placed on, and the alternative would be a floating window
    /// nobody asked for.
    pub fn start(role: Role) -> Result<Self, SurfaceError> {
        Self::start_with_url(role, QML_URL)
    }

    /// Loads `url` instead of the panel's own document.
    ///
    /// The only caller is a test: an integration test drives the same host with a
    /// document it can assert on, over a real compositor connection.
    pub fn start_with_url(role: Role, url: &str) -> Result<Self, SurfaceError> {
        let host = new_host();
        let mut surface = Self { host };

        let mut error = QString::default();
        let started = surface
            .host
            .as_mut()
            .map(|host| host.start(&QString::from(url), role.code(), &mut error))
            .unwrap_or(false);

        if !started {
            return Err(SurfaceError::Host(surface.error_or(error)));
        }

        Ok(surface)
    }

    /// Replaces the projection the QML root draws.
    ///
    /// # Errors
    ///
    /// [`SurfaceError::Projection`] when the value cannot be serialised, and
    /// [`SurfaceError::Host`] when the window is gone.
    pub fn set_model(&mut self, model: &Value) -> Result<(), SurfaceError> {
        let json = serde_json::to_string(model)
            .map_err(|error| SurfaceError::Projection(error.to_string()))?;

        let mut error = QString::default();
        let applied = self
            .host
            .as_mut()
            .map(|host| host.set_model(&QString::from(json.as_str()), &mut error))
            .unwrap_or(false);

        if !applied {
            return Err(SurfaceError::Host(self.error_or(error)));
        }

        Ok(())
    }

    /// Sends the placement.
    ///
    /// # Errors
    ///
    /// [`SurfaceError::Host`] when the window has no shell role — which cannot
    /// happen after [`Surface::start`] succeeded, and is reported rather than
    /// ignored because a placement that silently did nothing is a panel in the
    /// wrong place.
    pub fn place(&mut self, geometry: &Geometry) -> Result<(), SurfaceError> {
        let mut error = QString::default();

        let applied = self
            .host
            .as_mut()
            .map(|host| {
                host.place(
                    geometry.layer.code(),
                    geometry.anchor,
                    geometry.exclusive_zone,
                    geometry.margin_top,
                    geometry.margin_right,
                    geometry.margin_bottom,
                    geometry.margin_left,
                    geometry.width,
                    geometry.height,
                    &mut error,
                )
            })
            .unwrap_or(false);

        if !applied {
            return Err(SurfaceError::Host(self.error_or(error)));
        }

        Ok(())
    }

    /// Sends the motion hint for the surface's enter and exit.
    ///
    /// A hint, not a promise: the compositor may drop it for reduced motion, a
    /// missing renderer or shutdown, and the panel must look right when it does.
    ///
    /// # Errors
    ///
    /// [`SurfaceError::Host`] when the window has no shell role.
    pub fn transition(
        &mut self,
        enter: u32,
        exit: u32,
        duration: u32,
        curve: u32,
        origin: u32,
    ) -> Result<(), SurfaceError> {
        let mut error = QString::default();

        let applied = self
            .host
            .as_mut()
            .map(|host| host.transition(enter, exit, duration, curve, origin, &mut error))
            .unwrap_or(false);

        if !applied {
            return Err(SurfaceError::Host(self.error_or(error)));
        }

        Ok(())
    }

    /// Sends the keyboard mode.
    ///
    /// # Errors
    ///
    /// [`SurfaceError::Host`] when the window has no shell role.
    pub fn keyboard(&mut self, mode: KeyboardMode) -> Result<(), SurfaceError> {
        let mut error = QString::default();

        let applied = self
            .host
            .as_mut()
            .map(|host| host.keyboard(mode.code(), &mut error))
            .unwrap_or(false);

        if !applied {
            return Err(SurfaceError::Host(self.error_or(error)));
        }

        Ok(())
    }

    /// Creates this surface's glass object, when the material asks for one.
    ///
    /// Called after [`Surface::start`], because the compositor answers a surface
    /// with no known desktop layer with a zero maximum (ADR 0046 §6), and it
    /// must be on **this** connection: a `inox_glass_v1` object names a
    /// `wl_surface`, and a proxy belongs to the connection that created both
    /// (ADR 0046 §8). That is why the call is here and not on the panel's own
    /// `wayland-client` connection.
    ///
    /// # Errors
    ///
    /// [`SurfaceError::Host`] with the host's own message — a compositor without
    /// `inox-glass-v1`, a window with no surface yet, a failed bind. None of them
    /// is fatal: the pane is drawn as plain translucency and the caller says so
    /// once, which is the degradation [`crate::frost`] already describes.
    pub fn start_glass(&mut self) -> Result<(), SurfaceError> {
        let mut error = QString::default();

        let created = self
            .host
            .as_mut()
            .map(|host| host.start_glass(&mut error))
            .unwrap_or(false);

        if created {
            Ok(())
        } else {
            Err(SurfaceError::Host(self.error_or(error)))
        }
    }

    /// Sends one decided parameter set.
    ///
    /// The request has already been clamped by [`inox_glass::decide`]; this only
    /// carries it. On a version 1 object the two refraction requests are not
    /// sent at all — the protocol has no such request there, and sending one
    /// would be a protocol error rather than a request the compositor ignores.
    ///
    /// # Errors
    ///
    /// [`SurfaceError::Host`] when the surface has no glass object.
    pub fn glass(&mut self, request: &inox_glass::Request) -> Result<(), SurfaceError> {
        let batch = Batch::of(request, self.glass_version());
        let corners = batch.corners;

        let (refraction, band_height, band_amount, flags) = match batch.refraction {
            Some((height, amount, flags)) => (true, height, amount, flags),
            None => (false, 0, 0, 0),
        };

        let mut error = QString::default();

        let applied = self
            .host
            .as_mut()
            .map(|host| {
                host.glass(
                    batch.radius,
                    batch.region.x,
                    batch.region.y,
                    batch.region.width,
                    batch.region.height,
                    u32::from(corners.top_left),
                    u32::from(corners.top_right),
                    u32::from(corners.bottom_right),
                    u32::from(corners.bottom_left),
                    refraction,
                    band_height,
                    band_amount,
                    flags,
                    &mut error,
                )
            })
            .unwrap_or(false);

        if !applied {
            return Err(SurfaceError::Host(self.error_or(error)));
        }

        Ok(())
    }

    /// The version of this surface's glass object: 0 for none.
    pub fn glass_version(&self) -> u32 {
        self.host
            .as_ref()
            .map(|host| host.glass_version())
            .unwrap_or(0)
    }

    /// Drains Qt's event dispatch once.
    ///
    /// This is the panel's only way of learning anything: `pump` moves the
    /// compositor's answers through Qt's dispatcher into the queue
    /// [`Surface::next_event`] reads, and the renderer's actions into
    /// [`Surface::next_action`].
    pub fn pump(&mut self) {
        if let Some(host) = self.host.as_mut() {
            host.pump();
        }
    }

    /// Opens a popup of this surface — a menu, a tooltip, a submenu.
    ///
    /// `parent_anchor` and `popup_anchor` are [`anchor`] masks: which edge or
    /// corner of the parent the position is measured from, and which edge or
    /// corner of the popup is placed on it. Two anchors rather than one is what
    /// makes a menu that flips expressible without the client guessing where the
    /// compositor put the parent.
    ///
    /// # Errors
    ///
    /// [`SurfaceError::Host`] when the surface is not started, or when the
    /// window's platform surface cannot take the popup role.
    pub fn open_popup(
        &mut self,
        parent_anchor: u32,
        popup_anchor: u32,
        offset: (i32, i32),
        size: (u32, u32),
        dismissable: bool,
    ) -> Result<u32, SurfaceError> {
        let mut error = QString::default();

        let id = self
            .host
            .as_mut()
            .map(|host| {
                host.open_popup(
                    parent_anchor,
                    popup_anchor,
                    offset.0,
                    offset.1,
                    size.0,
                    size.1,
                    dismissable,
                    &mut error,
                )
            })
            .unwrap_or(0);

        if id == 0 {
            return Err(SurfaceError::Host(self.error_or(error)));
        }

        Ok(id)
    }

    /// Asks the compositor to close a popup it is showing.
    ///
    /// The compositor runs the popup's exit transition and then discards the
    /// surface; the client learns that it happened through
    /// [`Event::PopupDismissed`], which is also the only signal it should use to
    /// animate its own exit.
    pub fn dismiss_popup(&mut self, popup: u32) -> bool {
        self.host
            .as_mut()
            .map(|host| host.dismiss_popup(popup))
            .unwrap_or(false)
    }

    /// Unmaps the surface without destroying it.
    ///
    /// This is how the compositor is told to dismiss every popup of this surface
    /// with `parent_hidden` — the panel does not have to tear down a menu tree by
    /// hand (ADR 0046 §2).
    pub fn hide(&mut self) {
        if let Some(host) = self.host.as_mut() {
            host.hide();
        }
    }

    /// The next event, if the queue has one.
    pub fn next_event(&mut self) -> Option<Event> {
        let (mut a, mut b, mut c, mut d) = (0, 0, 0, 0);

        let kind = self
            .host
            .as_mut()
            .map(|host| host.take_event(&mut a, &mut b, &mut c, &mut d))
            .unwrap_or(0);

        Event::decode(kind, a, b, c, d)
    }

    /// The next action the QML side produced, as the JSON object it wrote, or
    /// `None` when there is none.
    ///
    /// The bytes are QML's, and this function does not parse them: what a click
    /// means is [`crate::app`]'s question, and the parse belongs next to the
    /// `Message` it produces rather than here.
    pub fn next_action(&mut self) -> Option<Vec<u8>> {
        let bytes = self.host.as_mut().map(|host| host.take_action())?;
        let bytes = bytes.as_slice();

        (!bytes.is_empty()).then(|| bytes.to_vec())
    }

    /// The configured size, logical pixels. Zero before the first `configure`.
    pub fn size(&self) -> (i32, i32) {
        self.host
            .as_ref()
            .map(|host| (host.width(), host.height()))
            .unwrap_or((0, 0))
    }

    /// Whether the compositor has configured the surface.
    pub fn configured(&self) -> bool {
        self.host
            .as_ref()
            .map(|host| host.configured())
            .unwrap_or(false)
    }

    /// Whether the capability bitfield offered `bit`.
    ///
    /// The panel decides from this whether to ask for a transition or for a
    /// popup keyboard at all: asking for what was not offered makes the shell and
    /// the compositor disagree about what the user is looking at.
    pub fn offered(bits: u32, bit: u32) -> bool {
        bits & bit == bit
    }

    /// The host's own message for the last failure, or `error` when it has none.
    fn error_or(&self, error: QString) -> String {
        let message = self
            .host
            .as_ref()
            .map(|host| host.last_error().to_string())
            .filter(|message| !message.is_empty())
            .unwrap_or_else(|| error.to_string());

        if message.is_empty() {
            "the panel's native surface is not available".to_owned()
        } else {
            message
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_role_maps_to_the_protocol_value() {
        assert_eq!(Role::Panel.code(), 0);
        assert_eq!(Role::Dock.code(), 1);
        assert_eq!(Role::Overlay.code(), 2);
        assert_eq!(Role::Menu.code(), 3);
    }

    #[test]
    fn the_layers_are_ordered_back_to_front() {
        assert_eq!(Layer::Background.code(), 0);
        assert_eq!(Layer::Bottom.code(), 1);
        assert_eq!(Layer::Top.code(), 2);
        assert_eq!(Layer::Overlay.code(), 3);
        // The default is the layer a panel wants: above ordinary windows.
        assert_eq!(Layer::default(), Layer::Top);
    }

    #[test]
    fn the_keyboard_modes_are_the_protocols_three() {
        assert_eq!(KeyboardMode::None.code(), 0);
        assert_eq!(KeyboardMode::OnDemand.code(), 1);
        assert_eq!(KeyboardMode::Exclusive.code(), 2);
    }

    #[test]
    fn a_capability_comes_back_as_a_bitfield_and_a_duration() {
        let bits = capability::TRANSITIONS | capability::POPUP_KEYBOARD | capability::WORKAREA;

        assert_eq!(
            Event::decode(1, bits as i32, 250, 0, 0),
            Some(Event::Capabilities {
                bits,
                max_transition_ms: 250
            })
        );
        assert!(Surface::offered(bits, capability::TRANSITIONS));
        assert!(!Surface::offered(bits, capability::NESTED_POPUPS));
    }

    #[test]
    fn a_configure_carries_the_size_and_nothing_else() {
        assert_eq!(
            Event::decode(2, 1280, 28, 0, 0),
            Some(Event::Configure {
                width: 1280,
                height: 28
            })
        );
    }

    #[test]
    fn a_work_area_is_four_signed_numbers() {
        assert_eq!(
            Event::decode(3, 0, 0, 1280, 692),
            Some(Event::Workarea {
                x: 0,
                y: 0,
                width: 1280,
                height: 692
            })
        );
    }

    #[test]
    fn the_keyboard_state_is_one_integer() {
        assert_eq!(
            Event::decode(4, 1, 0, 0, 0),
            Some(Event::Keyboard { focused: true })
        );
        assert_eq!(
            Event::decode(4, 0, 0, 0, 0),
            Some(Event::Keyboard { focused: false })
        );
    }

    #[test]
    fn closure_is_its_own_event() {
        assert_eq!(Event::decode(5, 0, 0, 0, 0), Some(Event::Closed));
    }

    #[test]
    fn a_dismissal_carries_the_popup_and_the_reason() {
        assert_eq!(
            Event::decode(6, 3, 0, 0, 0),
            Some(Event::PopupDismissed {
                popup: 3,
                reason: Some(DismissReason::Outside)
            })
        );
        assert_eq!(
            Event::decode(6, 1, 1, 0, 0),
            Some(Event::PopupDismissed {
                popup: 1,
                reason: Some(DismissReason::Escape)
            })
        );
        assert_eq!(
            Event::decode(6, 1, 99, 0, 0),
            Some(Event::PopupDismissed {
                popup: 1,
                reason: None
            }),
            "a reason this build does not know is not a reason to drop the event"
        );
    }

    #[test]
    fn a_popup_configure_is_its_id_then_its_size() {
        assert_eq!(
            Event::decode(7, 2, 180, 240, 0),
            Some(Event::PopupConfigure {
                popup: 2,
                width: 180,
                height: 240
            })
        );
    }

    #[test]
    fn a_popup_keyboard_state_is_its_id_then_the_answer() {
        assert_eq!(
            Event::decode(8, 2, 1, 0, 0),
            Some(Event::PopupKeyboard {
                popup: 2,
                focused: true
            })
        );
    }

    #[test]
    fn the_two_glass_answers_are_one_integer_and_then_four() {
        assert_eq!(
            Event::decode(9, 64, 0, 0, 0),
            Some(Event::GlassCapabilities { max_radius: 64 })
        );
        assert_eq!(
            Event::decode(10, 64, 128, 32, 3),
            Some(Event::LiquidCapabilities {
                max_radius: 64,
                max_refraction_height: 128,
                max_refraction_amount: 32,
                flags: 3
            })
        );
    }

    #[test]
    fn an_empty_slot_and_an_unknown_kind_are_both_none() {
        assert_eq!(Event::decode(0, 0, 0, 0, 0), None);
        assert_eq!(Event::decode(99, 1, 2, 3, 4), None);
    }

    #[test]
    fn the_anchors_are_the_four_edges() {
        assert_eq!(anchor::TOP | anchor::LEFT, 1 | 4);
        assert_eq!(anchor::BOTTOM | anchor::RIGHT, 2 | 8);
    }

    #[test]
    fn the_document_is_the_panels_own() {
        assert_eq!(
            QML_URL, "qrc:/qt/qml/com/inox/panel/qml/qml/Main.qml",
            "the native host loads the document build.rs registers"
        );
    }

    /// A request with an 8 px corner and, when asked for, a refraction band.
    fn request(radius: u32, refraction: Option<(u32, u32)>) -> inox_glass::Request {
        inox_glass::Request {
            radius,
            region: inox_glass::Region::pane(640, 68),
            corners: inox_glass::Corners::uniform(8),
            refraction: refraction.map(|(height, amount)| inox_glass::RefractionRequest {
                height,
                amount,
                flags: 3,
            }),
        }
    }

    #[test]
    fn a_version_two_object_gets_the_whole_parameter_set() {
        let batch = Batch::of(&request(24, Some((12, 6))), 2);

        assert_eq!(batch.radius, 24);
        assert_eq!(batch.region, inox_glass::Region::pane(640, 68));
        assert_eq!(batch.corners, inox_glass::Corners::uniform(8));
        assert_eq!(
            batch.refraction,
            Some((12, 6, 3)),
            "the band and its flags are what the compositor refracts with"
        );
    }

    #[test]
    fn a_version_one_object_gets_no_refraction_vocabulary() {
        let batch = Batch::of(&request(24, Some((12, 6))), 1);

        assert_eq!(batch.radius, 24);
        assert_eq!(batch.region, inox_glass::Region::pane(640, 68));
        assert_eq!(
            batch.refraction, None,
            "`set_refraction` does not exist on a version 1 object: sending it would be a protocol \
             error, not a request the compositor ignores"
        );
        assert_eq!(
            batch.corners,
            inox_glass::Corners::ZERO,
            "the corner radii travel with the band and never alone"
        );
    }

    #[test]
    fn a_blur_only_request_never_describes_a_shape() {
        let batch = Batch::of(&request(24, None), 2);

        assert_eq!(batch.radius, 24);
        assert_eq!(batch.refraction, None);
        assert_eq!(
            batch.corners,
            inox_glass::Corners::ZERO,
            "`set_corner_radii` only changes the shape of the refraction, so a blur-only pane \
             sends none"
        );
    }

    #[test]
    fn turning_the_effect_off_asks_for_a_radius_and_an_empty_region() {
        let batch = Batch::of(&crate::frost::off(), 2);

        assert_eq!(batch.radius, 0);
        assert!(
            batch.region.is_empty(),
            "a region with no area is the protocol's \"nothing to frost\""
        );
        assert_eq!(batch.refraction, None);
    }
}
