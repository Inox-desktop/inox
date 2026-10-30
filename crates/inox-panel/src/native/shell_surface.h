#pragma once

#include <QPoint>
#include <QRect>
#include <QSize>
#include <QString>
#include <QtGlobal>
#include <functional>

// The panel's native Wayland shell role, as the rest of the process sees it.
//
// This header is deliberately plain: Qt public types, integers and free
// functions. `QtWaylandClient`'s private ABI — the only way to give a `QWindow`
// something other than an `xdg_toplevel` role — is confined to
// `qt_shell_surface.cpp`, one translation unit, and none of it appears here or
// on the Rust boundary (ADR 0046 §10). That is what makes the private-ABI debt
// auditable: `rg 'QtWaylandClient/private' crates/inox-panel/src` names exactly
// one file.
//
// Everything below describes `inox-shell-v1`
// (`crates/inox-protocols/protocol/inox-shell-v1.xml`). The numbers are the
// protocol's own enum values: the caller passes the values it decided on
// (`placement.rs`, `app.rs`) and this layer does not reinterpret them.
//
// # Why free functions and not a class
//
// The role belongs to a `QWindow`, and a `QWindow` outlives the decisions taken
// about it. Keying the state by the window (`QHash<QWindow *, …>` inside the
// implementation) keeps that lifetime in one place instead of asking the
// caller to hold an object that must be destroyed before the window is — the
// mistake that leaves a `wl_surface` with a role whose owner is gone.

class QWindow;

