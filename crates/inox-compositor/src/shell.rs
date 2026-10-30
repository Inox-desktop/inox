//! The compositor half of `inox-shell-v1`.
//!
//! This module owns the *state* of the shell surfaces and popups: what a client
//! asked for, what is committed, where each surface goes, which of them has the
//! keyboard, and what the compositor has to tell them. The protocol objects
//! themselves are dispatched in [`crate::state`], because their handlers need the
//! whole compositor (the seat, the outputs, the render pass); everything that can
//! be decided without those lives here, which is also what makes it testable
//! without a GPU.
//!
//! Three rules from the protocol are load-bearing and are enforced here rather
//! than in the dispatch:
//!
//! - **State is committed, not applied on arrival.** `set_placement`,
//!   `set_keyboard` and `set_transition` write into a pending slot; only
//!   [`ShellState::commit`] copies it into the live state, so a reload that
//!   changes four values at once looks like one change. It also means the
//!   compositor can never observe half a placement and reserve the wrong strip.
//! - **Dismissal belongs to the compositor.** A popup declares that it may be
//!   dismissed; the outside click, the Escape key and the parent-unmapping rule
//!   are applied here, the popup is told *why* through `dismissed`, and the click
//!   that dismissed it is swallowed instead of reaching the window underneath.
//! - **Motion is a hint.** [`sample`] may be told to return "instant" for any
//!   reason — reduced motion, no renderer, a compositor shutting down — and every
//!   caller has to keep working when it does. There is deliberately no
//!   "transition finished" signal a client could wait on.
//!
//! Placement is deliberately *not* re-implemented: a [`Placement`] is turned into
//! the same [`LayerSurfaceCachedState`] `zwlr_layer_shell_v1` uses and handed to
//! [`crate::layers::geometry`] / [`crate::layers::reserve`]. Two shells on one
//! screen that disagreed about where a bar of `exclusive_zone = 32` ends would be
//! a bug the user sees, and there is no reason for the arithmetic to exist twice.

use std::time::{Duration, Instant};

use inox_protocols::shell::inox_shell::server::{
    inox_shell_popup_v1::{DismissReason, InoxShellPopupV1},
    inox_shell_surface_v1::InoxShellSurfaceV1,
    inox_shell_v1::Capability,
};
use smithay::{
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle, Size},
    wayland::shell::wlr_layer::{Anchor, ExclusiveZone, LayerSurfaceCachedState, Margins},
};

use crate::{layers, topology::OutputId};

/// The longest enter/exit transition this compositor animates, in milliseconds.
///
/// Advertised in `inox_shell_v1.capabilities` and used to clamp every
/// `set_transition`, so a client cannot ask for a ten-second slide that outlives
/// the interaction that opened it.
pub(crate) const MAX_TRANSITION_MS: u32 = 250;

/// The `fade` bit of the protocol's effect enum.
pub(crate) const EFFECT_FADE: u32 = 1;
/// The `slide` bit of the protocol's effect enum.
pub(crate) const EFFECT_SLIDE: u32 = 2;
/// The `scale` bit of the protocol's effect enum.
pub(crate) const EFFECT_SCALE: u32 = 4;

/// The capability bitfield this compositor reports.
///
/// Every bit is set because every part is implemented: transitions are drawn by
/// [`crate::render`], popups are granted the keyboard independently of their
/// parent, `workarea` is sent and kept up to date, and a popup may be the parent
/// of another popup.
pub(crate) fn capabilities() -> Capability {
    Capability::Transitions
        | Capability::PopupKeyboard
        | Capability::Workarea
        | Capability::NestedPopups
}

/// The shell layer a surface is drawn in, back to front.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ShellLayer {
    /// Wallpaper-like content.
    Background,
    /// Below ordinary windows.
    Bottom,
    /// Above ordinary windows.
    Top,
    /// Above everything, other shell surfaces included.
    Overlay,
}

impl ShellLayer {
    /// The layer a wire value names, or `None` for a value this build does not
    /// know.
    pub(crate) fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            0 => Self::Background,
            1 => Self::Bottom,
            2 => Self::Top,
            3 => Self::Overlay,
            _ => return None,
        })
    }
}

/// Whether a semantic role is one this compositor knows.
///
/// The role is a policy label (`panel`, `dock`, `overlay`, `menu`), so an unknown
/// one is refused rather than guessed: a compositor that drew a launcher as a
/// bar would be worse than one that said no.
pub(crate) fn role_is_known(role: u32) -> bool {
    role <= 3
}

/// The anchor bits the protocol defines.
pub(crate) const ANCHOR_TOP: u32 = 1;
/// The bottom anchor bit.
pub(crate) const ANCHOR_BOTTOM: u32 = 2;
/// The left anchor bit.
pub(crate) const ANCHOR_LEFT: u32 = 4;
/// The right anchor bit.
pub(crate) const ANCHOR_RIGHT: u32 = 8;

/// Whether `anchor` contains only bits the protocol defines.
pub(crate) fn anchor_is_known(anchor: u32) -> bool {
    anchor & !(ANCHOR_TOP | ANCHOR_BOTTOM | ANCHOR_LEFT | ANCHOR_RIGHT) == 0
}

/// Where a shell surface asked to be placed.
///
/// One struct rather than six setters, mirroring the protocol: anchor, margin,
/// exclusive zone and size only mean something together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Placement {
    /// The shell layer.
    pub layer: ShellLayer,
    /// Anchor bitfield, already validated against the protocol's four bits.
    pub anchor: u32,
    /// Strip to reserve: positive for a thickness, 0 for none, -1 to extend.
    pub exclusive_zone: i32,
    /// Distances from the anchored edges, as `[top, right, bottom, left]`.
    ///
    /// An array rather than [`Margins`] so a placement can be compared: "did the
    /// client change anything" is what decides whether this is a new placement at
    /// all, and `Margins` is not `PartialEq`.
    pub margins: [i32; 4],
    /// Requested size; 0 on an axis lets the compositor decide.
    pub size: Size<i32, Logical>,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            layer: ShellLayer::Top,
            anchor: 0,
            exclusive_zone: 0,
            margins: [0; 4],
            size: Size::from((0, 0)),
        }
    }
}

impl Placement {
    /// The shape the shared placement arithmetic understands.
    ///
    /// The anchor bits were validated when the request arrived, so
    /// `from_bits_retain` cannot silently drop one here.
    pub(crate) fn cached(self) -> LayerSurfaceCachedState {
        let [top, right, bottom, left] = self.margins;

        LayerSurfaceCachedState {
            size: self.size,
            anchor: Anchor::from_bits_retain(self.anchor),
            exclusive_zone: match self.exclusive_zone {
                zone if zone > 0 => ExclusiveZone::Exclusive(zone.unsigned_abs()),
                // `-1` is the protocol's "do not move me, let me reach the edge",
                // and any other non-positive value means the same thing to
                // `layers::reserve`: reserve nothing.
                _ => ExclusiveZone::Neutral,
            },
            margin: Margins {
                top,
                right,
                bottom,
                left,
            },
            ..Default::default()
        }
    }

