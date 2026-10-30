#include "shell_surface.h"

#include <QtCore/QDebug>
#include <QtCore/QHash>
#include <QtCore/QPointer>
#include <QtCore/QSize>
#include <QtCore/QVersionNumber>
#include <QtGui/QGuiApplication>
#include <QtGui/QWindow>

// This is the one translation unit of the panel that touches `QtWaylandClient`'s
// private ABI. ADR 0046 §10 authorises exactly that and forbids it anywhere
// else, so:
//
//   - the gate below is a **compile-time error**, not a warning: below Qt 6.5
//     `QWaylandShellSurface` has no stable inheritable shape, and a build that
//     quietly produced a panel with the wrong role would be worse than no build;
//   - `ensure_factory()` repeats the check at **run time**, because a binary can
//     be built against one Qt and started against another (a distribution
//     upgrade behind a running package), and a private-vtable mismatch is not
//     something to discover as a crash inside `QWaylandShellSurface`;
//   - a failure is **fatal to the caller**, never a fallback to `xdg_toplevel`
//     or `wlr-layer-shell`: both place the panel where the session did not ask
//     (ADR 0046 §9).
#if QT_VERSION < QT_VERSION_CHECK(6, 5, 0)
#  error "inox-panel's native shell surface needs Qt >= 6.5 (ADR 0046 §10)"
#endif

#include <QtWaylandClient/qtwaylandclientglobal.h>
#include <QtWaylandClient/private/qwaylanddisplay_p.h>
#include <QtWaylandClient/private/qwaylandshellintegration_p.h>
#include <QtWaylandClient/private/qwaylandshellsurface_p.h>
#include <QtWaylandClient/private/qwaylandwindow_p.h>

#include <wayland-client.h>

#include "inox-shell-client.h"
#include "inox-glass-client.h"

