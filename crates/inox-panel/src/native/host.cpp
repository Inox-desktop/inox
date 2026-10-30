#include "host.h"

#include <QtCore/QCoreApplication>
#include <QtCore/QEventLoop>
#include <QtCore/QHash>
#include <QtCore/QPointer>
#include <QtCore/QObject>
#include <QtCore/QQueue>
#include <QtCore/QVariant>
#include <QtCore/QVariantList>
#include <QtGui/QGuiApplication>
#include <QtGui/QSurfaceFormat>
#include <QtGui/QWindow>
#include <QtQml/QQmlApplicationEngine>
#include <QtQml/QQmlContext>
#include <QtQml/QQmlEngine>
#include <QtQuick/QQuickWindow>

#include <memory>

// The native host: one application, one window, one shell role. See `host.h`
// for why the event loop stays outside Qt and what crosses the boundary.
//
// Four details are load-bearing:
//
//   1. The platform is **not** forced. `QT_QPA_PLATFORM` is the operator's (or
//      the session's) business; this host needs the real `wayland` one and says
//      so with a readable error if it is on anything else. The offscreen host in
//      `offscreen.cpp` is the one that pins `offscreen`, and the two never run in
//      the same binary.
//   2. The window is loaded but **not shown** until the shell role is installed
//      (`inoxHostNative` in `Main.qml` binds `visible` to it). A shown window has
//      an `xdg_toplevel` role already, and a `wl_surface` has exactly one.
//   3. `QQuickWindow::setDefaultAlphaBuffer(true)` before the first window: every
//      panel surface is structural transparency with a translucent pane on it, so
//      the alpha is content and not decoration. On the offscreen backend this
//      needed a two-pass reconstruction; on Wayland the buffer carries it.
//   4. The QML root's actions arrive through the `dispatched(string)` signal on
//      the root object rather than a context property. A context property would
//      have to be a `QObject`, which means `moc` and a meta-object in the one
//      translation unit that is supposed to be a thin host.

namespace {

/// One queued shell event, as four integers and a kind (see `shell_surface.h`).
struct Event {
  quint32 kind = 0;
  qint32 a = 0;
  qint32 b = 0;
  qint32 c = 0;
  qint32 d = 0;
};

/// The one `QGuiApplication` of the process.
///
/// Qt allows no second one, and a panel that links both hosts (as it does until
/// the offscreen one is deleted, ADR 0046 P5) must not try. The platform is
/// deliberately left alone: forcing `wayland` would override an operator's
/// `QT_QPA_PLATFORM`, and the failure is reported where it can be explained.
QGuiApplication *ensure_app() {
  static QGuiApplication *app = nullptr;

  if (!app) {
    // Before any window exists: a window's surface carries an alpha channel only
    // when it was asked for before it was created.
    QQuickWindow::setDefaultAlphaBuffer(true);

    QSurfaceFormat format = QSurfaceFormat::defaultFormat();
    format.setAlphaBufferSize(8);
    QSurfaceFormat::setDefaultFormat(format);

    static int argc = 1;
    static char name[] = "inox-panel";
    static char *argv[] = {name, nullptr};

    app = new QGuiApplication(argc, argv);
  }

  return app;
}

} // namespace

struct Host::Impl {
  std::unique_ptr<QQmlApplicationEngine> engine;
  QObject *context = nullptr;
  QWindow *window = nullptr;
  QHash<quint32, QPointer<QWindow>> popups;
  QQueue<Event> events;
  QQueue<QByteArray> actions;
  QSize size;
  bool configured = false;
  QString error;
};

Host::Host() : d(new Impl) {
  (void)ensure_app();

  d->context = new QObject;

  inox::native::set_event_sink([this](quint32 kind, qint32 a, qint32 b, qint32 c, qint32 d_) {
    // The configured size is the panel's whole geometry, so it is kept here as
    // well as queued: the projection needs it before the next event is drained.
    if (kind == 2) {
      d->size = QSize(a, b);
      d->configured = true;
    }

    Event event;
    event.kind = kind;
    event.a = a;
    event.b = b;
    event.c = c;
    event.d = d_;
    d->events.enqueue(event);
  });
}