    /// Whether this placement asks the compositor for an exclusive strip.
    fn reserves(self) -> bool {
        self.exclusive_zone > 0
    }
}

/// An enter or exit transition the compositor draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Transition {
    /// Effect bitfield used when the surface maps.
    pub enter: u32,
    /// Effect bitfield used when the surface unmaps.
    pub exit: u32,
    /// Duration in milliseconds, clamped to [`MAX_TRANSITION_MS`].
    pub duration_ms: u32,
    /// Timing curve.
    pub curve: u32,
    /// Anchor point the motion grows from; empty means the centre.
    pub origin: u32,
}

impl Transition {
    /// Whether this transition would animate anything at all.
    pub(crate) fn is_animated(self) -> bool {
        self.duration_ms > 0 && (self.enter != 0 || self.exit != 0)
    }
}

/// Clamp a requested transition to what this compositor will animate.
pub(crate) fn clamp_transition(mut transition: Transition) -> Transition {
    transition.duration_ms = transition.duration_ms.min(MAX_TRANSITION_MS);
    let known = EFFECT_FADE | EFFECT_SLIDE | EFFECT_SCALE;
    transition.enter &= known;
    transition.exit &= known;

    transition
}

/// What one frame of a transition looks like.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sample {
    /// Opacity multiplier, 0 (invisible) to 1.
    pub alpha: f32,
    /// Translation, in the surface's own logical coordinates.
    pub offset: Point<f64, Logical>,
    /// Uniform scale about the transition's origin.
    pub scale: f64,
}

impl Sample {
    /// The sample that changes nothing.
    pub(crate) const FULL: Self = Self {
        alpha: 1.0,
        offset: Point::new(0.0, 0.0),
        scale: 1.0,
    };

    /// Whether this sample leaves the surface exactly as it drew itself.
    pub(crate) fn is_full(self) -> bool {
        self.alpha >= 1.0 && self.offset == Point::new(0.0, 0.0) && (self.scale - 1.0).abs() < 1e-9
    }
}

/// Evaluate a transition.
///
/// `elapsed_ms` is time since the transition started. `entering` says which of
/// the two effects applies. The second return value is `true` once the
/// transition has run its course, which is what tells the caller to schedule one
/// more frame and then stop.
///
/// `travel` is the surface's own size: a slide moves the surface in from (and
/// out towards) the edge `origin` names, so the distance is the size along that
/// axis. `origin` with no bits means "the centre", where a slide has nowhere to
/// come from and only what the other effect bits describe remains.
pub(crate) fn sample(
    transition: Transition,
    travel: Size<i32, Logical>,
    elapsed_ms: u32,
    entering: bool,
) -> (Sample, bool) {
    let effect = if entering {
        transition.enter
    } else {
        transition.exit
    };
    if transition.duration_ms == 0 || effect == 0 {
        return (Sample::FULL, true);
    }

    let total = transition.duration_ms;
    let elapsed = elapsed_ms.min(total);
    let progress = f64::from(elapsed) / f64::from(total);
    let shaped = curve(transition.curve, progress);
    // `visible` is how much of the surface is on screen: 0 when it has just
    // started entering (or finished exiting), 1 when it is fully there.
    let visible = if entering { shaped } else { 1.0 - shaped };

    let mut sample = Sample::FULL;
    if effect & EFFECT_FADE != 0 {
        sample.alpha = visible.clamp(0.0, 1.0) as f32;
    }
    if effect & EFFECT_SLIDE != 0 {
        let (dx, dy) = slide_direction(transition.origin, travel);
        let away = 1.0 - visible;
        sample.offset = Point::new(dx * away, dy * away);
    }
    if effect & EFFECT_SCALE != 0 {
        // Not from zero: a menu that grew from a point would look like it was
        // being born rather than opened.
        sample.scale = 0.85 + 0.15 * visible;
    }

    (sample, elapsed >= total)
}

/// The offset a slide starts from, in logical pixels.
fn slide_direction(origin: u32, travel: Size<i32, Logical>) -> (f64, f64) {
    let width = f64::from(travel.w.max(0));
    let height = f64::from(travel.h.max(0));
    let x = if origin & ANCHOR_LEFT != 0 {
        -width
    } else if origin & ANCHOR_RIGHT != 0 {
        width
    } else {
        0.0
    };
    let y = if origin & ANCHOR_TOP != 0 {
        -height
    } else if origin & ANCHOR_BOTTOM != 0 {
        height
    } else {
        0.0
    };

    (x, y)
}

/// Map a normalised time onto the protocol's named timing curves.
///
/// The names exist so the shell and the compositor share a feel without sharing
/// parameters; a value this build does not know falls back to linear, which is
/// the only curve whose shape a client can predict without being told.
pub(crate) fn curve(curve: u32, t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    match curve {
        // `ease_out`: fast start, settled end.
        1 => 1.0 - (1.0 - t).powi(3),
        // `spring`: a slight overshoot that settles at 1.
        2 => {
            let decay = (-5.0 * t).exp();
            let phase = 9.0 * t;
            1.0 - decay * (phase.cos() + (5.0 / 9.0) * phase.sin())
        }
        // `linear`, and anything this build was not told about.
        _ => t,
    }
}

/// A transition in flight.
#[derive(Debug, Clone, Copy)]
struct Animation {
    transition: Transition,
    travel: Size<i32, Logical>,
    entering: bool,
    started: Instant,
}

impl Animation {
    fn new(
        transition: Transition,
        travel: Size<i32, Logical>,
        entering: bool,
        now: Instant,
    ) -> Self {
        Self {
            transition,
            travel,
            entering,
            started: now,
        }
    }

    fn sample(&self, now: Instant) -> Sample {
        let elapsed = elapsed_ms(now.saturating_duration_since(self.started));

        sample(self.transition, self.travel, elapsed, self.entering).0
    }

    fn is_running(&self, now: Instant) -> bool {
        let total = Duration::from_millis(u64::from(self.transition.duration_ms));
        now.saturating_duration_since(self.started) < total
    }
}

/// Milliseconds since an instant, saturating at `u32::MAX`.
fn elapsed_ms(elapsed: Duration) -> u32 {
    u32::try_from(elapsed.as_millis()).unwrap_or(u32::MAX)
}

/// An animation started at `now`, or `None` when it would draw nothing.
fn start_animation(
    transition: Transition,
    travel: Size<i32, Logical>,
    entering: bool,
    now: Instant,
    enabled: bool,
) -> Option<Animation> {
    if !enabled || !transition.is_animated() {
        return None;
    }
    // A transition whose effect is not requested for this direction draws
    // nothing, and an animation that draws nothing must not keep the compositor
    // repainting.
    let effect = if entering {
        transition.enter
    } else {
        transition.exit
    };
    if effect == 0 {
        return None;
    }

    Some(Animation::new(transition, travel, entering, now))
}