namespace inox {
namespace native {

QPoint anchor_point(const QSize &size, quint32 anchor) {
    const int x = (anchor & AnchorLeft) ? 0 : (anchor & AnchorRight) ? size.width() : size.width() / 2;
    const int y = (anchor & AnchorTop) ? 0 : (anchor & AnchorBottom) ? size.height() : size.height() / 2;
    return QPoint(x, y);
}

namespace {

class InoxShellSurface;
class InoxShellPopup;

/// The event queue the Rust side drains, and the bookkeeping the protocol
/// objects need.
///
/// Static state, deliberately: one panel process owns one shell surface tree,
/// and the role objects are owned by their `QWindow`s, which outlive every
/// caller here. The alternative — a handle the caller must keep alive and
/// destroy in the right order — is the arrangement that leaves a `wl_surface`
/// with a shell role whose owner has been freed.
EventSink g_sink;

void publish(quint32 kind, qint32 a = 0, qint32 b = 0, qint32 c = 0, qint32 d = 0) {
    if (g_sink) {
        g_sink(kind, a, b, c, d);
    }
}

/// Everything decided about a shell surface before it is created.
///
/// The window is registered first and shown second, so placement, keyboard mode
/// and transition can all arrive before the `wl_surface` has a role. Holding
/// them here — and sending them at creation — is what makes the order the
/// caller uses irrelevant, and what keeps the first commit atomic.
struct RoleEntry {
    quint32 role = RolePanel;
    bool has_placement = false;
    Placement placement;
    bool has_keyboard = false;
    Keyboard keyboard;
    bool has_transition = false;
    Transition transition;
    struct inox_shell_surface_v1 *object = nullptr;
    InoxShellSurface *surface = nullptr;
};

QHash<QWindow *, RoleEntry> g_roles;

/// One popup and the window it draws into.
struct Popup {
    QPointer<QWindow> window;
    struct inox_shell_popup_v1 *object = nullptr;
    InoxShellPopup *surface = nullptr;
};

QHash<quint32, Popup> g_popups;
quint32 g_next_popup = 1;

/// Popups waiting for their window to be shown, keyed by that window.
struct PendingPopup {
    QPointer<QWindow> parent;
    PopupRequest request;
    quint32 id = 0;
};

QHash<QWindow *, PendingPopup> g_pending_popups;

/// The bound `inox_shell_v1`, once per process.
struct inox_shell_v1 *g_factory = nullptr;
bool g_bind_attempted = false;
QString g_bind_error;

/// Binds `inox_shell_v1`, or explains why this compositor cannot serve a shell
/// surface.
///
/// This is the runtime half of the version gate (ADR 0046 §10). It runs once:
/// the answer cannot change for the life of the process, and a second attempt
/// on a compositor that never advertises the global would only produce a second
/// identical error.
bool ensure_factory(QtWaylandClient::QWaylandDisplay *display, QString *error) {
    const QVersionNumber running = QVersionNumber::fromString(QString::fromLatin1(qVersion()));
    const QVersionNumber built = QVersionNumber::fromString(QString::fromLatin1(QT_VERSION_STR));

    if (running.majorVersion() != built.majorVersion() || running < built) {
        if (error) {
            *error = QStringLiteral("inox-panel was built against Qt %1 and is running against %2: the native "
                                    "shell surface uses QtWaylandClient's private ABI, which is only "
                                    "guaranteed forward within one major version (ADR 0046 §10)")
                         .arg(built.toString(), running.toString());
        }
        return false;
    }

    if (g_factory) {
        return true;
    }

    if (g_bind_attempted) {
        if (error) {
            *error = g_bind_error;
        }
        return false;
    }

    g_bind_attempted = true;

    if (!display) {
        g_bind_error = QStringLiteral("the Wayland display is not available");
        if (error) {
            *error = g_bind_error;
        }
        return false;
    }

    // The display's own global list, so no extra round trip is needed: by the
    // time a window has a platform window, Qt has received the registry dump.
    quint32 id = 0;
    quint32 version = 0;

    for (const auto &global : display->globals()) {
        if (global.interface == QLatin1String("inox_shell_v1")) {
            id = global.id;
            version = global.version;
            break;
        }
    }

    if (id == 0) {
        g_bind_error = QStringLiteral("this compositor does not offer inox_shell_v1, so the panel cannot be "
                                      "placed: Inox's own shell surfaces are not layer-shell ones "
                                      "(ADR 0046 §9)");
        if (error) {
            *error = g_bind_error;
        }
        return false;
    }

    struct wl_registry *registry = display->wl_registry();

    if (!registry) {
        g_bind_error = QStringLiteral("the Wayland registry is not available");
        if (error) {
            *error = g_bind_error;
        }
        return false;
    }

    g_factory = static_cast<struct inox_shell_v1 *>(
        wl_registry_bind(registry, id, &inox_shell_v1_interface, qMin<quint32>(version, 1)));

    if (!g_factory) {
        g_bind_error = QStringLiteral("binding inox_shell_v1 failed");
        if (error) {
            *error = g_bind_error;
        }
        return false;
    }

    return true;
}

void handle_capabilities(void *, struct inox_shell_v1 *, uint32_t capabilities,
                         uint32_t max_transition_duration) {
    publish(1, qint32(capabilities), qint32(max_transition_duration));
}

const struct inox_shell_v1_listener kFactoryListener = {
    handle_capabilities,
};

/// Commits the surface so the requests sent since the last commit take effect.
///
/// `inox-shell-v1` applies state on commit, exactly as layer-shell does. A shell
/// surface's first commit carries no buffer — it is what asks the compositor for
/// a configure — and every later one carries the frame Qt rendered.
void commit_surface(struct wl_surface *surface) {
    if (surface) {
        wl_surface_commit(surface);
    }
}

/// A shell surface: the panel's own role.
class InoxShellSurface final : public QtWaylandClient::QWaylandShellSurface {
public:
    InoxShellSurface(QtWaylandClient::QWaylandWindow *window, QWindow *qwindow, quint32 role)
        : QtWaylandClient::QWaylandShellSurface(window), m_window(qwindow) {
        struct wl_surface *surface = wlSurface();

        if (!surface || !g_factory) {
            return;
        }

        m_object = inox_shell_v1_get_surface(g_factory, surface, role);

        if (!m_object) {
            return;
        }

        inox_shell_surface_v1_add_listener(m_object, &kSurfaceListener, this);
    }

    ~InoxShellSurface() override {
        if (m_object) {
            inox_shell_surface_v1_destroy(m_object);
            m_object = nullptr;
        }
    }

    void applyConfigure() override {
        if (m_pending.isEmpty()) {
            return;
        }

        if (m_pending != window()->geometry().size()) {
            resizeFromApplyConfigure(m_pending);
        }

        m_configured = true;
        m_configured_size = m_pending;

        publish(2, m_pending.width(), m_pending.height());
    }