Host::~Host() {
  // Popups first: each is its own window with its own role, and a popup that
  // outlived its parent's surface would be a `wl_surface` the compositor holds
  // for a client that is leaving.
  for (auto &popup : d->popups) {
    delete popup.data();
  }

  d->popups.clear();

  d->engine.reset();

  inox::native::set_event_sink(nullptr);
  delete d->context;
  delete d;
}

bool Host::start(const QString &url, quint32 role, QString &error) {
  if (d->window) {
    return true;
  }

  d->engine = std::make_unique<QQmlApplicationEngine>();

  // Read by `Main.qml` as `inoxHostNative`. Set before the document is loaded,
  // because `visible` is bound while the component is completed.
  d->engine->rootContext()->setContextProperty(QStringLiteral("inoxHostNative"), QVariant(true));

  d->engine->load(QUrl(url));

  const QList<QObject *> roots = d->engine->rootObjects();

  if (roots.isEmpty()) {
    d->error = QStringLiteral("the QML engine produced no root object for %1 (a missing module, a syntax "
                              "error or a missing QML import)")
                   .arg(url);
    error = d->error;
    return false;
  }

  auto *window = qobject_cast<QWindow *>(roots.first());

  if (!window) {
    d->error = QStringLiteral("%1 is not a window, so it has no surface to place").arg(url);
    error = d->error;
    return false;
  }

  d->window = window;

  QString role_error;

  if (!inox::native::install_role(window, role, &role_error)) {
    d->error = role_error;
    error = d->error;
    return false;
  }

  window->show();

  return true;
}

bool Host::set_model(const QString &json, QString &error) {
  if (!d->window) {
    d->error = QStringLiteral("the window is not started");
    error = d->error;
    return false;
  }

  d->window->setProperty("modelJson", json);

  return true;
}

bool Host::place(quint32 layer, quint32 anchor, qint32 exclusive_zone, qint32 margin_top,
                 qint32 margin_right, qint32 margin_bottom, qint32 margin_left, quint32 width,
                 quint32 height, QString &error) {
  inox::native::Placement placement;
  placement.layer = layer;
  placement.anchor = anchor;
  placement.exclusive_zone = exclusive_zone;
  placement.margin_top = margin_top;
  placement.margin_right = margin_right;
  placement.margin_bottom = margin_bottom;
  placement.margin_left = margin_left;
  placement.width = width;
  placement.height = height;

  if (!inox::native::apply_placement(d->window, placement)) {
    d->error = QStringLiteral("the window has no shell role to place");
    error = d->error;
    return false;
  }

  return true;
}

bool Host::transition(quint32 enter, quint32 exit, quint32 duration, quint32 curve, quint32 origin,
                      QString &error) {
  inox::native::Transition transition_value;
  transition_value.enter = enter;
  transition_value.exit = exit;
  transition_value.duration = duration;
  transition_value.curve = curve;
  transition_value.origin = origin;

  if (!inox::native::apply_transition(d->window, transition_value)) {
    d->error = QStringLiteral("the window has no shell role to animate");
    error = d->error;
    return false;
  }

  return true;
}

bool Host::keyboard(quint32 mode, QString &error) {
  inox::native::Keyboard keyboard_value;
  keyboard_value.mode = mode;

  if (!inox::native::apply_keyboard(d->window, keyboard_value)) {
    d->error = QStringLiteral("the window has no shell role to give the keyboard to");
    error = d->error;
    return false;
  }

  return true;
}

void Host::pump() {
  QCoreApplication::processEvents(QEventLoop::AllEvents, 8);
  collect_actions();
}