/// One surface with a shell role.
pub(crate) struct ShellSurface {
    /// The output this surface is placed on.
    pub(crate) output: OutputId,
    surface: WlSurface,
    resource: InoxShellSurfaceV1,
    pending: Placement,
    pending_keyboard: Option<u32>,
    pending_transition: Option<Transition>,
    placement: Placement,
    keyboard_mode: u32,
    transition: Transition,
    /// Placement in the global logical coordinate space.
    pub(crate) geometry: Rectangle<i32, Logical>,
    /// Whether the surface has committed since it got its role.
    committed: bool,
    /// Whether the surface has a buffer and is being drawn.
    pub(crate) mapped: bool,
    /// The serial of the last configure sent, and whether a commit answered it.
    configure: Option<u32>,
    /// Whether a commit answered the last configure.
    acknowledged: bool,
    /// What was last configured, so an unchanged arrange sends nothing.
    configured_size: Option<Size<i32, Logical>>,
    animation: Option<Animation>,
    /// Whether the keyboard is currently focused here.
    keyboard: bool,
}

impl ShellSurface {
    /// Create the entry for a surface that has just been given a shell role.
    ///
    /// The role itself is validated in `state` and not stored: every shell role is
    /// hidden by a lock and frosted by the same rule, so no policy here branches
    /// on it. A revision that groups surfaces by role ("move the panel and its
    /// menus as one object") adds the field back together with the policy that
    /// reads it.
    pub(crate) fn new(output: OutputId, surface: &WlSurface, resource: InoxShellSurfaceV1) -> Self {
        Self {
            output,
            surface: surface.clone(),
            resource,
            pending: Placement::default(),
            pending_keyboard: None,
            pending_transition: None,
            placement: Placement::default(),
            keyboard_mode: 0,
            transition: Transition::default(),
            geometry: Rectangle::default(),
            committed: false,
            mapped: false,
            configure: None,
            acknowledged: false,
            configured_size: None,
            animation: None,
            keyboard: false,
        }
    }

    /// The `wl_surface` this role belongs to.
    pub(crate) fn wl_surface(&self) -> &WlSurface {
        &self.surface
    }

    /// The layer this surface is drawn in.
    pub(crate) fn layer(&self) -> ShellLayer {
        self.placement.layer
    }

    /// Whether the surface asked for the keyboard while mapped.
    fn wants_keyboard(&self) -> bool {
        self.mapped && self.keyboard_mode != 0
    }

    /// Whether the surface asked for the keyboard unconditionally.
    fn wants_keyboard_exclusively(&self) -> bool {
        self.mapped && self.keyboard_mode == 2
    }

    /// The sample to draw this surface with.
    pub(crate) fn sample(&self, now: Instant) -> Sample {
        match &self.animation {
            Some(animation) => animation.sample(now),
            None => Sample::FULL,
        }
    }

    /// Tell the client what size it has, if that changed since the last time.
    ///
    /// The configured size is authoritative, so this is the only place a shell
    /// surface learns how large it is.
    fn configure_if_changed(&mut self, size: Size<i32, Logical>) {
        if self.configured_size == Some(size) {
            return;
        }
        let serial = crate::state::next_serial();
        self.resource.configure(
            serial,
            u32::try_from(size.w.max(0)).unwrap_or(0),
            u32::try_from(size.h.max(0)).unwrap_or(0),
        );
        self.configured_size = Some(size);
        self.configure = Some(serial);
        self.acknowledged = false;
    }
}

/// Where a popup sits relative to its parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct PopupPosition {
    /// The parent's edge or corner the position is measured from.
    pub parent_anchor: u32,
    /// The popup's own edge or corner placed on it.
    pub popup_anchor: u32,
    /// Offset from that point, in the popup's logical coordinates.
    pub offset: Point<i32, Logical>,
}

/// The point `anchor` names inside `rect`.
///
/// A single bit names the midpoint of an edge, two perpendicular bits name a
/// corner, and an empty mask names the centre — the same reading
/// `xdg_positioner` uses, which is what makes a menu that flips expressible
/// without the client guessing where the compositor put the parent.
pub(crate) fn anchor_point(rect: Rectangle<i32, Logical>, anchor: u32) -> Point<i32, Logical> {
    let x = if anchor & ANCHOR_LEFT != 0 {
        rect.loc.x
    } else if anchor & ANCHOR_RIGHT != 0 {
        rect.loc.x + rect.size.w
    } else {
        rect.loc.x + rect.size.w / 2
    };
    let y = if anchor & ANCHOR_TOP != 0 {
        rect.loc.y
    } else if anchor & ANCHOR_BOTTOM != 0 {
        rect.loc.y + rect.size.h
    } else {
        rect.loc.y + rect.size.h / 2
    };

    Point::from((x, y))
}

/// Place a popup of `size` against its parent's `parent` rectangle.
pub(crate) fn place_popup(
    parent: Rectangle<i32, Logical>,
    position: PopupPosition,
    size: Size<i32, Logical>,
) -> Rectangle<i32, Logical> {
    let from = anchor_point(parent, position.parent_anchor);
    let own = anchor_point(Rectangle::from_size(size), position.popup_anchor);
    let loc = Point::from((
        from.x + position.offset.x - own.x,
        from.y + position.offset.y - own.y,
    ));

    Rectangle::new(loc, size)
}

/// Keep `rect` inside `area` by moving it, never by resizing it.
///
/// A popup that does not fit is clamped, not scaled: the configured size is what
/// the client laid itself out for, and a menu that is silently 40 px narrower
/// than its contents is a menu with clipped items.
pub(crate) fn clamp_into(
    rect: Rectangle<i32, Logical>,
    area: Rectangle<i32, Logical>,
) -> Rectangle<i32, Logical> {
    let x = if area.size.w >= rect.size.w {
        rect.loc
            .x
            .clamp(area.loc.x, area.loc.x + area.size.w - rect.size.w)
    } else {
        area.loc.x
    };
    let y = if area.size.h >= rect.size.h {
        rect.loc
            .y
            .clamp(area.loc.y, area.loc.y + area.size.h - rect.size.h)
    } else {
        area.loc.y
    };

    Rectangle::new(Point::from((x, y)), rect.size)
}

/// One popup: a menu, a tooltip, or a nested submenu.
pub(crate) struct ShellPopup {
    /// The output its root surface is on, copied from the parent at creation.
    output: OutputId,
    surface: WlSurface,
    resource: InoxShellPopupV1,
    /// The surface this popup hangs off: a shell surface's or another popup's.
    parent: WlSurface,
    position: PopupPosition,
    size: Size<i32, Logical>,
    dismissable: bool,
    keyboard_needed: bool,
    transition: Transition,
    /// Placement in the global logical coordinate space.
    pub(crate) geometry: Rectangle<i32, Logical>,
    /// Whether the popup has a buffer and is being drawn.
    pub(crate) mapped: bool,
    /// Whether the compositor has dismissed it. A dismissed popup leaves the
    /// input path and the dismissal stack at once; it stays in the scene only
    /// while its exit transition is drawing, which is the frame the user sees it
    /// disappear in.
    dismissed: bool,
    configure: Option<u32>,
    acknowledged: bool,
    configured_size: Option<Size<i32, Logical>>,
    animation: Option<Animation>,
    keyboard: bool,
}