namespace inox {
namespace native {

/// `inox_shell_v1.surface_role`: what a shell surface *is*, semantically.
enum Role : quint32 {
    RolePanel = 0,
    RoleDock = 1,
    RoleOverlay = 2,
    RoleMenu = 3,
};

/// `inox_shell_surface_v1.layer`: where it is drawn.
enum Layer : quint32 {
    LayerBackground = 0,
    LayerBottom = 1,
    LayerTop = 2,
    LayerOverlay = 3,
};

/// `inox_shell_surface_v1.anchor`, as a bitfield.
enum Anchor : quint32 {
    AnchorTop = 1,
    AnchorBottom = 2,
    AnchorLeft = 4,
    AnchorRight = 8,
};

/// `inox_shell_surface_v1.effect`, as a bitfield.
enum Effect : quint32 {
    EffectNone = 0,
    EffectFade = 1,
    EffectSlide = 2,
    EffectScale = 4,
};

/// `inox_shell_surface_v1.curve`.
enum Curve : quint32 {
    CurveLinear = 0,
    CurveEaseOut = 1,
    CurveSpring = 2,
};

/// `inox_shell_surface_v1.keyboard_mode`.
enum KeyboardMode : quint32 {
    KeyboardNone = 0,
    KeyboardOnDemand = 1,
    KeyboardExclusive = 2,
};

/// One `set_placement` request, which the protocol defines as one atomic unit.
struct Placement {
    quint32 layer = LayerTop;
    quint32 anchor = 0;
    qint32 exclusive_zone = 0;
    qint32 margin_top = 0;
    qint32 margin_right = 0;
    qint32 margin_bottom = 0;
    qint32 margin_left = 0;
    /// Requested size in logical pixels; 0 lets the compositor decide.
    quint32 width = 0;
    quint32 height = 0;
};

/// One `set_keyboard` request.
struct Keyboard {
    quint32 mode = KeyboardNone;
};

/// One `set_transition` request.
struct Transition {
    quint32 enter = EffectNone;
    quint32 exit = EffectNone;
    quint32 duration = 0;
    quint32 curve = CurveEaseOut;
    /// The anchor the slide or scale grows from; an empty mask means the centre.
    quint32 origin = 0;
};

/// Which edge or corner of a rectangle `anchor` names.
///
/// The protocol reads a single bit as the midpoint of that edge, two
/// perpendicular bits as a corner, and an empty mask as the centre. This is the
/// same reading `xdg_positioner` uses, and it is the reason a menu can be
/// positioned by *relation* to its parent instead of by coordinates the client
/// would have to guess.
QPoint anchor_point(const QSize &size, quint32 anchor);

/// Everything one popup needs at the moment it is created.
///
/// The protocol lets a popup re-position itself, but a shell menu is laid out
/// once and then clamped by the compositor, so the whole position arrives with
/// the role: an open popup is never half-positioned while it is already on
/// screen.
struct PopupRequest {
    quint32 parent_anchor = AnchorBottom;
    quint32 popup_anchor = AnchorTop;
    qint32 x = 0;
    qint32 y = 0;
    quint32 width = 0;
    quint32 height = 0;
    bool keyboard = false;
    bool dismissable = true;
    Transition transition;
};

/// One `inox-glass-v1` parameter set, as [`crate::frost`] decided it.
///
/// Integers only, and one field per protocol argument: the protocol's own
/// numbers are what the caller decided (`inox_glass::Request`), and this layer
/// reinterprets none of them. `refraction` is what makes the two version-2
/// requests conditional — a version 1 object has no such vocabulary and sending
/// them would be a protocol error, not a request the compositor ignores.
struct GlassParams {
    quint32 radius = 0;
    /// The region to frost, surface-local; a non-positive extent means "nothing".
    qint32 x = 0;
    qint32 y = 0;
    qint32 width = 0;
    qint32 height = 0;
    quint32 top_left = 0;
    quint32 top_right = 0;
    quint32 bottom_right = 0;
    quint32 bottom_left = 0;
    /// Whether `set_corner_radii` and `set_refraction` are sent at all.
    bool refraction = false;
    quint32 band_height = 0;
    quint32 band_amount = 0;
    quint32 flags = 0;
};

/// What a shell surface tells the rest of the process.
///
/// The kinds are the integers the Rust side reads:
///
/// | kind | meaning | a | b | c | d |
/// |---|---|---|---|---|---|
/// | 1 | capabilities | bitfield | max transition ms | | |
/// | 2 | configure | width | height | | |
/// | 3 | workarea | x | y | width | height |
/// | 4 | keyboard state | 1 focused, 0 not | | | |
/// | 5 | closed by the compositor | | | | |
/// | 6 | popup dismissed | popup id | reason | | |
/// | 7 | popup configured | popup id | width | height | |
/// | 8 | popup keyboard state | popup id | 1 focused, 0 not | | |
/// | 9 | glass capabilities | max radius | | | |
/// | 10 | glass liquid capabilities | max radius | max band | max amount | flags |
///
/// One integer queue rather than callbacks: Qt dispatches on the same thread the
/// Rust loop pumps from, so draining a queue is both simpler than a callback
/// that must not re-enter the compositor, and testable without a socket.
using EventSink = std::function<void(quint32 kind, qint32 a, qint32 b, qint32 c, qint32 d)>;

/// The reasons a popup can be dismissed, matching `dismiss_reason`.
enum DismissReason : qint32 {
    DismissedOutside = 0,
    DismissedEscape = 1,
    DismissedParentHidden = 2,
    DismissedPolicy = 3,
};

/// Installs `sink` as the destination of every event above.
///
/// One sink per process, because one panel process owns at most one shell
/// surface tree. Installing a second replaces the first; nothing here counts
/// references, and the Rust side (`crate::native`) installs it once before the
/// first window exists.
void set_event_sink(EventSink sink);

/// Whether this process can create a shell surface at all, and why not.
///
/// Checked before any window is shown: a panel that cannot be placed must fail
/// loudly rather than appear as a floating window (ADR 0046 §10, "failure is an
/// exit"). `error` is empty when the answer is true.
bool shell_available(QString *error);

/// Binds `inox_shell_v1` and gives `window` the `role` shell role.
///
/// Must be called **before** `window` is shown: a shown window already has an
/// `xdg_toplevel`, and a `wl_surface` has exactly one role. The window's
/// platform window is created here (that is what makes the role installable
/// before mapping), and its size is decided by the compositor from then on.
///
/// Returns false with a readable `error` when the platform is not Wayland, when
/// the compositor offers no `inox_shell_v1`, or when Qt's private integration
/// refuses the window. There is no fallback to `xdg_toplevel` and none to
/// layer-shell: both would place the panel somewhere the session did not ask
/// for.
bool install_role(QWindow *window, quint32 role, QString *error);

/// Sends `set_placement` for `window`'s role.
///
/// The role decides nothing about placement; this is the one call that does.
/// Returns false when `window` has no shell role.
bool apply_placement(QWindow *window, const Placement &placement);

/// Sends `set_keyboard` for `window`'s role.
bool apply_keyboard(QWindow *window, const Keyboard &keyboard);

/// Sends `set_transition` for `window`'s role.
bool apply_transition(QWindow *window, const Transition &transition);

/// Opens a popup of `parent`'s shell role on `popup_window`.
///
/// `popup_window` must not be shown yet: the role is created here, positioned
/// and sized before the first commit, so the popup never appears at the wrong
/// place for one frame. Returns a positive id the events above refer to, or 0
/// with `*error` set.
quint32 popup_open(QWindow *popup_window, QWindow *parent, const PopupRequest &request,
                   QString *error);

/// Asks the compositor to close `popup_id`'s popup, exit transition included.
bool popup_dismiss(quint32 popup_id);

/// Destroys `popup_id`'s popup object and unmaps its window.
void popup_destroy(quint32 popup_id);

/// The window a popup id refers to, or null for an id that is gone.
QWindow *popup_window(quint32 popup_id);

/// Binds `inox_glass_manager_v1` and gives `window`'s surface a glass object.
///
/// Called after the shell role exists, because that is what makes the
/// compositor's answer meaningful: a surface with no known desktop layer is
/// answered with a zero maximum (ADR 0046 §6), and a client that asked earlier
/// would be told a maximum the compositor would not honour.
///
/// Returns false with a readable `error` when this compositor offers no
/// `inox-glass-v1`, when the window has no Wayland surface, or when binding
/// failed. The first is a **degradation, not a failure**: the panel draws plain
/// translucency and says so once (see `crate::frost`).
bool glass_create(QWindow *window, QString *error);

/// Sends one parameter set, in the protocol's order: radius, region, and — only
/// on a version 2 object and only when the material has a refraction band — the
/// corner radii and the band.
///
/// Returns false when `window` has no glass object.
bool glass_apply(QWindow *window, const GlassParams &params);

/// Destroys `window`'s glass object. Frosting this surface ends with the next
/// committed frame; the surface itself is not touched.
void glass_destroy(QWindow *window);

/// The version of `window`'s glass object: 0 for none, 1 or 2 otherwise.
///
/// The Rust side needs it for one decision only — whether `set_corner_radii`
/// and `set_refraction` may be sent at all — and reading it from the object
/// keeps that answer next to the bind rather than guessed from the manager's.
quint32 glass_version(QWindow *window);

} // namespace native
} // namespace inox