    // Exposed once the compositor has said how large the surface is: before
    // that there is nothing to show, and after it the surface is on screen
    // whether or not Qt has finished the resize.
    bool isExposed() const override { return m_configured || !m_pending.isEmpty(); }

    struct inox_shell_surface_v1 *object() const { return m_object; }

    void send_placement(const Placement &placement) {
        if (!m_object) {
            return;
        }

        inox_shell_surface_v1_set_placement(m_object, placement.layer, placement.anchor,
                                            placement.exclusive_zone, placement.margin_top,
                                            placement.margin_right, placement.margin_bottom,
                                            placement.margin_left, placement.width, placement.height);
    }

    void send_keyboard(const Keyboard &keyboard) {
        if (m_object) {
            inox_shell_surface_v1_set_keyboard(m_object, keyboard.mode);
        }
    }

    void send_transition(const Transition &transition) {
        if (m_object) {
            inox_shell_surface_v1_set_transition(m_object, transition.enter, transition.exit,
                                                 transition.duration, transition.curve,
                                                 transition.origin);
        }
    }

    void commit() { commit_surface(wlSurface()); }

private:
    static void handle_configure(void *data, struct inox_shell_surface_v1 *, uint32_t,
                                 uint32_t width, uint32_t height) {
        auto *self = static_cast<InoxShellSurface *>(data);

        if (width == 0 || height == 0) {
            return;
        }

        self->m_pending = QSize(qint32(width), qint32(height));
        self->applyConfigureWhenPossible();
    }

    static void handle_workarea(void *, struct inox_shell_surface_v1 *, int32_t x, int32_t y,
                                int32_t width, int32_t height) {
        publish(3, x, y, width, height);
    }

    static void handle_keyboard_state(void *, struct inox_shell_surface_v1 *, uint32_t focused) {
        publish(4, focused ? 1 : 0);
    }

    static void handle_closed(void *, struct inox_shell_surface_v1 *) { publish(5); }

    static const struct inox_shell_surface_v1_listener kSurfaceListener;

    QPointer<QWindow> m_window;
    struct inox_shell_surface_v1 *m_object = nullptr;
    QSize m_pending;
    QSize m_configured_size;
    bool m_configured = false;
};

const struct inox_shell_surface_v1_listener InoxShellSurface::kSurfaceListener = {
    InoxShellSurface::handle_configure,
    InoxShellSurface::handle_workarea,
    InoxShellSurface::handle_keyboard_state,
    InoxShellSurface::handle_closed,
};

/// A popup: a menu, a tooltip, a nested submenu.
class InoxShellPopup final : public QtWaylandClient::QWaylandShellSurface {
public:
    InoxShellPopup(QtWaylandClient::QWaylandWindow *window, QWindow *qwindow, quint32 id,
                   struct inox_shell_popup_v1 *object)
        : QtWaylandClient::QWaylandShellSurface(window), m_window(qwindow), m_id(id), m_object(object) {
        if (m_object) {
            inox_shell_popup_v1_add_listener(m_object, &kPopupListener, this);
        }
    }

    ~InoxShellPopup() override {
        if (m_object) {
            inox_shell_popup_v1_destroy(m_object);
            m_object = nullptr;
        }
    }

    void applyConfigure() override {
        if (m_pending.isEmpty()) {
            return;
        }

        if (m_pending != window()->geometry().size()) {
            resizeFromApplyConfigure(m_pending);
        }

        m_configured = true;
        publish(7, qint32(m_id), m_pending.width(), m_pending.height());
    }

    bool isExposed() const override { return m_configured; }

    void commit() { commit_surface(wlSurface()); }

private:
    static void handle_configure(void *data, struct inox_shell_popup_v1 *, uint32_t, uint32_t width,
                                 uint32_t height) {
        auto *self = static_cast<InoxShellPopup *>(data);

        if (width == 0 || height == 0) {
            return;
        }

        self->m_pending = QSize(qint32(width), qint32(height));
        self->applyConfigureWhenPossible();
    }

    static void handle_dismissed(void *data, struct inox_shell_popup_v1 *, uint32_t reason) {
        auto *self = static_cast<InoxShellPopup *>(data);

        // The compositor has taken the popup away: a click outside, Escape, the
        // parent unmapping, or its own policy. The window stops being shown
        // here rather than in the caller, because the compositor is drawing the
        // exit transition and a client that kept drawing would paint over it.
        if (QWindow *window = self->m_window) {
            window->hide();
        }

        publish(6, qint32(self->m_id), qint32(reason));
    }