impl ShellPopup {
    /// Create the entry for a surface that has just been given the popup role.
    pub(crate) fn new(
        output: OutputId,
        surface: &WlSurface,
        resource: InoxShellPopupV1,
        parent: &WlSurface,
    ) -> Self {
        Self {
            output,
            surface: surface.clone(),
            resource,
            parent: parent.clone(),
            position: PopupPosition::default(),
            size: Size::from((0, 0)),
            dismissable: false,
            keyboard_needed: false,
            transition: Transition::default(),
            geometry: Rectangle::default(),
            mapped: false,
            dismissed: false,
            configure: None,
            acknowledged: false,
            configured_size: None,
            animation: None,
            keyboard: false,
        }
    }

    /// The `wl_surface` this popup owns.
    pub(crate) fn wl_surface(&self) -> &WlSurface {
        &self.surface
    }

    /// Whether the popup takes part in input and dismissal.
    pub(crate) fn is_visible(&self) -> bool {
        self.mapped && !self.dismissed
    }

    /// Whether the popup is still part of the composed scene.
    ///
    /// A dismissed popup keeps drawing until its exit transition ends: the
    /// compositor promised the client it would run one, and dropping it on the
    /// dismissal frame would make the "exit effect" request a lie.
    fn is_drawn(&self, now: Instant) -> bool {
        self.mapped
            && (!self.dismissed || self.animation.as_ref().is_some_and(|a| a.is_running(now)))
    }

    fn configure_if_changed(&mut self, size: Size<i32, Logical>) {
        if self.configured_size == Some(size) {
            return;
        }
        let serial = crate::state::next_serial();
        self.resource.configure(
            serial,
            u32::try_from(size.w.max(0)).unwrap_or(0),
            u32::try_from(size.h.max(0)).unwrap_or(0),
        );
        self.configured_size = Some(size);
        self.configure = Some(serial);
        self.acknowledged = false;
    }

    /// The sample to draw this popup with.
    pub(crate) fn sample(&self, now: Instant) -> Sample {
        match &self.animation {
            Some(animation) => animation.sample(now),
            None => Sample::FULL,
        }
    }
}

/// Everything this compositor knows about its own shell surfaces.
#[derive(Default)]
pub(crate) struct ShellState {
    surfaces: Vec<ShellSurface>,
    popups: Vec<ShellPopup>,
    /// Whether the user asked for reduced motion. When set, every transition is
    /// instantaneous — the protocol's "a compositor may drop them entirely".
    reduce_motion: bool,
    /// The surface the pointer last clicked, used for `on_demand` focus.
    focused: Option<WlSurface>,
}

impl ShellState {
    /// Adopt the user's reduced-motion preference.
    pub(crate) fn set_reduce_motion(&mut self, reduce: bool) {
        self.reduce_motion = reduce;
        if reduce {
            // Drop whatever is in flight: a transition that keeps running after
            // the preference changed is a transition the user turned off.
            for surface in &mut self.surfaces {
                surface.animation = None;
            }
            for popup in &mut self.popups {
                popup.animation = None;
            }
        }
    }

    /// Whether `surface` has a shell role, as a surface or as a popup.
    ///
    /// This is half of the glass eligibility rule (ADR 0046 §6): a shell surface
    /// is a known desktop layer, which is what lets the compositor frost what is
    /// behind it.
    pub(crate) fn has_role(&self, surface: &WlSurface) -> bool {
        self.surfaces
            .iter()
            .any(|entry| entry.wl_surface() == surface)
            || self
                .popups
                .iter()
                .any(|entry| entry.wl_surface() == surface)
    }

    /// Whether there is anything to walk when rendering or hit-testing.
    pub(crate) fn is_empty(&self) -> bool {
        self.surfaces.is_empty() && self.popups.is_empty()
    }

    /// The output a shell surface or popup belongs to.
    pub(crate) fn output_of(&self, surface: &WlSurface) -> Option<OutputId> {
        self.surfaces
            .iter()
            .find(|entry| &entry.surface == surface)
            .map(|entry| entry.output)
            .or_else(|| {
                self.popups
                    .iter()
                    .find(|entry| &entry.surface == surface)
                    .map(|entry| entry.output)
            })
    }

    /// The mapped shell surfaces of one output and layer, newest first.
    ///
    /// Newest first is the stacking order within a layer: it is the compositor's
    /// own choice (the protocol says a client must not depend on it), and putting
    /// a reloaded panel above its predecessor is the one a user expects.
    pub(crate) fn surfaces_in(&self, output: OutputId, layer: ShellLayer) -> Vec<&ShellSurface> {
        self.surfaces
            .iter()
            .rev()
            .filter(|entry| entry.mapped && entry.output == output && entry.layer() == layer)
            .collect()
    }

    /// Every shell surface of `output`, mapped or not.
    ///
    /// Used when an output goes away: a surface that has not drawn yet still has
    /// to be told, and it still owns frosting parameters that must not outlive
    /// the placement they described.
    pub(crate) fn surfaces_of(&self, output: OutputId) -> Vec<&ShellSurface> {
        self.surfaces
            .iter()
            .filter(|entry| entry.output == output)
            .collect()
    }

    /// Every popup drawn for `surface`'s subtree, in front-to-back order.
    ///
    /// A submenu is in front of the menu that opened it, so the deepest popup
    /// comes first in a front-to-back list. The walk is breadth-first over the
    /// parent links, which keeps a three-level menu in creation-independent
    /// order.
    pub(crate) fn popup_stack(&self, surface: &WlSurface, now: Instant) -> Vec<&ShellPopup> {
        let mut stack = Vec::new();
        let mut frontier: Vec<WlSurface> = vec![surface.clone()];
        while !frontier.is_empty() {
            let mut next = Vec::new();
            for parent in frontier {
                for popup in self
                    .popups
                    .iter()
                    .filter(|popup| popup.parent == parent && popup.is_drawn(now))
                {
                    next.push(popup.surface.clone());
                    stack.push(popup);
                }
            }
            frontier = next;
        }
        stack.reverse();

        stack
    }

    /// Whether a dismissable popup is open, which is what Escape acts on.
    pub(crate) fn has_dismissable_popups(&self) -> bool {
        self.popups
            .iter()
            .any(|popup| popup.is_visible() && popup.dismissable)
    }