quint32 Host::open_popup(quint32 parent_anchor, quint32 popup_anchor, qint32 x, qint32 y,
                         quint32 width, quint32 height, bool dismissable, QString &error) {
  if (!d->window) {
    error = QStringLiteral("the surface is not started, so it has no popups");
    return 0;
  }

  // One window per popup, because a popup is one `wl_surface`: Qt creates the
  // wayland surface for a window when it is shown, and the role is installed on
  // it before that (see `popup_open`).
  auto *popup = new QQuickWindow();
  popup->setColor(Qt::transparent);
  popup->resize(qint32(width), qint32(height));

  inox::native::PopupRequest request;
  request.parent_anchor = parent_anchor;
  request.popup_anchor = popup_anchor;
  request.x = x;
  request.y = y;
  request.width = width;
  request.height = height;
  request.keyboard = false;
  request.dismissable = dismissable;

  QString open_error;
  const quint32 id = inox::native::popup_open(popup, d->window, request, &open_error);

  if (id == 0) {
    delete popup;
    error = open_error;
    return 0;
  }

  d->popups.insert(id, popup);
  popup->show();

  return id;
}

bool Host::dismiss_popup(quint32 popup) { return inox::native::popup_dismiss(popup); }

bool Host::start_glass(QString &error) {
  if (!d->window) {
    d->error = QStringLiteral("the window is not started, so it has no surface to frost");
    error = d->error;
    return false;
  }

  if (!inox::native::glass_create(d->window, &error)) {
    d->error = error;
    return false;
  }

  return true;
}

bool Host::glass(quint32 radius, qint32 x, qint32 y, qint32 width, qint32 height, quint32 top_left,
                 quint32 top_right, quint32 bottom_right, quint32 bottom_left, bool refraction,
                 quint32 band_height, quint32 band_amount, quint32 flags, QString &error) {
  if (!d->window) {
    d->error = QStringLiteral("the window is not started, so it has no surface to frost");
    error = d->error;
    return false;
  }

  inox::native::GlassParams params;
  params.radius = radius;
  params.x = x;
  params.y = y;
  params.width = width;
  params.height = height;
  params.top_left = top_left;
  params.top_right = top_right;
  params.bottom_right = bottom_right;
  params.bottom_left = bottom_left;
  params.refraction = refraction;
  params.band_height = band_height;
  params.band_amount = band_amount;
  params.flags = flags;

  if (!inox::native::glass_apply(d->window, params)) {
    d->error = QStringLiteral("the surface has no glass object to set parameters on");
    error = d->error;
    return false;
  }

  return true;
}

quint32 Host::glass_version() const {
  return d->window ? inox::native::glass_version(d->window) : 0;
}

void Host::hide() {
  if (d->window) {
    d->window->hide();
  }
}

void Host::collect_actions() {
  if (!d->window) {
    return;
  }

  // QML appends to `outbox` and this empties it. A property rather than a
  // signal because a signal would need a `Q_OBJECT` receiver — `moc` in this
  // translation unit — and a queue is what the caller wants anyway: actions
  // produced while the loop was busy must all arrive, in order, not just the
  // last one.
  const QVariant value = d->window->property("outbox");

  if (!value.isValid()) {
    return;
  }

  const QVariantList pending = value.toList();

  if (pending.isEmpty()) {
    return;
  }

  for (const QVariant &action : pending) {
    d->actions.enqueue(action.toString().toUtf8());
  }

  d->window->setProperty("outbox", QVariantList());
}

quint32 Host::take_event(qint32 &a, qint32 &b, qint32 &c, qint32 &d_out) {
  if (d->events.isEmpty()) {
    return 0;
  }

  const Event event = d->events.dequeue();

  a = event.a;
  b = event.b;
  c = event.c;
  d_out = event.d;

  return event.kind;
}

QByteArray Host::take_action() {
  if (d->actions.isEmpty()) {
    return QByteArray();
  }

  return d->actions.dequeue();
}

QSize Host::size() const { return d->size; }

qint32 Host::width() const { return d->size.width(); }

qint32 Host::height() const { return d->size.height(); }

bool Host::configured() const { return d->configured; }

QString Host::last_error() const { return d->error; }

QWindow *Host::window() const { return d->window; }

std::unique_ptr<Host> new_host() { return std::make_unique<Host>(); }