    static void handle_keyboard_state(void *data, struct inox_shell_popup_v1 *, uint32_t focused) {
        auto *self = static_cast<InoxShellPopup *>(data);
        publish(8, qint32(self->m_id), focused ? 1 : 0);
    }

    static const struct inox_shell_popup_v1_listener kPopupListener;

    QPointer<QWindow> m_window;
    quint32 m_id = 0;
    struct inox_shell_popup_v1 *m_object = nullptr;
    QSize m_pending;
    bool m_configured = false;
};

const struct inox_shell_popup_v1_listener InoxShellPopup::kPopupListener = {
    InoxShellPopup::handle_configure,
    InoxShellPopup::handle_dismissed,
    InoxShellPopup::handle_keyboard_state,
};

/// The one integration every window of this process uses.
///
/// Qt calls it when a window is first shown (or when the integration is
/// installed on a window that already has a `wl_surface`), so a window whose
/// role is decided in advance — every one of the panel's — never receives an
/// `xdg_toplevel` role it would then have to be talked out of. A `wl_surface`
/// has exactly one role; creating the wrong one first is a protocol error, not a
/// mistake that can be corrected afterwards.
class InoxShellIntegration final : public QtWaylandClient::QWaylandShellIntegration {
public:
    bool initialize(QtWaylandClient::QWaylandDisplay *display) override {
        m_display = display;

        QString error;

        if (!ensure_factory(display, &error)) {
            m_error = error;
            return false;
        }

        inox_shell_v1_add_listener(g_factory, &kFactoryListener, nullptr);

        return true;
    }

    QtWaylandClient::QWaylandShellSurface *createShellSurface(
        QtWaylandClient::QWaylandWindow *window) override {
        QWindow *qwindow = window->window();

        if (!qwindow) {
            return nullptr;
        }

        if (!m_display) {
            initialize(window->display());
        }

        // A window registered as a popup gets the popup role; a window
        // registered as a shell surface gets that role. Everything else is a
        // programming error in this crate: no other kind of window exists in a
        // panel process.
        const auto pending = g_pending_popups.constFind(qwindow);

        if (pending != g_pending_popups.constEnd()) {
            return create_popup(window, qwindow, pending.value());
        }

        auto role = g_roles.find(qwindow);

        if (role == g_roles.end()) {
            qWarning("inox-panel: a window was shown without a shell role; it will not be mapped");

            return nullptr;
        }

        auto *surface = new InoxShellSurface(window, qwindow, role.value().role);

        if (!surface->object()) {
            delete surface;

            return nullptr;
        }

        role.value().surface = surface;
        role.value().object = surface->object();

        // Everything decided before the window was shown reaches the compositor
        // now, on one commit, so the first configure already describes the final
        // placement.
        if (role.value().has_placement) {
            surface->send_placement(role.value().placement);
        }

        if (role.value().has_keyboard) {
            surface->send_keyboard(role.value().keyboard);
        }

        if (role.value().has_transition) {
            surface->send_transition(role.value().transition);
        }

        surface->commit();

        return surface;
    }

    const QString &error() const { return m_error; }

private:
    static QtWaylandClient::QWaylandShellSurface *create_popup(
        QtWaylandClient::QWaylandWindow *window, QWindow *qwindow, const PendingPopup &pending) {
        QWindow *parent = pending.parent;

        if (!parent) {
            return nullptr;
        }

        const auto parent_role = g_roles.constFind(parent);

        if (parent_role == g_roles.constEnd() || !parent_role.value().object) {
            qWarning("inox-panel: a popup's parent has no shell surface yet; the popup is not mapped");

            return nullptr;
        }

        struct wl_surface *surface = window->wlSurface();

        if (!surface) {
            return nullptr;
        }

        const PopupRequest &request = pending.request;
        struct inox_shell_popup_v1 *object =
            inox_shell_surface_v1_get_popup(parent_role.value().object, surface);

        if (!object) {
            return nullptr;
        }

        inox_shell_popup_v1_set_position(object, request.parent_anchor, request.popup_anchor,
                                         request.x, request.y);
        inox_shell_popup_v1_set_size(object, request.width, request.height);
        inox_shell_popup_v1_set_keyboard(object, request.keyboard ? 1 : 0);
        inox_shell_popup_v1_set_dismissable(object, request.dismissable ? 1 : 0);
        inox_shell_popup_v1_set_transition(object, request.transition.enter, request.transition.exit,
                                           request.transition.duration, request.transition.curve);

        auto *popup = new InoxShellPopup(window, qwindow, pending.id, object);

        Popup &record = g_popups[pending.id];
        record.window = qwindow;
        record.object = object;
        record.surface = popup;

        popup->commit();

        return popup;
    }