    /// The surface the keyboard should go to, if a shell surface wants it more
    /// than whatever is underneath.
    ///
    /// Order: a popup that asked for keys wins — that is the whole point of
    /// `inox_shell_popup_v1.set_keyboard` — then a shell surface that asked for
    /// exclusive focus, then the shell surface the user last clicked while it
    /// asked for `on_demand`.
    pub(crate) fn keyboard_target(&self) -> Option<WlSurface> {
        self.popups
            .iter()
            .rev()
            .find(|popup| popup.is_visible() && popup.keyboard_needed)
            .map(|popup| popup.surface.clone())
            .or_else(|| {
                self.surfaces
                    .iter()
                    .rev()
                    .find(|surface| surface.wants_keyboard_exclusively())
                    .map(|surface| surface.surface.clone())
            })
            .or_else(|| {
                self.focused
                    .as_ref()
                    .filter(|focused| {
                        self.surfaces
                            .iter()
                            .any(|surface| surface.wants_keyboard() && &surface.surface == *focused)
                            || self
                                .popups
                                .iter()
                                .any(|popup| popup.is_visible() && &popup.surface == *focused)
                    })
                    .cloned()
            })
    }

    /// Record that the pointer clicked `surface`, for `on_demand` focus.
    pub(crate) fn note_click(&mut self, surface: &WlSurface) {
        let wants_focus = self
            .surfaces
            .iter()
            .any(|entry| &entry.surface == surface && entry.wants_keyboard())
            || self
                .popups
                .iter()
                .any(|popup| popup.is_visible() && &popup.surface == surface);
        if wants_focus {
            self.focused = Some(surface.clone());
        }
    }

    /// Tell every surface whose keyboard state changed what it is now.
    ///
    /// Called by [`crate::state`] from the one place the keyboard focus changes,
    /// so a shell surface never has to guess whether asking for focus was
    /// granted.
    pub(crate) fn announce_keyboard(&mut self, focused: Option<&WlSurface>) {
        for surface in &mut self.surfaces {
            let has = focused == Some(&surface.surface);
            if surface.keyboard != has {
                surface.keyboard = has;
                surface.resource.keyboard_state(u32::from(has));
            }
        }
        for popup in &mut self.popups {
            let has = focused == Some(&popup.surface);
            if popup.keyboard != has {
                popup.keyboard = has;
                popup.resource.keyboard_state(u32::from(has));
            }
        }
    }

    /// Whether any transition is still running, which keeps the compositor
    /// repainting.
    ///
    /// A compositor that stopped scheduling frames the moment a transition began
    /// would draw one frame of it and freeze: this is what `render_frame` asks
    /// after every frame to keep the loop going until the last sample.
    pub(crate) fn is_animating(&self, now: Instant) -> bool {
        self.surfaces.iter().any(|surface| {
            surface
                .animation
                .as_ref()
                .is_some_and(|a| a.is_running(now))
        }) || self
            .popups
            .iter()
            .any(|popup| popup.animation.as_ref().is_some_and(|a| a.is_running(now)))
    }

    /// Finish every transition that has run its course.
    ///
    /// A dismissed popup whose exit has finished stops being drawn here, which is
    /// also the moment the compositor stops considering it part of the scene at
    /// all.
    pub(crate) fn tick(&mut self, now: Instant) {
        for surface in &mut self.surfaces {
            if surface
                .animation
                .as_ref()
                .is_some_and(|a| !a.is_running(now))
            {
                surface.animation = None;
            }
        }
        for popup in &mut self.popups {
            if popup.animation.as_ref().is_some_and(|a| !a.is_running(now)) {
                popup.animation = None;
            }
            if popup.dismissed && popup.animation.is_none() {
                popup.mapped = false;
            }
        }
    }

    /// Add a surface that has just been given a role.
    pub(crate) fn insert_surface(&mut self, surface: ShellSurface) {
        self.surfaces.push(surface);
    }

    /// Add a popup that has just been given the popup role.
    pub(crate) fn insert_popup(&mut self, popup: ShellPopup) {
        self.popups.push(popup);
    }

    /// Stage the placement a client asked for; it applies on commit.
    pub(crate) fn stage_placement(&mut self, surface: &WlSurface, placement: Placement) {
        if let Some(entry) = self
            .surfaces
            .iter_mut()
            .find(|entry| &entry.surface == surface)
        {
            entry.pending = placement;
        }
    }

    /// Stage the keyboard mode a client asked for; it applies on commit.
    pub(crate) fn stage_keyboard(&mut self, surface: &WlSurface, mode: u32) {
        if let Some(entry) = self
            .surfaces
            .iter_mut()
            .find(|entry| &entry.surface == surface)
        {
            entry.pending_keyboard = Some(mode);
        }
    }

    /// Stage the transition a client asked for; it applies on commit.
    pub(crate) fn stage_transition(&mut self, surface: &WlSurface, transition: Transition) {
        if let Some(entry) = self
            .surfaces
            .iter_mut()
            .find(|entry| &entry.surface == surface)
        {
            entry.pending_transition = Some(clamp_transition(transition));
        }
    }

    /// Stage a popup's position; it applies on commit.
    pub(crate) fn stage_popup_position(&mut self, surface: &WlSurface, position: PopupPosition) {
        if let Some(entry) = self
            .popups
            .iter_mut()
            .find(|entry| &entry.surface == surface)
        {
            entry.position = position;
        }
    }

    /// Stage a popup's size; it applies on commit.
    pub(crate) fn stage_popup_size(&mut self, surface: &WlSurface, size: Size<i32, Logical>) {
        if let Some(entry) = self
            .popups
            .iter_mut()
            .find(|entry| &entry.surface == surface)
        {
            entry.size = size;
        }
    }

    /// Record a popup's keyboard request. Not staged: the request is about
    /// whether the popup *needs* keys while it is open, and a compositor that
    /// waited for a commit would leave the menu unresponsive for a frame.
    pub(crate) fn set_popup_keyboard(&mut self, surface: &WlSurface, needed: bool) {
        if let Some(entry) = self
            .popups
            .iter_mut()
            .find(|entry| &entry.surface == surface)
        {
            entry.keyboard_needed = needed;
        }
    }

    /// Record whether a popup may be dismissed by the compositor.
    pub(crate) fn set_popup_dismissable(&mut self, surface: &WlSurface, dismissable: bool) {
        if let Some(entry) = self
            .popups
            .iter_mut()
            .find(|entry| &entry.surface == surface)
        {
            entry.dismissable = dismissable;
        }
    }

    /// Stage a popup's transition.
    pub(crate) fn stage_popup_transition(&mut self, surface: &WlSurface, transition: Transition) {
        if let Some(entry) = self
            .popups
            .iter_mut()
            .find(|entry| &entry.surface == surface)
        {
            entry.transition = clamp_transition(transition);
        }
    }