    QtWaylandClient::QWaylandDisplay *m_display = nullptr;
    QString m_error;
};

/// The integration, created once and installed on every window of this process.
InoxShellIntegration *integration() {
    static InoxShellIntegration *instance = nullptr;

    if (!instance) {
        instance = new InoxShellIntegration;
    }

    return instance;
}

} // namespace

void set_event_sink(EventSink sink) { g_sink = std::move(sink); }

bool shell_available(QString *error) {
    if (QGuiApplication::platformName() != QLatin1String("wayland")) {
        if (error) {
            *error = QStringLiteral("the Qt platform is \"%1\", not \"wayland\": a shell role is a Wayland role "
                                    "(ADR 0046 §1)")
                         .arg(QGuiApplication::platformName());
        }

        return false;
    }

    if (g_bind_attempted && !g_factory) {
        if (error) {
            *error = g_bind_error;
        }

        return false;
    }

    return true;
}

bool install_role(QWindow *window, quint32 role, QString *error) {
    if (!window) {
        if (error) {
            *error = QStringLiteral("install_role was given no window");
        }

        return false;
    }

    if (QGuiApplication::platformName() != QLatin1String("wayland")) {
        if (error) {
            *error = QStringLiteral("inox-panel needs the Qt \"wayland\" platform to have a shell role; this "
                                    "process is on \"%1\"")
                         .arg(QGuiApplication::platformName());
        }

        return false;
    }

    if (window->isVisible()) {
        if (error) {
            *error = QStringLiteral("the window is already shown, so it already has an xdg role: the shell role "
                                    "has to be installed before the first show (ADR 0046 §1)");
        }

        return false;
    }

    // Forcing the platform window here is what makes the role installable: the
    // integration can only take effect before the window's first commit, and Qt
    // creates the platform window lazily.
    window->winId();

    auto *wayland_window = dynamic_cast<QtWaylandClient::QWaylandWindow *>(window->handle());

    if (!wayland_window) {
        if (error) {
            *error = QStringLiteral("the window has no Wayland platform window");
        }

        return false;
    }

    if (!ensure_factory(wayland_window->display(), error)) {
        return false;
    }

    if (!g_roles.contains(window)) {
        RoleEntry entry;
        entry.role = role;
        g_roles.insert(window, entry);
    }

    wayland_window->setShellIntegration(integration());

    return true;
}

bool apply_placement(QWindow *window, const Placement &placement) {
    auto role = g_roles.find(window);

    if (role == g_roles.end()) {
        return false;
    }

    role.value().has_placement = true;
    role.value().placement = placement;

    if (role.value().surface) {
        role.value().surface->send_placement(placement);
        role.value().surface->commit();
    }

    return true;
}

bool apply_keyboard(QWindow *window, const Keyboard &keyboard) {
    auto role = g_roles.find(window);

    if (role == g_roles.end()) {
        return false;
    }

    role.value().has_keyboard = true;
    role.value().keyboard = keyboard;

    if (role.value().surface) {
        role.value().surface->send_keyboard(keyboard);
        role.value().surface->commit();
    }

    return true;
}

bool apply_transition(QWindow *window, const Transition &transition) {
    auto role = g_roles.find(window);

    if (role == g_roles.end()) {
        return false;
    }

    role.value().has_transition = true;
    role.value().transition = transition;

    if (role.value().surface) {
        role.value().surface->send_transition(transition);
        role.value().surface->commit();
    }

    return true;
}

quint32 popup_open(QWindow *popup_window, QWindow *parent, const PopupRequest &request,
                   QString *error) {
    if (!popup_window || !parent) {
        if (error) {
            *error = QStringLiteral("a popup needs both its own window and its parent's");
        }

        return 0;
    }

    if (!g_roles.contains(parent)) {
        if (error) {
            *error = QStringLiteral("the popup's parent has no shell role");
        }

        return 0;
    }

    const quint32 id = g_next_popup++;
    g_pending_popups.insert(popup_window, PendingPopup{parent, request, id});

    // The role itself is created when the window is shown — see
    // `InoxShellIntegration::createShellSurface` — so the position and the size
    // reach the compositor before the popup's first commit.
    popup_window->winId();

    auto *wayland_window = dynamic_cast<QtWaylandClient::QWaylandWindow *>(popup_window->handle());

    if (!wayland_window) {
        g_pending_popups.remove(popup_window);

        if (error) {
            *error = QStringLiteral("the popup's window has no Wayland platform window");
        }

        return 0;
    }

    wayland_window->setShellIntegration(integration());

    return id;
}

bool popup_dismiss(quint32 popup_id) {
    const auto popup = g_popups.find(popup_id);

    if (popup == g_popups.end() || !popup.value().object) {
        return false;
    }

    inox_shell_popup_v1_dismiss(popup.value().object);

    return true;
}

void popup_destroy(quint32 popup_id) {
    const auto popup = g_popups.find(popup_id);

    if (popup != g_popups.end()) {
        if (QWindow *window = popup.value().window) {
            g_pending_popups.remove(window);
        }

        g_popups.erase(popup);
    }
}

QWindow *popup_window(quint32 popup_id) {
    const auto popup = g_popups.constFind(popup_id);

    if (popup == g_popups.constEnd()) {
        return nullptr;
    }

    return popup.value().window;
}

// -- liquid glass ------------------------------------------------------------
//
// The glass object has to live on the **same** Wayland connection as the
// surface it describes: `inox_glass_manager_v1.get_glass` takes a `wl_surface`,
// and a proxy belongs to the connection that created it. Qt owns that
// connection, so the object is created here — in the one translation unit
// allowed to touch QtWaylandClient's private display — and the decision about
// *what* to ask for stays in Rust (`crate::frost` over `inox_glass::decide`).
// This file only turns the decision into requests, and the compositor's answer
// into the two events above.
//
// Nothing here reads back: the protocol carries no pixels and no file
// descriptors, so this is not a privileged interface (ADR 0046 §2).

/// The newest `inox-glass-v1` this build understands. Version 2 adds the
/// refraction vocabulary; a newer compositor is used at this version, because a
/// client that asked for more would have to guess what the extra version means.
constexpr quint32 kGlassVersion = 2;

namespace {

/// The bound manager, and the version it was bound at.
///
/// Bound once per process, like `inox_shell_v1`: the registry is not going to
/// grow a second manager, and a second `wl_registry_bind` of the same global
/// would create a second proxy nothing gains from.
struct inox_glass_manager_v1 *g_glass_manager = nullptr;
quint32 g_glass_manager_version = 0;
bool g_glass_attempted = false;
QString g_glass_error;

/// One surface's glass object.
///
/// Keyed by window rather than stored in `RoleEntry` because a window may gain a
/// glass object after it already has a role, and because the object outlives no
/// window: it is destroyed with the display at process exit, which is the only
/// teardown a one-surface panel has.
struct GlassObject {
    struct inox_glass_v1 *object = nullptr;
    quint32 version = 1;
};

QHash<QWindow *, GlassObject> g_glass;

void handle_glass_capabilities(void *, struct inox_glass_v1 *, uint32_t max_radius) {
    publish(9, qint32(max_radius));
}

void handle_glass_liquid_capabilities(void *, struct inox_glass_v1 *, uint32_t max_radius,
                                      uint32_t max_refraction_height,
                                      uint32_t max_refraction_amount, uint32_t flags) {
    publish(10, qint32(max_radius), qint32(max_refraction_height), qint32(max_refraction_amount),
            qint32(flags));
}

const struct inox_glass_v1_listener kGlassListener = {
    handle_glass_capabilities,
    handle_glass_liquid_capabilities,
};

/// Sends everything queued on this connection.
///
/// Qt flushes on its own commits, and a glass parameter set is sent between two
/// of them; without this the compositor would not hear about the effect until
/// the next frame Qt happened to flush, which for a quiescent panel is never.
/// `flushRequests()` is Qt's own flush path, so this never races its event
/// thread's reads with a second writer.
void flush_display(QtWaylandClient::QWaylandDisplay *display) {
    if (display) {
        display->flushRequests();
    }
}

} // namespace

bool glass_create(QWindow *window, QString *error) {
    if (!window) {
        if (error) {
            *error = QStringLiteral("glass_create was given no window");
        }

        return false;
    }

    if (g_glass.contains(window)) {
        return true;
    }

    auto *wayland_window = dynamic_cast<QtWaylandClient::QWaylandWindow *>(window->handle());

    if (!wayland_window) {
        if (error) {
            *error = QStringLiteral("the window has no Wayland platform window, so it has no "
                                    "surface to frost");
        }

        return false;
    }

    QtWaylandClient::QWaylandDisplay *display = wayland_window->display();

    if (!display) {
        if (error) {
            *error = QStringLiteral("the Wayland display is not available");
        }

        return false;
    }

    if (!g_glass_manager) {
        if (g_glass_attempted) {
            if (error) {
                *error = g_glass_error;
            }

            return false;
        }

        g_glass_attempted = true;

        quint32 id = 0;
        quint32 version = 0;

        for (const auto &global : display->globals()) {
            if (global.interface == QLatin1String("inox_glass_manager_v1")) {
                id = global.id;
                version = global.version;
                break;
            }
        }

        if (id == 0) {
            g_glass_error = QStringLiteral("this compositor does not offer inox-glass-v1, so the pane is "
                                           "drawn as plain translucency");

            if (error) {
                *error = g_glass_error;
            }

            return false;
        }

        struct wl_registry *registry = display->wl_registry();

        if (!registry) {
            g_glass_error = QStringLiteral("the Wayland registry is not available");

            if (error) {
                *error = g_glass_error;
            }

            return false;
        }

        g_glass_manager_version = qMin<quint32>(version, kGlassVersion);
        g_glass_manager = static_cast<struct inox_glass_manager_v1 *>(
            wl_registry_bind(registry, id, &inox_glass_manager_v1_interface, g_glass_manager_version));

        if (!g_glass_manager) {
            g_glass_error = QStringLiteral("binding inox_glass_manager_v1 failed");

            if (error) {
                *error = g_glass_error;
            }

            return false;
        }
    }

    struct wl_surface *surface = wayland_window->wlSurface();

    if (!surface) {
        if (error) {
            *error = QStringLiteral("the window's Wayland surface does not exist yet, so it cannot "
                                    "be frosted");
        }

        return false;
    }

    GlassObject glass;
    glass.version = g_glass_manager_version;
    glass.object = inox_glass_manager_v1_get_glass(g_glass_manager, surface);

    if (!glass.object) {
        if (error) {
            *error = QStringLiteral("inox_glass_manager_v1.get_glass failed");
        }

        return false;
    }

    inox_glass_v1_add_listener(glass.object, &kGlassListener, nullptr);

    g_glass.insert(window, glass);
    flush_display(display);

    return true;
}

bool glass_apply(QWindow *window, const GlassParams &params) {
    const auto glass = g_glass.find(window);

    if (glass == g_glass.end() || !glass.value().object) {
        return false;
    }

    struct inox_glass_v1 *object = glass.value().object;

    inox_glass_v1_set_radius(object, params.radius);
    inox_glass_v1_set_region(object, params.x, params.y, params.width, params.height);

    // The protocol says `set_corner_radii` "only changes the shape of the
    // refraction", so it travels with the band and never alone. A version 1
    // object has neither request: sending one would be an unknown request, which
    // is a protocol error and would kill the connection.
    if (glass.value().version >= 2 && params.refraction) {
        inox_glass_v1_set_corner_radii(object, params.top_left, params.top_right, params.bottom_right,
                                       params.bottom_left);
        inox_glass_v1_set_refraction(object, params.band_height, params.band_amount, params.flags);
    }

    auto *wayland_window = dynamic_cast<QtWaylandClient::QWaylandWindow *>(window->handle());

    if (wayland_window) {
        flush_display(wayland_window->display());
    }

    return true;
}

void glass_destroy(QWindow *window) {
    const auto glass = g_glass.find(window);

    if (glass == g_glass.end()) {
        return;
    }

    if (glass.value().object) {
        inox_glass_v1_destroy(glass.value().object);
    }

    g_glass.erase(glass);
}

quint32 glass_version(QWindow *window) {
    const auto glass = g_glass.constFind(window);

    if (glass == g_glass.constEnd()) {
        return 0;
    }

    return glass.value().version;
}

} // namespace native
} // namespace inox