    /// Apply a surface's staged state. Returns whether anything changed.
    ///
    /// `has_buffer` is what the surface committed with: a commit that carries no
    /// buffer unmaps the surface, and remapping goes through a fresh configure
    /// exactly as a layer surface does.
    pub(crate) fn commit(&mut self, surface: &WlSurface, has_buffer: bool, now: Instant) -> bool {
        let enabled = !self.reduce_motion;
        let Some(entry) = self
            .surfaces
            .iter_mut()
            .find(|entry| &entry.surface == surface)
        else {
            if let Some(popup) = self
                .popups
                .iter_mut()
                .find(|entry| &entry.surface == surface)
            {
                return commit_popup(popup, has_buffer, now, enabled);
            }

            return false;
        };

        let was_placement = entry.placement;
        let was_mapped = entry.mapped;
        let was_keyboard = entry.keyboard_mode;
        entry.placement = entry.pending;
        if let Some(mode) = entry.pending_keyboard.take() {
            entry.keyboard_mode = mode;
        }
        if let Some(transition) = entry.pending_transition.take() {
            entry.transition = transition;
        }
        entry.committed = true;
        // A commit after a configure *is* the acknowledgement in this protocol:
        // there is no ack request, and the surface cannot legally draw before it
        // has been told its size.
        let was_acknowledged = entry.acknowledged;
        entry.acknowledged = entry.configure.is_some();
        entry.mapped = has_buffer && entry.acknowledged;

        if !was_mapped && entry.mapped {
            entry.animation =
                start_animation(entry.transition, entry.geometry.size, true, now, enabled);
        } else if was_mapped && !entry.mapped {
            // No buffer, nothing to animate: an exit effect needs pixels to
            // shrink and fade, and a detached surface has none.
            entry.animation = None;
        }

        was_placement != entry.placement
            || was_mapped != entry.mapped
            || was_keyboard != entry.keyboard_mode
            || was_acknowledged != entry.acknowledged
    }

    /// Place one output's shell surfaces and reserve their exclusive zones.
    ///
    /// Called once per reservation pass by [`crate::state`], which is what keeps
    /// shell surfaces and layer surfaces in one consistent order: every surface
    /// that reserves comes before every surface that does not, and within a pass
    /// the layers go back to front.
    pub(crate) fn arrange_pass(
        &mut self,
        output: OutputId,
        layer: ShellLayer,
        exclusive: bool,
        area: &mut Rectangle<i32, Logical>,
        output_area: Rectangle<i32, Logical>,
    ) {
        for index in (0..self.surfaces.len()).rev() {
            let entry = &self.surfaces[index];
            if entry.output != output || !entry.committed || entry.placement.layer != layer {
                continue;
            }
            let placement = entry.placement;
            if placement.reserves() != exclusive {
                continue;
            }
            let source = if placement.exclusive_zone < 0 {
                // A surface that asked not to be moved by other clients' zones
                // is placed on the whole output, so it also derives its size from
                // the whole output. This is layer-shell's `DontCare` rule and it
                // is shared with the layer path by construction.
                output_area
            } else {
                *area
            };
            let geometry = layers::geometry(source, placement.cached());

            let entry = &mut self.surfaces[index];
            entry.geometry = geometry;
            entry.configure_if_changed(geometry.size);
            if entry.mapped {
                layers::reserve(area, placement.cached());
            }
        }
    }

    /// Place every popup against its parent, clamped into the work area.
    ///
    /// Parents are placed before children because a popup's geometry is derived
    /// from its parent's; walking in creation order is enough, because a popup can
    /// only be created after the object it hangs off exists.
    pub(crate) fn arrange_popups(&mut self, work_areas: &[(OutputId, Rectangle<i32, Logical>)]) {
        for index in 0..self.popups.len() {
            let (parent, position, size, output) = {
                let popup = &self.popups[index];
                (
                    popup.parent.clone(),
                    popup.position,
                    popup.size,
                    popup.output,
                )
            };
            let parent_geometry = self
                .surfaces
                .iter()
                .find(|entry| entry.surface == parent)
                .map(|entry| entry.geometry)
                .or_else(|| {
                    self.popups
                        .iter()
                        .find(|entry| entry.surface == parent)
                        .map(|entry| entry.geometry)
                });
            let Some(parent_geometry) = parent_geometry else {
                continue;
            };
            let area = work_areas
                .iter()
                .find(|(id, _)| *id == output)
                .map(|(_, area)| *area)
                .unwrap_or_else(|| Rectangle::from_size(parent_geometry.size));
            let placed = clamp_into(place_popup(parent_geometry, position, size), area);
            let popup = &mut self.popups[index];
            popup.geometry = placed;
            popup.configure_if_changed(placed.size);
        }
    }

    /// Tell every surface of `output` what the work area now is.
    ///
    /// This is the event `zwlr_layer_shell_v1` does not have; a shell that had to
    /// re-derive it from configuration would drift the moment another surface
    /// reserved a strip.
    pub(crate) fn announce_work_area(&self, areas: &[(OutputId, Rectangle<i32, Logical>)]) {
        for entry in &self.surfaces {
            if let Some((_, area)) = areas.iter().find(|(id, _)| *id == entry.output) {
                entry
                    .resource
                    .workarea(area.loc.x, area.loc.y, area.size.w, area.size.h);
            }
        }
    }

    /// Dismiss popups whose parent is no longer on screen.
    ///
    /// Run after every commit and every arrange: a popup cannot outlive the
    /// surface it hangs off, and the compositor is the only one that knows when
    /// that surface unmapped.
    pub(crate) fn dismiss_orphans(&mut self, now: Instant) -> bool {
        let mut dismissed_any = false;
        let surfaces: Vec<(WlSurface, bool)> = self
            .surfaces
            .iter()
            .map(|entry| (entry.surface.clone(), entry.mapped))
            .collect();
        let popups: Vec<(WlSurface, bool)> = self
            .popups
            .iter()
            .map(|entry| (entry.surface.clone(), entry.is_visible()))
            .collect();

        for popup in &mut self.popups {
            if !popup.is_visible() {
                continue;
            }
            let parent_shown = surfaces
                .iter()
                .find(|(surface, _)| *surface == popup.parent)
                .map(|(_, mapped)| *mapped)
                .or_else(|| {
                    popups
                        .iter()
                        .find(|(surface, _)| *surface == popup.parent)
                        .map(|(_, visible)| *visible)
                })
                .unwrap_or(false);
            if !parent_shown {
                dismiss_one(popup, DismissReason::ParentHidden, now);
                dismissed_any = true;
            }
        }

        dismissed_any
    }

    /// Dismiss every dismissable popup because the pointer was pressed outside.
    ///
    /// Returns whether something was dismissed, which is what makes the press
    /// get swallowed: a click that closes a menu must not also reach the window
    /// underneath it.
    pub(crate) fn dismiss_outside(&mut self, point: Point<f64, Logical>, now: Instant) -> bool {
        let inside = self
            .popups
            .iter()
            .any(|popup| popup.is_visible() && popup.geometry.to_f64().contains(point));
        if inside {
            return false;
        }
        self.dismiss_stack(DismissReason::Outside, now, true)
    }

    /// Dismiss every dismissable popup because the user pressed Escape.
    pub(crate) fn dismiss_escape(&mut self, now: Instant) -> bool {
        self.dismiss_stack(DismissReason::Escape, now, true)
    }

    /// Dismiss one popup and everything nested in it, on the client's request.
    pub(crate) fn dismiss_requested(&mut self, surface: &WlSurface, now: Instant) {
        let Some(index) = self
            .popups
            .iter()
            .position(|popup| &popup.surface == surface)
        else {
            return;
        };
        let root = self.popups[index].surface.clone();
        let nested: Vec<usize> = self
            .popups
            .iter()
            .enumerate()
            .filter(|(_, popup)| popup.is_visible() && self.is_descendant(&popup.surface, &root))
            .map(|(index, _)| index)
            .collect();
        for index in nested {
            dismiss_one(&mut self.popups[index], DismissReason::Policy, now);
        }
    }

    /// Whether `candidate` is `root` or hangs off it.
    fn is_descendant(&self, candidate: &WlSurface, root: &WlSurface) -> bool {
        let mut current = Some(candidate.clone());
        while let Some(surface) = current {
            if surface == *root {
                return true;
            }
            current = self
                .popups
                .iter()
                .find(|popup| popup.surface == surface)
                .map(|popup| popup.parent.clone());
        }

        false
    }

    /// Dismiss every popup topmost-first, optionally only the dismissable ones.
    fn dismiss_stack(
        &mut self,
        reason: DismissReason,
        now: Instant,
        dismissable_only: bool,
    ) -> bool {
        let targets: Vec<usize> = (0..self.popups.len())
            .rev()
            .filter(|index| {
                let popup = &self.popups[*index];
                popup.is_visible() && (!dismissable_only || popup.dismissable)
            })
            .collect();
        if targets.is_empty() {
            return false;
        }
        for index in targets {
            dismiss_one(&mut self.popups[index], reason, now);
        }

        true
    }

    /// The visible popup under `point`, topmost first.
    pub(crate) fn popup_under(&self, point: Point<f64, Logical>) -> Option<&ShellPopup> {
        self.popups
            .iter()
            .rev()
            .find(|popup| popup.is_visible() && popup.geometry.to_f64().contains(point))
    }

    /// The mapped shell surface of `layer` under `point`, newest first.
    pub(crate) fn surface_under(
        &self,
        layer: ShellLayer,
        point: Point<f64, Logical>,
    ) -> Option<&ShellSurface> {
        self.surfaces.iter().rev().find(|entry| {
            entry.mapped && entry.layer() == layer && entry.geometry.to_f64().contains(point)
        })
    }

    /// Drop a surface's role because the client destroyed the object.
    ///
    /// No event is sent: the client was the one that ended it. Popups hanging off
    /// the surface still have to be told, because they are separate objects and
    /// their client has no other way to learn that the anchor they belong to is
    /// gone.
    pub(crate) fn remove_surface(&mut self, surface: &WlSurface, now: Instant) {
        self.dismiss_children(surface, now);
        if let Some(index) = self
            .surfaces
            .iter()
            .position(|entry| &entry.surface == surface)
        {
            drop(self.surfaces.remove(index));
        }
        if self.focused.as_ref() == Some(surface) {
            self.focused = None;
        }
    }

    /// Remove a popup because the client destroyed the object.
    pub(crate) fn remove_popup(&mut self, surface: &WlSurface, now: Instant) {
        self.dismiss_children(surface, now);
        if let Some(index) = self
            .popups
            .iter()
            .position(|entry| &entry.surface == surface)
        {
            drop(self.popups.remove(index));
        }
        if self.focused.as_ref() == Some(surface) {
            self.focused = None;
        }
    }

    /// Tell every popup hanging off `surface` that it has to go.
    ///
    /// The children are removed from the scene at once rather than animated: the
    /// surface they were anchored to no longer exists, so there is no rectangle
    /// left to run an exit towards.
    fn dismiss_children(&mut self, surface: &WlSurface, now: Instant) {
        let children: Vec<usize> = self
            .popups
            .iter()
            .enumerate()
            .filter(|(_, popup)| popup.is_visible() && popup.parent == *surface)
            .map(|(index, _)| index)
            .collect();
        for index in children {
            let popup = &mut self.popups[index];
            popup.dismissed = true;
            popup.mapped = false;
            popup.animation = None;
            popup.keyboard_needed = false;
            popup.resource.dismissed(DismissReason::ParentHidden);
            let _ = now;
        }
    }

    /// Tell every surface of `output` that the compositor will not show it.
    ///
    /// This is what an output removal looks like from the shell's side: the
    /// surface cannot be drawn anywhere, and `closed` is how the protocol says so
    /// before the client has lost anything.
    pub(crate) fn close_output(&mut self, output: OutputId, now: Instant) {
        let leaving: Vec<WlSurface> = self
            .surfaces
            .iter()
            .filter(|entry| entry.output == output)
            .map(|entry| entry.surface.clone())
            .collect();
        for surface in leaving {
            self.dismiss_children(&surface, now);
            if let Some(index) = self
                .surfaces
                .iter()
                .position(|entry| entry.surface == surface)
            {
                let entry = self.surfaces.remove(index);
                entry.resource.closed();
            }
        }
        if self
            .focused
            .as_ref()
            .is_some_and(|focused| !self.surfaces.iter().any(|entry| &entry.surface == focused))
        {
            self.focused = None;
        }
    }
}

/// Apply a popup's staged state on commit.
fn commit_popup(popup: &mut ShellPopup, has_buffer: bool, now: Instant, enabled: bool) -> bool {
    let was_mapped = popup.mapped;
    popup.acknowledged = popup.configure.is_some();
    popup.mapped = has_buffer && popup.acknowledged && !popup.dismissed;
    if !was_mapped && popup.mapped {
        popup.animation =
            start_animation(popup.transition, popup.geometry.size, true, now, enabled);
    } else if was_mapped && !popup.mapped {
        popup.animation = None;
    }

    was_mapped != popup.mapped
}

/// Dismiss one popup: tell the client why, then run the exit transition.
fn dismiss_one(popup: &mut ShellPopup, reason: DismissReason, now: Instant) {
    if popup.dismissed {
        return;
    }
    popup.dismissed = true;
    popup.resource.dismissed(reason);
    popup.animation = start_animation(popup.transition, popup.geometry.size, false, now, true);
    if popup.animation.is_none() {
        popup.mapped = false;
    }
    // Nothing may keep the keyboard after being dismissed; `announce_keyboard`
    // in the focus pass tells the object it lost it.
    popup.keyboard_needed = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transition(effect: u32, duration_ms: u32, curve: u32, origin: u32) -> Transition {
        Transition {
            enter: effect,
            exit: effect,
            duration_ms,
            curve,
            origin,
        }
    }

    #[test]
    fn a_zero_duration_transition_is_instant() {
        let (sample, finished) =
            sample(transition(EFFECT_FADE, 0, 0, 0), (100, 40).into(), 0, true);

        assert_eq!(sample, Sample::FULL);
        assert!(finished);
    }

    #[test]
    fn a_fade_interpolates_strictly_between_its_ends() {
        let transition = transition(EFFECT_FADE, 200, 1, 0);
        let (start, start_done) = sample(transition, (100, 40).into(), 0, true);
        let (middle, middle_done) = sample(transition, (100, 40).into(), 100, true);
        let (end, end_done) = sample(transition, (100, 40).into(), 200, true);

        assert!(!start_done && !middle_done);
        assert!(end_done);
        assert_eq!(start.alpha, 0.0);
        assert_eq!(
            middle.alpha, 0.875,
            "ease-out is past half way at half time"
        );
        assert_eq!(end.alpha, 1.0);
        assert!(start.alpha < middle.alpha && middle.alpha < end.alpha);
    }

    #[test]
    fn an_exit_runs_the_same_way_round_the_other_way() {
        let transition = transition(EFFECT_FADE, 200, 0, 0);
        let (start, _) = sample(transition, (10, 10).into(), 0, false);
        let (middle, finished) = sample(transition, (10, 10).into(), 100, false);
        let (end, end_finished) = sample(transition, (10, 10).into(), 200, false);

        assert_eq!(start.alpha, 1.0);
        assert_eq!(middle.alpha, 0.5);
        assert!(!finished);
        assert_eq!(end.alpha, 0.0);
        assert!(end_finished);
    }

    #[test]
    fn a_slide_moves_in_from_the_origin_edge() {
        let from_top = transition(EFFECT_SLIDE, 100, 0, ANCHOR_TOP);
        let (start, _) = sample(from_top, (80, 30).into(), 0, true);
        let (end, _) = sample(from_top, (80, 30).into(), 100, true);
        assert_eq!(start.offset, Point::new(0.0, -30.0));
        assert_eq!(end.offset, Point::new(0.0, 0.0));

        let from_left = transition(EFFECT_SLIDE, 100, 0, ANCHOR_LEFT);
        let (start, _) = sample(from_left, (80, 30).into(), 0, true);
        assert_eq!(start.offset, Point::new(-80.0, 0.0));
    }

    #[test]
    fn a_scale_grows_from_almost_its_own_size() {
        let transition = transition(EFFECT_SCALE, 100, 0, 0);
        let (start, _) = sample(transition, (10, 10).into(), 0, true);
        let (end, _) = sample(transition, (10, 10).into(), 100, true);

        assert!((start.scale - 0.85).abs() < 1e-9);
        assert!((end.scale - 1.0).abs() < 1e-9);
    }

    /// A compositor that never animated anything is what `duration_ms == 0`
    /// means, and the shell has to look right without waiting for a frame it will
    /// never be sent.
    #[test]
    fn every_effect_is_instant_when_the_duration_is_zero() {
        for effect in [EFFECT_FADE, EFFECT_SLIDE, EFFECT_SCALE, 7] {
            let (sample, finished) = sample(
                transition(effect, 0, 2, ANCHOR_TOP),
                (10, 10).into(),
                0,
                true,
            );
            assert_eq!(sample, Sample::FULL);
            assert!(finished);
        }
    }

    #[test]
    fn the_spring_curve_overshoots_and_settles_at_one() {
        let peak = (1..=99)
            .map(|step| curve(2, f64::from(step) / 100.0))
            .fold(f64::MIN, f64::max);

        assert!(peak > 1.0, "a spring has to overshoot: {peak}");
        assert!((curve(2, 1.0) - 1.0).abs() < 0.01);
        assert_eq!(curve(0, 0.5), 0.5, "the default curve is linear");
        assert_eq!(curve(99, 0.5), 0.5, "an unknown curve falls back to linear");
    }

    #[test]
    fn a_duration_beyond_the_cap_is_clamped() {
        let clamped = clamp_transition(Transition {
            enter: EFFECT_FADE,
            exit: EFFECT_FADE,
            duration_ms: MAX_TRANSITION_MS * 4,
            curve: 0,
            origin: 0,
        });

        assert_eq!(clamped.duration_ms, MAX_TRANSITION_MS);
    }

    #[test]
    fn an_unknown_effect_bit_is_dropped_rather_than_guessed() {
        let clamped = clamp_transition(Transition {
            enter: 0xffff,
            exit: 0,
            duration_ms: 100,
            curve: 0,
            origin: 0,
        });

        assert_eq!(clamped.enter, EFFECT_FADE | EFFECT_SLIDE | EFFECT_SCALE);
    }

    #[test]
    fn a_placement_becomes_the_same_state_a_layer_surface_uses() {
        let placement = Placement {
            layer: ShellLayer::Top,
            anchor: ANCHOR_TOP | ANCHOR_LEFT | ANCHOR_RIGHT,
            exclusive_zone: 32,
            margins: [0, 4, 0, 4],
            size: (0, 32).into(),
        };
        let area = Rectangle::from_size((1920, 1080).into());
        let geometry = layers::geometry(area, placement.cached());

        assert_eq!(
            geometry,
            Rectangle::new((4, 0).into(), (1912, 32).into()),
            "the bar spans the output minus its margins, at the top"
        );

        let mut work = area;
        layers::reserve(&mut work, placement.cached());
        assert_eq!(work, Rectangle::new((0, 32).into(), (1920, 1048).into()));
    }

    #[test]
    fn an_anchor_point_reads_edges_corners_and_the_centre() {
        let rect = Rectangle::new((10, 20).into(), (100, 40).into());

        assert_eq!(anchor_point(rect, ANCHOR_TOP), Point::from((60, 20)));
        assert_eq!(anchor_point(rect, ANCHOR_LEFT), Point::from((10, 40)));
        assert_eq!(
            anchor_point(rect, ANCHOR_TOP | ANCHOR_LEFT),
            Point::from((10, 20))
        );
        assert_eq!(anchor_point(rect, 0), Point::from((60, 40)));
    }

    #[test]
    fn a_popup_hangs_off_the_corner_its_anchors_name() {
        let parent = Rectangle::new((100, 100).into(), (40, 32).into());
        let below = PopupPosition {
            parent_anchor: ANCHOR_BOTTOM | ANCHOR_LEFT,
            popup_anchor: ANCHOR_TOP | ANCHOR_LEFT,
            offset: Point::from((0, 4)),
        };

        assert_eq!(
            place_popup(parent, below, (200, 120).into()),
            Rectangle::new((100, 136).into(), (200, 120).into())
        );

        let to_the_right = PopupPosition {
            parent_anchor: ANCHOR_RIGHT,
            popup_anchor: ANCHOR_LEFT,
            offset: Point::from((2, 0)),
        };
        assert_eq!(
            place_popup(parent, to_the_right, (120, 80).into()),
            Rectangle::new((142, 76).into(), (120, 80).into())
        );
    }

    #[test]
    fn a_popup_that_does_not_fit_is_moved_not_shrunk() {
        let area = Rectangle::from_size((800, 600).into());
        let rect = Rectangle::new((700, 560).into(), (200, 120).into());
        let clamped = clamp_into(rect, area);

        assert_eq!(
            clamped.size, rect.size,
            "the configured size is authoritative"
        );
        assert_eq!(clamped.loc, Point::from((600, 480)));
    }

    #[test]
    fn a_popup_larger_than_the_work_area_is_pinned_to_its_origin() {
        let area = Rectangle::from_size((800, 600).into());
        let rect = Rectangle::new((100, 100).into(), (900, 700).into());

        assert_eq!(clamp_into(rect, area).loc, Point::from((0, 0)));
    }
}
